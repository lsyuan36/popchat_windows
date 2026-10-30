//! LaTeX 公式點陣化：對應原版 `MarkdownText.swift` 的
//! `mathImage(latex:fontSize:display:)` + `mathCache`。
//!
//! 原版用 SwiftMath（iosMath）在主執行緒外算出 `NSImage`，`NSCache` 上限 128，
//! 失敗回 `nil` 讓呼叫方退回等寬 code block（`$latex$` / `$$latex$$`），絕不擋住訊息主流程。
//! 這裡的對應：
//!
//! - `latex_rust` 2.1.1 解析真正的數學 AST，以 OpenType MATH metrics 排版，
//!   再輸出 STIX Two Math 字形輪廓 SVG；`resvg` 光柵化後轉 `egui::ColorImage`。
//!   不使用 SVG `<text>`、系統字型、JavaScript、webview 或外部程式。
//! - 記憶體 cache：`HashMap<(latex, size_bits, display)>` + 插入序 `VecDeque`，
//!   上限 [`MAX_CACHED_FORMULAS`] 與 32 MiB 點陣資料，命中搬到隊尾；失敗結果不寫入。
//! - 顏色策略與原版不同：原版把 `.labelColor` 烘進點陣，cache key 含 appearance；
//!   這裡點陣固定為白色 alpha mask（來源的顏色也轉為白色），交給呼叫方
//!   `egui::Image::tint(ui.visuals().text_color())` 上主題色。
//!   因此 [`math_cache_key`] 不含主題，淺/深色共用同一份快取。
//!
//! 支援範圍由 `latex_rust` 決定，不宣稱與 SwiftMath 完全相容。
//! 分式、根式、上下標、希臘字母、矩陣和 aligned 使用真正的二維排版；
//! 未知命令、缺字、壞語法、過深或過大的公式回 `Err`，由 UI 顯示原始碼。

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};

/// 快取上限，與原版 `mathCache.countLimit = 128` 一致。
pub const MAX_CACHED_FORMULAS: usize = 128;
/// 超出資源上限時退回原始碼，不能裁掉公式內容。
const MAX_CANVAS_WIDTH: f32 = 2400.0;
const MAX_CANVAS_HEIGHT: f32 = 1600.0;
const MAX_CANVAS_PIXELS: u64 = 2_000_000;
const MAX_SOURCE_BYTES: usize = 16 * 1024;
const MAX_FONT_SIZE: f32 = 256.0;
const MAX_CACHE_BYTES: usize = 32 * 1024 * 1024;
const CANVAS_PADDING: f32 = 4.0;
/// 判定「有墨水」的 alpha 門檻（tiny-skia 存的是 premultiplied 通道）。
const INK_ALPHA_THRESHOLD: u8 = 10;

/// `(latex 原文, font_size.to_bits(), display)` —— 行內/展示字級與版面不同，必須進 key。
type CacheKey = (String, u32, bool);

struct MathCache {
    map: HashMap<CacheKey, egui::ColorImage>,
    order: VecDeque<CacheKey>,
    bytes: usize,
}

impl MathCache {
    fn get(&mut self, key: &CacheKey) -> Option<egui::ColorImage> {
        let hit = self.map.get(key)?.clone();
        // 命中搬到隊尾：近似 LRU（128 條內線性搬移成本可忽略）。
        if let Some(pos) = self.order.iter().position(|k| k == key) {
            self.order.remove(pos);
            self.order.push_back(key.clone());
        }
        Some(hit)
    }

    fn put(&mut self, key: CacheKey, image: egui::ColorImage) {
        let bytes = image.pixels.len() * std::mem::size_of::<egui::Color32>();
        if self.map.contains_key(&key) || bytes > MAX_CACHE_BYTES {
            return;
        }
        while self.map.len() >= MAX_CACHED_FORMULAS || self.bytes + bytes > MAX_CACHE_BYTES {
            match self.order.pop_front() {
                Some(old) => {
                    if let Some(old_image) = self.map.remove(&old) {
                        self.bytes -= old_image.pixels.len() * std::mem::size_of::<egui::Color32>();
                    }
                }
                None => break,
            }
        }
        self.order.push_back(key.clone());
        self.bytes += bytes;
        self.map.insert(key, image);
    }
}

fn math_cache() -> &'static Mutex<MathCache> {
    static CACHE: OnceLock<Mutex<MathCache>> = OnceLock::new();
    CACHE.get_or_init(|| {
        Mutex::new(MathCache {
            map: HashMap::new(),
            order: VecDeque::new(),
            bytes: 0,
        })
    })
}

fn lock_cache() -> std::sync::MutexGuard<'static, MathCache> {
    math_cache().lock().unwrap_or_else(|e| e.into_inner())
}

/// 對應原版 cache key `"\(appearance)|\(d/t)|\(fontSize)|\(latex)"` ——
/// 主題（appearance）刻意省略，理由見模組文件（白色字 + 呼叫方 tint）。
/// 也作為 egui 紋理名（見 `panel::MathTextures`）。
pub fn math_cache_key(latex: &str, font_size: f32, display: bool) -> String {
    format!(
        "{}|{:08x}|{}",
        if display { "d" } else { "t" },
        font_size.to_bits(),
        latex
    )
}

/// 對應原版 `mathImage`：成功回點陣，失敗回 `Err`
///（呼叫方退回 code block 顯示，不擋主流程；失敗不寫快取）。
/// `font_size` 是 CSS px（行內取 [`crate::ui::panel::INLINE_MATH_FONT_SIZE`]、
/// 展示取 [`crate::ui::panel::DISPLAY_MATH_FONT_SIZE`]）。
pub fn render_latex(
    latex: &str,
    font_size: f32,
    display: bool,
) -> anyhow::Result<egui::ColorImage> {
    let latex = latex.trim();
    anyhow::ensure!(!latex.is_empty(), "empty latex");
    anyhow::ensure!(latex.len() <= MAX_SOURCE_BYTES, "latex source too large");
    anyhow::ensure!(
        font_size.is_finite() && font_size > 0.0 && font_size <= MAX_FONT_SIZE,
        "bad font size {font_size}"
    );
    let key: CacheKey = (latex.to_owned(), font_size.to_bits(), display);
    if let Some(hit) = lock_cache().get(&key) {
        return Ok(hit);
    }
    let image = render_uncached(latex, font_size, display)?;
    lock_cache().put(key, image.clone());
    Ok(image)
}

fn render_uncached(latex: &str, font_size: f32, display: bool) -> anyhow::Result<egui::ColorImage> {
    let svg = math_svg(latex, font_size, display)?;
    let opt = resvg::usvg::Options::default();
    let tree = resvg::usvg::Tree::from_str(&svg, &opt)
        .map_err(|e| anyhow::anyhow!("svg parse failed: {e}"))?;
    // 使用實際輪廓邊界而不是字元 advance，保留斜體 overhang 與伸展根號。
    let bounds = tree.root().abs_layer_bounding_box();
    let (width, height) = checked_canvas(
        bounds.width() + CANVAS_PADDING * 2.0,
        bounds.height() + CANVAS_PADDING * 2.0,
    )?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| anyhow::anyhow!("pixmap alloc failed ({width}x{height})"))?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_translate(
            CANVAS_PADDING - bounds.x(),
            CANVAS_PADDING - bounds.y(),
        ),
        &mut pixmap.as_mut(),
    );

    // 去透明邊 + 轉 egui 點陣；全空視為失敗，讓 UI 退回 code block。
    let (cw, ch, rgba) = crop_premultiplied(&pixmap)
        .ok_or_else(|| anyhow::anyhow!("blank render (missing glyphs?)"))?;
    Ok(egui::ColorImage::from_rgba_premultiplied([cw, ch], &rgba))
}

fn checked_canvas(width: f32, height: f32) -> anyhow::Result<(u32, u32)> {
    anyhow::ensure!(
        width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0,
        "invalid math canvas"
    );
    anyhow::ensure!(
        width <= MAX_CANVAS_WIDTH && height <= MAX_CANVAS_HEIGHT,
        "math canvas too large ({width}x{height})"
    );
    let (width, height) = (width.ceil() as u32, height.ceil() as u32);
    anyhow::ensure!(
        u64::from(width) * u64::from(height) <= MAX_CANVAS_PIXELS,
        "math canvas pixel limit exceeded"
    );
    Ok((width, height))
}

/// 真正的 AST → 二維數學版面 → SVG 字形輪廓，絕不輸出原始 TeX 文本。
fn math_svg(latex: &str, font_size: f32, display: bool) -> anyhow::Result<String> {
    static FONT: OnceLock<Result<latex_rust::MathFont, String>> = OnceLock::new();
    let font = FONT
        .get_or_init(|| latex_rust::MathFont::stix_two_math().map_err(|e| e.to_string()))
        .as_ref()
        .map_err(|e| anyhow::anyhow!("math font failed: {e}"))?;
    // Windows 主執行緒可能只有 1 MiB stack；使用低於 crate 預設的遞迴上限。
    let (ast, _) =
        latex_rust::parse_with_options(latex, &latex_rust::ParseOptions::new().with_max_depth(16))
            .map_err(|e| anyhow::anyhow!("latex parse failed: {e}"))?;
    let mut options = latex_rust::SvgOptions::new();
    // API 是 CSS px，SVG 的實體單位是 pt；usvg 預設 96 dpi。
    options.font_size_pt = latex_rust::Dim::from_ieee32_bits((font_size * 0.75).to_bits());
    options.color = latex_rust::Color::rgb(255, 255, 255);
    options.display = display;
    let style = if display {
        latex_rust::MathStyle::Display
    } else {
        latex_rust::MathStyle::Text
    };
    let boxed = latex_rust::layout_with_em_size_pt(&ast, font, style, &options.font_size_pt)
        .map_err(|e| anyhow::anyhow!("math layout failed: {e}"))?;
    // 在序列化大量 path 或配置點陣之前檢查理論版面；光柵化前再檢查墨水邊界。
    checked_canvas(
        f32::from_bits(boxed.width.to_ieee32_bits()) * font_size + CANVAS_PADDING * 2.0,
        f32::from_bits((&boxed.height + &boxed.depth).to_ieee32_bits()) * font_size
            + CANVAS_PADDING * 2.0,
    )?;
    latex_rust::render_svg(&boxed, font, &options)
        .map_err(|e| anyhow::anyhow!("math svg failed: {e}"))
}

/// 找非透明外框（含 3px 留白）並裁出 premultiplied RGBA；全空回 `None`。
fn crop_premultiplied(pix: &resvg::tiny_skia::Pixmap) -> Option<(usize, usize, Vec<u8>)> {
    let (w, h) = (pix.width() as usize, pix.height() as usize);
    let data = pix.data();
    let (mut min_x, mut min_y) = (w, h);
    let (mut max_x, mut max_y) = (0usize, 0usize);
    let mut found = false;
    for y in 0..h {
        for x in 0..w {
            if data[(y * w + x) * 4 + 3] > INK_ALPHA_THRESHOLD {
                found = true;
                min_x = min_x.min(x);
                max_x = max_x.max(x);
                min_y = min_y.min(y);
                max_y = max_y.max(y);
            }
        }
    }
    if !found {
        return None;
    }
    let pad = 3usize;
    min_x = min_x.saturating_sub(pad);
    min_y = min_y.saturating_sub(pad);
    max_x = (max_x + pad).min(w - 1);
    max_y = (max_y + pad).min(h - 1);
    let (cw, ch) = (max_x - min_x + 1, max_y - min_y + 1);
    let mut rgba = Vec::with_capacity(cw * ch * 4);
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let off = (y * w + x) * 4;
            // Premultiplied white: all RGB channels equal alpha. Source colors
            // must not multiply the caller's light/dark theme tint.
            let alpha = data[off + 3];
            rgba.extend_from_slice(&[alpha, alpha, alpha, alpha]);
        }
    }
    Some((cw, ch, rgba))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Inspect post-layout outline positions, not merely whether some pixels exist.
    fn geometry(latex: &str) -> (String, Vec<[f32; 4]>) {
        let svg = math_svg(latex, 24.0, true).expect("supported math SVG");
        assert!(
            !svg.contains("<text"),
            "must emit glyph outlines, never raw TeX"
        );
        let tree =
            resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default()).expect("valid SVG");
        fn collect(group: &resvg::usvg::Group, out: &mut Vec<[f32; 4]>) {
            for node in group.children() {
                match node {
                    resvg::usvg::Node::Group(group) => collect(group, out),
                    resvg::usvg::Node::Path(path) => {
                        let b = path.abs_bounding_box();
                        out.push([b.x(), b.y(), b.width(), b.height()]);
                    }
                    _ => panic!("unexpected non-outline SVG node"),
                }
            }
        }
        let mut bounds = Vec::new();
        collect(tree.root(), &mut bounds);
        (svg, bounds)
    }

    fn center(b: [f32; 4]) -> (f32, f32) {
        (b[0] + b[2] / 2.0, b[1] + b[3] / 2.0)
    }

    #[test]
    fn fraction_has_stacked_glyphs_and_a_rule() {
        let (svg, b) = geometry(r"\frac{1}{2}");
        assert_eq!(svg.matches("<path ").count(), 2);
        assert_eq!(svg.matches("<rect ").count(), 1, "real fraction bar");
        // SVG emitter visits numerator, fraction rule, denominator.
        assert_eq!(b.len(), 3);
        assert!(b[0][1] + b[0][3] < b[1][1]);
        assert!(b[1][1] + b[1][3] < b[2][1]);
        assert!((center(b[0]).0 - center(b[2]).0).abs() < 3.0);
    }

    #[test]
    fn roots_and_scripts_have_two_dimensional_geometry() {
        let (svg, root) = geometry(r"\sqrt{x}");
        assert!(svg.contains("<rect "), "radical needs a vinculum");
        assert!(svg.matches("<path ").count() >= 2, "radical plus radicand");
        assert!(root.iter().any(|b| b[3] > 15.0));
        let bar = root.iter().min_by(|a, b| a[3].total_cmp(&b[3])).unwrap();
        let (_, full_x) = geometry("x");
        let radicand = root
            .iter()
            .find(|b| (b[3] - full_x[0][3]).abs() < 0.1)
            .expect("full-size x radicand");
        assert!(bar[3] < 2.0, "thin radical vinculum");
        assert!(bar[1] + bar[3] < radicand[1], "bar is above x");
        assert!(
            bar[0] <= radicand[0] && bar[0] + bar[2] >= radicand[0] + radicand[2],
            "vinculum spans the radicand"
        );

        let (svg, b) = geometry(r"x_i^2");
        assert_eq!(svg.matches("<path ").count(), 3);
        let base = b[0];
        let scripts = &b[1..];
        assert!(scripts.iter().all(|s| s[0] > base[0] + base[2] * 0.5));
        // Emitter order is base, superscript, subscript. Compare like glyphs:
        // a 70%-size dotted i can still be taller than a full-size x-height x.
        let (_, full_two) = geometry("2");
        let (_, full_i) = geometry("i");
        for (script, full) in [(b[1], full_two[0]), (b[2], full_i[0])] {
            let ratio = script[3] / full[3];
            assert!(
                (0.5..0.85).contains(&ratio),
                "reduced script glyph: {ratio}"
            );
        }
        assert!(scripts.iter().any(|s| center(*s).1 < center(base).1));
        assert!(scripts.iter().any(|s| center(*s).1 > center(base).1));
    }

    #[test]
    fn greek_matrix_and_aligned_are_laid_out_not_printed() {
        let (svg, _) = geometry(r"\alpha+\beta");
        assert_eq!(svg.matches("<path ").count(), 3);
        assert!(!svg.contains("alpha"));
        let (_, b) = geometry(r"\begin{matrix}1&2\\3&4\end{matrix}");
        assert_eq!(b.len(), 4);
        assert!((center(b[0]).1 - center(b[1]).1).abs() < 2.0);
        assert!((center(b[2]).1 - center(b[3]).1).abs() < 2.0);
        assert!(center(b[2]).1 - center(b[0]).1 > 15.0);
        assert!((center(b[0]).0 - center(b[2]).0).abs() < 3.0);
        assert!((center(b[1]).0 - center(b[3]).0).abs() < 3.0);
        assert!(center(b[1]).0 - center(b[0]).0 > 10.0);

        let (_, b) = geometry(r"\begin{aligned}x&=1\\y&=2\end{aligned}");
        assert_eq!(b.len(), 6);
        assert!((b[1][0] - b[4][0]).abs() < 0.1, "equals signs align");
        assert!(b[4][1] - b[1][1] > 15.0, "distinct equation rows");
    }

    #[test]
    fn simple_formula_renders() {
        let img = render_latex(r"\frac{1}{2}", 17.0, true).expect("simple formula must render");
        assert!(img.size[0] > 0 && img.size[1] > 0);
        assert!(
            img.pixels.iter().any(|p| p.a() > 0),
            "rendered image must contain ink"
        );
    }

    #[test]
    fn real_rasters_preserve_layout_and_monochrome_tint() {
        let baseline = render_latex("12", 24.0, true).unwrap();
        let fraction = render_latex(r"\frac{1}{2}", 24.0, true).unwrap();
        assert!(fraction.size[1] > baseline.size[1] + 10);
        assert!(fraction.size[0] < baseline.size[0]);
        let x = render_latex("x", 24.0, true).unwrap();
        for source in [
            r"\sqrt{x}",
            r"x_i^2",
            r"\alpha+\beta",
            r"\begin{pmatrix}1&2\\3&4\end{pmatrix}",
            r"\begin{aligned}x&=1\\y&=2\end{aligned}",
            r"\color{red}{x}+\color{black}{y}",
        ] {
            let image = render_latex(source, 24.0, true).expect(source);
            assert!(image.pixels.iter().any(|p| p.a() > 0), "{source}");
            assert!(image.size[0] > x.size[0], "{source}");
            assert!(
                image
                    .pixels
                    .iter()
                    .all(|p| p.r() == p.a() && p.g() == p.a() && p.b() == p.a()),
                "white premultiplied mask: {source}"
            );
        }
        let small = render_latex("x", 12.0, false).unwrap();
        let large = render_latex("x", 24.0, false).unwrap();
        assert!(large.size[1] > small.size[1]);
        let inline = render_latex(r"\sum_{i=1}^n i", 24.0, false).unwrap();
        let display = render_latex(r"\sum_{i=1}^n i", 24.0, true).unwrap();
        assert!(display.size[1] > inline.size[1], "display operator limits");
    }

    #[test]
    fn invalid_formula_errors() {
        for source in [
            r"\begin{notarealenv}x\end{notarealenv}",
            r"\notarealcommand{x}",
            r"\frac{1}",
            r"\sqrt{",
            r"\left(x",
        ] {
            assert!(render_latex(source, 17.0, true).is_err(), "{source}");
            assert!(!lock_cache()
                .map
                .contains_key(&(source.to_owned(), 17.0f32.to_bits(), true)));
        }
        assert!(render_latex("", 17.0, true).is_err());
        assert!(render_latex("   ", 13.0, false).is_err());
        for size in [f32::NAN, f32::INFINITY, -1.0, 0.0, MAX_FONT_SIZE + 1.0] {
            assert!(render_latex("x", size, false).is_err());
        }
        assert!(render_latex(&"x".repeat(MAX_SOURCE_BYTES + 1), 17.0, true).is_err());
        assert!(render_latex(&"x".repeat(2000), 17.0, true).is_err());
        let deep = "{".repeat(100) + "x" + &"}".repeat(100);
        assert!(render_latex(&deep, 17.0, true).is_err());
        assert!(render_latex(r"\hspace{1000000pt}x", 17.0, true).is_err());
    }

    #[test]
    fn cache_hits() {
        // 不依賴共用快取的長度，其他測試可能同時插入。
        let latex = "7zq1+7zq2";
        let first = render_latex(latex, 13.0, false).expect("probe renders");
        let second = render_latex(latex, 13.0, false).expect("probe renders again");
        assert_eq!(first.size, second.size);
        assert_eq!(first.pixels, second.pixels);
    }

    #[test]
    fn cache_is_bounded_and_hits_refresh_lru() {
        let mut cache = MathCache {
            map: HashMap::new(),
            order: VecDeque::new(),
            bytes: 0,
        };
        let key = |i| (format!("{i}"), 13.0f32.to_bits(), false);
        let image = egui::ColorImage::new([1, 1], vec![egui::Color32::WHITE]);
        for i in 0..MAX_CACHED_FORMULAS {
            cache.put(key(i), image.clone());
        }
        assert!(cache.get(&key(0)).is_some());
        cache.put(key(MAX_CACHED_FORMULAS), image);
        assert!(cache.map.contains_key(&key(0)));
        assert!(!cache.map.contains_key(&key(1)));
        assert_eq!(cache.map.len(), MAX_CACHED_FORMULAS);
        assert_eq!(cache.bytes, MAX_CACHED_FORMULAS * 4);

        let image = egui::ColorImage::new([512, 512], vec![egui::Color32::WHITE; 512 * 512]);
        for i in 0..40 {
            cache.put(
                (format!("large{i}"), 17.0f32.to_bits(), true),
                image.clone(),
            );
        }
        assert!(cache.bytes <= MAX_CACHE_BYTES);
        assert!(cache.map.len() <= 32);
        assert_eq!(cache.order.len(), cache.map.len());
    }

    #[test]
    fn cache_key_varies() {
        let key = math_cache_key("x", 13.0, false);
        assert_eq!(key, math_cache_key("x", 13.0, false));
        assert_ne!(key, math_cache_key("x", 13.0, true));
        assert_ne!(key, math_cache_key("x", 17.0, false));
        assert_ne!(key, math_cache_key("y", 13.0, false));
    }
}
