//! Searchable installed fonts, independent language choices and glyph fallbacks.

use super::preference_layout as form;
use super::{font_catalog, font_cmap};
use crate::logf;
use ab_glyph::Font as _;
use font_catalog::{fonts_dir, read_font};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Empty strings retain egui's English fonts and the original automatic CJK fallback.
/// Old filename values remain readable; new choices identify installed files/faces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct FontPreferences {
    pub english: String,
    pub chinese: String,
}

const CACHE_TTL: Duration = Duration::from_secs(60);

#[derive(Clone, Default)]
struct AppliedFonts {
    preferences: Option<FontPreferences>,
    failed: Option<(FontPreferences, Instant, String)>,
    cjk_path: Option<String>,
    rebuilds: usize,
}

fn load_choice(value: &str, chinese: bool) -> Result<egui::FontData, String> {
    let (path, index) = font_catalog::resolve_selection(value)?;
    let bytes = read_font(&path)?;
    let bytes = font_cmap::subset(
        bytes,
        index,
        if chinese {
            font_cmap::Script::Chinese
        } else {
            font_cmap::Script::English
        },
    )?;
    let mut data = egui::FontData::from_owned(bytes);
    data.index = index;
    Ok(data)
}

#[derive(Clone, Default)]
struct PickerSearch {
    query: String,
    open: bool,
}

fn font_picker(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut String,
    catalog: &font_catalog::FontCatalog,
) -> egui::Response {
    let search_id = ui.make_persistent_id(("installed-font-search", label));
    let mut search = ui
        .data_mut(|data| data.get_temp::<PickerSearch>(search_id))
        .unwrap_or_default();
    let selected = catalog.choices.iter().find(|face| {
        &face.id == value
            || (!value.starts_with("system-font-v1:")
                && face.index == 0
                && face
                    .path
                    .file_name()
                    .is_some_and(|f| f.eq_ignore_ascii_case(value.as_str())))
    });
    let selected_text = if value.is_empty() {
        "預設字體".to_owned()
    } else {
        selected.map(|face| face.label.clone()).unwrap_or_else(|| {
            value
                .rsplit(['\\', '/'])
                .next()
                .unwrap_or("已選字體")
                .to_owned()
        })
    };
    let width = ui.available_width();
    let max_height = (ui.ctx().screen_rect().height() - 48.0).clamp(160.0, 360.0);
    let response = egui::ComboBox::from_id_salt(("installed-font-picker", label))
        .width(width)
        .height(max_height)
        .truncate()
        .selected_text(selected_text)
        .show_ui(ui, |ui| {
            ui.set_min_width(width);
            let edit = ui.add(
                egui::TextEdit::singleline(&mut search.query)
                    .hint_text("搜尋字體名稱…")
                    .desired_width(f32::INFINITY),
            );
            if !search.open {
                edit.request_focus();
            }
            ui.add_space(4.0);
            if ui.selectable_label(value.is_empty(), "預設字體").clicked() {
                value.clear();
                ui.close();
            }
            let query = search.query.trim().to_lowercase();
            let choices: Vec<_> = catalog
                .choices
                .iter()
                .filter(|face| face.search.contains(&query))
                .collect();
            egui::ScrollArea::vertical()
                .id_salt(("installed-font-rows", label))
                .max_height((max_height - 108.0).max(50.0))
                .auto_shrink([false, true])
                .show_rows(ui, 32.0, choices.len(), |ui, rows| {
                    for index in rows {
                        let face = choices[index];
                        let selected =
                            value == &face.id || selected.is_some_and(|s| s.id == face.id);
                        let response = ui.add_sized(
                            egui::vec2(ui.available_width(), 32.0),
                            egui::Button::selectable(selected, &face.label).truncate(),
                        );
                        if response.clicked() {
                            *value = face.id.clone();
                            ui.close();
                        }
                        response.on_hover_text(format!(
                            "{}\n{} · 字面 {}{}",
                            face.label,
                            face.path.display(),
                            face.index,
                            if label == "中文字體" && !face.has_chinese {
                                "\n未含常用中文，將使用備援字體。"
                            } else {
                                ""
                            }
                        ));
                    }
                });
            if choices.is_empty() {
                form::caption(ui, "沒有符合的字體，試試英文名稱或其他關鍵字。");
            }
            form::caption(ui, &format!("{} 個結果", choices.len()));
        });
    search.open = response.inner.is_some();
    ui.data_mut(|data| data.insert_temp(search_id, search));
    response.response
}

/// The complete supported installed outline catalog, searchable without loading
/// a new font for every row. Scanning never runs on the UI thread.
pub fn show_font_preferences(ui: &mut egui::Ui, prefs: &mut FontPreferences) {
    let snapshot = font_catalog::catalog(ui.ctx(), false);
    let catalog = &snapshot.catalog;
    form::heading(ui, "字體");
    form::card(ui, |ui| {
        for (label, value) in [
            ("中文字體", &mut prefs.chinese),
            ("英文字體", &mut prefs.english),
        ] {
            form::row(ui, label, |ui| {
                font_picker(ui, label, value, catalog);
            });
            ui.add_space(4.0);
            if let Some(face) = catalog.choices.iter().find(|face| &face.id == value) {
                if label == "中文字體" && !face.has_chinese {
                    form::caption(ui, "所選字體未含常用中文，將由備援字體補足。");
                } else if label == "英文字體" && !face.has_latin {
                    form::caption(ui, "所選字體未含英文字母，將由預設字體補足。");
                }
            }
        }
        ui.horizontal(|ui| {
            if snapshot.scanning {
                ui.spinner();
                form::caption(ui, "正在讀取已安裝字體…");
            } else {
                ui.label(egui::RichText::new(format!("{} 個已安裝字體", catalog.choices.len())).size(12.0).weak())
                    .on_hover_text(format!("包含系統與使用者安裝的 TTF／OTF／TTC／OTC 字體及字面。\n略過 {} 個無法讀取、僅含點陣或超過 64 MiB 的檔案。", catalog.skipped_files));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(!snapshot.scanning, egui::Button::new("重新掃描"))
                    .clicked()
                {
                    font_catalog::catalog(ui.ctx(), true);
                }
            });
        });
        if let Some(error) = &snapshot.error {
            form::caption(ui, error);
        }
        form::divider(ui);
        form::caption(ui, "即時預覽");
        ui.add_space(4.0);
        ui.add(egui::Label::new(egui::RichText::new("讓想法，自然成為對話。").size(19.0)).wrap());
        ui.add(
            egui::Label::new(egui::RichText::new("The quick brown fox · 0123456789").size(15.0))
                .wrap(),
        );
        ui.add_space(6.0);
        form::caption(ui, "缺字自動使用備援；程式碼維持等寬字體。");
    });
}

fn build_definitions(
    prefs: &FontPreferences,
) -> Result<(egui::FontDefinitions, Option<String>), String> {
    let mut defs = egui::FontDefinitions::default();
    if !prefs.english.is_empty() {
        let data = load_choice(&prefs.english, false)?;
        defs.font_data
            .insert("selected-english".to_owned(), std::sync::Arc::new(data));
        defs.families
            .get_mut(&egui::FontFamily::Proportional)
            .unwrap()
            .insert(0, "selected-english".to_owned());
    }
    if !prefs.chinese.is_empty() {
        let data = load_choice(&prefs.chinese, true)?;
        defs.font_data
            .insert("selected-chinese".to_owned(), std::sync::Arc::new(data));
        defs.families
            .get_mut(&egui::FontFamily::Proportional)
            .unwrap()
            .insert(
                usize::from(!prefs.english.is_empty()),
                "selected-chinese".to_owned(),
            );
        defs.families
            .get_mut(&egui::FontFamily::Monospace)
            .unwrap()
            .push("selected-chinese".to_owned());
    }
    // Always retain full-coverage CJK backup, even when the chosen face has holes.
    let cjk = find_cjk_font()
        .map(|path| {
            read_font(&path).map(|bytes| {
                (
                    egui::FontData::from_owned(bytes),
                    path.display().to_string(),
                )
            })
        })
        .transpose()?;
    let mut cjk_path = None;
    if let Some((data, path)) = cjk {
        defs.font_data
            .insert("cjk-fallback".to_owned(), std::sync::Arc::new(data));
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            // Embedded Latin/code/emoji fonts remain intact and ahead of CJK.
            defs.families
                .get_mut(&family)
                .unwrap()
                .push("cjk-fallback".to_owned());
        }
        cjk_path = Some(path);
    }
    Ok((defs, cjk_path))
}

/// Atomically apply a pair. Invalid selections leave the last working fonts intact.
/// Success is context-local and permanent until preferences change; failures retry
/// at most once per minute (e.g. after an installed font is repaired).
pub fn apply_font_preferences(ctx: &egui::Context, prefs: &FontPreferences) -> Result<(), String> {
    let id = egui::Id::new("popchat-applied-font-preferences");
    let mut state = ctx
        .data_mut(|data| data.get_temp::<AppliedFonts>(id))
        .unwrap_or_default();
    if state.preferences.as_ref() == Some(prefs) {
        return Ok(());
    }
    if let Some((failed, checked, error)) = &state.failed {
        if failed == prefs && checked.elapsed() < CACHE_TTL {
            return Err(error.clone());
        }
    }
    match build_definitions(prefs) {
        Ok((defs, path)) => {
            ctx.set_fonts(defs);
            state.preferences = Some(prefs.clone());
            state.failed = None;
            state.cjk_path = path;
            state.rebuilds += 1;
            ctx.request_repaint();
            ctx.data_mut(|data| data.insert_temp(id, state));
            Ok(())
        }
        Err(error) => {
            state.failed = Some((prefs.clone(), Instant::now(), error.clone()));
            ctx.data_mut(|data| data.insert_temp(id, state));
            Err(error)
        }
    }
}

const CANDIDATES: &[&str] = &[
    "msjh.ttc",
    "msyh.ttc",
    "mingliu.ttc",
    "malgun.ttf",
    "kaiu.ttf",
];

/// 需要覆蓋的常用字（UI 空態提示 + 設定頁關鍵字）。兼作選字門檻：
/// 缺任何一個的候選字型直接跳過（不分 test/release）。
const MUST_HAVE: &[char] = &[
    '呼', '叫', '下', '方', '輸', '入', '開', '始', '聊', '天', '設', '定', '搜', '尋', '提', '供',
    '者', '模', '型', '密', '鑰', '錯', '誤', '等', '待', '回', '覆', '思', '考', '過', '程', '附',
    '件', '更', '新', '熱', '鍵', '托', '盤', '問', '點', '什', '麼', '使', '用', '快', '捷', '指',
    '令', '儲', '存', '外', '觀', '憑', '證', '選', '擇', '回', '傳', '顯', '示', '隱', '藏', '釘',
    '選', '關', '閉',
];

/// 找第一個存在、ab_glyph 打得開、且常用字全覆蓋的字型檔。
pub fn find_cjk_font() -> Option<PathBuf> {
    let dir = fonts_dir();
    for name in CANDIDATES {
        let path = dir.join(name);
        let bytes = match read_font(&path) {
            Ok(b) => b,
            Err(_) => continue,
        };
        match ab_glyph::FontRef::try_from_slice(&bytes) {
            Ok(font) => {
                let missing: Vec<char> = MUST_HAVE
                    .iter()
                    .copied()
                    .filter(|&c| font.glyph_id(c).0 == 0)
                    .collect();
                if !missing.is_empty() {
                    logf!(
                        "[font] {} 缺 {} 個常用字（跳過）：{:?}",
                        path.display(),
                        missing.len(),
                        missing.iter().take(8).collect::<Vec<_>>()
                    );
                    continue;
                }
                return Some(path);
            }
            Err(e) => {
                logf!("[font] {} 解析失敗（跳過）：{e}", path.display());
            }
        }
    }
    None
}

/// 載入並掛到 egui。回傳選中的檔名（供 log）；None 表示沒找到。
pub fn install_cjk_font(ctx: &egui::Context) -> Option<String> {
    apply_font_preferences(ctx, &FontPreferences::default()).ok()?;
    let name = ctx.data_mut(|data| {
        data.get_temp::<AppliedFonts>(egui::Id::new("popchat-applied-font-preferences"))
            .and_then(|state| state.cjk_path)
    })?;
    logf!("[font] CJK fallback 已載入：{name}");
    // 注意：此處不能調 ctx.fonts() 自檢（首次 run() 前會 panic，
    // "No fonts available until first call to Context::run()"）。
    // 覆蓋率由單元測試 egui_fallback_resolves_cjk 保證。
    Some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fonts_for(prefs: &FontPreferences) -> egui::epaint::text::Fonts {
        egui::epaint::text::Fonts::new(
            1.0,
            2048,
            egui::epaint::image::AlphaFromCoverage::default(),
            build_definitions(prefs).unwrap().0,
        )
    }

    fn applied(ctx: &egui::Context) -> AppliedFonts {
        ctx.data_mut(|data| {
            data.get_temp::<AppliedFonts>(egui::Id::new("popchat-applied-font-preferences"))
                .unwrap()
        })
    }

    #[test]
    fn preferences_are_eq_serializable_and_missing_fields_default() {
        let prefs: FontPreferences = serde_json::from_str(r#"{"english":"arial.ttf"}"#).unwrap();
        assert_eq!(prefs.chinese, "");
        assert_eq!(
            serde_json::from_str::<FontPreferences>(&serde_json::to_string(&prefs).unwrap())
                .unwrap(),
            prefs
        );
        assert_eq!(
            serde_json::from_str::<FontPreferences>("{}").unwrap(),
            FontPreferences::default()
        );
    }

    #[test]
    fn multilingual_and_incomplete_fonts_are_allowed_without_cross_language_capture() {
        let latin = load_choice("msjh.ttc", false).unwrap();
        let font = ab_glyph::FontRef::try_from_slice_and_index(&latin.font, latin.index).unwrap();
        assert_ne!(font.glyph_id('A').0, 0);
        assert_eq!(font.glyph_id('中').0, 0);
        let chinese = load_choice("msjh.ttc", true).unwrap();
        let font =
            ab_glyph::FontRef::try_from_slice_and_index(&chinese.font, chinese.index).unwrap();
        assert_ne!(font.glyph_id('中').0, 0);
        assert_eq!(font.glyph_id('A').0, 0);
        // A Latin-only face is still a valid Chinese choice, with missing Chinese
        // transparently resolved by the permanently retained CJK fallback.
        let prefs = FontPreferences {
            chinese: "arial.ttf".into(),
            ..Default::default()
        };
        assert!(fonts_for(&prefs).has_glyphs(&egui::FontId::proportional(18.0), "中文呼叫搜尋"));
    }

    #[test]
    fn english_selection_changes_latin_metrics_not_chinese_or_code() {
        let a = FontPreferences {
            english: "arial.ttf".into(),
            chinese: "msjh.ttc".into(),
        };
        let b = FontPreferences {
            english: "georgia.ttf".into(),
            ..a.clone()
        };
        let first = fonts_for(&a);
        let second = fonts_for(&b);
        let id = egui::FontId::proportional(18.0);
        let width = |fonts: &egui::epaint::text::Fonts, text: &str, id: egui::FontId| {
            fonts
                .layout_no_wrap(text.to_owned(), id, egui::Color32::WHITE)
                .size()
                .x
        };
        assert!(
            (width(&first, "The quick brown fox WWW iii 0123456789", id.clone())
                - width(
                    &second,
                    "The quick brown fox WWW iii 0123456789",
                    id.clone()
                ))
            .abs()
                > 1.0
        );
        assert_eq!(
            width(&first, "呼叫下方輸入開始聊天", id.clone()),
            width(&second, "呼叫下方輸入開始聊天", id.clone())
        );
        assert_eq!(
            width(
                &first,
                "let answer = 42; 中文",
                egui::FontId::monospace(18.0)
            ),
            width(
                &second,
                "let answer = 42; 中文",
                egui::FontId::monospace(18.0)
            )
        );
        assert!(first.has_glyphs(&id, "呼叫下方輸入開始聊天"));
        let defs = build_definitions(&a).unwrap().0;
        let mono = &defs.families[&egui::FontFamily::Monospace];
        assert_ne!(mono[0], "selected-english");
        assert_ne!(mono[0], "cjk-fallback");
        for (name, data) in egui::FontDefinitions::default().font_data {
            assert_eq!(
                defs.font_data[&name].font, data.font,
                "lost embedded fallback {name}"
            );
        }
    }

    #[test]
    fn chinese_selection_replaces_the_actual_fallback() {
        let first = FontPreferences {
            english: "arial.ttf".into(),
            chinese: "msjh.ttc".into(),
        };
        let second = FontPreferences {
            chinese: "kaiu.ttf".into(),
            ..first.clone()
        };
        let a = build_definitions(&first).unwrap().0;
        let b = build_definitions(&second).unwrap().0;
        assert_ne!(
            a.font_data["selected-chinese"].font,
            b.font_data["selected-chinese"].font
        );
        assert_eq!(
            a.font_data["cjk-fallback"].font,
            b.font_data["cjk-fallback"].font
        );
        assert_eq!(
            a.font_data["selected-english"].font,
            b.font_data["selected-english"].font
        );
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            assert_eq!(b.families[&family].last().unwrap(), "cjk-fallback");
            assert!(fonts_for(&second)
                .has_glyphs(&egui::FontId::new(16.0, family), "呼叫下方輸入開始聊天"));
        }
    }

    #[test]
    fn same_preferences_do_not_rebuild_and_contexts_are_independent() {
        let ctx = egui::Context::default();
        let prefs = FontPreferences {
            english: "arial.ttf".into(),
            ..Default::default()
        };
        for _ in 0..10 {
            apply_font_preferences(&ctx, &prefs).unwrap();
        }
        assert_eq!(applied(&ctx).rebuilds, 1);
        let other = egui::Context::default();
        apply_font_preferences(&other, &prefs).unwrap();
        assert_eq!(applied(&other).rebuilds, 1);
        apply_font_preferences(&ctx, &FontPreferences::default()).unwrap();
        assert_eq!(applied(&ctx).rebuilds, 2);
        assert_eq!(applied(&other).rebuilds, 1);
    }

    #[test]
    fn invalid_selection_keeps_prior_fonts_and_caches_the_error() {
        let ctx = egui::Context::default();
        let good = FontPreferences {
            english: "arial.ttf".into(),
            ..Default::default()
        };
        apply_font_preferences(&ctx, &good).unwrap();
        let _ = ctx.run(Default::default(), |_| {});
        let width = || {
            ctx.fonts(|fonts| {
                fonts
                    .layout_no_wrap(
                        "WWiii0123".into(),
                        egui::FontId::proportional(18.0),
                        egui::Color32::WHITE,
                    )
                    .size()
                    .x
            })
        };
        let before = width();
        let bad = FontPreferences {
            chinese: "missing-font.ttf".into(),
            ..good.clone()
        };
        let error = apply_font_preferences(&ctx, &bad).unwrap_err();
        let checked = applied(&ctx).failed.unwrap().1;
        assert_eq!(apply_font_preferences(&ctx, &bad).unwrap_err(), error);
        assert_eq!(applied(&ctx).failed.unwrap().1, checked);
        assert_eq!(applied(&ctx).preferences, Some(good));
        assert_eq!(applied(&ctx).rebuilds, 1);
        let _ = ctx.run(Default::default(), |_| {});
        assert_eq!(width(), before);
        assert!(load_choice(r"..\arial.ttf", false).is_err());
        assert!(load_choice("msjh.ttc", false).is_ok());
    }

    #[test]
    fn font_reads_reject_oversize_and_invalid_fixture_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("oversize.ttf");
        std::fs::File::create(&path)
            .unwrap()
            .set_len(font_catalog::MAX_FONT_BYTES + 1)
            .unwrap();
        assert!(read_font(&path).unwrap_err().contains("64 MiB"));
        std::fs::write(&path, b"not a font").unwrap();
        assert!(ab_glyph::FontRef::try_from_slice(&read_font(&path).unwrap()).is_err());
        assert!(read_font(&dir.path().join("missing.ttf")).is_err());
    }

    #[test]
    fn mixed_cjk_graphemes_and_emoji_layout_safely() {
        let prefs = FontPreferences {
            english: "arial.ttf".into(),
            chinese: "msjh.ttc".into(),
        };
        let fonts = fonts_for(&prefs);
        // Variation selectors/ZWJ are not standalone display glyphs. Check safe,
        // lossless layout rather than claiming egui provides full emoji shaping.
        let text = "繁體中文，呼叫！ e\u{301} 禰\u{e0100} 中\u{fe0f} 👩\u{200d}💻 😀";
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            let galley = fonts.layout_no_wrap(
                text.into(),
                egui::FontId::new(18.0, family),
                egui::Color32::WHITE,
            );
            assert_eq!(galley.text(), text);
            assert!(galley.size().is_finite());
            assert!(galley.size().x > 0.0);
        }
    }

    #[test]
    fn selectors_render_available_labels_and_live_previews() {
        let ctx = egui::Context::default();
        let mut prefs = FontPreferences::default();
        apply_font_preferences(&ctx, &prefs).unwrap();
        let output = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| show_font_preferences(ui, &mut prefs));
        });
        let texts: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text.galley.text()),
                _ => None,
            })
            .collect();
        assert!(texts.contains(&"中文字體"));
        assert!(texts.contains(&"英文字體"));
        assert!(texts.contains(&"即時預覽"));
        assert!(texts
            .iter()
            .any(|text| text.contains("The quick brown fox")));
        assert!(texts
            .iter()
            .any(|text| text.contains("讓想法，自然成為對話。")));
        assert!(!texts
            .iter()
            .any(|text| text.contains("Default /") || text.contains("Code preview:")));
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| show_font_preferences(ui, &mut prefs));
        });
        assert_eq!(prefs, FontPreferences::default());
    }

    #[test]
    fn multilingual_english_choice_does_not_capture_selected_chinese_glyphs() {
        let chinese = FontPreferences {
            english: "arial.ttf".into(),
            chinese: "kaiu.ttf".into(),
        };
        let multilingual = FontPreferences {
            english: "msjh.ttc".into(),
            ..chinese.clone()
        };
        let first = fonts_for(&chinese);
        let second = fonts_for(&multilingual);
        let id = egui::FontId::proportional(18.0);
        let text = "中文字體，呼叫搜尋。";
        // Egui uses the primary font's ascent for the line box, even for a CJK
        // fallback run. Its height may change; CJK glyph selection/width must not.
        assert_eq!(
            first
                .layout_no_wrap(text.into(), id.clone(), egui::Color32::BLACK)
                .size()
                .x,
            second
                .layout_no_wrap(text.into(), id, egui::Color32::BLACK)
                .size()
                .x
        );
        assert_eq!(
            build_definitions(&chinese).unwrap().0.font_data["selected-chinese"].font,
            build_definitions(&multilingual).unwrap().0.font_data["selected-chinese"].font
        );
    }

    #[test]
    fn selected_user_fonts_rasterize_body_and_code_at_transcript_sizes() {
        let chinese = font_catalog::selection_id(&fonts_dir().join("YuGothR.ttc"), 1);
        for english in ["arial.ttf", "TT Norms Pro Normal.ttf"] {
            if !fonts_dir().join(english).exists() {
                continue;
            }
            let fonts = fonts_for(&FontPreferences {
                english: english.into(),
                chinese: chinese.clone(),
            });
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                for size in [12.0, 13.0, 15.0, 19.0] {
                    let galley = fonts.layout_no_wrap(
                        "輸出中文ABC123".into(),
                        egui::FontId::new(size, family.clone()),
                        egui::Color32::BLACK,
                    );
                    let image = fonts.image();
                    for glyph in galley.rows.iter().flat_map(|row| &row.glyphs) {
                        let uv = glyph.uv_rect;
                        assert!(
                            !uv.is_nothing(),
                            "{english} {family:?} {size}: {} has no raster",
                            glyph.chr
                        );
                        let ink = (usize::from(uv.min[1])..usize::from(uv.max[1]))
                            .flat_map(|y| {
                                (usize::from(uv.min[0])..usize::from(uv.max[0]))
                                    .map(move |x| (x, y))
                            })
                            .filter(|&(x, y)| image[(x, y)].a() > 0)
                            .count();
                        assert!(
                            ink > 0,
                            "{english} {family:?} {size}: {} empty atlas region",
                            glyph.chr
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn searchable_picker_filters_and_selects_an_actual_installed_face() {
        let ctx = egui::Context::default();
        let catalog = font_catalog::FontCatalog {
            choices: ["arial.ttf", "consola.ttf"]
                .into_iter()
                .map(|name| {
                    let path = fonts_dir().join(name).canonicalize().unwrap();
                    let label = if name == "arial.ttf" {
                        "Arial"
                    } else {
                        "Consolas"
                    }
                    .to_owned();
                    font_catalog::FontChoice {
                        id: font_catalog::selection_id(&path, 0),
                        search: label.to_lowercase(),
                        label,
                        path,
                        index: 0,
                        has_latin: true,
                        has_chinese: false,
                    }
                })
                .collect(),
            skipped_files: 0,
        };
        let mut value = String::new();
        let mut draw = |events| {
            let mut rect = egui::Rect::NOTHING;
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(604.0, 644.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        rect = font_picker(ui, "英文字體", &mut value, &catalog).rect;
                    });
                },
            );
            (output, rect)
        };
        let (_, rect) = draw(vec![]);
        draw(vec![]);
        let pos = rect.center();
        for pressed in [true, false] {
            draw(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
        }
        draw(vec![]);
        let (filtered, _) = draw(vec![egui::Event::Text("consolas".into())]);
        fn find(shape: &egui::Shape, name: &str) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Text(text) if text.galley.text() == name => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| find(s, name)),
                _ => None,
            }
        }
        assert!(!filtered
            .shapes
            .iter()
            .any(|s| find(&s.shape, "Arial").is_some()));
        let pos = filtered
            .shapes
            .iter()
            .find_map(|s| find(&s.shape, "Consolas"))
            .expect("matching installed face remains clickable");
        for pressed in [true, false] {
            draw(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
        }
        drop(draw);
        assert_eq!(value, catalog.choices[1].id);
    }

    #[test]
    fn cjk_font_covers_ui_chars() {
        let path = find_cjk_font().expect("系統應有可用 CJK 字型");
        logf!("選中字型：{}", path.display());
        let bytes = std::fs::read(&path).unwrap();
        let font = ab_glyph::FontRef::try_from_slice(&bytes).unwrap();
        let missing: Vec<char> = MUST_HAVE
            .iter()
            .copied()
            .filter(|&c| font.glyph_id(c).0 == 0)
            .collect();
        assert!(missing.is_empty(), "缺字：{missing:?}");
    }

    #[test]
    fn egui_fallback_resolves_cjk() {
        // 端到端：用與 install 完全相同的 defs 建 epaint Fonts，
        // 確認 fallback 真的解得到中文（而不只是字型檔能解析）。
        let path = find_cjk_font().expect("系統應有可用 CJK 字型");
        let bytes = std::fs::read(&path).unwrap();
        let mut defs = egui::FontDefinitions::default();
        defs.font_data.insert(
            "cjk-fallback".to_owned(),
            std::sync::Arc::new(egui::FontData::from_owned(bytes)),
        );
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            defs.families
                .get_mut(&family)
                .unwrap()
                .push("cjk-fallback".to_owned());
        }
        let fonts = egui::epaint::text::Fonts::new(
            1.0,
            2048,
            egui::epaint::image::AlphaFromCoverage::default(),
            defs,
        );
        let id = egui::FontId::new(14.0, egui::FontFamily::Proportional);
        assert!(fonts.has_glyph(&id, '呼'), "Proportional 解不到 呼");
        assert!(
            fonts.has_glyphs(&id, "Alt+Down 呼叫 · 下方輸入開始聊天"),
            "空態提示字串有缺字"
        );
    }

    #[test]
    fn kaiu_glyph_rasterizes_nonblank() {
        // cmap 有字不代表畫得出來：實際光柵化「呼」並斷言有墨點。
        use ab_glyph::Font as _;
        let path = find_cjk_font().expect("系統應有可用 CJK 字型");
        let bytes = std::fs::read(&path).unwrap();
        let font = ab_glyph::FontRef::try_from_slice(&bytes).unwrap();
        let glyph = font
            .glyph_id('呼')
            .with_scale_and_position(32.0, ab_glyph::point(0.0, 0.0));
        let outlined = font.outline_glyph(glyph).expect("呼 應有輪廓");
        let bounds = outlined.px_bounds();
        assert!(bounds.width() > 0.0 && bounds.height() > 0.0);
        let mut ink = 0u32;
        outlined.draw(|_, _, v| {
            if v > 0.1 {
                ink += 1;
            }
        });
        assert!(ink > 10, "呼 光柵化為空白，換字型");
    }
}
