//! Rendering counterparts of MarkdownText.swift: prose, quotes, pipe tables,
//! highlighted fences and verbatim, copyable pasteable cards. The authoritative
//! segment order lives in panel::split_math_segments and is used by MathTextures.
//!
//! A Swift SelectableText is a selection island: prose paragraphs and links share
//! selection, but code, quotes, pasteables and individual table cells do not.
//! egui supports cross-label selection; restrict it to those same source islands.

use eframe::egui;
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::ui::panel::{split_inline_math, InlinePart, MathTextures, INLINE_MATH_FONT_SIZE};

// ---------------------------------------------------------------------------
// 純資料層（方便測試）
// ---------------------------------------------------------------------------

/// 行內文字樣式快照（parse 時從 emphasis/strong/strikethrough 深度轉成 bool）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct SpanStyle {
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
}

/// 行內片段。`Text` 保證不含行內碼（行內碼是獨立的 [`Inline::Code`]），
/// Math is protected before markdown parsing and restored as [`Inline::Math`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Inline {
    Text(String, SpanStyle),
    Math(String),
    Code(String),
    Link {
        label: String,
        url: String,
        style: SpanStyle,
    },
    /// pulldown `SoftBreak`：渲染時合併成前一段的空白（見 [`merge_spaces`]）。
    Space,
    /// pulldown `HardBreak`：渲染時另起一行。
    HardBreak,
}

/// 有序/無序列表：連續 item 合併成一個 block，`ordered`/`number` 逐項記錄
/// （巢狀混排 `1.` 包 `-` 時前綴各自正確）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ListItem {
    /// Zero-based nesting depth, with the original 16pt indent per level.
    pub depth: u8,
    pub ordered: bool,
    pub number: u64,
    pub inlines: Vec<Inline>,
}

/// 塊級片段（fence 已由 [`split_fences`] 預先剝離，這裡的 Code 只來自縮排 codeblock）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FlowBlock {
    Heading {
        level: u8,
        inlines: Vec<Inline>,
    },
    Paragraph(Vec<Inline>),
    Quote(Vec<Inline>),
    List(Vec<ListItem>),
    Code {
        language: Option<String>,
        content: String,
    },
    Rule,
}

/// fence 預切分結果（純函式，見 [`split_fences`]）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FenceBlock {
    Prose(String),
    Code {
        language: Option<String>,
        content: String,
    },
}

// MarkdownRenderer.buildProse headerScales (system base size 13pt).
fn heading_size(level: u8) -> f32 {
    match level {
        1 => 13.0 * 1.35,
        2 => 13.0 * 1.2,
        3 => 13.0 * 1.1,
        _ => 13.0 * 1.05,
    }
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

// ---------------------------------------------------------------------------
// split_fences：fence 預切分（純函式）
// ---------------------------------------------------------------------------

/// 按行切 fence：開 fence 行（`trim` 後以 ```` ``` ```` 起頭）到閉合行為止原樣收進
/// [`FenceBlock::Code`]，其餘行併成 [`FenceBlock::Prose`]（首尾空白修剪、空段丟棄）。
/// 未閉合 fence 視為 code（與原版 + panel `split_math_segments` 同語義）。
/// 語言標籤保留開 fence 行 ```` ``` ```` 後的完整 hint（與 Swift 相同）。
pub(crate) fn split_fences(text: &str) -> Vec<FenceBlock> {
    fn flush_prose(prose: &mut String, out: &mut Vec<FenceBlock>) {
        let trimmed = prose.trim();
        if !trimmed.is_empty() {
            out.push(FenceBlock::Prose(trimmed.to_string()));
        }
        prose.clear();
    }

    let mut out = Vec::new();
    let mut prose = String::new();
    let mut in_fence = false;
    let mut language: Option<String> = None;
    let mut body: Vec<&str> = Vec::new();
    for line in text.split('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            if !in_fence {
                flush_prose(&mut prose, &mut out);
                in_fence = true;
                language = Some(trimmed[3..].trim().to_string()).filter(|s| !s.is_empty());
                body.clear();
            } else {
                in_fence = false;
                out.push(FenceBlock::Code {
                    language: language.take(),
                    content: body.join("\n"),
                });
            }
            continue;
        }
        if in_fence {
            body.push(line);
        } else {
            if !prose.is_empty() {
                prose.push('\n');
            }
            prose.push_str(line);
        }
    }
    if in_fence {
        out.push(FenceBlock::Code {
            language: language.take(),
            content: body.join("\n"),
        });
    }
    flush_prose(&mut prose, &mut out);
    out
}

// ---------------------------------------------------------------------------
// parse_prose：pulldown 事件 → FlowBlock（純函式）
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum CurKind {
    Para,
    Heading(u8),
    Quote,
    Item {
        depth: u8,
        ordered: bool,
        number: u64,
    },
}

struct Cur {
    kind: CurKind,
    inlines: Vec<Inline>,
}

struct ListFrame {
    ordered: bool,
    next: u64,
}

struct CodeCapture {
    language: Option<String>,
    buf: String,
}

struct Builder {
    blocks: Vec<FlowBlock>,
    open_list: Vec<ListItem>,
    cur: Option<Cur>,
    list_stack: Vec<ListFrame>,
    quote_depth: u32,
    bold: u32,
    italic: u32,
    strike: u32,
    link_stack: Vec<String>,
    image_depth: u32,
    codeblock: Option<CodeCapture>,
}

impl Builder {
    fn new() -> Self {
        Self {
            blocks: Vec::new(),
            open_list: Vec::new(),
            cur: None,
            list_stack: Vec::new(),
            quote_depth: 0,
            bold: 0,
            italic: 0,
            strike: 0,
            link_stack: Vec::new(),
            image_depth: 0,
            codeblock: None,
        }
    }

    fn style(&self) -> SpanStyle {
        SpanStyle {
            bold: self.bold > 0,
            italic: self.italic > 0,
            strike: self.strike > 0,
        }
    }

    fn ensure_cur(&mut self) {
        if self.codeblock.is_some() || self.cur.is_some() {
            return;
        }
        let kind = if self.quote_depth > 0 {
            CurKind::Quote
        } else {
            CurKind::Para
        };
        self.cur = Some(Cur {
            kind,
            inlines: Vec::new(),
        });
    }

    fn push_inline(&mut self, inline: Inline) {
        self.ensure_cur();
        if let Some(cur) = self.cur.as_mut() {
            cur.inlines.push(inline);
        }
    }

    /// prose 文本事件：codeblock 內原樣收走（不解析 markdown），否則按 link/image
    /// 上下文轉成 Text / Link（alt 文本按普通文字）。
    fn text(&mut self, s: String) {
        if let Some(cap) = self.codeblock.as_mut() {
            cap.buf.push_str(&s);
            return;
        }
        self.ensure_cur();
        let style = self.style();
        let inline = if self.image_depth > 0 {
            Inline::Text(s, style)
        } else if let Some(url) = self.link_stack.last() {
            Inline::Link {
                label: s,
                url: url.clone(),
                style,
            }
        } else {
            Inline::Text(s, style)
        };
        if let Some(cur) = self.cur.as_mut() {
            cur.inlines.push(inline);
        }
    }

    fn flush_list(&mut self) {
        if !self.open_list.is_empty() {
            let items = std::mem::take(&mut self.open_list);
            self.blocks.push(FlowBlock::List(items));
        }
    }

    fn finalize_cur(&mut self) {
        let Some(cur) = self.cur.take() else {
            return;
        };
        let inlines = merge_spaces(cur.inlines);
        if !inlines.iter().any(|i| !matches!(i, Inline::Space)) {
            return;
        }
        match cur.kind {
            CurKind::Para => {
                self.flush_list();
                self.blocks.push(FlowBlock::Paragraph(inlines));
            }
            CurKind::Heading(level) => {
                self.flush_list();
                self.blocks.push(FlowBlock::Heading { level, inlines });
            }
            CurKind::Quote => {
                self.flush_list();
                // 同一個 BlockQuote 內多段落合併（段間換行保留）。
                if let Some(FlowBlock::Quote(existing)) = self.blocks.last_mut() {
                    existing.push(Inline::HardBreak);
                    existing.extend(inlines);
                } else {
                    self.blocks.push(FlowBlock::Quote(inlines));
                }
            }
            CurKind::Item {
                depth,
                ordered,
                number,
            } => {
                self.open_list.push(ListItem {
                    depth,
                    ordered,
                    number,
                    inlines,
                });
            }
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => self.ensure_cur(),
            Tag::Heading { level, .. } => {
                self.finalize_cur();
                self.flush_list();
                self.cur = Some(Cur {
                    kind: CurKind::Heading(heading_level(level)),
                    inlines: Vec::new(),
                });
            }
            Tag::BlockQuote(_) => {
                self.finalize_cur();
                self.flush_list();
                self.quote_depth += 1;
            }
            Tag::CodeBlock(kind) => {
                self.finalize_cur();
                self.flush_list();
                let language = match kind {
                    CodeBlockKind::Fenced(info) => info
                        .into_string()
                        .split_whitespace()
                        .next()
                        .map(str::to_string),
                    CodeBlockKind::Indented => None,
                };
                self.codeblock = Some(CodeCapture {
                    language,
                    buf: String::new(),
                });
            }
            Tag::HtmlBlock => {}
            Tag::List(start) => self.list_stack.push(ListFrame {
                ordered: start.is_some(),
                next: start.unwrap_or(1),
            }),
            Tag::Item => {
                if matches!(
                    self.cur,
                    Some(Cur {
                        kind: CurKind::Item { .. },
                        ..
                    })
                ) {
                    self.finalize_cur();
                }
                let depth = self
                    .list_stack
                    .len()
                    .saturating_sub(1)
                    .min(u8::MAX as usize) as u8;
                let (ordered, number) = match self.list_stack.last_mut() {
                    Some(frame) => {
                        let n = if frame.ordered {
                            let n = frame.next;
                            frame.next += 1;
                            n
                        } else {
                            0
                        };
                        (frame.ordered, n)
                    }
                    None => (false, 0),
                };
                self.cur = Some(Cur {
                    kind: CurKind::Item {
                        depth,
                        ordered,
                        number,
                    },
                    inlines: Vec::new(),
                });
            }
            Tag::Emphasis => self.italic += 1,
            Tag::Strong => self.bold += 1,
            Tag::Strikethrough => self.strike += 1,
            Tag::Link { dest_url, .. } => self.link_stack.push(dest_url.into_string()),
            Tag::Image { .. } => self.image_depth += 1,
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => match self.cur {
                // 鬆散列表的 item 內多段落：段間換行，item 到 End(Item) 才結算。
                Some(Cur {
                    kind: CurKind::Item { .. },
                    ..
                }) => self.push_inline(Inline::HardBreak),
                _ => self.finalize_cur(),
            },
            TagEnd::Heading(_) => self.finalize_cur(),
            TagEnd::BlockQuote(_) => {
                self.quote_depth = self.quote_depth.saturating_sub(1);
                self.finalize_cur();
            }
            TagEnd::CodeBlock => {
                if let Some(cap) = self.codeblock.take() {
                    self.blocks.push(FlowBlock::Code {
                        language: cap.language,
                        content: cap.buf.trim_end_matches('\n').to_string(),
                    });
                }
            }
            TagEnd::List(_) => {
                self.list_stack.pop();
                if self.list_stack.is_empty() {
                    self.flush_list();
                }
            }
            TagEnd::Item => self.finalize_cur(),
            TagEnd::Emphasis => self.italic = self.italic.saturating_sub(1),
            TagEnd::Strong => self.bold = self.bold.saturating_sub(1),
            TagEnd::Strikethrough => self.strike = self.strike.saturating_sub(1),
            TagEnd::Link => {
                self.link_stack.pop();
            }
            TagEnd::Image => {
                self.image_depth = self.image_depth.saturating_sub(1);
            }
            _ => {}
        }
    }

    fn finish(mut self) -> Vec<FlowBlock> {
        self.finalize_cur();
        self.flush_list();
        self.blocks
    }
}

/// 把 prose（已剝 fence）解析成塊級片段。只開 `ENABLE_STRIKETHROUGH` +
/// `ENABLE_TASKLISTS`（刻意不開 `ENABLE_MATH`：行內 `$..$` 走 panel 的
/// `split_inline_math` 啟發式，語義與現況一致）。
pub(crate) fn parse_prose(text: &str) -> Vec<FlowBlock> {
    // Swift buildProse extracts math BEFORE markdown parsing; underscores, stars,
    // backslashes and braces inside a formula must never become markdown syntax.
    let (source, math_parts) = protect_inline_math(text);
    let options = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut builder = Builder::new();
    for event in Parser::new_ext(&source, options) {
        match event {
            Event::Start(tag) => builder.start(tag),
            Event::End(tag) => builder.end(tag),
            Event::Text(t) => builder.text(t.into_string()),
            // 行內碼獨立事件：`*` 等語法字元天然不受解析影響。
            Event::Code(c) => {
                if let Some(cap) = builder.codeblock.as_mut() {
                    cap.buf.push_str(&c);
                } else {
                    builder.push_inline(Inline::Code(c.into_string()));
                }
            }
            // ENABLE_MATH 未開，不會出現；留著防禦（同當行內數學渲染）。
            Event::InlineMath(m) | Event::DisplayMath(m) => {
                builder.push_inline(Inline::Math(m.into_string()));
            }
            // HTML 子集外：丟棄標籤本身。
            Event::Html(_) | Event::InlineHtml(_) => {}
            Event::FootnoteReference(name) => {
                builder.text(format!("[^{name}]"));
            }
            Event::SoftBreak => builder.push_inline(Inline::Space),
            Event::HardBreak => builder.push_inline(Inline::HardBreak),
            Event::Rule => {
                builder.finalize_cur();
                builder.flush_list();
                builder.blocks.push(FlowBlock::Rule);
            }
            Event::TaskListMarker(checked) => builder.push_inline(Inline::Text(
                if checked { "☑ " } else { "☐ " }.to_string(),
                SpanStyle::default(),
            )),
        }
    }
    let mut blocks = builder.finish();
    for block in &mut blocks {
        match block {
            FlowBlock::Paragraph(inlines)
            | FlowBlock::Heading { inlines, .. }
            | FlowBlock::Quote(inlines) => {
                *inlines = restore_inline_math(std::mem::take(inlines), &math_parts);
            }
            FlowBlock::List(items) => {
                for item in items {
                    item.inlines =
                        restore_inline_math(std::mem::take(&mut item.inlines), &math_parts);
                }
            }
            FlowBlock::Code { content, .. } => {
                // Indented code is parsed as prose in Swift, rather than as a
                // top-level fence; do not expose our internal placeholder text.
                for (token, latex) in &math_parts {
                    *content = content.replace(token, &format!("${latex}$"));
                }
            }
            FlowBlock::Rule => {}
        }
    }
    if blocks.is_empty() && !text.is_empty() {
        blocks.push(FlowBlock::Paragraph(vec![Inline::Text(
            text.to_owned(),
            SpanStyle::default(),
        )]));
    }
    blocks
}

fn protect_inline_math(text: &str) -> (String, Vec<(String, String)>) {
    let mut source = String::new();
    let mut parts = Vec::new();
    for part in split_inline_math(text) {
        match part {
            InlinePart::Text(text) => source.push_str(text),
            InlinePart::Math(latex) => {
                // Choose a collision-free token even if a user types our marker.
                let mut token = format!("⟦POPMATH{}⟧", parts.len());
                while text.contains(&token) {
                    token.insert('⟦'.len_utf8(), 'P');
                }
                source.push_str(&token);
                parts.push((token, latex.to_owned()));
            }
        }
    }
    (source, parts)
}

fn restore_inline_math(inlines: Vec<Inline>, parts: &[(String, String)]) -> Vec<Inline> {
    let mut result = Vec::new();
    for inline in inlines {
        let text = match &inline {
            Inline::Text(text, _) | Inline::Code(text) => text.as_str(),
            Inline::Link { label, .. } => label.as_str(),
            _ => {
                result.push(inline);
                continue;
            }
        };
        let mut rest = text;
        while !rest.is_empty() {
            let found = parts
                .iter()
                .filter_map(|(token, latex)| rest.find(token).map(|offset| (offset, token, latex)))
                .min_by_key(|(offset, _, _)| *offset);
            let (prefix, found) = match found {
                Some((offset, token, latex)) => (&rest[..offset], Some((offset, token, latex))),
                None => (rest, None),
            };
            if !prefix.is_empty() {
                result.push(match &inline {
                    Inline::Code(_) => Inline::Code(prefix.to_owned()),
                    Inline::Link { url, style, .. } => Inline::Link {
                        label: prefix.to_owned(),
                        url: url.clone(),
                        style: *style,
                    },
                    Inline::Text(_, style) => Inline::Text(prefix.to_owned(), *style),
                    _ => unreachable!(),
                });
            }
            if let Some((offset, token, latex)) = found {
                result.push(Inline::Math(latex.clone()));
                rest = &rest[offset + token.len()..];
            } else {
                break;
            }
        }
    }
    result
}

/// `SoftBreak` 合併：接到前一段文字尾（無前段/重複時丟棄），避免渲染時多出空格 label。
fn merge_spaces(inlines: Vec<Inline>) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::with_capacity(inlines.len());
    for inline in inlines {
        if matches!(inline, Inline::Space) {
            match out.last_mut() {
                Some(Inline::Text(s, _)) => s.push(' '),
                Some(Inline::Link { label, .. }) => label.push(' '),
                Some(Inline::Space) | Some(Inline::HardBreak) | None => {}
                _ => out.push(Inline::Space),
            }
        } else {
            out.push(inline);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// highlight：手寫三色掃描器（純函式，不加 syntect）
// ---------------------------------------------------------------------------

/// 高亮種類：只有三色 + 原文（行號/更多 token 类型不在子集內）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HlKind {
    Plain,
    Keyword,
    Str,
    Comment,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HlSpan {
    pub text: String,
    pub kind: HlKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodeLang {
    Rust,
    Python,
    Swift,
    Plain,
}

fn classify_lang(language: &str) -> CodeLang {
    match language.trim().to_ascii_lowercase().as_str() {
        "rust" | "rs" => CodeLang::Rust,
        "python" | "py" | "pyi" => CodeLang::Python,
        "swift" => CodeLang::Swift,
        _ => CodeLang::Plain,
    }
}

const RUST_KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
    "ref", "return", "self", "Self", "static", "struct", "trait", "true", "type", "unsafe", "use",
    "where", "while", "try", "union", "bool", "str", "char", "i8", "i16", "i32", "i64", "i128",
    "u8", "u16", "u32", "u64", "u128", "f32", "f64", "usize", "isize", "String", "Vec", "Option",
    "Result", "Some", "None", "Ok", "Err",
];

const PYTHON_KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "case", "class",
    "continue", "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if",
    "import", "in", "is", "lambda", "match", "nonlocal", "not", "or", "pass", "raise", "return",
    "try", "while", "with", "yield", "self",
];

const SWIFT_KEYWORDS: &[&str] = &[
    "actor",
    "as",
    "associatedtype",
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "continue",
    "convenience",
    "default",
    "defer",
    "deinit",
    "do",
    "dynamic",
    "else",
    "enum",
    "extension",
    "fallthrough",
    "false",
    "fileprivate",
    "final",
    "for",
    "func",
    "guard",
    "if",
    "import",
    "in",
    "indirect",
    "init",
    "inout",
    "internal",
    "is",
    "lazy",
    "let",
    "mutating",
    "nil",
    "nonisolated",
    "open",
    "optional",
    "override",
    "prefix",
    "private",
    "protocol",
    "public",
    "repeat",
    "required",
    "rethrows",
    "return",
    "self",
    "Self",
    "some",
    "static",
    "struct",
    "subscript",
    "super",
    "switch",
    "throw",
    "throws",
    "true",
    "try",
    "typealias",
    "unowned",
    "var",
    "weak",
    "where",
    "while",
];

fn lang_keywords(lang: CodeLang) -> &'static [&'static str] {
    match lang {
        CodeLang::Rust => RUST_KEYWORDS,
        CodeLang::Python => PYTHON_KEYWORDS,
        CodeLang::Swift => SWIFT_KEYWORDS,
        CodeLang::Plain => &[],
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn push_span(spans: &mut Vec<HlSpan>, buf: &mut String, kind: HlKind) {
    if buf.is_empty() {
        return;
    }
    let text = std::mem::take(buf);
    match spans.last_mut() {
        Some(last) if last.kind == kind => last.text.push_str(&text),
        _ => spans.push(HlSpan { text, kind }),
    }
}

/// 單行高亮。`in_block_comment` 跨行傳遞（rust/swift 的 `/* … */`）；
/// 未閉合的字串按「到行尾都是字串」處理（streaming 中間態常見）。
pub(crate) fn highlight_line(
    line: &str,
    language: &str,
    in_block_comment: &mut bool,
) -> Vec<HlSpan> {
    let lang = classify_lang(language);
    if lang == CodeLang::Plain {
        return vec![HlSpan {
            text: line.to_string(),
            kind: HlKind::Plain,
        }];
    }
    let keywords = lang_keywords(lang);
    let chars: Vec<char> = line.chars().collect();
    let mut spans: Vec<HlSpan> = Vec::new();
    let mut buf = String::new();
    let mut buf_kind = HlKind::Plain;

    // 上一行沒閉合的 /* … */：先找本行 */，找不到則整行都是註解。
    let mut i = 0;
    if *in_block_comment {
        let mut closed = None;
        let mut j = 0;
        while j + 1 < chars.len() {
            if chars[j] == '*' && chars[j + 1] == '/' {
                closed = Some(j + 2);
                break;
            }
            j += 1;
        }
        match closed {
            Some(end) => {
                spans.push(HlSpan {
                    text: chars[..end].iter().collect(),
                    kind: HlKind::Comment,
                });
                i = end;
                *in_block_comment = false;
            }
            None => {
                return vec![HlSpan {
                    text: line.to_string(),
                    kind: HlKind::Comment,
                }];
            }
        }
    }

    // 普通字元進 buf（必要時先把 buf 按舊 kind 落袋）。
    while i < chars.len() {
        let c = chars[i];
        // rust/swift 行註解與塊註解。
        if lang != CodeLang::Python && c == '/' && i + 1 < chars.len() {
            if chars[i + 1] == '/' {
                push_span(&mut spans, &mut buf, buf_kind);
                buf_kind = HlKind::Plain;
                spans.push(HlSpan {
                    text: chars[i..].iter().collect(),
                    kind: HlKind::Comment,
                });
                break;
            }
            if chars[i + 1] == '*' {
                push_span(&mut spans, &mut buf, buf_kind);
                buf_kind = HlKind::Plain;
                let mut j = i + 2;
                let mut closed = None;
                while j + 1 < chars.len() {
                    if chars[j] == '*' && chars[j + 1] == '/' {
                        closed = Some(j + 2);
                        break;
                    }
                    j += 1;
                }
                match closed {
                    Some(end) => {
                        spans.push(HlSpan {
                            text: chars[i..end].iter().collect(),
                            kind: HlKind::Comment,
                        });
                        i = end;
                    }
                    None => {
                        spans.push(HlSpan {
                            text: chars[i..].iter().collect(),
                            kind: HlKind::Comment,
                        });
                        *in_block_comment = true;
                        break;
                    }
                }
                continue;
            }
        }
        // python 行註解（字串已在下方貪婪消費，走不到這裡的 # 即程式碼區）。
        if lang == CodeLang::Python && c == '#' {
            push_span(&mut spans, &mut buf, buf_kind);
            buf_kind = HlKind::Plain;
            spans.push(HlSpan {
                text: chars[i..].iter().collect(),
                kind: HlKind::Comment,
            });
            break;
        }
        // 雙引號字串（python 的 """ 單行配對會自然全染成字串，可接受）。
        if c == '"' || (lang == CodeLang::Python && c == '\'') {
            push_span(&mut spans, &mut buf, buf_kind);
            buf_kind = HlKind::Plain;
            let quote = c;
            let mut j = i + 1;
            while j < chars.len() {
                if chars[j] == '\\' {
                    j = (j + 2).min(chars.len());
                    continue;
                }
                if chars[j] == quote {
                    j += 1;
                    break;
                }
                j += 1;
            }
            spans.push(HlSpan {
                text: chars[i..j].iter().collect(),
                kind: HlKind::Str,
            });
            i = j;
            continue;
        }
        // rust/swift 字元字面量 'x' / '\n'；其餘單引號（lifetime 等）按原文。
        if (lang == CodeLang::Rust || lang == CodeLang::Swift) && c == '\'' {
            let is_char = (i + 2 < chars.len() + 1)
                && chars.get(i + 1).is_some_and(|&q| q != '\'' && q != '\\')
                && chars.get(i + 2) == Some(&'\'')
                || chars.get(i + 1) == Some(&'\\')
                    && chars.get(i + 2).is_some_and(|q| *q != '\'')
                    && chars.get(i + 3) == Some(&'\'');
            if is_char {
                push_span(&mut spans, &mut buf, buf_kind);
                buf_kind = HlKind::Plain;
                let end = if chars.get(i + 1) == Some(&'\\') {
                    i + 4
                } else {
                    i + 3
                };
                spans.push(HlSpan {
                    text: chars[i..end.min(chars.len())].iter().collect(),
                    kind: HlKind::Str,
                });
                i = end.min(chars.len());
                continue;
            }
        }
        // 單詞：關鍵字表命中才上色。
        if is_word_char(c) {
            let mut j = i + 1;
            while j < chars.len() && is_word_char(chars[j]) {
                j += 1;
            }
            let word: String = chars[i..j].iter().collect();
            let target = if keywords.contains(&word.as_str()) {
                HlKind::Keyword
            } else {
                HlKind::Plain
            };
            if buf_kind != target {
                push_span(&mut spans, &mut buf, buf_kind);
                buf_kind = target;
            }
            buf.push_str(&word);
            i = j;
            continue;
        }
        if buf_kind != HlKind::Plain {
            push_span(&mut spans, &mut buf, buf_kind);
            buf_kind = HlKind::Plain;
        }
        buf.push(c);
        i += 1;
    }
    push_span(&mut spans, &mut buf, buf_kind);
    spans
}

// ---------------------------------------------------------------------------
// 渲染層
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
struct ProseCopyDocument {
    /// egui's geometric concatenation, and byte ranges of its EXTRA newlines.
    geometric: String,
    extra_newlines: Vec<usize>,
    previous_rect: Option<egui::Rect>,
}

impl ProseCopyDocument {
    fn push(&mut self, pos: egui::Pos2, galley: &egui::Galley) {
        let rect = egui::Rect::from_min_size(pos, galley.size());
        if let Some(previous) = self.previous_rect {
            if previous.bottom() <= rect.top() {
                self.geometric.push('\n');
                let row_height = galley
                    .rows
                    .first()
                    .map_or(galley.size().y, |row| row.height());
                if row_height * 0.5 < rect.top() - previous.bottom() {
                    self.extra_newlines.push(self.geometric.len());
                    self.geometric.push('\n');
                }
            } else if !self.geometric.ends_with(char::is_whitespace)
                && !galley
                    .text()
                    .starts_with(|c: char| c.is_whitespace() || c.is_ascii_punctuation())
            {
                // Match egui's copy_text joining rule, not the visual spacing.
                self.geometric.push(' ');
            }
        }
        self.geometric.push_str(galley.text());
        self.previous_rect = Some(rect);
    }

    fn normalize(&self, copied: &str) -> Option<String> {
        if copied.is_empty() {
            return None;
        }
        let mut normalized = None;
        for (start, _) in self.geometric.match_indices(copied) {
            let mut text = copied.to_owned();
            for &extra in self.extra_newlines.iter().rev() {
                if start <= extra && extra < start + copied.len() {
                    text.remove(extra - start);
                }
            }
            // Repeated text with ambiguous provenance must not rewrite a literal
            // blank line. Only fix separators when every matching range agrees.
            if normalized.as_ref().is_some_and(|old| old != &text) {
                return None;
            }
            normalized = Some(text);
        }
        normalized
    }
}

fn prose_copy_key(ctx: &egui::Context) -> egui::Id {
    egui::Id::new((ctx.viewport_id(), "markdown-prose-copy"))
}

/// egui's selection still determines the exact selected characters and emits the
/// real CopyText command. Its geometry-based extra paragraph newline is the only
/// thing rewritten, using the very same production galleys. Literal body newlines
/// (notably in code/pasteables) are never collapsed.
fn prose_selection_island(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    let installed = egui::Id::new("markdown-prose-copy-hook-installed");
    if !ui
        .ctx()
        .data(|data| data.get_temp::<bool>(installed).unwrap_or(false))
    {
        ui.ctx().data_mut(|data| data.insert_temp(installed, true));
        ui.ctx().on_end_pass(
            "markdown prose copy separators",
            std::sync::Arc::new(|ctx| {
                let key = prose_copy_key(ctx).with("pending");
                let document = ctx.data_mut(|data| data.remove_temp::<ProseCopyDocument>(key));
                if let Some(document) = document {
                    ctx.output_mut(|output| {
                        for command in &mut output.commands {
                            if let egui::OutputCommand::CopyText(text) = command {
                                if let Some(normalized) = document.normalize(text) {
                                    *text = normalized;
                                }
                            }
                        }
                    });
                }
            }),
        );
    }
    selection_island(ui, |ui| {
        let copying = ui.style().interaction.selectable_labels
            && ui.input(|i| {
                i.events
                    .iter()
                    .any(|event| matches!(event, egui::Event::Copy | egui::Event::Cut))
            });
        let key = prose_copy_key(ui.ctx());
        if copying {
            ui.ctx()
                .data_mut(|data| data.insert_temp(key, ProseCopyDocument::default()));
        }
        add(ui);
        if copying {
            ui.ctx().data_mut(|data| {
                if let Some(document) = data.remove_temp::<ProseCopyDocument>(key) {
                    data.insert_temp(key.with("pending"), document);
                }
            });
        }
    });
}

/// NSTextField selection cannot leave its source view. egui's label selection is
/// viewport-wide, so only the island where the press began participates until a
/// new press. Previous-frame bounds are used before any of its labels are drawn.
/// This preserves the existing renderer/layout and egui's actual Copy handling.
fn selection_island(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    let id = ui.next_auto_id().with("markdown-selection-island");
    let active_key = egui::Id::new((ui.ctx().viewport_id(), "markdown-active-selection-island"));
    let bounds_key = id.with("bounds");
    if ui.input(|i| i.pointer.any_pressed()) {
        let press = ui.input(|i| i.pointer.press_origin());
        let contains_press = ui.ctx().data(|data| {
            data.get_temp::<egui::Rect>(bounds_key)
                .zip(press)
                .is_some_and(|(rect, pos)| rect.contains(pos))
        });
        if contains_press {
            ui.ctx().data_mut(|data| data.insert_temp(active_key, id));
        }
    }
    let active = ui.ctx().data(|data| data.get_temp::<egui::Id>(active_key));
    let response = ui.scope(|ui| {
        ui.style_mut().interaction.selectable_labels = active.is_none_or(|active| active == id);
        ui.style_mut().interaction.multi_widget_text_select = true;
        add(ui);
    });
    ui.ctx()
        .data_mut(|data| data.insert_temp(bounds_key, response.response.rect));
}

/// 渲染入口：prose（panel 已剝 fence/`$$`，但這裡自己再切一次 fence 以策安全，
/// 殘留 fence 照樣是 code 塊）→ pulldown 排版 → 行內公式混排。
pub fn show_prose(ui: &mut egui::Ui, text: &str, math: &mut MathTextures) {
    if text.trim().is_empty() {
        return;
    }
    for block in split_fences(text) {
        match block {
            FenceBlock::Code { language, content } => {
                show_code_block(ui, language.as_deref(), &content);
            }
            FenceBlock::Prose(prose) => {
                prose_selection_island(ui, |ui| {
                    for flow in parse_prose(&prose) {
                        show_flow_block(ui, math, flow);
                    }
                });
            }
        }
    }
}

/// QuoteBlockView: markdown-aware text with a secondary tone and a 2pt rule.
pub(crate) fn show_quote(ui: &mut egui::Ui, text: &str, math: &mut MathTextures) {
    prose_selection_island(ui, |ui| {
        ui.horizontal_top(|ui| {
            ui.add_space(10.0);
            let color = ui.visuals().weak_text_color();
            let response = ui
                .vertical(|ui| {
                    ui.visuals_mut().override_text_color = Some(color);
                    for block in parse_prose(text) {
                        show_flow_block(ui, math, block);
                    }
                })
                .response;
            let x = response.rect.left() - 8.0;
            ui.painter().line_segment(
                [
                    egui::pos2(x, response.rect.top()),
                    egui::pos2(x, response.rect.bottom()),
                ],
                egui::Stroke::new(2.0_f32, color),
            );
        });
        ui.add_space(8.0);
    });
}

/// Table cells use inline-only markdown, with no math extraction (tableCell in
/// Swift does not call buildProse). A leading sentinel disables block syntax.
fn table_cell_inlines(text: &str) -> Vec<Inline> {
    let source = format!("\u{2060}{text}");
    let mut builder = Builder::new();
    for event in Parser::new_ext(&source, Options::ENABLE_STRIKETHROUGH) {
        match event {
            Event::Start(tag) => builder.start(tag),
            Event::End(tag) => builder.end(tag),
            Event::Text(text) => builder.text(text.into_string()),
            Event::Code(text) => builder.push_inline(Inline::Code(text.into_string())),
            Event::SoftBreak | Event::HardBreak => builder.text("\n".to_owned()),
            _ => {}
        }
    }
    let mut result = Vec::new();
    for block in builder.finish() {
        if let FlowBlock::Paragraph(inlines) = block {
            result.extend(inlines);
        }
    }
    if let Some(Inline::Text(text, _)) = result.first_mut() {
        if text.starts_with('\u{2060}') {
            text.remove(0);
        }
    }
    result
}

/// TableBlockView: header-first grid, inline markdown, selectable cells. Keep
/// ragged rows (including extra cells), just as the original Grid does.
pub(crate) fn show_table(ui: &mut egui::Ui, header: &[String], rows: &[Vec<String>]) {
    let columns = rows
        .iter()
        .map(Vec::len)
        .chain([header.len()])
        .max()
        .unwrap_or(1)
        .max(1);
    let width = ((ui.available_width() - 20.0 * columns as f32) / columns as f32).max(80.0);
    egui::ScrollArea::horizontal()
        .id_salt(ui.next_auto_id())
        .show(ui, |ui| {
            egui::Grid::new(ui.next_auto_id().with("markdown-table"))
                .spacing(egui::vec2(20.0, 10.0))
                .striped(true)
                .show(ui, |ui| {
                    for (row_index, row) in std::iter::once(header)
                        .chain(rows.iter().map(Vec::as_slice))
                        .enumerate()
                    {
                        for cell in row {
                            show_table_cell(ui, cell, width, row_index == 0);
                        }
                        ui.end_row();
                    }
                });
        });
    ui.add_space(8.0);
}

fn show_table_cell(ui: &mut egui::Ui, cell: &str, width: f32, header: bool) {
    selection_island(ui, |ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(width, 0.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_max_width(width);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    let mut job = egui::text::LayoutJob::default();
                    let mut links = Vec::new();
                    for inline in table_cell_inlines(cell) {
                        let (text, strike, link) = match &inline {
                            Inline::Text(text, style) => (text, style.strike, None),
                            Inline::Link { label, style, url } => (label, style.strike, Some(url)),
                            Inline::Code(text) => (text, false, None),
                            _ => continue,
                        };
                        // tableCell resets parsed fonts to 12pt; links and strike remain.
                        let rich = styled_rich(
                            text,
                            SpanStyle {
                                strike,
                                ..Default::default()
                            },
                            Some(12.0),
                            false,
                            header,
                            ui,
                        );
                        let rich = if let Some(url) = link {
                            let start = job.text.chars().count();
                            links.push((start..start + text.chars().count(), url.clone()));
                            rich.color(ui.visuals().hyperlink_color)
                        } else {
                            rich
                        };
                        rich.append_to(
                            &mut job,
                            ui.style(),
                            egui::FontSelection::Default,
                            egui::Align::Center,
                        );
                    }
                    if !job.text.is_empty() {
                        show_text_job(ui, job, &links);
                    }
                });
            },
        );
    });
}

/// PasteableBlockView: a lifted, titled card. Both display and clipboard receive
/// the exact captured body, without markdown, math or fence interpretation.
pub(crate) fn show_pasteable(ui: &mut egui::Ui, title: Option<&str>, content: &str) {
    selection_island(ui, |ui| {
        let dark = ui.visuals().dark_mode;
        let fill = if dark {
            egui::Color32::from_rgb(52, 55, 64)
        } else {
            super::theme::LIGHT_RAISED
        };
        egui::Frame::new()
            .fill(fill)
            .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
            .corner_radius(12)
            .inner_margin(egui::Margin::symmetric(12, 10))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.add(egui::Label::new(egui::RichText::new("▣").weak()).selectable(false));
                    // Reserve the action's width before letting a long title grow.
                    let title_width = (ui.available_width() - 80.0).max(0.0);
                    ui.add_sized(
                        [title_width, 18.0],
                        egui::Label::new(
                            egui::RichText::new(title.unwrap_or("Pasteable"))
                                .size(11.5)
                                .weak(),
                        )
                        .truncate()
                        .selectable(false),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        copy_button(ui, content);
                    });
                });
                ui.separator();
                ui.add_space(4.0);
                show_text_label(
                    ui,
                    egui::Label::new(egui::RichText::new(content).size(12.5)).wrap(),
                    &[],
                );
            });
        ui.add_space(8.0);
    });
}

fn show_flow_block(ui: &mut egui::Ui, math: &mut MathTextures, block: FlowBlock) {
    match block {
        FlowBlock::Heading { level, inlines } => {
            show_inline_rows(ui, math, &inlines, Some(heading_size(level)), false, true);
            ui.add_space(8.0);
        }
        FlowBlock::Paragraph(inlines) => {
            show_inline_rows(ui, math, &inlines, None, false, false);
            ui.add_space(9.0);
        }
        FlowBlock::Quote(inlines) => {
            ui.horizontal_top(|ui| {
                ui.add_space(2.0);
                let weak_col = ui.visuals().weak_text_color();
                let inner = ui
                    .vertical(|ui| {
                        show_inline_rows(ui, math, &inlines, None, true, false);
                    })
                    .response;
                let rect = inner.rect;
                if rect.height() > 1.0 {
                    let x = rect.left() - 4.0;
                    ui.painter().line_segment(
                        [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                        egui::Stroke::new(2.0_f32, weak_col),
                    );
                }
            });
            ui.add_space(9.0);
        }
        FlowBlock::List(items) => {
            for item in &items {
                ui.horizontal_top(|ui| {
                    ui.add_space(item.depth as f32 * 16.0);
                    let prefix = if item.ordered {
                        format!("{}. ", item.number)
                    } else {
                        "•  ".to_string()
                    };
                    let mut job = egui::text::LayoutJob::default();
                    egui::RichText::new(prefix).append_to(
                        &mut job,
                        ui.style(),
                        egui::FontSelection::Default,
                        egui::Align::Center,
                    );
                    show_text_job(ui, job, &[]);
                    ui.vertical(|ui| {
                        show_inline_rows(ui, math, &item.inlines, None, false, false);
                    });
                });
            }
            ui.add_space(9.0);
        }
        FlowBlock::Code { language, content } => {
            // Indented code belongs to the surrounding Swift prose text field;
            // only top-level fenced segments get their own selection island.
            show_code_block_body(ui, language.as_deref(), &content);
        }
        FlowBlock::Rule => {
            ui.add(egui::Separator::default().horizontal());
            ui.add_space(2.0);
        }
    }
}

/// `HardBreak` 切行，每行一個 `horizontal_wrapped`（空行跳過）。
fn show_inline_rows(
    ui: &mut egui::Ui,
    math: &mut MathTextures,
    inlines: &[Inline],
    size: Option<f32>,
    weak: bool,
    bold_all: bool,
) {
    let mut start = 0;
    for (idx, inline) in inlines.iter().enumerate() {
        if matches!(inline, Inline::HardBreak) {
            show_inline_row(ui, math, &inlines[start..idx], size, weak, bold_all);
            start = idx + 1;
        }
    }
    show_inline_row(ui, math, &inlines[start..], size, weak, bold_all);
}

fn show_inline_row(
    ui: &mut egui::Ui,
    math: &mut MathTextures,
    row: &[Inline],
    size: Option<f32>,
    weak: bool,
    bold_all: bool,
) {
    if !row.iter().any(|i| !matches!(i, Inline::Space)) {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        // Text fragments already carry their own whitespace. Standard widget
        // spacing creates conspicuous holes around bold/linked CJK fragments.
        ui.spacing_mut().item_spacing.x = 0.0;
        let mut job = egui::text::LayoutJob::default();
        let mut links = Vec::new();
        for inline in row {
            let rich = match inline {
                Inline::Text(text, style) => {
                    Some(styled_rich(text, *style, size, weak, bold_all, ui))
                }
                Inline::Code(text) => Some(
                    egui::RichText::new(text)
                        .monospace()
                        .background_color(ui.visuals().code_bg_color),
                ),
                Inline::Space => Some(egui::RichText::new(" ")),
                Inline::Link { label, url, style } => {
                    let start = job.text.chars().count();
                    links.push((start..start + label.chars().count(), url.clone()));
                    Some(
                        styled_rich(label, *style, size, false, bold_all, ui)
                            .color(ui.visuals().hyperlink_color),
                    )
                }
                _ => None,
            };
            if let Some(rich) = rich {
                rich.append_to(
                    &mut job,
                    ui.style(),
                    egui::FontSelection::Default,
                    egui::Align::Center,
                );
            } else {
                if !job.text.is_empty() {
                    show_text_job(ui, std::mem::take(&mut job), &links);
                    links.clear();
                }
                show_one_inline(ui, math, inline, size, weak, bold_all);
            }
        }
        if !job.text.is_empty() {
            show_text_job(ui, job, &links);
        }
    });
}

/// Keep links in the same selectable galley as their surrounding styled text.
/// Hit-testing uses that exact Label layout (including wrapped rows), not a
/// second invisible widget. Only an actual click opens a URL, never a drag.
fn show_text_job(
    ui: &mut egui::Ui,
    job: egui::text::LayoutJob,
    links: &[(std::ops::Range<usize>, String)],
) {
    show_text_label(ui, egui::Label::new(job).wrap(), links);
}

fn show_text_label(
    ui: &mut egui::Ui,
    label: egui::Label,
    links: &[(std::ops::Range<usize>, String)],
) {
    let selectable = ui.style().interaction.selectable_labels;
    // Keep sensing presses even while another island owns selection. egui hit
    // tests against the PREVIOUS frame's widgets, before we activate this island.
    let sense = if links.is_empty() {
        egui::Sense::click_and_drag() - egui::Sense::FOCUSABLE
    } else {
        egui::Sense::click_and_drag()
    };
    let label = label.sense(sense);
    let (pos, galley, response) = label.layout_in_ui(ui);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Label, ui.is_enabled(), galley.text())
    });
    let copy_key = prose_copy_key(ui.ctx());
    ui.ctx().data_mut(|data| {
        if let Some(mut document) = data.remove_temp::<ProseCopyDocument>(copy_key) {
            document.push(pos, &galley);
            data.insert_temp(copy_key, document);
        }
    });
    if !ui.is_rect_visible(response.rect) {
        return;
    }
    let url = ui.input(|i| i.pointer.hover_pos()).and_then(|pointer| {
        if !response.contains_pointer() {
            return None;
        }
        let pointer = pointer - pos;
        let mut index = 0;
        let mut hit = None;
        for row in &galley.rows {
            for (column, glyph) in row.glyphs.iter().enumerate() {
                let right = row
                    .glyphs
                    .get(column + 1)
                    .map_or(glyph.max_x(), |next| next.pos.x);
                // A short font can sit inside a taller line (table rows, mixed
                // inline code, etc.). The whole line-height band is clickable.
                if row.rect().y_range().contains(pointer.y)
                    && (row.pos.x + glyph.pos.x..row.pos.x + right).contains(&pointer.x)
                {
                    hit = Some(index + column);
                    break;
                }
            }
            if hit.is_some() {
                break;
            }
            index += row.char_count_including_newline();
        }
        links
            .iter()
            .find(|(range, _)| hit.is_some_and(|index| range.contains(&index)))
            .map(|(_, url)| url)
    });
    if let Some(url) = url {
        let _ = response.clone().on_hover_text(url);
        if response.clicked() {
            ui.ctx().open_url(egui::OpenUrl::new_tab(url));
        }
    }
    let color = ui.visuals().text_color();
    if selectable {
        egui::text_selection::LabelSelectionState::label_text_selection(
            ui,
            &response,
            pos,
            galley,
            color,
            egui::Stroke::NONE,
        );
    } else {
        ui.painter().galley(pos, galley, color);
    }
    if url.is_some() && response.hovered() && !ui.input(|i| i.pointer.any_down()) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
}

fn styled_rich(
    text: &str,
    style: SpanStyle,
    size: Option<f32>,
    weak: bool,
    bold_all: bool,
    ui: &egui::Ui,
) -> egui::RichText {
    let mut rich = egui::RichText::new(text);
    if style.bold || bold_all {
        rich = rich.strong();
    }
    if style.italic {
        rich = rich.italics();
    }
    // 刪除線是繪製效果（非字型），egui 恆有支援；原版退回普通的要求自然滿足。
    if style.strike {
        rich = rich.strikethrough();
    }
    if let Some(s) = size {
        rich = rich.size(s);
    }
    if weak {
        rich = rich.color(ui.visuals().weak_text_color());
    }
    rich
}

fn show_one_inline(
    ui: &mut egui::Ui,
    math: &mut MathTextures,
    inline: &Inline,
    size: Option<f32>,
    weak: bool,
    bold_all: bool,
) {
    match inline {
        Inline::Text(s, style) => {
            // parse 後不應再有 $…$（已拆成 Math），這裡是雙保險。
            for part in split_inline_math(s) {
                match part {
                    InlinePart::Text(t) => {
                        ui.add(egui::Label::new(styled_rich(
                            t, *style, size, weak, bold_all, ui,
                        )));
                    }
                    InlinePart::Math(latex) => show_inline_math(ui, math, latex),
                }
            }
        }
        Inline::Math(latex) => show_inline_math(ui, math, latex),
        Inline::Code(s) => {
            ui.add(egui::Label::new(
                egui::RichText::new(s)
                    .monospace()
                    .background_color(ui.visuals().code_bg_color),
            ));
        }
        Inline::Link { label, url, style } => {
            let mut job = egui::text::LayoutJob::default();
            styled_rich(label, *style, size, false, bold_all, ui)
                .color(ui.visuals().hyperlink_color)
                .append_to(
                    &mut job,
                    ui.style(),
                    egui::FontSelection::Default,
                    egui::Align::Center,
                );
            show_text_job(ui, job, &[(0..label.chars().count(), url.clone())]);
        }
        Inline::Space => {
            ui.label(" ");
        }
        Inline::HardBreak => {}
    }
}

/// 行內公式：照抄 panel 私有 `show_inline_math`（只用公開 [`MathTextures::texture_for`]）；
/// 失敗退回 `$latex$` 等寬原文，不擋主流程。
fn show_inline_math(ui: &mut egui::Ui, math: &mut MathTextures, latex: &str) {
    let tint = ui.visuals().text_color();
    let Some(handle) = math.texture_for(ui.ctx(), latex, INLINE_MATH_FONT_SIZE, false) else {
        show_text_label(
            ui,
            egui::Label::new(egui::RichText::new(format!("${latex}$")).monospace()),
            &[],
        );
        return;
    };
    let size = handle.size_vec2();
    if size.x <= 0.0 || size.y <= 0.0 {
        show_text_label(
            ui,
            egui::Label::new(egui::RichText::new(format!("${latex}$")).monospace()),
            &[],
        );
        return;
    }
    let height = (INLINE_MATH_FONT_SIZE * 1.35).ceil();
    let width = (height * size.x / size.y).min(ui.available_width().max(16.0));
    ui.add(
        egui::Image::from_texture(&handle)
            .fit_to_exact_size(egui::vec2(width.max(4.0), height))
            .tint(tint),
    );
}

// 複製按鈕狀態：上次複製成功的時間（秒，egui 輸入時鐘）；1.5s 後恢復按鈕文案。
const COPY_FEEDBACK_SECS: f64 = 1.5;
const CODE_FONT_SIZE: f32 = 13.0;

fn copy_button(ui: &mut egui::Ui, content: &str) -> egui::Response {
    // Auto-id includes the card's position, so duplicate bodies do not share
    // transient feedback. No OS clipboard access is needed in headless tests.
    let id = ui.next_auto_id().with("markdown-copy-feedback");
    let now = ui.input(|i| i.time);
    let copied = ui
        .ctx()
        .data(|data| data.get_temp::<f64>(id))
        .is_some_and(|time| now - time < COPY_FEEDBACK_SECS);
    let response = ui
        .small_button(if copied { "Copied" } else { "Copy" })
        .on_hover_text("Copy this block");
    if response.clicked() {
        ui.ctx().copy_text(content.to_owned());
        ui.ctx().data_mut(|data| data.insert_temp(id, now));
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs_f64(COPY_FEEDBACK_SECS));
    }
    response
}

fn hl_color(kind: HlKind, dark: bool, visuals: &egui::Visuals) -> egui::Color32 {
    use egui::Color32;
    match kind {
        HlKind::Plain => visuals.text_color(),
        // VS Dark / Light 經典三色。
        HlKind::Keyword => {
            if dark {
                Color32::from_rgb(86, 156, 214)
            } else {
                Color32::from_rgb(0, 0, 255)
            }
        }
        HlKind::Str => {
            if dark {
                Color32::from_rgb(206, 145, 120)
            } else {
                Color32::from_rgb(163, 21, 21)
            }
        }
        HlKind::Comment => {
            if dark {
                Color32::from_rgb(106, 153, 85)
            } else {
                // The stock light-theme green drops below 4.5:1 on cream.
                Color32::from_rgb(0, 112, 0)
            }
        }
    }
}

/// code fence 塊：語言標籤 + 複製按鈕（`ctx.copy_text` 走 `output_mut` 剪貼簿，
/// 按鈕文案「Copied」1.5s 即成功提示）+ 橫向滾動的高亮等寬正文。
pub(crate) fn show_code_block(ui: &mut egui::Ui, language: Option<&str>, content: &str) {
    selection_island(ui, |ui| show_code_block_body(ui, language, content));
}

fn show_code_block_body(ui: &mut egui::Ui, language: Option<&str>, content: &str) {
    let lang = language.filter(|s| !s.is_empty()).unwrap_or("code");
    let frame = egui::Frame {
        fill: ui.visuals().code_bg_color,
        corner_radius: egui::CornerRadius::same(10),
        inner_margin: egui::Margin::symmetric(14, 12),
        ..Default::default()
    };
    frame.show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.add(egui::Label::new(egui::RichText::new(lang).small().weak()).selectable(false));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                copy_button(ui, content);
            });
        });
        ui.separator();
        ui.add_space(5.0);
        egui::ScrollArea::horizontal().show(ui, |ui| {
            let visuals = ui.visuals().clone();
            let dark = visuals.dark_mode;
            let mut in_block_comment = false;
            let mut job = egui::epaint::text::LayoutJob::default();
            for (index, line) in content.split('\n').enumerate() {
                if index > 0 {
                    job.append(
                        "\n",
                        0.0,
                        egui::TextFormat {
                            font_id: egui::FontId::monospace(CODE_FONT_SIZE),
                            color: visuals.text_color(),
                            ..Default::default()
                        },
                    );
                }
                let spans = highlight_line(
                    line,
                    lang.split_whitespace().next().unwrap_or(lang),
                    &mut in_block_comment,
                );
                for span in &spans {
                    job.append(
                        &span.text,
                        0.0,
                        egui::epaint::text::TextFormat {
                            font_id: egui::FontId::monospace(CODE_FONT_SIZE),
                            color: hl_color(span.kind, dark, &visuals),
                            ..Default::default()
                        },
                    );
                }
            }
            show_text_label(ui, egui::Label::new(job).extend(), &[]);
        });
    });
}

// ---------------------------------------------------------------------------
// 測試
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn light_syntax_colors_remain_readable_on_cream_code_surface() {
        let luminance = |color: egui::Color32| {
            let linear = |value: u8| {
                let v = f64::from(value) / 255.0;
                if v <= 0.04045 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * linear(color.r()) + 0.7152 * linear(color.g()) + 0.0722 * linear(color.b())
        };
        let ctx = egui::Context::default();
        super::super::theme::apply(&ctx, "Light", super::super::theme::DEFAULT_ACCENT_HEX);
        let style = ctx.style();
        let visuals = &style.visuals;
        for kind in [HlKind::Plain, HlKind::Keyword, HlKind::Str, HlKind::Comment] {
            let foreground = hl_color(kind, false, visuals);
            let a = luminance(foreground);
            let b = luminance(visuals.code_bg_color);
            let ratio = (a.max(b) + 0.05) / (a.min(b) + 0.05);
            assert!(ratio >= 4.5, "code foreground {foreground:?}: {ratio}");
        }
    }

    #[test]
    fn swift_prose_math_is_protected_before_emphasis_and_links() {
        assert_eq!(
            paragraph_inlines(parse_prose("$a_b_c$")),
            vec![Inline::Math("a_b_c".into())]
        );
        assert_eq!(
            paragraph_inlines(parse_prose("$a*b*c$")),
            vec![Inline::Math("a*b*c".into())]
        );
        assert_eq!(
            paragraph_inlines(parse_prose("**$x_1$**")),
            vec![Inline::Math("x_1".into())]
        );
        let inlines = paragraph_inlines(parse_prose("⟦POPMATH0⟧ $x$"));
        assert!(matches!(&inlines[0], Inline::Text(text, _) if text == "⟦POPMATH0⟧ "));
        assert!(matches!(&inlines[1], Inline::Math(text) if text == "x"));
    }

    #[test]
    fn swift_table_cells_are_inline_only_without_math_substitution() {
        let cells = table_cell_inlines("# heading **bold** `$x$` $a_b_c$");
        assert!(matches!(&cells[0], Inline::Text(text, _) if text.starts_with("# heading ")));
        assert!(cells.iter().any(
            |inline| matches!(inline, Inline::Text(text, style) if text == "bold" && style.bold)
        ));
        assert!(cells
            .iter()
            .any(|inline| matches!(inline, Inline::Code(text) if text == "$x$")));
        assert!(!cells.iter().any(|inline| matches!(inline, Inline::Math(_))));
    }

    #[test]
    fn streaming_highlighter_accepts_trailing_string_escape() {
        let spans = highlight_line("let s = \"tail\\", "rust", &mut false);
        assert_eq!(
            spans
                .iter()
                .map(|span| span.text.as_str())
                .collect::<String>(),
            "let s = \"tail\\"
        );
    }

    // No window, GPU, clipboard, external browser or network is needed: these
    // exercise the REAL MathTextures path through egui layout / paint output.
    fn headless_frame(
        ctx: &egui::Context,
        math: &mut MathTextures,
        source: &str,
        width: f32,
        time: f64,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 1200.0),
                )),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    math.show_assistant_message(ui, source);
                });
            },
        )
    }

    fn painted_text(output: &egui::FullOutput) -> Vec<(String, egui::Rect)> {
        fn collect(shape: &egui::Shape, text: &mut Vec<(String, egui::Rect)>) {
            match shape {
                egui::Shape::Text(shape) => text.push((
                    shape.galley.job.text.clone(),
                    egui::Rect::from_min_size(shape.pos, shape.galley.size()),
                )),
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        collect(shape, text);
                    }
                }
                _ => {}
            }
        }
        let mut text = Vec::new();
        for shape in &output.shapes {
            collect(&shape.shape, &mut text);
        }
        text
    }

    fn text_cursor(output: &egui::FullOutput, text: &str, index: usize) -> egui::Pos2 {
        fn find(shape: &egui::Shape, text: &str, index: usize) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Text(shape) if shape.galley.job.text == text => {
                    let mut pos = shape.pos
                        + shape
                            .galley
                            .pos_from_cursor(egui::text::CCursor::new(index))
                            .center()
                            .to_vec2();
                    // Keep the endpoints inside the label's hit rectangle.
                    pos.x += if index == 0 { 0.1 } else { -0.1 };
                    Some(pos)
                }
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| find(s, text, index)),
                _ => None,
            }
        }
        output
            .shapes
            .iter()
            .find_map(|s| find(&s.shape, text, index))
            .unwrap_or_else(|| panic!("missing text {text:?}: {:?}", painted_text(output)))
    }

    /// Feed real desktop pointer events and Copy to the production assistant
    /// renderer. Only inspect egui platform commands: never touch the clipboard.
    fn drag_copy(
        source: &str,
        from: (&str, usize),
        to: (&str, usize),
        width: f32,
    ) -> Vec<egui::OutputCommand> {
        let ctx = egui::Context::default();
        let mut math = MathTextures::new();
        let _ = headless_frame(&ctx, &mut math, source, width, 0.0, vec![]);
        let output = headless_frame(&ctx, &mut math, source, width, 0.1, vec![]);
        let start = text_cursor(&output, from.0, from.1);
        let end = text_cursor(&output, to.0, to.1);
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let mut commands = Vec::new();
        for (time, events) in [
            (
                0.2,
                vec![egui::Event::PointerMoved(start), button(start, true)],
            ),
            (0.3, vec![egui::Event::PointerMoved(end)]),
            (0.4, vec![egui::Event::PointerMoved(end)]),
            (0.5, vec![button(end, false)]),
            (0.6, vec![egui::Event::Copy]),
        ] {
            commands.extend(
                headless_frame(&ctx, &mut math, source, width, time, events)
                    .platform_output
                    .commands,
            );
        }
        assert!(
            !commands
                .iter()
                .any(|c| matches!(c, egui::OutputCommand::OpenUrl(_))),
            "drag must not open links: {commands:?}"
        );
        commands
    }

    fn assert_copy(commands: &[egui::OutputCommand], expected: &str) {
        let copies: Vec<_> = commands
            .iter()
            .filter_map(|command| match command {
                egui::OutputCommand::CopyText(text) => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(copies, vec![expected]);
    }

    #[test]
    fn actual_selection_copies_rich_prose_and_inline_links_without_inserted_spaces() {
        let source = "Alpha **bold** *italic* ~~gone~~ `raw` a[link](https://example.invalid)b end";
        let text = "Alpha bold italic gone raw alinkb end";
        for width in [190.0, 680.0] {
            assert_copy(
                &drag_copy(source, (text, 0), (text, text.chars().count()), width),
                text,
            );
            // A drag beginning ON a link selects text instead of activating it.
            assert_copy(&drag_copy(source, (text, 28), (text, 32), width), "link");
        }
    }

    #[test]
    fn actual_selection_spans_prose_paragraphs_and_headings() {
        let source = "# Heading\n\nFirst **paragraph**.\n\nLast [link](https://example.invalid).";
        for (from, to, expected) in [
            (
                ("Heading", 0),
                ("Last link.", 10),
                "Heading\nFirst paragraph.\nLast link.",
            ),
            (
                ("Last link.", 10),
                ("Heading", 0),
                "Heading\nFirst paragraph.\nLast link.",
            ),
            (
                ("First paragraph.", 6),
                ("Last link.", 4),
                "paragraph.\nLast",
            ),
        ] {
            assert_copy(&drag_copy(source, from, to, 680.0), expected);
        }
    }

    #[test]
    fn actual_selection_includes_list_prefixes_and_indented_prose_code() {
        let source = "First\n\n- one\n- two\n\nLast";
        assert_copy(
            &drag_copy(source, ("First", 0), ("Last", 4), 680.0),
            "First\n•  one\n•  two\nLast",
        );
        let source = "First\n\n    raw code\n\n    tail code\n\nLast";
        assert_copy(
            &drag_copy(source, ("First", 0), ("Last", 4), 680.0),
            "First\nraw code\n\ntail code\nLast",
        );
    }

    #[test]
    fn actual_selection_crosses_inline_bitmap_but_copies_only_neighboring_text() {
        // Swift embeds an NSTextAttachment in the selected text field; egui's
        // image has no selectable object character or rich clipboard payload.
        // Document that remaining limitation, rather than claiming full math
        // selection parity or that selection cannot cross neighboring labels.
        let source = "Before $x^2$ after";
        let ctx = egui::Context::default();
        let mut math = MathTextures::new();
        let _ = headless_frame(&ctx, &mut math, source, 680.0, 0.0, vec![]);
        let output = headless_frame(&ctx, &mut math, source, 680.0, 0.1, vec![]);
        assert!(
            output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Rect(rect) if rect.fill_texture_id() != egui::TextureId::default())),
            "fixture must actually paint a math image"
        );
        assert!(
            !painted_text(&output)
                .iter()
                .any(|(text, _)| text.contains("$x^2$")),
            "fixture must exercise a real bitmap, not the textual fallback"
        );
        assert_copy(
            &drag_copy(source, ("Before ", 0), (" after", 6), 680.0),
            "Before  after",
        );
    }

    #[test]
    fn actual_selection_stops_at_swift_segment_and_table_cell_boundaries() {
        for suffix in [
            "```rust language caption\ncode body\n```",
            "<pasteable title=\"Decorative title\">\ncard body\n</pasteable>",
            "| Header | Other |\n| - | - |\n| cell body | next cell |",
            "> quote body",
            "$$x^2$$",
        ] {
            let source = format!("Before\n\n{suffix}\n\nAfter");
            assert_copy(
                &drag_copy(&source, ("Before", 0), ("After", 5), 680.0),
                "Before",
            );
            assert_copy(
                &drag_copy(&source, ("After", 5), ("Before", 0), 680.0),
                "After",
            );
        }
        let source =
            "| **Header** | Other |\n| - | - |\n| a[link](https://example.invalid)b | next cell |";
        assert_copy(
            &drag_copy(source, ("alinkb", 0), ("next cell", 9), 680.0),
            "alinkb",
        );
        assert_copy(
            &drag_copy(source, ("alinkb", 0), ("alinkb", 6), 680.0),
            "alinkb",
        );
    }

    #[test]
    fn actual_selection_copies_code_and_pasteable_bodies_not_their_decoration() {
        let code = "let x = 1;\n\n// tail";
        let card = "**literal** $x$\n\n  tail";
        for (source, body) in [
            (
                format!("Before\n```rust full caption\n{code}\n```\nAfter"),
                code,
            ),
            (
                format!(
                    "Before\n<pasteable title=\"Decorative title\">\n{card}\n</pasteable>\nAfter"
                ),
                card,
            ),
        ] {
            assert_copy(&drag_copy(&source, (body, 0), ("After", 5), 680.0), body);
            assert_copy(
                &drag_copy(&source, (body, body.chars().count()), ("Before", 0), 680.0),
                body,
            );
            assert_copy(
                &drag_copy(&source, (body, 0), (body, body.chars().count()), 680.0),
                body,
            );
        }
    }

    #[test]
    fn actual_selection_can_start_again_in_another_source_island() {
        let source = "Before\n```rust\ncode body\n```\nAfter";
        let ctx = egui::Context::default();
        let mut math = MathTextures::new();
        let _ = headless_frame(&ctx, &mut math, source, 680.0, 0.0, vec![]);
        let output = headless_frame(&ctx, &mut math, source, 680.0, 0.1, vec![]);
        for (step, text) in ["Before", "code body", "After", "Before"]
            .into_iter()
            .enumerate()
        {
            let start = text_cursor(&output, text, 0);
            let end = text_cursor(&output, text, text.chars().count());
            let button = |pos, pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            for (frame, events) in [
                vec![egui::Event::PointerMoved(start), button(start, true)],
                vec![egui::Event::PointerMoved(end)],
                vec![button(end, false)],
            ]
            .into_iter()
            .enumerate()
            {
                let _ = headless_frame(
                    &ctx,
                    &mut math,
                    source,
                    680.0,
                    1.0 + step as f64 + frame as f64 * 0.1,
                    events,
                );
            }
            let output = headless_frame(
                &ctx,
                &mut math,
                source,
                680.0,
                1.3 + step as f64,
                vec![egui::Event::Copy],
            );
            assert_copy(&output.platform_output.commands, text);
        }
    }

    #[test]
    fn actual_mixed_text_links_hit_test_the_last_glyph_on_wrapped_rows() {
        for source in [
            "before [Open](https://example.invalid) after",
            "| Header |\n| - |\n| before [Open](https://example.invalid) after |",
        ] {
            let text = "before Open after";
            let ctx = egui::Context::default();
            let mut math = MathTextures::new();
            let _ = headless_frame(&ctx, &mut math, source, 100.0, 0.0, vec![]);
            let output = headless_frame(&ctx, &mut math, source, 100.0, 0.1, vec![]);
            assert!(painted_text(&output)
                .iter()
                .any(|(painted, rect)| painted == text && rect.height() > 18.0));
            // Right edge of the last glyph, not merely the link's center. The
            // nearest insertion cursor would be AFTER the link at this point.
            let pos = text_cursor(&output, text, 11);
            let button = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            let _ = headless_frame(
                &ctx,
                &mut math,
                source,
                100.0,
                0.2,
                vec![egui::Event::PointerMoved(pos), button(true)],
            );
            let output = headless_frame(&ctx, &mut math, source, 100.0, 0.3, vec![button(false)]);
            let urls: Vec<_> = output
                .platform_output
                .commands
                .iter()
                .filter_map(|c| match c {
                    egui::OutputCommand::OpenUrl(url) => Some(url.url.as_str()),
                    _ => None,
                })
                .collect();
            assert_eq!(
                urls,
                ["https://example.invalid"],
                "source={source:?}, pos={pos:?}, text={:?}",
                painted_text(&output)
            );
        }
    }

    #[test]
    fn actual_assistant_path_renders_table_and_verbatim_card_not_raw_tags() {
        let body = "**literal** $x_1$\n```rust\nlet a = 1;\n```";
        let source = format!("| **Name** | Value |\n| - | - |\n| alpha | `beta` |\n<pasteable title=\"Reusable reply\">\n{body}\n</pasteable>");
        for dark in [false, true] {
            for width in [520.0, 680.0] {
                let ctx = egui::Context::default();
                ctx.set_visuals(if dark {
                    egui::Visuals::dark()
                } else {
                    egui::Visuals::light()
                });
                let mut math = MathTextures::new();
                let _ = headless_frame(&ctx, &mut math, &source, width, 0.0, vec![]);
                let output = headless_frame(&ctx, &mut math, &source, width, 0.1, vec![]);
                let text = painted_text(&output);
                for expected in [
                    "Name",
                    "Value",
                    "alpha",
                    "beta",
                    "Reusable reply",
                    body,
                    "Copy",
                ] {
                    assert!(
                        text.iter().any(|(text, _)| text == expected),
                        "missing {expected:?}: {text:?}"
                    );
                }
                assert_eq!(
                    text.iter().filter(|(text, _)| text == "Copy").count(),
                    1,
                    "card body must not create a nested code block"
                );
                assert!(!text.iter().any(|(text, _)| text.contains("<pasteable")
                    || text.contains("</pasteable>")
                    || text.contains("| - |")
                    || text.contains("**Name**")));
                assert!(
                    text.iter()
                        .all(|(_, rect)| rect.min.x >= 0.0 && rect.max.x <= width),
                    "horizontal overflow at {width}: {text:?}"
                );
            }
        }
    }

    #[test]
    fn actual_assistant_path_copy_actions_and_feedback_expiry() {
        let body = "**literal**\n  indented $x$\n```\nverbatim\n```";
        for source in [
            format!("<pasteable title=\"Reply\">\n{body}\n</pasteable>"),
            format!("```rust complete hint\n{body}\n```"),
        ] {
            // For the fence fixture, an inner ``` closes the fence exactly as
            // Swift does. Expected clipboard text comes from its FIRST block.
            let expected = match crate::ui::panel::split_math_segments(&source).remove(0) {
                crate::ui::panel::MessageSegment::Code { content, .. }
                | crate::ui::panel::MessageSegment::Pasteable { content, .. } => content,
                _ => unreachable!(),
            };
            let ctx = egui::Context::default();
            let mut math = MathTextures::new();
            let _ = headless_frame(&ctx, &mut math, &source, 680.0, 0.0, vec![]);
            let output = headless_frame(&ctx, &mut math, &source, 680.0, 0.1, vec![]);
            let pos = painted_text(&output)
                .into_iter()
                .find(|(text, _)| text == "Copy")
                .unwrap()
                .1
                .center();
            let pointer = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            let _ = headless_frame(
                &ctx,
                &mut math,
                &source,
                680.0,
                0.2,
                vec![egui::Event::PointerMoved(pos), pointer(true)],
            );
            let output = headless_frame(&ctx, &mut math, &source, 680.0, 0.3, vec![pointer(false)]);
            assert!(output.platform_output.commands.iter().any(|command| matches!(command, egui::OutputCommand::CopyText(text) if text == &expected)), "copy action did not emit exact body: {:?}", output.platform_output.commands);
            let output = headless_frame(&ctx, &mut math, &source, 680.0, 0.4, vec![]);
            assert!(painted_text(&output)
                .iter()
                .any(|(text, _)| text == "Copied"));
            let output = headless_frame(&ctx, &mut math, &source, 680.0, 2.0, vec![]);
            assert!(!painted_text(&output)
                .iter()
                .any(|(text, _)| text == "Copied"));
        }
    }

    #[test]
    fn actual_assistant_path_prose_quote_lists_and_highlighted_code() {
        let code = "let x = \"hi\"; // comment\nlet y = 2;";
        let source = format!("# Heading\n\n**bold** and *em* and `raw`\n\n3. three\n4. four\n\n> ## Quoted\n> line\n```rust full hint\n{code}\n```");
        let ctx = egui::Context::default();
        let mut math = MathTextures::new();
        let _ = headless_frame(&ctx, &mut math, &source, 680.0, 0.0, vec![]);
        let output = headless_frame(&ctx, &mut math, &source, 680.0, 0.1, vec![]);
        let text = painted_text(&output);
        for expected in [
            "Heading",
            "bold and em and raw",
            "3. ",
            "4. ",
            "three",
            "four",
            "Quoted",
            "line",
            "rust full hint",
            code,
        ] {
            assert!(
                text.iter().any(|(text, _)| text == expected),
                "missing {expected:?}: {text:?}"
            );
        }
        assert!(!text.iter().any(|(text, _)| text.contains("# Heading")
            || text.contains("> ##")
            || text.contains("**bold**")));
        let code_shape = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(shape) if shape.galley.job.text == code => Some(shape),
                _ => None,
            })
            .expect("code should be one selectable label across its lines");
        let colors: std::collections::HashSet<_> = code_shape
            .galley
            .job
            .sections
            .iter()
            .map(|section| section.format.color)
            .collect();
        assert!(
            colors.len() >= 3,
            "preserve keyword/string/comment highlighting"
        );
    }

    #[test]
    fn actual_assistant_path_table_and_prose_links_emit_one_url_command() {
        // Merely inspect egui output: do not execute its OpenUrl command.
        for source in [
            "[Open](https://example.invalid)",
            "| Link |\n| - |\n| [Open](https://example.invalid) |",
        ] {
            let ctx = egui::Context::default();
            let mut math = MathTextures::new();
            let _ = headless_frame(&ctx, &mut math, source, 680.0, 0.0, vec![]);
            let output = headless_frame(&ctx, &mut math, source, 680.0, 0.1, vec![]);
            let pos = painted_text(&output)
                .into_iter()
                .find(|(text, _)| text == "Open")
                .unwrap()
                .1
                .center();
            let pointer = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            let _ = headless_frame(
                &ctx,
                &mut math,
                source,
                680.0,
                0.2,
                vec![egui::Event::PointerMoved(pos), pointer(true)],
            );
            let output = headless_frame(&ctx, &mut math, source, 680.0, 0.3, vec![pointer(false)]);
            let urls: Vec<_> = output
                .platform_output
                .commands
                .iter()
                .filter_map(|command| match command {
                    egui::OutputCommand::OpenUrl(url) => Some(url.url.as_str()),
                    _ => None,
                })
                .collect();
            assert_eq!(urls, vec!["https://example.invalid"]);
        }
    }

    fn paragraph_inlines(blocks: Vec<FlowBlock>) -> Vec<Inline> {
        assert_eq!(blocks.len(), 1);
        match blocks.into_iter().next().unwrap() {
            FlowBlock::Paragraph(inlines) => inlines,
            other => panic!("expected paragraph, got {other:?}"),
        }
    }

    #[test]
    fn fence_shields_markdown_and_math() {
        // fence 內原文保留：同樣文字走 prose 會變粗體，證明 fence 繞過解析。
        let blocks = split_fences("**bold**\n```rust\n**raw** $x^2$\n```\n*em*");
        assert_eq!(blocks.len(), 3);
        assert!(matches!(&blocks[0], FenceBlock::Prose(p) if p == "**bold**"));
        assert!(matches!(&blocks[1], FenceBlock::Code { language, content }
                if language.as_deref() == Some("rust") && content == "**raw** $x^2$"));
        assert!(matches!(&blocks[2], FenceBlock::Prose(p) if p == "*em*"));

        let parsed = parse_prose("**raw** $x^2$");
        let inlines = paragraph_inlines(parsed);
        assert!(
            matches!(&inlines[0], Inline::Text(t, s) if t == "raw" && s.bold),
            "sanity: outside a fence the same text IS bold"
        );
    }

    #[test]
    fn unclosed_fence_is_code() {
        let blocks = split_fences("```py\n$x^2$\nprint(1)");
        assert_eq!(blocks.len(), 1);
        assert!(matches!(&blocks[0], FenceBlock::Code { language, content }
                if language.as_deref() == Some("py") && content.contains("$x^2$")));
    }

    #[test]
    fn inline_code_protects_stars() {
        let blocks = parse_prose("`**x**` and *em*");
        let inlines = paragraph_inlines(blocks);
        assert!(
            matches!(&inlines[0], Inline::Code(c) if c == "**x**"),
            "code span keeps asterisks verbatim: {inlines:?}"
        );
        assert!(
            !inlines
                .iter()
                .any(|i| matches!(i, Inline::Text(_, s) if s.bold)),
            "no bold anywhere: {inlines:?}"
        );
        assert!(
            inlines
                .iter()
                .any(|i| matches!(i, Inline::Text(t, s) if t == "em" && s.italic)),
            "emphasis outside code still works: {inlines:?}"
        );
    }

    #[test]
    fn inline_math_extraction_precedes_inline_code_like_swift() {
        // buildProse extracts before markdown, then substitutes in every run,
        // including inline code. FENCES, unlike inline code, are segmented first.
        let inlines = paragraph_inlines(parse_prose("`$x^2$`"));
        assert_eq!(inlines.len(), 1);
        assert!(matches!(&inlines[0], Inline::Math(c) if c == "x^2"));
    }

    #[test]
    fn inline_math_splits_in_text() {
        let inlines = paragraph_inlines(parse_prose("a $x^2$ b"));
        assert_eq!(inlines.len(), 3);
        assert!(matches!(&inlines[0], Inline::Text(t, _) if t == "a "));
        assert!(matches!(&inlines[1], Inline::Math(m) if m == "x^2"));
        assert!(matches!(&inlines[2], Inline::Text(t, _) if t == " b"));
    }

    #[test]
    fn heading_strikethrough_link_quote() {
        let blocks = parse_prose("# Title");
        assert!(matches!(&blocks[0], FlowBlock::Heading { level: 1, .. }));

        let inlines = paragraph_inlines(parse_prose("~~gone~~"));
        assert!(matches!(&inlines[0], Inline::Text(t, s) if t == "gone" && s.strike));

        let inlines = paragraph_inlines(parse_prose("[eg](https://example.com)"));
        assert!(matches!(&inlines[0], Inline::Link { label, url, .. }
                if label == "eg" && url == "https://example.com"));

        let blocks = parse_prose("> hello\n> world");
        assert_eq!(blocks.len(), 1);
        assert!(matches!(&blocks[0], FlowBlock::Quote(_)));
    }

    #[test]
    fn ordered_list_numbering() {
        let blocks = parse_prose("1. a\n2. b\n");
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            FlowBlock::List(items) => {
                assert_eq!(items.len(), 2);
                assert!(items.iter().all(|i| i.ordered && i.depth == 0));
                assert_eq!(items[0].number, 1);
                assert_eq!(items[1].number, 2);
            }
            other => panic!("expected list, got {other:?}"),
        }
    }

    #[test]
    fn nested_list_preserves_all_depths_like_swift() {
        let blocks = parse_prose("- a\n  - b\n    - c\n");
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            FlowBlock::List(items) => {
                assert_eq!(items.len(), 3);
                let depths: Vec<u8> = items.iter().map(|i| i.depth).collect();
                assert_eq!(depths, vec![0, 1, 2]);
                assert!(items.iter().all(|i| !i.ordered));
            }
            other => panic!("expected list, got {other:?}"),
        }
    }

    #[test]
    fn rust_highlight_tricolor() {
        let mut block = false;
        let spans = highlight_line(r#"let x = "hi"; // hi"#, "rust", &mut block);
        assert!(!block);
        assert!(
            spans
                .iter()
                .any(|s| s.kind == HlKind::Keyword && s.text == "let"),
            "{spans:?}"
        );
        assert!(
            spans
                .iter()
                .any(|s| s.kind == HlKind::Str && s.text == "\"hi\""),
            "{spans:?}"
        );
        assert!(
            spans
                .iter()
                .any(|s| s.kind == HlKind::Comment && s.text == "// hi"),
            "{spans:?}"
        );
    }

    #[test]
    fn block_comment_spans_lines() {
        let mut block = false;
        let first = highlight_line("/* open", "rust", &mut block);
        assert!(block);
        assert!(first.iter().all(|s| s.kind == HlKind::Comment));
        let second = highlight_line("still comment */ let", "rust", &mut block);
        assert!(!block);
        assert!(matches!(second[0].kind, HlKind::Comment));
        assert!(
            second
                .iter()
                .any(|s| s.kind == HlKind::Keyword && s.text == "let"),
            "{second:?}"
        );
    }

    #[test]
    fn python_comment_and_unknown_lang_plain() {
        let mut block = false;
        let spans = highlight_line("x = 1 # note", "py", &mut block);
        assert!(
            spans
                .iter()
                .any(|s| s.kind == HlKind::Comment && s.text == "# note"),
            "{spans:?}"
        );
        let spans = highlight_line("let x = 1", "nonsense-lang", &mut block);
        assert_eq!(spans.len(), 1);
        assert!(matches!(spans[0].kind, HlKind::Plain));
    }
}
