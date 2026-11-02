//! Installed outline-font discovery. Scans on a worker, retains names, not font bytes.

use ab_glyph::Font as _;
use resvg::usvg::fontdb;
use std::{
    collections::BTreeMap,
    io::Read as _,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

pub(super) const MAX_FONT_BYTES: u64 = 64 * 1024 * 1024;
const PREFIX: &str = "system-font-v1:";

#[derive(Debug, Clone)]
pub(super) struct FontChoice {
    pub id: String,
    pub label: String,
    pub search: String,
    pub path: PathBuf,
    pub index: u32,
    pub families: Vec<String>,
    pub style: fontdb::Style,
    pub weight: u16,
    pub stretch: fontdb::Stretch,
    pub has_latin: bool,
    pub has_chinese: bool,
}

#[derive(Debug, Default)]
pub(super) struct FontCatalog {
    pub choices: Vec<FontChoice>,
    pub skipped_files: usize,
}

#[derive(Clone, Default)]
pub(super) struct Snapshot {
    pub catalog: Arc<FontCatalog>,
    pub scanning: bool,
    pub error: Option<String>,
}

#[derive(Default)]
struct Loader {
    snapshot: Snapshot,
    started: bool,
}

fn loader() -> &'static Arc<Mutex<Loader>> {
    static LOADER: OnceLock<Arc<Mutex<Loader>>> = OnceLock::new();
    LOADER.get_or_init(|| Arc::new(Mutex::new(Loader::default())))
}

pub(super) fn catalog(ctx: &egui::Context, refresh: bool) -> Snapshot {
    let shared = loader();
    let mut state = shared.lock().unwrap_or_else(|e| e.into_inner());
    if (!state.started || refresh) && !state.snapshot.scanning {
        state.started = true;
        state.snapshot.scanning = true;
        state.snapshot.error = None;
        let shared = Arc::clone(shared);
        let ctx = ctx.clone();
        if let Err(error) = std::thread::Builder::new()
            .name("popchat-font-catalog".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(discover);
                let mut state = shared.lock().unwrap_or_else(|e| e.into_inner());
                state.snapshot.scanning = false;
                match result {
                    Ok(catalog) => {
                        state.snapshot.error = catalog
                            .choices
                            .is_empty()
                            .then(|| "沒有找到可讀取的字體，請重新掃描。".into());
                        state.snapshot.catalog = Arc::new(catalog);
                    }
                    Err(_) => state.snapshot.error = Some("字體掃描未完成，請重新掃描。".into()),
                }
                drop(state);
                ctx.request_repaint();
            })
        {
            state.snapshot.scanning = false;
            state.snapshot.error = Some(format!("無法啟動字體掃描：{error}"));
        }
    }
    state.snapshot.clone()
}

pub(super) fn fonts_dir() -> PathBuf {
    Path::new(&std::env::var_os("WINDIR").unwrap_or_else(|| r"C:\Windows".into())).join("Fonts")
}

fn roots() -> Vec<PathBuf> {
    let mut paths = vec![fonts_dir()];
    for (variable, suffix) in [
        ("LOCALAPPDATA", r"Microsoft\Windows\Fonts"),
        ("APPDATA", r"Microsoft\Windows\Fonts"),
        ("USERPROFILE", r"AppData\Local\Microsoft\Windows\Fonts"),
        ("USERPROFILE", r"AppData\Roaming\Microsoft\Windows\Fonts"),
    ] {
        if let Some(base) = std::env::var_os(variable) {
            paths.push(PathBuf::from(base).join(suffix));
        }
    }
    paths
}

fn key(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

fn font_extension(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        matches!(
            e.to_ascii_lowercase().as_str(),
            "ttf" | "otf" | "ttc" | "otc"
        )
    })
}

fn add_file(paths: &mut BTreeMap<String, PathBuf>, path: PathBuf) {
    if !font_extension(&path) {
        return;
    }
    if !local_path(&path) {
        return;
    }
    if let Ok(path) = path.canonicalize() {
        if local_path(&path) && path.is_file() {
            paths.entry(key(&path)).or_insert(path);
        }
    }
}

fn collect_roots(roots: &[PathBuf]) -> BTreeMap<String, PathBuf> {
    let mut paths = BTreeMap::new();
    let mut pending: Vec<_> = roots.iter().map(|p| (p.clone(), 0)).collect();
    let mut visited = std::collections::BTreeSet::new();
    while let Some((directory, depth)) = pending.pop() {
        if !local_path(&directory) {
            continue;
        }
        let Ok(canonical) = directory.canonicalize() else {
            continue;
        };
        if !local_path(&canonical) {
            continue;
        }
        if !visited.insert(key(&canonical)) {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() && !kind.is_symlink() && depth < 8 {
                pending.push((entry.path(), depth + 1));
            } else if kind.is_file() {
                add_file(&mut paths, entry.path());
            }
        }
    }
    paths
}

#[cfg(windows)]
fn registered_files_matching(matches: impl Fn(&str) -> bool) -> BTreeMap<String, PathBuf> {
    use winreg::{enums::*, RegKey};
    let mut paths = BTreeMap::new();
    for hive in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        for view in [KEY_WOW64_64KEY, KEY_WOW64_32KEY] {
            let Ok(registry) = RegKey::predef(hive).open_subkey_with_flags(
                r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Fonts",
                KEY_READ | view,
            ) else {
                continue;
            };
            for (name, _) in registry.enum_values().flatten() {
                if !matches(&name) {
                    continue;
                }
                let Ok(value) = registry.get_value::<String, _>(&name) else {
                    continue;
                };
                let path = PathBuf::from(expand_environment(&value));
                if path.is_absolute() {
                    add_file(&mut paths, path);
                } else {
                    for root in roots() {
                        add_file(&mut paths, root.join(&path));
                    }
                }
            }
        }
    }
    paths
}

#[cfg(not(windows))]
fn registered_files_matching(_matches: impl Fn(&str) -> bool) -> BTreeMap<String, PathBuf> {
    BTreeMap::new()
}

fn registered_files() -> BTreeMap<String, PathBuf> {
    registered_files_matching(|_| true)
}

fn canonical_family(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Metadata, not filenames or display labels, decides the default. In particular,
/// TT Norms Pro Serif/Condensed and non-regular faces must never win.
pub(super) fn default_english_rank(face: &FontChoice) -> Option<u8> {
    if face.style != fontdb::Style::Normal
        || face.weight != 400
        || face.stretch != fontdb::Stretch::Normal
        || !face.has_latin
    {
        return None;
    }
    face.families
        .iter()
        .filter_map(|family| match canonical_family(family).as_str() {
            "ttnormspro" | "ttnormspronormal" | "ttnormsproregular" => Some(0),
            "arial" | "arialregular" | "arialnormal" => Some(1),
            _ => None,
        })
        .min()
}

fn default_candidate_name(name: &str) -> bool {
    let name = canonical_family(name);
    // A collection's registry label can list several families joined with '&'.
    // This is only a candidate filter; exact metadata matching happens above.
    name.contains("ttnormspro") || name.contains("arial")
}

/// Startup reads only likely default files, including registry entries with
/// arbitrary filenames outside the font directories. The full byte scan remains
/// exclusively on the catalog worker. A caller caches this result for the process.
pub(super) fn default_english_candidates() -> FontCatalog {
    let mut paths = registered_files_matching(default_candidate_name);
    // Directory enumeration is metadata-only; no unrelated font bytes are read.
    paths.extend(collect_roots(&roots()).into_iter().filter(|(_, path)| {
        path.file_stem()
            .and_then(|s| s.to_str())
            .is_some_and(default_candidate_name)
    }));
    let mut catalog = scan_files(paths.into_values());
    catalog
        .choices
        .retain(|face| default_english_rank(face).is_some());
    catalog
        .choices
        .sort_by_cached_key(|face| (default_english_rank(face), key(&face.path), face.index));
    catalog
}

#[cfg(windows)]
fn expand_environment(mut rest: &str) -> String {
    let mut expanded = String::new();
    while let Some(start) = rest.find('%') {
        let tail = &rest[start + 1..];
        let Some(end) = tail.find('%') else {
            break;
        };
        expanded.push_str(&rest[..start]);
        let name = &tail[..end];
        expanded.push_str(&std::env::var(name).unwrap_or_else(|_| format!("%{name}%")));
        rest = &tail[end + 1..];
    }
    expanded.push_str(rest);
    expanded
}

fn local_path(path: &Path) -> bool {
    let text = path.to_string_lossy();
    (!text.starts_with(r"\\") || text.starts_with(r"\\?\"))
        && !text.to_ascii_lowercase().starts_with(r"\\?\unc\")
}

pub(super) fn read_font(path: &Path) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let size = file.metadata().map_err(|e| e.to_string())?.len();
    if size == 0 || size > MAX_FONT_BYTES {
        return Err("字體檔案必須介於 1 byte 與 64 MiB。".into());
    }
    let mut bytes = Vec::with_capacity(size as usize);
    file.take(MAX_FONT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_FONT_BYTES {
        return Err("字體檔案超過 64 MiB。".into());
    }
    Ok(bytes)
}

pub(super) fn selection_id(path: &Path, index: u32) -> String {
    format!("{PREFIX}{index}:{}", path.to_string_lossy())
}

/// Legacy filename preferences remain valid; new values include the TTC face.
pub(super) fn resolve_selection(value: &str) -> Result<(PathBuf, u32), String> {
    let (path, index) = if let Some(rest) = value.strip_prefix(PREFIX) {
        let (index, path) = rest.split_once(':').ok_or("字體選項格式錯誤。")?;
        let index = index.parse::<u32>().map_err(|_| "字體索引格式錯誤。")?;
        let path = PathBuf::from(path);
        if !path.is_absolute() {
            return Err("字體路徑必須是已安裝的本機字體。".into());
        }
        (path, index)
    } else {
        if value.is_empty() || value.contains(['/', '\\', ':']) {
            return Err("字體選項格式錯誤。".into());
        }
        (fonts_dir().join(value), 0)
    };
    if !font_extension(&path) {
        return Err("不支援此字體格式。".into());
    }
    // Never let an edited settings file trigger UNC access/credential exchange.
    if !local_path(&path) {
        return Err("不允許網路字體路徑。".into());
    }
    let canonical = path
        .canonicalize()
        .map_err(|e| format!("字體不存在：{e}"))?;
    if !local_path(&canonical) {
        return Err("不允許網路字體路徑。".into());
    }
    let installed = roots()
        .iter()
        .filter_map(|r| r.canonicalize().ok())
        .any(|r| key(&canonical).starts_with(&(key(&r) + std::path::MAIN_SEPARATOR_STR)))
        || registered_files().contains_key(&key(&canonical));
    if !installed {
        return Err("這個檔案不是已安裝的字體。".into());
    }
    Ok((canonical, index))
}

fn face_label(face: &fontdb::FaceInfo) -> String {
    let family = face
        .families
        .iter()
        .find(|(_, language)| *language == fontdb::Language::Chinese_Taiwan)
        .or_else(|| face.families.first())
        .map(|(name, _)| name.as_str())
        .unwrap_or(&face.post_script_name);
    let mut label: String = family
        .chars()
        .filter(|c| !c.is_control())
        .take(160)
        .collect();
    let weight = match face.weight.0 {
        400 => "",
        100 => "極細",
        200 => "特細",
        300 => "細體",
        500 => "中等",
        600 => "半粗",
        700 => "粗體",
        800 => "特粗",
        900 => "黑體",
        _ => "",
    };
    if !weight.is_empty() {
        label.push_str(&format!(" · {weight}"));
    }
    if face.style != fontdb::Style::Normal {
        label.push_str(" · 斜體");
    }
    if !matches!(face.stretch, fontdb::Stretch::Normal) {
        label.push_str(&format!(" · {:?}", face.stretch));
    }
    label
}

fn scan_files(paths: impl IntoIterator<Item = PathBuf>) -> FontCatalog {
    let mut catalog = FontCatalog::default();
    for path in paths {
        let Ok(bytes) = read_font(&path) else {
            catalog.skipped_files += 1;
            continue;
        };
        if bytes.starts_with(b"ttcf")
            && bytes
                .get(8..12)
                .and_then(|b| b.try_into().ok())
                .map(u32::from_be_bytes)
                .is_none_or(|count| count > 256)
        {
            catalog.skipped_files += 1;
            continue;
        }
        let bytes = Arc::new(bytes);
        let mut db = fontdb::Database::new();
        db.load_font_source(fontdb::Source::Binary(bytes.clone()));
        let before = catalog.choices.len();
        for face in db.faces() {
            let Ok(font) = ab_glyph::FontRef::try_from_slice_and_index(&bytes, face.index) else {
                continue;
            };
            if !font
                .codepoint_ids()
                .take(128)
                .any(|(id, _)| font.outline(id).is_some())
            {
                continue;
            }
            let label = face_label(face);
            let search = format!(
                "{label} {} {} {}",
                face.post_script_name,
                face.families
                    .iter()
                    .map(|(name, _)| name.as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
                path.file_stem().unwrap_or_default().to_string_lossy()
            )
            .to_lowercase();
            catalog.choices.push(FontChoice {
                id: selection_id(&path, face.index),
                label,
                search,
                path: path.clone(),
                index: face.index,
                families: face.families.iter().map(|(name, _)| name.clone()).collect(),
                style: face.style,
                weight: face.weight.0,
                stretch: face.stretch,
                has_latin: ['A', 'a', '0'].iter().all(|&c| font.glyph_id(c).0 != 0),
                has_chinese: ['中', '文', '的', '人']
                    .iter()
                    .any(|&c| font.glyph_id(c).0 != 0),
            });
        }
        if catalog.choices.len() == before {
            catalog.skipped_files += 1;
        }
    }
    catalog
        .choices
        .sort_by_cached_key(|face| (face.label.to_lowercase(), key(&face.path), face.index));
    catalog.choices.dedup_by(|a, b| a.id == b.id);
    catalog
}

fn discover() -> FontCatalog {
    let mut paths = collect_roots(&roots());
    paths.extend(registered_files());
    scan_files(paths.into_values())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face(family: &str, path: &str, index: u32) -> FontChoice {
        let path = PathBuf::from(path);
        FontChoice {
            id: selection_id(&path, index),
            label: family.into(),
            search: family.to_lowercase(),
            families: vec![family.into()],
            path,
            index,
            style: fontdb::Style::Normal,
            weight: 400,
            stretch: fontdb::Stretch::Normal,
            has_latin: true,
            has_chinese: false,
        }
    }

    #[test]
    fn default_ranking_uses_exact_metadata_family_and_regular_upright_normal_face() {
        let mut tt = face("TT Norms Pro Normal", r"C:\Users\test\Fonts\custom.ttc", 3);
        assert_eq!(default_english_rank(&tt), Some(0));
        assert_eq!(tt.index, 3);
        for family in ["TT Norms Pro", "TTNormsPro", "TT Norms Pro Regular"] {
            tt.families = vec![family.into()];
            assert_eq!(default_english_rank(&tt), Some(0));
        }
        for family in [
            "TT Norms Pro Serif",
            "TT Norms Pro Condensed",
            "TT Norms",
            "Arial Narrow",
        ] {
            tt.families = vec![family.into()];
            assert_eq!(default_english_rank(&tt), None, "{family}");
        }
        tt.families = vec!["TT Norms Pro".into()];
        tt.style = fontdb::Style::Italic;
        assert_eq!(default_english_rank(&tt), None);
        tt.style = fontdb::Style::Oblique;
        assert_eq!(default_english_rank(&tt), None);
        tt.style = fontdb::Style::Normal;
        tt.weight = 700;
        assert_eq!(default_english_rank(&tt), None);
        tt.weight = 400;
        tt.stretch = fontdb::Stretch::Condensed;
        assert_eq!(default_english_rank(&tt), None);
        tt.stretch = fontdb::Stretch::Normal;
        tt.has_latin = false;
        assert_eq!(default_english_rank(&tt), None);
        let arial = face("Arial", r"C:\Windows\Fonts\arial.ttf", 0);
        assert_eq!(default_english_rank(&arial), Some(1));
    }

    #[test]
    fn system_catalog_is_not_a_whitelist_and_preserves_collection_faces() {
        let catalog = discover();
        assert!(
            catalog.choices.len() > 30,
            "only {} fonts",
            catalog.choices.len()
        );
        assert!(catalog
            .choices
            .iter()
            .any(|f| f.search.contains("consolas")));
        assert!(catalog.choices.iter().any(|f| f.index > 0
            && f.path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("ttc"))));
        for choice in catalog
            .choices
            .iter()
            .filter(|f| f.search.contains("arial"))
        {
            let (path, index) = resolve_selection(&choice.id).unwrap();
            assert_eq!(path, choice.path);
            assert_eq!(index, choice.index);
        }
    }

    #[test]
    fn both_roots_unicode_paths_and_nested_fonts_are_enumerated_without_global_installs() {
        let dir = tempfile::tempdir().unwrap();
        let system = dir.path().join("system");
        let user = dir.path().join("使用者字體");
        std::fs::create_dir_all(system.join("nested")).unwrap();
        std::fs::create_dir_all(&user).unwrap();
        // Use egui's redistributable embedded fixture, never copy installed fonts.
        let defs = egui::FontDefinitions::default();
        let source = &defs.font_data["Ubuntu-Light"].font;
        std::fs::write(system.join("nested/custom.TTF"), source.as_ref()).unwrap();
        std::fs::write(user.join("我的字體.ttf"), source.as_ref()).unwrap();
        std::fs::write(user.join("not-font.txt"), "ignore").unwrap();
        let paths = collect_roots(&[system.clone(), user.clone(), system]);
        assert_eq!(paths.len(), 2);
        let catalog = scan_files(paths.into_values());
        assert_eq!(catalog.choices.len(), 2);
        assert!(catalog.choices.iter().any(|f| f.id.contains("我的字體")));
        assert!(catalog.choices.iter().all(|f| f.has_latin));
        assert!(catalog.choices.iter().all(|f| !f.has_chinese));
        assert!(
            catalog.choices.iter().all(|f| !f.search.contains("ttf")),
            "searching a font name must not match every .ttf extension"
        );
    }

    #[test]
    fn invalid_collection_and_oversize_fonts_are_bounded_and_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("bad.ttc");
        std::fs::write(&bad, b"ttcf\0\x01\0\0\xff\xff\xff\xff").unwrap();
        let large = dir.path().join("large.ttf");
        std::fs::File::create(&large)
            .unwrap()
            .set_len(MAX_FONT_BYTES + 1)
            .unwrap();
        let catalog = scan_files([bad, large]);
        assert!(catalog.choices.is_empty());
        assert_eq!(catalog.skipped_files, 2);
    }

    #[test]
    fn legacy_selection_survives_and_uninstalled_or_network_paths_are_rejected() {
        assert_eq!(resolve_selection("arial.ttf").unwrap().1, 0);
        for value in [
            r"..\arial.ttf",
            "system-font-v1:0:relative.ttf",
            r"system-font-v1:0:\\server\share\font.ttf",
            r"system-font-v1:0:\\?\UNC\server\font.ttf",
        ] {
            assert!(resolve_selection(value).is_err(), "{value}");
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("not-installed.ttf");
        std::fs::write(&path, b"not an installed font").unwrap();
        assert!(resolve_selection(&selection_id(&path, 0)).is_err());
    }
}
