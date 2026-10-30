//! Attachment loading, ported from `Chat/Attachment.swift`.
//!
//! A file attached to a draft message. Text-type content is inlined into the
//! user message; images become `image_url` content parts.
//!
//! macOS → Rust mapping:
//! - `NSImage` load/resize/encode → [`image`] crate (`load_from_memory`,
//!   `resize` with Lanczos3, JPEG/PNG encode). Longest edge is capped at
//!   [`MAX_IMAGE_EDGE`].
//! - `CoreXLSX` → ZIP/XML (first [`MAX_ROWS_PER_SHEET`] actual XML rows per
//!   sheet rendered as CSV-ish text, preserving numeric cells as numbers).
//! - `UTType`/UTI → [`mime_guess`] (extension) + [`infer`] (content sniff).
//! - `PDFKit` → [`lopdf`], keeping the original PDF and extracted text until send.
//! - Word/ODT → ZIP/XML; RTF → an in-process, group-aware text decoder.
//! - Legacy Word DOC → [`rwml`]; RTFD → the bundle's direct `TXT.rtf` text.

use std::path::Path;

use serde::{Deserialize, Deserializer, Serialize};
use unicode_segmentation::UnicodeSegmentation;

fn text_count(text: &str) -> usize {
    text.graphemes(true).count()
}

fn truncate_text(text: &str) -> String {
    text.graphemes(true).take(MAX_TEXT_CHARS).collect()
}

// ---------------------------------------------------------------------------
// Limits (mirrors `AttachmentLoader`)
// ---------------------------------------------------------------------------

/// Files larger than this are rejected outright.
pub const MAX_FILE_BYTES: u64 = 25 * 1024 * 1024;
/// Text attachments are truncated to this many characters (with a note).
pub const MAX_TEXT_CHARS: usize = 100_000;
/// Spreadsheet extraction keeps the first N rows per sheet.
pub const MAX_ROWS_PER_SHEET: usize = 200;
/// Images are downscaled so the longest edge fits this.
pub const MAX_IMAGE_EDGE: u32 = 2048;
pub const MAX_PDF_PAGES: usize = 100;
pub const MAX_DIRECT_PDF_BYTES: usize = 10 * 1024 * 1024;
/// Encoded images larger than this are rejected even after downscaling.
pub const MAX_INLINE_IMAGE_BYTES: u64 = 10 * 1024 * 1024;

const IMAGE_EXTS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "heic", "heif", "tiff", "tif", "bmp",
];
const SHEET_EXTS: &[&str] = &["xlsx", "xlsm"];
const WORD_EXTS: &[&str] = &["docx", "doc", "rtf", "rtfd", "odt"];

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// Plain message error, mirroring Swift's `AttachError`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachError {
    pub message: String,
}

impl AttachError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for AttachError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for AttachError {}

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

/// What the attachment carries. `Text` payloads are UTF-8 bytes;
/// `Image`/`Pdf` payloads are the raw encoded bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AttachKind {
    #[default]
    Text,
    Image,
    Pdf,
}

fn de_attach_kind<'de, D: Deserializer<'de>>(d: D) -> Result<AttachKind, D::Error> {
    // Missing `kind` (older files) backfills to `Text`; unknown strings also
    // fall back instead of failing the whole conversation decode.
    let s = Option::<String>::deserialize(d)?.unwrap_or_default();
    Ok(match s.as_str() {
        "image" => AttachKind::Image,
        "pdf" => AttachKind::Pdf,
        _ => AttachKind::Text,
    })
}

/// A file attached to a draft message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    pub filename: String,
    pub mime: String,
    /// Base64 (standard alphabet, padded) of the payload bytes.
    pub data_base64: String,
    /// Missing in older files → backfilled to [`AttachKind::Text`].
    #[serde(default, deserialize_with = "de_attach_kind")]
    pub kind: AttachKind,
    /// Extracted PDF text, separate from the original bytes. Older JSON defaults
    /// to no extraction; never interpret raw PDF bytes as UTF-8 text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extracted_text: Option<String>,
    /// Processing note, e.g. "downscaled to 2048×1365".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// `true` = lossy/suspicious (shown prominently, sent to the model);
    /// `false` = routine info (user-facing only). Mirrors `NoteKind`.
    #[serde(default)]
    pub note_is_warning: bool,
}

impl Attachment {
    fn new(
        filename: String,
        mime: String,
        raw: Vec<u8>,
        kind: AttachKind,
        note: Option<String>,
        note_is_warning: bool,
    ) -> Self {
        Self {
            filename,
            mime,
            data_base64: base64_encode(&raw),
            extracted_text: None,
            kind,
            note,
            note_is_warning,
        }
    }

    /// Raw payload bytes.
    pub fn raw_bytes(&self) -> Result<Vec<u8>, AttachError> {
        base64_decode(&self.data_base64)
    }

    /// Decoded UTF-8 text for [`AttachKind::Text`].
    pub fn text_content(&self) -> Option<String> {
        if self.kind != AttachKind::Text {
            return None;
        }
        base64_decode(&self.data_base64)
            .ok()
            .map(|b| String::from_utf8_lossy(&b).into_owned())
    }

    /// `data:<mime>;base64,…` URL for the wire.
    pub fn data_url(&self) -> String {
        format!("data:{};base64,{}", self.mime, self.data_base64)
    }

    /// Which model capabilities this attachment needs. Used to record
    /// "don't send X to this model again" exceptions after a rejection:
    /// images need image input, PDFs need native file input.
    pub fn capability_exceptions(&self) -> Vec<ContentCapability> {
        match self.kind {
            AttachKind::Image => vec![ContentCapability::Images],
            AttachKind::Pdf => vec![ContentCapability::Files],
            AttachKind::Text => vec![],
        }
    }

    /// Single-attachment wire form. Images are always sent (a refusing model
    /// warns in the composer; silently dropping them is forbidden). A PDF
    /// becomes a native `file` part exactly when `files_supported` is true,
    /// else its text form — or an explicit absence block when there is none.
    pub fn wire_part(&self, files_supported: bool) -> WirePart {
        match self.kind {
            AttachKind::Image => WirePart::image_data_url(self.data_url()),
            AttachKind::Pdf => {
                if files_supported {
                    WirePart::file_data_url(self.filename.clone(), self.data_url())
                } else if let Some(text) = &self.extracted_text {
                    if text.trim().is_empty() {
                        WirePart::text(format!(
                            "[File: {} — a PDF with no extractable text; its content could not be included because this model does not accept PDF files directly]",
                            self.filename
                        ))
                    } else {
                        let suffix = self
                            .note
                            .as_ref()
                            .filter(|_| self.note_is_warning)
                            .map(|n| format!(" — {n}"))
                            .unwrap_or_default();
                        WirePart::text(format!("[File: {}{suffix}]\n{text}", self.filename))
                    }
                } else {
                    WirePart::text(format!(
                        "[File: {} — a PDF with no extractable text; its content could not be included because this model does not accept PDF files directly]",
                        self.filename
                    ))
                }
            }
            AttachKind::Text => {
                let suffix = self
                    .note
                    .as_ref()
                    .filter(|_| self.note_is_warning)
                    .map(|n| format!(" — {n}"))
                    .unwrap_or_default();
                let body = self.text_content().unwrap_or_default();
                WirePart::text(format!("[File: {}{suffix}]\n{body}", self.filename))
            }
        }
    }
}

/// Capability kinds an attachment can demand of a model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContentCapability {
    Images,
    Files,
}

impl ContentCapability {
    pub fn as_str(self) -> &'static str {
        match self {
            ContentCapability::Images => "images",
            ContentCapability::Files => "files",
        }
    }
}

// ---------------------------------------------------------------------------
// OpenAI wire format (mirrors `OpenAIChatClient.WirePart` / `WireContent`)
// ---------------------------------------------------------------------------

/// One typed content part: a bare string is maximum-compatibility text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WirePart {
    #[serde(rename = "type")]
    pub part_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_url: Option<WireImageUrl>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<WireFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireImageUrl {
    pub url: String,
}

/// Native file input (OpenAI, OpenRouter): the original file as a base64
/// data URL, no upload round-trip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireFile {
    pub filename: String,
    pub file_data: String,
}

impl WirePart {
    pub fn text(s: impl Into<String>) -> Self {
        Self {
            part_type: "text".to_string(),
            text: Some(s.into()),
            image_url: None,
            file: None,
        }
    }

    pub fn image_data_url(url: impl Into<String>) -> Self {
        Self {
            part_type: "image_url".to_string(),
            text: None,
            image_url: Some(WireImageUrl { url: url.into() }),
            file: None,
        }
    }

    pub fn file_data_url(filename: impl Into<String>, data_url: impl Into<String>) -> Self {
        Self {
            part_type: "file".to_string(),
            text: None,
            image_url: None,
            file: Some(WireFile {
                filename: filename.into(),
                file_data: data_url.into(),
            }),
        }
    }
}

/// Message content: bare string for plain text, typed parts when
/// images/files are involved. Serializes as a single JSON value either way.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum WireContent {
    Text(String),
    Parts(Vec<WirePart>),
}

/// Message-level encoding, mirroring `ChatStore.wireContent`: text
/// attachments are inlined ahead of `base_text`; images become content parts.
/// Stays a bare string when there are no typed parts.
pub fn encode_for_wire(
    attachments: &[Attachment],
    base_text: &str,
    files_supported: bool,
) -> WireContent {
    let mut text_blocks: Vec<String> = Vec::new();
    let mut typed_parts: Vec<WirePart> = Vec::new();
    for attachment in attachments {
        match attachment.kind {
            AttachKind::Text => {
                let suffix = attachment
                    .note
                    .as_ref()
                    .filter(|_| attachment.note_is_warning)
                    .map(|n| format!(" — {n}"))
                    .unwrap_or_default();
                let body = attachment.text_content().unwrap_or_default();
                text_blocks.push(format!("[File: {}{suffix}]\n{body}", attachment.filename));
            }
            AttachKind::Image => typed_parts.push(attachment.wire_part(files_supported)),
            AttachKind::Pdf => {
                if files_supported {
                    typed_parts.push(attachment.wire_part(true));
                } else {
                    // Reuse the text/absence rendering as an inline block.
                    match attachment.wire_part(false) {
                        WirePart { text: Some(t), .. } => text_blocks.push(t),
                        part => typed_parts.push(part),
                    }
                }
            }
        }
    }
    let mut combined_blocks: Vec<&str> = text_blocks.iter().map(String::as_str).collect();
    if !base_text.is_empty() {
        combined_blocks.push(base_text);
    }
    let combined = combined_blocks.join("\n\n");
    if typed_parts.is_empty() {
        return WireContent::Text(combined);
    }
    typed_parts.push(WirePart::text(if combined.is_empty() {
        "See the attached file(s).".to_string()
    } else {
        combined
    }));
    WireContent::Parts(typed_parts)
}

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------

/// Load a file from disk into an [`Attachment`].
pub fn load_file(path: &Path) -> Result<Attachment, AttachError> {
    let filename = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_string());
    let ext = path
        .extension()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    let meta = std::fs::metadata(path)
        .map_err(|_| AttachError::new(format!("{filename}: couldn't read file.")))?;
    if meta.len() > MAX_FILE_BYTES {
        return Err(AttachError::new(format!(
            "{filename}: too large ({} MB — limit is {} MB).",
            meta.len() / 1_048_576,
            MAX_FILE_BYTES / 1_048_576
        )));
    }

    if ext == "rtfd" && meta.is_dir() {
        return load_rtfd_bundle(path, &filename);
    }
    let bytes = read_file_bytes(path, &filename)?;
    load_bytes(&bytes, &filename)
}

/// The same decoder used for path attachments, for a drop carrying file bytes
/// instead of a filesystem path. No temporary files or clipboard text guessing.
pub fn load_bytes(bytes: &[u8], suggested_name: &str) -> Result<Attachment, AttachError> {
    let path = Path::new(suggested_name);
    let filename = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "file".to_owned());
    let ext = path
        .extension()
        .map(|ext| ext.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(AttachError::new(format!(
            "{filename}: too large (limit is 25 MB)."
        )));
    }
    if IMAGE_EXTS.contains(&ext.as_str()) {
        if ext == "heic" || ext == "heif" {
            return Err(AttachError::new(format!(
                "{filename}: HEIC/HEIF decoding is unsupported in this build — convert to PNG or JPEG and attach that instead."
            )));
        }
        return load_image_bytes(bytes, &filename);
    }
    if ext == "pdf" {
        return load_pdf_bytes(bytes, &filename);
    }
    if WORD_EXTS.contains(&ext.as_str()) {
        return load_word_bytes(bytes, &ext, &filename);
    }
    if SHEET_EXTS.contains(&ext.as_str()) {
        return load_spreadsheet_bytes(bytes, &filename);
    }
    load_plain_bytes(bytes, path, &filename)
}

fn read_file_bytes(path: &Path, filename: &str) -> Result<Vec<u8>, AttachError> {
    use std::io::Read;
    let file = std::fs::File::open(path)
        .map_err(|_| AttachError::new(format!("{filename}: couldn't read file.")))?;
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| AttachError::new(format!("{filename}: couldn't read file.")))?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(AttachError::new(format!(
            "{filename}: too large (limit is 25 MB)."
        )));
    }
    Ok(bytes)
}

/// Alias retained for callers that name the path-loading entry point explicitly.
pub fn load_from_path(path: &Path) -> Result<Attachment, AttachError> {
    load_file(path)
}

/// Convert the persistence-compatible loader shape into the active chat model.
pub fn into_store_attachment(attachment: Attachment) -> super::store::Attachment {
    use super::store::{Attachment as Stored, AttachmentContent, NoteKind};
    let content = match attachment.kind {
        AttachKind::Text => AttachmentContent::Text(attachment.text_content().unwrap_or_default()),
        AttachKind::Image => AttachmentContent::Image {
            data_url: attachment.data_url(),
        },
        AttachKind::Pdf => AttachmentContent::Pdf {
            data_url: attachment.data_url(),
            extracted_text: attachment.extracted_text.clone().unwrap_or_default(),
        },
    };
    let mut stored = Stored::new(attachment.filename, content);
    stored.note = attachment.note;
    stored.note_kind = if attachment.note_is_warning {
        NoteKind::Warning
    } else {
        NoteKind::Info
    };
    stored
}

pub fn load_store_attachment(path: &Path) -> anyhow::Result<super::store::Attachment> {
    Ok(into_store_attachment(load_file(path)?))
}

/// Reads only an image; a text-only clipboard returns `None`. Call off the UI thread.
pub fn load_clipboard_store_attachment() -> anyhow::Result<Option<super::store::Attachment>> {
    let mut clipboard = arboard::Clipboard::new()?;
    match clipboard.get_image() {
        Ok(image) => {
            let width = u32::try_from(image.width)?;
            let height = u32::try_from(image.height)?;
            let image = image::RgbaImage::from_raw(width, height, image.bytes.into_owned())
                .ok_or_else(|| anyhow::anyhow!("Clipboard image has invalid RGBA data."))?;
            Ok(Some(into_store_attachment(encode_image(
                image::DynamicImage::ImageRgba8(image),
                "Pasted image",
            )?)))
        }
        Err(arboard::Error::ContentNotAvailable) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// Encode in-memory image bytes (e.g. from the clipboard) into an [`Attachment`].
pub fn load_image_bytes(bytes: &[u8], suggested_name: &str) -> Result<Attachment, AttachError> {
    let img = image::load_from_memory(bytes)
        .map_err(|_| AttachError::new(format!("{suggested_name}: couldn't decode image.")))?;
    encode_image(img, suggested_name)
}

fn encode_image(img: image::DynamicImage, suggested_name: &str) -> Result<Attachment, AttachError> {
    let (width, height) = (img.width(), img.height());
    let longest = width.max(height).max(1);

    let mut out = img;
    let mut note: Option<String> = None;
    if longest > MAX_IMAGE_EDGE {
        let scale = MAX_IMAGE_EDGE as f64 / longest as f64;
        let target_w = ((width as f64 * scale) as u32).max(1);
        let target_h = ((height as f64 * scale) as u32).max(1);
        out = out.resize_exact(target_w, target_h, image::imageops::FilterType::Lanczos3);
        note = Some(format!("downscaled to {target_w}×{target_h}"));
    }

    // Source always re-encodes as JPEG at compressionFactor 0.8, even PNGs.
    let mut encoded = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut encoded, 80)
        .encode_image(&out.to_rgb8())
        .map_err(|_| AttachError::new(format!("{suggested_name}: couldn't encode image.")))?;
    if encoded.len() as u64 > MAX_INLINE_IMAGE_BYTES {
        return Err(AttachError::new(format!(
            "{suggested_name}: image still exceeds {} MB after downscaling.",
            MAX_INLINE_IMAGE_BYTES / 1_048_576
        )));
    }
    Ok(Attachment::new(
        suggested_name.to_string(),
        "image/jpeg".to_string(),
        encoded,
        AttachKind::Image,
        note,
        false,
    ))
}

// ---------------------------------------------------------------------------
// PDF: page-limited extraction plus capped original retention
// ---------------------------------------------------------------------------

fn load_pdf_bytes(bytes: &[u8], filename: &str) -> Result<Attachment, AttachError> {
    let document = lopdf::Document::load_mem(bytes)
        .map_err(|_| AttachError::new(format!("{filename}: couldn't open PDF.")))?;
    if document.is_encrypted() && document.encryption_state.is_none() {
        return Err(AttachError::new(format!(
            "{filename}: PDF is password-protected."
        )));
    }
    let pages = document.get_pages();
    let used_pages: Vec<u32> = pages.keys().copied().take(MAX_PDF_PAGES).collect();
    let keeps_raw = bytes.len() <= MAX_DIRECT_PDF_BYTES;
    let mut notes = Vec::new();
    if pages.len() > MAX_PDF_PAGES {
        notes.push(format!(
            "first {MAX_PDF_PAGES} of {} pages when sent as text",
            pages.len()
        ));
    }
    // Extract separately: a damaged page must not discard the original or the
    // readable pages, but the loss must be visible to both user and model.
    let mut extracted = Vec::new();
    let mut failed_pages = 0;
    for page in &used_pages {
        match document.extract_text(&[*page]) {
            Ok(text) => extracted.push(text),
            Err(_) => failed_pages += 1,
        }
    }
    if failed_pages > 0 {
        notes.push(format!("couldn't extract text from {failed_pages} page(s)"));
    }
    let mut text = extracted.join("\n").trim().to_string();
    if text_count(&text) < used_pages.len() * 80 {
        notes.push(if keeps_raw {
            "very little extractable text — likely scanned; needs a model that accepts PDFs directly"
        } else {
            "very little extractable text — likely a scanned PDF; content may be missing"
        }.to_string());
    }
    if text_count(&text) > MAX_TEXT_CHARS {
        text = truncate_text(&text);
        notes.push(format!(
            "truncated at {MAX_TEXT_CHARS} characters when sent as text"
        ));
    }
    if text.is_empty() && !keeps_raw {
        return Err(AttachError::new(format!("{filename}: no extractable text (scanned PDF?), and too large to send as a PDF file. Attach page screenshots instead for vision models.")));
    }
    if keeps_raw {
        let mut attachment = Attachment::new(
            filename.to_string(),
            "application/pdf".into(),
            bytes.to_vec(),
            AttachKind::Pdf,
            (!notes.is_empty()).then(|| notes.join("; ")),
            true,
        );
        attachment.extracted_text = Some(text);
        Ok(attachment)
    } else {
        notes.push("over 10 MB — sent as extracted text, not the original PDF".into());
        Ok(Attachment::new(
            filename.to_string(),
            "text/plain".into(),
            text.into_bytes(),
            AttachKind::Text,
            Some(notes.join("; ")),
            true,
        ))
    }
}

// ---------------------------------------------------------------------------
// Word / OpenDocument / RTF (no subprocesses or platform converters)
// ---------------------------------------------------------------------------

/// An RTFD package has a known main document, not a tree of documents to read.
/// Images/other assets remain opaque and are never opened during extraction.
fn load_rtfd_bundle(path: &Path, filename: &str) -> Result<Attachment, AttachError> {
    let error =
        |reason: &str| AttachError::new(format!("{filename}: couldn't extract text ({reason})."));
    let root = std::fs::symlink_metadata(path).map_err(|_| error("couldn't read RTFD bundle"))?;
    if !root.is_dir() || is_linklike(&root) {
        return Err(error("RTFD bundle must be a direct directory, not a link"));
    }
    // The second spelling supports case-sensitive filesystems too. Deliberately
    // do not enumerate directories, follow linked entries, or guess other files.
    for name in ["TXT.rtf", "txt.rtf"] {
        let entry = path.join(name);
        let metadata = match std::fs::symlink_metadata(&entry) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return Err(error("couldn't read RTFD main text")),
        };
        if !metadata.is_file() || is_linklike(&metadata) {
            return Err(error(
                "RTFD TXT.rtf must be a regular file, not a link or directory",
            ));
        }
        if metadata.len() > MAX_FILE_BYTES {
            return Err(AttachError::new(format!(
                "{filename}: RTFD main text too large (limit is 25 MB)."
            )));
        }
        let bytes = read_file_bytes(&entry, filename)?;
        return load_word_bytes(&bytes, "rtf", filename);
    }
    Err(error("RTFD bundle has no direct TXT.rtf main text"))
}

fn is_linklike(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // Also reject junctions and other reparse points, not just symlinks.
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn extract_doc(bytes: &[u8]) -> anyhow::Result<String> {
    // AppKit's NSAttributedString auto-detects document contents. In particular,
    // Word-generated RTF is often saved with a .doc extension, not an OLE header.
    let payload = bytes
        .strip_prefix(b"\xef\xbb\xbf")
        .unwrap_or(bytes)
        .trim_ascii_start();
    if payload.starts_with(br"{\rtf") {
        return extract_rtf(payload);
    }
    match rwml::extract_text(bytes) {
        Ok(text) => Ok(text),
        Err(rwml::Error::NoText) => Ok(String::new()),
        Err(error) => Err(error.into()),
    }
}

fn load_word_bytes(bytes: &[u8], ext: &str, filename: &str) -> Result<Attachment, AttachError> {
    let text = match ext {
        "docx" | "odt" => extract_zip_document(bytes, ext),
        "rtf" | "rtfd" => extract_rtf(bytes),
        "doc" => extract_doc(bytes),
        _ => {
            return Err(AttachError::new(format!(
                "{filename}: unsupported word-processing format (.{ext})."
            )))
        }
    }
    .map_err(|error| AttachError::new(format!("{filename}: couldn't extract text ({error}).")))?;
    let mut text = text.trim().to_string();
    if text.is_empty() {
        return Err(AttachError::new(format!(
            "{filename}: document contains no text."
        )));
    }
    let note = if text_count(&text) > MAX_TEXT_CHARS {
        text = truncate_text(&text);
        Some(format!("truncated at {MAX_TEXT_CHARS} characters"))
    } else {
        None
    };
    Ok(Attachment::new(
        filename.into(),
        "text/plain".into(),
        text.into_bytes(),
        AttachKind::Text,
        note,
        true,
    ))
}

fn extract_zip_document(bytes: &[u8], ext: &str) -> anyhow::Result<String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))?;
    let entry = if ext == "docx" {
        "word/document.xml"
    } else {
        "content.xml"
    };
    let xml = read_zip_xml(&mut archive, entry)?;
    extract_document_xml(&xml, ext == "docx")
}

fn extract_document_xml(xml: &[u8], word: bool) -> anyhow::Result<String> {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_reader(xml);
    let mut out = String::new();
    let mut text_depth = 0usize;
    let mut paragraph_depth = 0usize;
    let mut body_depth = 0usize;
    let mut saw_root = false;
    let mut depth = 0usize;
    loop {
        match reader.read_event()? {
            Event::Start(tag) => {
                saw_root = true;
                depth += 1;
                let local = tag.local_name();
                let name = local.as_ref();
                if word {
                    if name == b"t" {
                        text_depth += 1;
                    }
                } else {
                    if name == b"text" {
                        body_depth += 1;
                    }
                    if body_depth > 0 && (name == b"p" || name == b"h") {
                        paragraph_depth += 1;
                    }
                }
                if word || paragraph_depth > 0 {
                    append_xml_separator(&tag, word, &mut out)?;
                }
            }
            Event::Empty(tag) => {
                if word || paragraph_depth > 0 {
                    append_xml_separator(&tag, word, &mut out)?;
                }
            }
            Event::End(tag) => {
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| anyhow::anyhow!("unbalanced document XML"))?;
                let local = tag.local_name();
                let name = local.as_ref();
                if word {
                    if name == b"t" {
                        text_depth = text_depth.saturating_sub(1);
                    }
                    if name == b"p" || name == b"tr" {
                        out.push('\n');
                    }
                    if name == b"tc" {
                        out.push('\t');
                    }
                } else {
                    if body_depth > 0 && (name == b"p" || name == b"h") {
                        paragraph_depth = paragraph_depth.saturating_sub(1);
                        out.push('\n');
                    }
                    if name == b"text" {
                        body_depth = body_depth.saturating_sub(1);
                    }
                }
            }
            Event::Text(text)
                if if word {
                    text_depth > 0
                } else {
                    paragraph_depth > 0
                } =>
            {
                out.push_str(&text.unescape()?);
            }
            Event::CData(text)
                if if word {
                    text_depth > 0
                } else {
                    paragraph_depth > 0
                } =>
            {
                out.push_str(&reader.decoder().decode(text.as_ref())?);
            }
            Event::DocType(_) => anyhow::bail!("XML document types are unsupported"),
            Event::Eof => break,
            _ => {}
        }
        anyhow::ensure!(
            out.len() <= 32 * 1024 * 1024,
            "expanded document text exceeds 32 MB extraction limit"
        );
    }
    anyhow::ensure!(saw_root, "missing document XML root");
    anyhow::ensure!(depth == 0, "unclosed document XML elements");
    Ok(out)
}

fn append_xml_separator(
    tag: &quick_xml::events::BytesStart<'_>,
    word: bool,
    out: &mut String,
) -> anyhow::Result<()> {
    match tag.local_name().as_ref() {
        b"tab" => out.push('\t'),
        b"br" | b"cr" if word => out.push('\n'),
        b"line-break" if !word => out.push('\n'),
        b"s" if !word => {
            let mut count = 1;
            for attribute in tag.attributes() {
                let attribute = attribute?;
                if attribute.key.local_name().as_ref() == b"c" {
                    count = std::str::from_utf8(attribute.value.as_ref())?.parse::<usize>()?;
                }
            }
            anyhow::ensure!(
                count <= MAX_TEXT_CHARS,
                "OpenDocument space run exceeds extraction limit"
            );
            out.extend(std::iter::repeat_n(' ', count));
        }
        _ => {}
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct RtfState {
    skip: bool,
    hidden: bool,
    unicode_fallback: usize,
    encoding: &'static encoding_rs::Encoding,
    default_encoding: &'static encoding_rs::Encoding,
    font_table: bool,
    font_id: i32,
}

fn rtf_codepage(codepage: i32) -> Option<&'static encoding_rs::Encoding> {
    let label = match codepage {
        65001 => "utf-8".to_string(),
        932 => "shift_jis".to_string(),
        936 => "gbk".to_string(),
        949 => "euc-kr".to_string(),
        950 => "big5".to_string(),
        10000 => "macintosh".to_string(),
        _ => format!("windows-{codepage}"),
    };
    encoding_rs::Encoding::for_label(label.as_bytes())
}

/// Decode text-bearing RTF groups, excluding metadata, pictures and field
/// instructions. Unicode escapes are UTF-16 (including surrogate pairs), not
/// independent Rust chars. ANSI hex escapes obey the document code page.
fn extract_rtf(bytes: &[u8]) -> anyhow::Result<String> {
    anyhow::ensure!(bytes.starts_with(b"{\\rtf"), "invalid RTF header");
    let mut state = RtfState {
        skip: false,
        hidden: false,
        unicode_fallback: 1,
        encoding: encoding_rs::WINDOWS_1252,
        default_encoding: encoding_rs::WINDOWS_1252,
        font_table: false,
        font_id: 0,
    };
    let mut fonts = std::collections::HashMap::<i32, Option<&'static encoding_rs::Encoding>>::new();
    let mut default_font = 0;
    let mut stack = Vec::new();
    let mut out: Vec<u16> = Vec::new();
    let mut ansi = Vec::new();
    let mut fallback = 0usize;
    let mut i = 0;
    let flush = |ansi: &mut Vec<u8>, out: &mut Vec<u16>, state: RtfState| {
        if !state.skip && !state.hidden {
            let (text, _) = state.encoding.decode_without_bom_handling(ansi);
            out.extend(text.encode_utf16());
        }
        ansi.clear();
    };
    while i < bytes.len() {
        match bytes[i] {
            b'{' => {
                flush(&mut ansi, &mut out, state);
                anyhow::ensure!(
                    stack.len() < 512,
                    "RTF group nesting exceeds extraction limit"
                );
                stack.push(state);
                i += 1;
            }
            b'}' => {
                flush(&mut ansi, &mut out, state);
                state = stack
                    .pop()
                    .ok_or_else(|| anyhow::anyhow!("unbalanced RTF groups"))?;
                fallback = 0;
                i += 1;
            }
            b'\r' | b'\n' => i += 1,
            b'\\' => {
                i += 1;
                let symbol = *bytes
                    .get(i)
                    .ok_or_else(|| anyhow::anyhow!("incomplete RTF escape"))?;
                if symbol == b'\'' {
                    let hex = bytes
                        .get(i + 1..i + 3)
                        .ok_or_else(|| anyhow::anyhow!("incomplete RTF hex escape"))?;
                    let byte = u8::from_str_radix(std::str::from_utf8(hex)?, 16)?;
                    if fallback > 0 {
                        fallback -= 1;
                    } else {
                        ansi.push(byte);
                    }
                    i += 3;
                    continue;
                }
                if matches!(symbol, b'\\' | b'{' | b'}') {
                    if fallback > 0 {
                        fallback -= 1;
                    } else {
                        ansi.push(symbol);
                    }
                    i += 1;
                    continue;
                }
                flush(&mut ansi, &mut out, state);
                if !symbol.is_ascii_alphabetic() {
                    i += 1;
                    if symbol == b'*' {
                        state.skip = true;
                    } else if matches!(symbol, b'~' | b'_' | b'-') {
                        if fallback > 0 {
                            fallback -= 1;
                        } else if !state.skip && !state.hidden {
                            out.push(match symbol {
                                b'~' => 0x00a0,
                                b'_' => 0x2011,
                                _ => 0x00ad,
                            });
                        }
                    }
                    continue;
                }
                let start = i;
                while bytes.get(i).is_some_and(u8::is_ascii_alphabetic) {
                    i += 1;
                }
                let control = &bytes[start..i];
                let negative = bytes.get(i) == Some(&b'-');
                if negative {
                    i += 1;
                }
                let start_number = i;
                while bytes.get(i).is_some_and(u8::is_ascii_digit) {
                    i += 1;
                }
                let number = if i > start_number {
                    let number = std::str::from_utf8(&bytes[start_number..i])?.parse::<i32>()?;
                    Some(if negative { -number } else { number })
                } else {
                    None
                };
                if bytes.get(i) == Some(&b' ') {
                    i += 1;
                }
                match control {
                    b"NeXTGraphic" => {
                        // AppKit exposes an embedded attachment as an object-
                        // replacement character in attributed.string. Preserve
                        // that textual marker without reading the named asset.
                        if !state.skip && !state.hidden {
                            out.push(0xfffc);
                        }
                        state.skip = true;
                    }
                    b"fonttbl" => {
                        state.skip = true;
                        state.font_table = true;
                    }
                    b"colortbl" | b"stylesheet" | b"info" | b"pict" | b"object" | b"objdata"
                    | b"fldinst" | b"datastore" | b"themedata" | b"listtable"
                    | b"listoverridetable" | b"generator" | b"xmlnstbl" => state.skip = true,
                    b"v" => state.hidden = number != Some(0),
                    b"deff" => default_font = number.unwrap_or(0),
                    b"f" => {
                        state.font_id = number.unwrap_or(0);
                        if !state.skip {
                            state.encoding = fonts
                                .get(&state.font_id)
                                .copied()
                                .unwrap_or(Some(state.default_encoding))
                                .ok_or_else(|| {
                                    anyhow::anyhow!("unsupported RTF font character set")
                                })?;
                        }
                    }
                    b"fcharset" if state.font_table => {
                        let charset = number.unwrap_or(0);
                        let codepage = match charset {
                            0 | 1 => None,
                            77 => Some(10000),
                            128 => Some(932),
                            129 => Some(949),
                            134 => Some(936),
                            136 => Some(950),
                            161 => Some(1253),
                            162 => Some(1254),
                            163 => Some(1258),
                            177 => Some(1255),
                            178 => Some(1256),
                            186 => Some(1257),
                            204 => Some(1251),
                            222 => Some(874),
                            238 => Some(1250),
                            _ => Some(-1),
                        };
                        fonts.insert(
                            state.font_id,
                            codepage
                                .map(rtf_codepage)
                                .unwrap_or(Some(state.default_encoding)),
                        );
                    }
                    b"cpg" if state.font_table => {
                        fonts.insert(state.font_id, rtf_codepage(number.unwrap_or(1252)));
                    }
                    b"plain" => {
                        state.hidden = false;
                        state.encoding = fonts
                            .get(&default_font)
                            .copied()
                            .unwrap_or(Some(state.default_encoding))
                            .ok_or_else(|| anyhow::anyhow!("unsupported RTF font character set"))?;
                    }
                    b"uc" => {
                        let count = number.unwrap_or(1);
                        anyhow::ensure!(
                            (0..=32767).contains(&count),
                            "invalid RTF Unicode fallback count"
                        );
                        state.unicode_fallback = count as usize;
                    }
                    b"u" => {
                        let value =
                            number.ok_or_else(|| anyhow::anyhow!("missing RTF Unicode value"))?;
                        anyhow::ensure!(
                            (-32768..=65535).contains(&value),
                            "invalid RTF Unicode value"
                        );
                        if !state.skip && !state.hidden {
                            out.push(value as u16);
                        }
                        fallback = state.unicode_fallback;
                    }
                    b"bin" => {
                        let count = usize::try_from(number.unwrap_or(-1))?;
                        i = i
                            .checked_add(count)
                            .filter(|end| *end <= bytes.len())
                            .ok_or_else(|| anyhow::anyhow!("truncated RTF binary data"))?;
                    }
                    b"ansicpg" => {
                        let codepage = number.unwrap_or(1252);
                        state.encoding = rtf_codepage(codepage).ok_or_else(|| {
                            anyhow::anyhow!("unsupported RTF code page {codepage}")
                        })?;
                        state.default_encoding = state.encoding;
                    }
                    b"mac" => {
                        state.encoding = encoding_rs::MACINTOSH;
                        state.default_encoding = state.encoding;
                    }
                    b"pc" | b"pca" => anyhow::bail!("unsupported legacy DOS RTF code page"),
                    _ if !state.skip && !state.hidden => {
                        let character = match control {
                            b"par" | b"line" | b"row" => Some('\n'),
                            b"tab" | b"cell" => Some('\t'),
                            b"emdash" => Some('—'),
                            b"endash" => Some('–'),
                            b"bullet" => Some('•'),
                            b"lquote" => Some('‘'),
                            b"rquote" => Some('’'),
                            b"ldblquote" => Some('“'),
                            b"rdblquote" => Some('”'),
                            _ => None,
                        };
                        if let Some(character) = character {
                            if fallback > 0 {
                                fallback -= 1;
                            } else {
                                out.push(character as u16);
                            }
                        }
                    }
                    _ => {}
                }
            }
            byte => {
                if fallback > 0 {
                    fallback -= 1;
                } else {
                    ansi.push(byte);
                }
                i += 1;
            }
        }
    }
    flush(&mut ansi, &mut out, state);
    anyhow::ensure!(stack.is_empty(), "unbalanced RTF groups");
    Ok(String::from_utf16_lossy(&out))
}

// ---------------------------------------------------------------------------
// Spreadsheets (OOXML, matching CoreXLSX's actual rows/cells, not padded ranges)
// ---------------------------------------------------------------------------

fn load_spreadsheet_bytes(bytes: &[u8], filename: &str) -> Result<Attachment, AttachError> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|_| AttachError::new(format!("{filename}: couldn't open spreadsheet.")))?;
    spreadsheet_from_archive(&mut archive, filename)
        .map_err(|e| AttachError::new(format!("{filename}: couldn't parse spreadsheet ({e}).")))
}

fn read_zip_xml<R: std::io::Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
) -> anyhow::Result<Vec<u8>> {
    use std::io::Read;
    let mut file = archive.by_name(name)?;
    const MAX_XML_BYTES: u64 = 32 * 1024 * 1024;
    anyhow::ensure!(
        file.size() <= MAX_XML_BYTES,
        "XML exceeds 32 MB extraction limit"
    );
    let mut bytes = Vec::new();
    (&mut file)
        .take(MAX_XML_BYTES + 1)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() as u64 <= MAX_XML_BYTES,
        "XML exceeds 32 MB extraction limit"
    );
    Ok(bytes)
}

fn spreadsheet_from_archive<R: std::io::Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    filename: &str,
) -> anyhow::Result<Attachment> {
    use quick_xml::events::Event;
    let shared = read_zip_xml(archive, "xl/sharedStrings.xml")
        .ok()
        .and_then(|xml| parse_shared_strings(&xml).ok())
        .unwrap_or_default();
    let has_workbook = archive.file_names().any(|name| name == "xl/workbook.xml");
    let mut sheets = Vec::new();
    if has_workbook {
        let workbook = read_zip_xml(archive, "xl/workbook.xml")?;
        let rels = read_zip_xml(archive, "xl/_rels/workbook.xml.rels")?;
        let mut relationships = std::collections::HashMap::new();
        let mut reader = quick_xml::Reader::from_reader(rels.as_slice());
        loop {
            match reader.read_event()? {
                Event::Start(tag) | Event::Empty(tag)
                    if tag.local_name().as_ref() == b"Relationship" =>
                {
                    let id = xml_attribute(&tag, b"Id")?.unwrap_or_default();
                    if xml_attribute(&tag, b"TargetMode")?.as_deref() == Some("External") {
                        continue;
                    }
                    if let Some(target) = xml_attribute(&tag, b"Target")? {
                        let path = if target.starts_with('/') {
                            target.trim_start_matches('/').to_string()
                        } else {
                            format!("xl/{target}")
                        };
                        // Normalize archive-relative relationships, never filesystem paths.
                        let mut components = Vec::new();
                        for component in path.split('/') {
                            match component {
                                ".." => {
                                    components.pop();
                                }
                                "." | "" => {}
                                _ => components.push(component),
                            }
                        }
                        relationships.insert(id, components.join("/"));
                    }
                }
                Event::DocType(_) => anyhow::bail!("XML document types are unsupported"),
                Event::Eof => break,
                _ => {}
            }
        }
        let mut reader = quick_xml::Reader::from_reader(workbook.as_slice());
        loop {
            match reader.read_event()? {
                Event::Start(tag) | Event::Empty(tag) if tag.local_name().as_ref() == b"sheet" => {
                    let id = xml_attribute(&tag, b"id")?.unwrap_or_default();
                    let path = relationships
                        .get(&id)
                        .ok_or_else(|| anyhow::anyhow!("missing worksheet relationship"))?;
                    let name = xml_attribute(&tag, b"name")?
                        .unwrap_or_else(|| format!("Sheet {}", sheets.len() + 1));
                    sheets.push((name, path.clone()));
                }
                Event::DocType(_) => anyhow::bail!("XML document types are unsupported"),
                Event::Eof => break,
                _ => {}
            }
        }
    } else {
        for path in archive.file_names() {
            if path.starts_with("xl/worksheets/") && path.ends_with(".xml") {
                sheets.push((format!("Sheet {}", sheets.len() + 1), path.to_string()));
            }
        }
    }
    let mut sections: Vec<String> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    for (sheet_name, path) in sheets {
        let xml = read_zip_xml(archive, &path)?;
        let (total_rows, csv) = parse_sheet_rows(&xml, &shared)?;
        sections.push(format!("--- {sheet_name} ({total_rows} rows) ---\n{csv}"));
        if total_rows > MAX_ROWS_PER_SHEET {
            notes.push(format!(
                "{sheet_name}: first {MAX_ROWS_PER_SHEET} of {total_rows} rows"
            ));
        }
    }
    let mut text = sections.join("\n\n");
    anyhow::ensure!(!text.is_empty(), "spreadsheet contains no data");
    if text_count(&text) > MAX_TEXT_CHARS {
        text = truncate_text(&text);
        notes.push(format!("truncated at {MAX_TEXT_CHARS} characters"));
    }
    let has_notes = !notes.is_empty();
    Ok(Attachment::new(
        filename.to_string(),
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet".to_string(),
        text.into_bytes(),
        AttachKind::Text,
        if has_notes {
            Some(notes.join("; "))
        } else {
            None
        },
        has_notes,
    ))
}

fn xml_attribute(
    tag: &quick_xml::events::BytesStart<'_>,
    key: &[u8],
) -> anyhow::Result<Option<String>> {
    for attribute in tag.attributes() {
        let attribute = attribute?;
        if attribute.key.local_name().as_ref() == key {
            return Ok(Some(attribute.unescape_value()?.into_owned()));
        }
    }
    Ok(None)
}

fn parse_shared_strings(xml: &[u8]) -> anyhow::Result<Vec<String>> {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_reader(xml);
    let mut strings = Vec::new();
    let mut current = String::new();
    let mut in_text = false;
    loop {
        match reader.read_event()? {
            Event::Start(tag) if tag.local_name().as_ref() == b"si" => current.clear(),
            Event::Start(tag) if tag.local_name().as_ref() == b"t" => in_text = true,
            Event::Text(text) if in_text => current.push_str(&text.unescape()?),
            Event::End(tag) if tag.local_name().as_ref() == b"t" => in_text = false,
            Event::End(tag) if tag.local_name().as_ref() == b"si" => {
                strings.push(std::mem::take(&mut current))
            }
            Event::Empty(tag) if tag.local_name().as_ref() == b"si" => strings.push(String::new()),
            Event::DocType(_) => anyhow::bail!("XML document types are unsupported"),
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(strings)
}

fn parse_sheet_rows(xml: &[u8], shared: &[String]) -> anyhow::Result<(usize, String)> {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_reader(xml);
    let mut total_rows = 0usize;
    let mut rows = Vec::new();
    let mut cells = Vec::new();
    let mut cell_type = String::new();
    let mut value = String::new();
    let mut inline = String::new();
    let mut in_value = false;
    let mut in_text = false;
    loop {
        match reader.read_event()? {
            Event::Start(tag) => match tag.local_name().as_ref() {
                b"row" => {
                    total_rows += 1;
                    cells.clear();
                }
                b"c" => {
                    cell_type = xml_attribute(&tag, b"t")?.unwrap_or_default();
                    value.clear();
                    inline.clear();
                }
                b"v" => in_value = true,
                b"t" => in_text = true,
                _ => {}
            },
            Event::Empty(tag) => match tag.local_name().as_ref() {
                b"row" => {
                    total_rows += 1;
                    if total_rows <= MAX_ROWS_PER_SHEET {
                        rows.push(String::new());
                    }
                }
                b"c" if total_rows <= MAX_ROWS_PER_SHEET => cells.push(String::new()),
                _ => {}
            },
            Event::Text(text) if total_rows <= MAX_ROWS_PER_SHEET => {
                if in_value {
                    value.push_str(&text.unescape()?);
                }
                if in_text {
                    inline.push_str(&text.unescape()?);
                }
            }
            Event::End(tag) => match tag.local_name().as_ref() {
                b"v" => in_value = false,
                b"t" => in_text = false,
                b"c" if total_rows <= MAX_ROWS_PER_SHEET => {
                    let text = match cell_type.as_str() {
                        "s" => value
                            .parse::<usize>()
                            .ok()
                            .and_then(|i| shared.get(i))
                            .cloned()
                            .unwrap_or_else(|| value.clone()),
                        "inlineStr" => inline.clone(),
                        "d" => chrono::DateTime::parse_from_rfc3339(&value)
                            .ok()
                            .map(|date| {
                                date.with_timezone(&chrono::Utc)
                                    .format("%Y-%m-%dT%H:%M:%SZ")
                                    .to_string()
                            })
                            .unwrap_or_else(|| value.clone()),
                        _ => value.clone(),
                    };
                    cells.push(escape_csv(&text));
                }
                b"row" if total_rows <= MAX_ROWS_PER_SHEET => rows.push(cells.join(",")),
                _ => {}
            },
            Event::DocType(_) => anyhow::bail!("XML document types are unsupported"),
            Event::Eof => break,
            _ => {}
        }
    }
    Ok((total_rows, rows.join("\n")))
}

fn escape_csv(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

// ---------------------------------------------------------------------------
// Plain text & everything else
// ---------------------------------------------------------------------------

fn load_plain_bytes(bytes: &[u8], path: &Path, filename: &str) -> Result<Attachment, AttachError> {
    if bytes.contains(&0) {
        let ext = path
            .extension()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        return Err(AttachError::new(format!(
            "{filename}: unsupported binary format (.{}). Supported: images, PDF, docx/rtf, xlsx, and text files.",
            ext.to_lowercase()
        )));
    }
    let mut text = String::from_utf8_lossy(bytes).trim().to_string();
    if text.is_empty() {
        return Err(AttachError::new(format!("{filename}: file is empty.")));
    }
    let mut note: Option<String> = None;
    if text_count(&text) > MAX_TEXT_CHARS {
        text = truncate_text(&text);
        note = Some(format!("truncated at {MAX_TEXT_CHARS} characters"));
    }
    let is_warning = note.is_some();
    Ok(Attachment::new(
        filename.to_string(),
        sniff_mime(bytes, path),
        text.into_bytes(),
        AttachKind::Text,
        note,
        is_warning,
    ))
}

/// Extension guess first, content sniff as fallback — the `mime_guess` +
/// `infer` replacement for UTI deduction.
fn sniff_mime(bytes: &[u8], path: &Path) -> String {
    let ext_mime = mime_guess::from_path(path).first().map(|m| m.to_string());
    match ext_mime {
        Some(m) if m != "application/octet-stream" => m,
        _ => infer::get(bytes)
            .map(|t| t.mime_type().to_string())
            .unwrap_or_else(|| ext_mime.unwrap_or_else(|| "text/plain".to_string())),
    }
}

// ---------------------------------------------------------------------------
// Minimal base64 (standard alphabet, padded).
// Kept local to preserve the existing persistence/wire helpers and avoid
// requiring another direct dependency for a small standard encoding.
// ---------------------------------------------------------------------------

const B64_ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding.
pub fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(B64_ALPHABET[((n >> 18) & 63) as usize] as char);
        out.push(B64_ALPHABET[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(B64_ALPHABET[((n >> 6) & 63) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(B64_ALPHABET[(n & 63) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// Standard base64 with padding; rejects non-alphabet characters.
pub fn base64_decode(s: &str) -> Result<Vec<u8>, AttachError> {
    fn val(c: u8) -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some(u32::from(c - b'A')),
            b'a'..=b'z' => Some(u32::from(c - b'a') + 26),
            b'0'..=b'9' => Some(u32::from(c - b'0') + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            b'=' => Some(0),
            _ => None,
        }
    }
    let bytes = s.as_bytes();
    if bytes.len() % 4 != 0 {
        return Err(AttachError::new("invalid base64 length."));
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.chunks(4) {
        let mut n: u32 = 0;
        let mut pad = 0;
        for (i, &c) in chunk.iter().enumerate() {
            if c == b'=' {
                pad += 1;
            } else if pad > 0 {
                return Err(AttachError::new("invalid base64 padding."));
            }
            n |= val(c).ok_or_else(|| AttachError::new("invalid base64 character."))?
                << (18 - 6 * i);
        }
        out.push((n >> 16) as u8);
        if pad < 2 {
            out.push((n >> 8) as u8);
        }
        if pad < 1 {
            out.push(n as u8);
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_drop_and_path_load_use_the_same_text_pdf_rtf_and_image_decoders() {
        let dir = tempfile::tempdir().unwrap();
        let mut image_bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(8, 6)
            .write_to(&mut image_bytes, image::ImageFormat::Png)
            .unwrap();
        for (name, bytes) in [
            ("文字.txt", "拖拉與貼上走同一個 loader".as_bytes().to_vec()),
            ("document.pdf", pdf_fixture(&["byte drop PDF"])),
            ("rich.rtf", br"{\rtf1\ansi direct drop text}".to_vec()),
            ("image.png", image_bytes.into_inner()),
        ] {
            let path = dir.path().join(name);
            std::fs::write(&path, &bytes).unwrap();
            assert_eq!(
                load_bytes(&bytes, name).unwrap(),
                load_file(&path).unwrap(),
                "{name}"
            );
        }
        let huge = vec![0; MAX_FILE_BYTES as usize + 1];
        assert!(load_bytes(&huge, "oversized.png")
            .unwrap_err()
            .message
            .contains("too large"));
        assert!(load_bytes(b"not a valid image", "image.png").is_err());
    }
    use std::io::Cursor;

    fn test_png(width: u32, height: u32) -> Vec<u8> {
        let img = image::RgbImage::from_fn(width, height, |x, y| {
            image::Rgb([(x % 256) as u8, (y % 256) as u8, 128])
        });
        let mut cursor = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut cursor, image::ImageFormat::Png)
            .unwrap();
        cursor.into_inner()
    }

    #[test]
    fn large_image_is_downscaled_to_edge_cap() {
        let bytes = test_png(3000, 2000);
        let att = load_image_bytes(&bytes, "big.png").unwrap();
        assert_eq!(att.kind, AttachKind::Image);
        assert!(att.note.as_deref().unwrap_or("").contains("downscaled"));
        let raw = base64_decode(&att.data_base64).unwrap();
        let back = image::load_from_memory(&raw).unwrap();
        assert!(
            back.width().max(back.height()) <= MAX_IMAGE_EDGE,
            "got {}x{}",
            back.width(),
            back.height()
        );
        // Source truncates the scaled dimensions to integers.
        assert_eq!((back.width(), back.height()), (2048, 1365));
        assert_eq!(att.mime, "image/jpeg");
        assert!(!att.note_is_warning);
    }

    #[test]
    fn small_image_is_kept_as_is() {
        let bytes = test_png(100, 80);
        let att = load_image_bytes(&bytes, "small.png").unwrap();
        assert_eq!(att.note, None);
        let raw = base64_decode(&att.data_base64).unwrap();
        let back = image::load_from_memory(&raw).unwrap();
        assert_eq!((back.width(), back.height()), (100, 80));
        assert_eq!(att.mime, "image/jpeg");
        assert_eq!(raw, {
            let mut expected = Vec::new();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut expected, 80)
                .encode_image(&image::load_from_memory(&bytes).unwrap().to_rgb8())
                .unwrap();
            expected
        });
        assert_eq!(att.filename, "small.png");
    }

    #[test]
    fn invalid_image_bytes_fail_explicitly() {
        let err = load_image_bytes(b"not an image", "x.png").unwrap_err();
        assert!(err.message.contains("couldn't decode image"));
    }

    #[test]
    fn base64_roundtrip() {
        for len in [0, 1, 2, 3, 4, 5, 100, 1000] {
            let data: Vec<u8> = (0..len).map(|i| (i * 7 + 3) as u8).collect();
            assert_eq!(base64_decode(&base64_encode(&data)).unwrap(), data);
        }
        assert_eq!(base64_encode(b"ABC"), "QUJD");
        assert!(base64_decode("!!!").is_err());
    }

    #[test]
    fn invalid_pdf_is_an_explicit_decode_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("doc.pdf");
        std::fs::write(&path, b"%PDF-1.4 fake").unwrap();
        let err = load_file(&path).unwrap_err();
        assert!(
            err.message.contains("couldn't open PDF"),
            "got: {}",
            err.message
        );
    }

    #[test]
    fn plain_text_load_and_wire_inline() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.txt");
        std::fs::write(&path, "hello world").unwrap();
        let att = load_file(&path).unwrap();
        assert_eq!(att.kind, AttachKind::Text);
        assert_eq!(att.text_content().as_deref(), Some("hello world"));
        match encode_for_wire(std::slice::from_ref(&att), "ask", false) {
            WireContent::Text(combined) => {
                assert!(combined.contains("[File: note.txt]"));
                assert!(combined.contains("hello world"));
                assert!(combined.contains("ask"));
            }
            WireContent::Parts(_) => panic!("text-only must stay a bare string"),
        }
    }

    #[test]
    fn image_wire_part_survives_refusal() {
        // Images are ALWAYS sent, even when the model is recorded as refusing
        // them — the warning lives in the composer, never as a silent drop.
        let bytes = test_png(10, 10);
        let att = load_image_bytes(&bytes, "shot.png").unwrap();
        match encode_for_wire(std::slice::from_ref(&att), "look", false) {
            WireContent::Parts(parts) => {
                assert!(parts.iter().any(|p| p.part_type == "image_url"));
            }
            WireContent::Text(_) => panic!("image must produce typed parts"),
        }
        assert_eq!(att.capability_exceptions(), vec![ContentCapability::Images]);
    }

    #[test]
    fn missing_kind_backfills_to_text() {
        // Old files without `kind` must still decode.
        let json = serde_json::json!({
            "filename": "old.txt",
            "mime": "text/plain",
            "data_base64": base64_encode(b"legacy"),
        });
        let att: Attachment = serde_json::from_value(json).unwrap();
        assert_eq!(att.kind, AttachKind::Text);
        assert_eq!(att.text_content().as_deref(), Some("legacy"));
        assert_eq!(att.extracted_text, None);
    }

    // All fixtures below are generated in memory and written only to private
    // temporary directories. No test reads a user file, clipboard or profile.
    fn pdf_fixture(texts: &[&str]) -> Vec<u8> {
        use lopdf::{dictionary, Document, Object, Stream};
        let mut document = Document::with_version("1.5");
        let pages_id = document.new_object_id();
        let font_id = document.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
            "Encoding" => "WinAnsiEncoding",
        });
        let resources_id =
            document.add_object(dictionary! { "Font" => dictionary! { "F1" => font_id } });
        let mut kids = Vec::new();
        for text in texts {
            let operations = if text.is_empty() {
                Vec::new()
            } else {
                vec![
                    lopdf::content::Operation::new("BT", vec![]),
                    lopdf::content::Operation::new("Tf", vec!["F1".into(), 12.into()]),
                    lopdf::content::Operation::new("Tj", vec![Object::string_literal(*text)]),
                    lopdf::content::Operation::new("ET", vec![]),
                ]
            };
            let content = lopdf::content::Content { operations }.encode().unwrap();
            let content_id = document.add_object(Stream::new(dictionary! {}, content));
            let page_id = document.add_object(dictionary! {
                "Type" => "Page", "Parent" => pages_id, "Resources" => resources_id,
                "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
                "Contents" => content_id,
            });
            kids.push(page_id.into());
        }
        document.objects.insert(
            pages_id,
            dictionary! {
                "Type" => "Pages", "Kids" => kids, "Count" => texts.len() as i64,
            }
            .into(),
        );
        let catalog_id =
            document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        document.trailer.set("Root", catalog_id);
        let mut bytes = Vec::new();
        document.save_to(&mut bytes).unwrap();
        bytes
    }

    fn zip_fixture(entries: &[(&str, &str)]) -> Vec<u8> {
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, content) in entries {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    fn pad_pdf_to(mut bytes: Vec<u8>, length: usize) -> Vec<u8> {
        // Padding before the final startxref leaves all object offsets valid
        // and keeps the required EOF marker within the last 512 bytes.
        let end = bytes
            .windows(9)
            .rposition(|window| window == b"startxref")
            .unwrap();
        let trailer = bytes.split_off(end);
        bytes.resize(length - trailer.len(), b' ');
        bytes.extend_from_slice(&trailer);
        bytes
    }

    fn active_wire(
        attachment: Attachment,
        files: Option<bool>,
    ) -> super::super::client::WireContent {
        let mut message =
            super::super::store::ChatMessage::new(super::super::store::ChatRole::User, "question");
        message.attachments = vec![into_store_attachment(attachment)];
        super::super::store::wire_content_for_message(
            &message,
            &super::super::providers::AttachmentCapabilities {
                images: Some(false),
                files,
            },
        )
    }

    #[test]
    fn real_pdf_loader_store_wire_retains_both_forms_and_redecides_at_send() {
        let text = "An actual PDF text stream. ".repeat(6);
        let bytes = pdf_fixture(&[&text]);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("report.PDF");
        std::fs::write(&path, &bytes).unwrap();
        let loaded = load_file(&path).unwrap();
        assert_eq!(loaded.kind, AttachKind::Pdf);
        assert_eq!(loaded.raw_bytes().unwrap(), bytes);
        assert_eq!(loaded.extracted_text.as_deref(), Some(text.trim()));
        assert_eq!(loaded.note, None);
        let persisted: Attachment =
            serde_json::from_slice(&serde_json::to_vec(&loaded).unwrap()).unwrap();
        assert_eq!(persisted, loaded);
        let stored = load_store_attachment(&path).unwrap();
        assert!(matches!(
            stored.content,
            super::super::store::AttachmentContent::Pdf { .. }
        ));
        for files in [None, Some(false)] {
            let wire = active_wire(persisted.clone(), files);
            assert_eq!(
                wire,
                super::super::client::WireContent::Text(format!(
                    "[File: report.PDF]\n{}\n\nquestion",
                    text.trim()
                ))
            );
        }
        match active_wire(persisted, Some(true)) {
            super::super::client::WireContent::Parts(parts) => {
                assert_eq!(
                    parts[0].file.as_ref().unwrap().file_data,
                    format!("data:application/pdf;base64,{}", base64_encode(&bytes))
                );
                assert_eq!(parts[0].file.as_ref().unwrap().filename, "report.PDF");
            }
            _ => panic!("native PDF must be a file part"),
        }
    }

    #[test]
    fn scanned_pdf_retains_original_and_text_only_path_explicitly_states_absence() {
        let loaded = load_pdf_bytes(&pdf_fixture(&[""]), "scan.pdf").unwrap();
        assert!(loaded.note.as_deref().unwrap().contains("likely scanned"));
        assert_eq!(loaded.extracted_text.as_deref(), Some(""));
        assert!(loaded.note_is_warning);
        let wire = serde_json::to_string(&active_wire(loaded.clone(), Some(false))).unwrap();
        assert!(wire.contains("no extractable text"));
        assert!(!wire.contains("%PDF"));
        assert!(matches!(
            active_wire(loaded, Some(true)),
            super::super::client::WireContent::Parts(_)
        ));
    }

    #[test]
    fn locked_pdf_is_rejected_but_empty_password_encryption_is_not_a_lock() {
        for password in ["secret", ""] {
            let mut document =
                lopdf::Document::load_mem(&pdf_fixture(&["Readable protected document"])).unwrap();
            document.trailer.set(
                "ID",
                vec![
                    lopdf::Object::string_literal("0123456789abcdef"),
                    lopdf::Object::string_literal("fedcba9876543210"),
                ],
            );
            let state = lopdf::EncryptionState::try_from(lopdf::EncryptionVersion::V2 {
                document: &document,
                owner_password: "owner",
                user_password: password,
                key_length: 128,
                permissions: lopdf::Permissions::all(),
            })
            .unwrap();
            document.encrypt(&state).unwrap();
            let mut bytes = Vec::new();
            document.save_to(&mut bytes).unwrap();
            let loaded = load_pdf_bytes(&bytes, "protected.pdf");
            if password.is_empty() {
                assert_eq!(
                    loaded.unwrap().extracted_text.as_deref(),
                    Some("Readable protected document")
                );
            } else {
                assert!(loaded.unwrap_err().message.contains("password-protected"));
            }
        }
    }

    #[test]
    fn pdf_page_character_and_raw_caps_match_source() {
        let mut pages = vec!["page "; MAX_PDF_PAGES + 1];
        pages[MAX_PDF_PAGES] = "MUST NOT EXTRACT LAST PAGE";
        let loaded = load_pdf_bytes(&pdf_fixture(&pages), "pages.pdf").unwrap();
        assert!(loaded
            .note
            .as_deref()
            .unwrap()
            .contains("first 100 of 101 pages when sent as text"));
        assert!(!loaded
            .extracted_text
            .as_deref()
            .unwrap()
            .contains("MUST NOT"));
        let long_text = "a".repeat(MAX_TEXT_CHARS + 1);
        let loaded = load_pdf_bytes(&pdf_fixture(&[&long_text]), "long.pdf").unwrap();
        assert_eq!(
            loaded.extracted_text.as_ref().unwrap().chars().count(),
            MAX_TEXT_CHARS
        );
        assert!(loaded
            .note
            .unwrap()
            .contains("truncated at 100000 characters when sent as text"));
        let exact_cap = pad_pdf_to(
            pdf_fixture(&["retained at the exact cap"]),
            MAX_DIRECT_PDF_BYTES,
        );
        let retained = load_pdf_bytes(&exact_cap, "exact.pdf").unwrap();
        assert_eq!(retained.kind, AttachKind::Pdf);
        assert_eq!(retained.raw_bytes().unwrap().len(), MAX_DIRECT_PDF_BYTES);
        let large = pad_pdf_to(
            pdf_fixture(&["some readable text"]),
            MAX_DIRECT_PDF_BYTES + 1,
        );
        let loaded = load_pdf_bytes(&large, "large.pdf").unwrap();
        assert_eq!(loaded.kind, AttachKind::Text);
        assert_eq!(loaded.text_content().as_deref(), Some("some readable text"));
        assert!(loaded
            .note
            .as_deref()
            .unwrap()
            .contains("over 10 MB — sent as extracted text"));
        assert!(serde_json::to_string(&active_wire(loaded, Some(true)))
            .unwrap()
            .contains("over 10 MB"));
        let scan = pad_pdf_to(pdf_fixture(&[""]), MAX_DIRECT_PDF_BYTES + 1);
        assert!(load_pdf_bytes(&scan, "large-scan.pdf")
            .unwrap_err()
            .message
            .contains("too large to send as a PDF file"));
    }

    #[test]
    fn docx_and_odt_extract_runs_entities_paragraphs_tabs_and_spaces() {
        let docx = zip_fixture(&[(
            "word/document.xml",
            r#"<w:document xmlns:w="urn:word"><w:body><w:p><w:r><w:t>Hello &amp; </w:t></w:r><w:r><w:t>世界</w:t><w:tab/><w:t>next</w:t><w:br/><w:t>line</w:t></w:r></w:p><w:p><w:r><w:t>Second</w:t></w:r></w:p></w:body></w:document>"#,
        )]);
        let odt = zip_fixture(&[(
            "content.xml",
            r#"<office:document-content xmlns:office="urn:office" xmlns:text="urn:text"><office:body><office:text><text:p>Hello &amp; <text:span>世界</text:span><text:tab/>next<text:line-break/>line</text:p><text:p>Second<text:s text:c="2"/>end</text:p></office:text></office:body></office:document-content>"#,
        )]);
        let directory = tempfile::tempdir().unwrap();
        for (ext, bytes, expected) in [
            ("docx", docx, "Hello & 世界\tnext\nline\nSecond"),
            ("odt", odt, "Hello & 世界\tnext\nline\nSecond  end"),
        ] {
            let path = directory.path().join(format!("document.{ext}"));
            std::fs::write(&path, bytes).unwrap();
            let loaded = load_file(&path).unwrap();
            assert_eq!(loaded.text_content().as_deref(), Some(expected));
            assert_eq!(
                active_wire(loaded, Some(false)),
                super::super::client::WireContent::Text(format!(
                    "[File: document.{ext}]\n{expected}\n\nquestion"
                ))
            );
        }
    }

    #[test]
    fn rtf_extracts_unicode_surrogates_ansi_fields_and_nested_groups() {
        let rtf = br#"{\rtf1\ansi\ansicpg1252{\fonttbl{\f0 Arial;}}Hello {\b bold} \'e9 \u20013?\u25991? \u-10179?\u-8704?\par {\*\unknown secret}{\pict\bin4 abcd}{\field{\*\fldinst HYPERLINK "secret"}{\fldrslt visible}}\tab end}"#;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("document.rtf");
        std::fs::write(&path, rtf).unwrap();
        let loaded = load_file(&path).unwrap();
        let expected = "Hello bold é 中文 😀\nvisible\tend";
        assert_eq!(loaded.text_content().as_deref(), Some(expected));
        assert_eq!(
            active_wire(loaded, None),
            super::super::client::WireContent::Text(format!(
                "[File: document.rtf]\n{expected}\n\nquestion"
            ))
        );
        assert!(extract_rtf(br"{\rtf1 broken").is_err());
        assert!(extract_rtf(br"not rtf").is_err());
        assert!(extract_rtf(br"{\rtf1\ansicpg99999 text}").is_err());
    }

    #[test]
    fn rtf_multibyte_codepage_and_unicode_fallback_scope() {
        assert_eq!(
            extract_rtf(br"{\rtf1\ansi\ansicpg950 \'a4\'a4\'a4\'e5}").unwrap(),
            "中文"
        );
        assert_eq!(
            extract_rtf(br"{\rtf1\uc0\u20013{\uc2\u25991??}\u20013}").unwrap(),
            "中文中"
        );
        assert_eq!(extract_rtf(br"{\rtf1\ansi\ansicpg1252{\fonttbl{\f0\fcharset0 Arial;}{\f1\fcharset136 Ming;}}\f1 \'a4\'a4\'a4\'e5\f0 \'e9}").unwrap(), "中文é");
    }

    #[test]
    fn word_formats_reject_empty_and_corrupt_documents_honestly() {
        for ext in ["doc", "rtfd"] {
            assert!(load_word_bytes(b"legacy", ext, "legacy")
                .unwrap_err()
                .message
                .contains("couldn't extract text"));
        }
        assert!(load_word_bytes(b"not a zip", "docx", "broken.docx")
            .unwrap_err()
            .message
            .contains("couldn't extract text"));
        assert!(load_word_bytes(br"{\rtf1 }", "rtf", "empty.rtf")
            .unwrap_err()
            .message
            .contains("contains no text"));
        assert!(extract_document_xml(b"<!DOCTYPE doc><doc/>", true).is_err());
        assert!(extract_document_xml(b"<document><p><t>unclosed", true).is_err());
    }

    /// Generated here from the public MS-CFB/MS-DOC layouts: a 9.5-KiB CFB v3
    /// file with WordDocument/1Table streams and one UTF-16LE CLX text piece.
    /// No third-party document, executable, fixture, or licensed asset is used.
    fn binary_doc_fixture(text: &str) -> Vec<u8> {
        fn put16(bytes: &mut [u8], offset: usize, value: u16) {
            bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
        }
        fn put32(bytes: &mut [u8], offset: usize, value: u32) {
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        const FREE: u32 = 0xffff_ffff;
        const END: u32 = 0xffff_fffe;
        let mut bytes = vec![0u8; 19 * 512];
        bytes[..8].copy_from_slice(&[0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1]);
        put16(&mut bytes, 24, 0x003e); // CFB minor version.
        put16(&mut bytes, 26, 3); // v3, 512-byte sectors.
        put16(&mut bytes, 28, 0xfffe);
        put16(&mut bytes, 30, 9);
        put16(&mut bytes, 32, 6); // 64-byte mini sectors (unused).
        put32(&mut bytes, 44, 1); // One FAT sector, sector 17.
        put32(&mut bytes, 48, 0); // Directory at sector 0.
        put32(&mut bytes, 56, 4096);
        put32(&mut bytes, 60, END); // No mini-FAT.
        put32(&mut bytes, 68, END); // No extra DIFAT sectors.
        for index in 0..109 {
            put32(&mut bytes, 76 + 4 * index, FREE);
        }
        put32(&mut bytes, 76, 17);
        // Directory tree: root -> WordDocument (black) -> 1Table (red left).
        for (id, name, kind, color, left, child, start, size) in [
            (0, "Root Entry", 5, 1, FREE, 1, END, 0),
            (1, "WordDocument", 2, 1, 2, FREE, 1, 4096),
            (2, "1Table", 2, 0, FREE, FREE, 9, 4096),
        ] {
            let entry = 512 + id * 128;
            let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
            for (index, unit) in name.iter().enumerate() {
                put16(&mut bytes, entry + index * 2, *unit);
            }
            put16(&mut bytes, entry + 64, (name.len() * 2) as u16);
            bytes[entry + 66] = kind;
            bytes[entry + 67] = color;
            put32(&mut bytes, entry + 68, left);
            put32(&mut bytes, entry + 72, FREE);
            put32(&mut bytes, entry + 76, child);
            put32(&mut bytes, entry + 116, start);
            put32(&mut bytes, entry + 120, size);
        }
        let fat = 18 * 512;
        for index in 0..128 {
            put32(&mut bytes, fat + index * 4, FREE);
        }
        for sector in 0..18 {
            let next = match sector {
                0 | 8 | 16 => END,
                17 => 0xffff_fffd,
                _ => sector as u32 + 1,
            };
            put32(&mut bytes, fat + sector * 4, next);
        }
        // Full Word-97 FIB: FibRgW97[14], FibRgLw97[22], 93 fc/lcb pairs.
        let word = &mut bytes[2 * 512..10 * 512];
        let units: Vec<u16> = text.encode_utf16().collect();
        assert!(1024 + units.len() * 2 <= word.len());
        put16(word, 0, 0xa5ec);
        put16(word, 2, 0x00c1);
        put16(word, 6, 0x0409);
        put16(word, 10, 0x0204); // Complex piece table, 1Table, not encrypted.
        put16(word, 12, 0x00bf);
        put16(word, 20, 0x0409);
        put32(word, 24, 1024);
        put32(word, 28, (1024 + units.len() * 2) as u32);
        put16(word, 32, 14);
        put16(word, 62, 22);
        put32(word, 64, 4096);
        put32(word, 76, units.len() as u32); // ccpText counts UTF-16 units.
        put16(word, 152, 93);
        put32(word, 154 + 33 * 8, 0); // fcClx.
        put32(word, 154 + 33 * 8 + 4, 21); // lcbClx.
        for (index, unit) in units.iter().enumerate() {
            put16(word, 1024 + index * 2, *unit);
        }
        let table = &mut bytes[10 * 512..18 * 512];
        table[0] = 2; // Pcdt: one PlcPcd with CPs 0..ccpText and one PCD.
        put32(table, 1, 16);
        put32(table, 9, units.len() as u32);
        put32(table, 15, 1024); // Uncompressed UTF-16 piece byte offset.
        bytes
    }

    #[test]
    fn binary_doc_loader_store_wire_extracts_real_ole_utf16_text() {
        let bytes = binary_doc_fixture("  Legacy DOC: 中文 😀\rSecond paragraph.\r");
        assert_eq!(
            &bytes[..8],
            &[0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1]
        );
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("legacy.DOC");
        std::fs::write(&path, &bytes).unwrap();
        let loaded = load_file(&path).unwrap();
        let expected = "Legacy DOC: 中文 😀\nSecond paragraph.";
        assert_eq!(loaded.kind, AttachKind::Text);
        assert_eq!(loaded.text_content().as_deref(), Some(expected));
        assert_eq!(loaded.note, None);
        let stored = load_store_attachment(&path).unwrap();
        assert_eq!(
            stored.content,
            super::super::store::AttachmentContent::Text(expected.into())
        );
        assert_eq!(
            active_wire(loaded, None),
            super::super::client::WireContent::Text(format!(
                "[File: legacy.DOC]\n{expected}\n\nquestion"
            ))
        );
    }

    #[test]
    fn doc_rejects_corruption_encryption_and_empty_text_without_fake_extraction() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("bad.doc");
        let mut bytes = binary_doc_fixture("secret\r");
        bytes[2 * 512 + 11] |= 1; // fEncrypted bit 8.
        std::fs::write(&path, bytes).unwrap();
        assert!(load_file(&path).unwrap_err().message.contains("encrypted"));
        std::fs::write(&path, b"not a DOC or RTF").unwrap();
        assert!(load_file(&path)
            .unwrap_err()
            .message
            .contains("couldn't extract text"));
        std::fs::write(&path, binary_doc_fixture("\r")).unwrap();
        assert!(load_file(&path)
            .unwrap_err()
            .message
            .contains("document contains no text"));
    }

    #[test]
    fn rtf_disguised_as_doc_uses_detection_and_common_truncation_pipeline() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("word.doc");
        std::fs::write(
            &path,
            b"\xef\xbb\xbf \r\n{\\rtf1\\ansi  RTF \\u20013?\\u25991?\\par text  }",
        )
        .unwrap();
        let loaded = load_file(&path).unwrap();
        assert_eq!(loaded.text_content().as_deref(), Some("RTF 中文\ntext"));
        assert_eq!(
            active_wire(loaded, None),
            super::super::client::WireContent::Text(
                "[File: word.doc]\nRTF 中文\ntext\n\nquestion".into()
            )
        );
        std::fs::write(
            &path,
            format!("{{\\rtf1 {} }}", "a".repeat(MAX_TEXT_CHARS + 1)),
        )
        .unwrap();
        let loaded = load_file(&path).unwrap();
        assert_eq!(text_count(&loaded.text_content().unwrap()), MAX_TEXT_CHARS);
        assert_eq!(
            loaded.note.as_deref(),
            Some("truncated at 100000 characters")
        );
        assert!(serde_json::to_string(&active_wire(loaded, None))
            .unwrap()
            .contains("truncated at 100000 characters"));
    }

    #[test]
    fn rtfd_directory_loader_store_wire_reads_only_direct_main_rtf() {
        let directory = tempfile::tempdir().unwrap();
        let bundle = directory.path().join("notes.rtfd");
        std::fs::create_dir(&bundle).unwrap();
        std::fs::write(bundle.join("TXT.rtf"), br"{\rtf1\ansi  Bundle \u20013?\u25991?\par before{\NeXTGraphic outside.png \width100 \height100}after  }").unwrap();
        // Unreadable/oversized/unrelated assets must never be decoded or scanned.
        std::fs::File::create(bundle.join("outside.png"))
            .unwrap()
            .set_len(MAX_FILE_BYTES + 1)
            .unwrap();
        std::fs::create_dir(bundle.join("nested")).unwrap();
        std::fs::write(bundle.join("nested/TXT.rtf"), br"{\rtf1 MUST NOT INCLUDE}").unwrap();
        let loaded = load_file(&bundle).unwrap();
        let expected = "Bundle 中文\nbefore\u{fffc}after";
        assert_eq!(loaded.filename, "notes.rtfd");
        assert_eq!(loaded.text_content().as_deref(), Some(expected));
        assert_eq!(
            load_store_attachment(&bundle).unwrap().content,
            super::super::store::AttachmentContent::Text(expected.into())
        );
        assert_eq!(
            active_wire(loaded, None),
            super::super::client::WireContent::Text(format!(
                "[File: notes.rtfd]\n{expected}\n\nquestion"
            ))
        );
    }

    #[test]
    fn rtfd_requires_bounded_regular_direct_main_file_and_preserves_warnings() {
        let directory = tempfile::tempdir().unwrap();
        let bundle = directory.path().join("notes.rtfd");
        std::fs::create_dir_all(bundle.join("nested")).unwrap();
        std::fs::write(
            bundle.join("nested/TXT.rtf"),
            br"{\rtf1 not the main document}",
        )
        .unwrap();
        assert!(load_file(&bundle)
            .unwrap_err()
            .message
            .contains("no direct TXT.rtf"));
        std::fs::create_dir(bundle.join("TXT.rtf")).unwrap();
        assert!(load_file(&bundle)
            .unwrap_err()
            .message
            .contains("regular file"));
        std::fs::remove_dir(bundle.join("TXT.rtf")).unwrap();
        let main = bundle.join("TXT.rtf");
        std::fs::File::create(&main)
            .unwrap()
            .set_len(MAX_FILE_BYTES + 1)
            .unwrap();
        assert!(load_file(&bundle)
            .unwrap_err()
            .message
            .contains("main text too large"));
        std::fs::write(&main, br"{\rtf1 }").unwrap();
        assert!(load_file(&bundle)
            .unwrap_err()
            .message
            .contains("document contains no text"));
        std::fs::write(
            &main,
            format!("{{\\rtf1 {} }}", "b".repeat(MAX_TEXT_CHARS + 1)),
        )
        .unwrap();
        let loaded = load_file(&bundle).unwrap();
        assert_eq!(text_count(&loaded.text_content().unwrap()), MAX_TEXT_CHARS);
        assert!(serde_json::to_string(&active_wire(loaded, None))
            .unwrap()
            .contains("truncated at 100000 characters"));
    }

    #[test]
    fn word_character_truncation_warning_survives_store_conversion() {
        let text = "word ".repeat(MAX_TEXT_CHARS / 5 + 2);
        let xml = format!("<document><body><p><r><t>{text}</t></r></p></body></document>");
        let bytes = zip_fixture(&[("word/document.xml", &xml)]);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("long.docx");
        std::fs::write(&path, bytes).unwrap();
        let loaded = load_file(&path).unwrap();
        assert_eq!(text_count(&loaded.text_content().unwrap()), MAX_TEXT_CHARS);
        assert_eq!(
            loaded.note.as_deref(),
            Some("truncated at 100000 characters")
        );
        assert!(serde_json::to_string(&active_wire(loaded, None))
            .unwrap()
            .contains("truncated at 100000 characters"));
    }

    #[test]
    fn text_limits_empty_binary_and_lossy_utf8_match_source() {
        let path = Path::new("plain.txt");
        for empty in [b"".as_slice(), b" \r\n\t"] {
            assert!(load_plain_bytes(empty, path, "plain.txt")
                .unwrap_err()
                .message
                .contains("empty"));
        }
        assert!(load_plain_bytes(b"a\0b", path, "plain.txt")
            .unwrap_err()
            .message
            .contains("unsupported binary format"));
        assert_eq!(
            load_plain_bytes(b" a\xff ", path, "plain.txt")
                .unwrap()
                .text_content()
                .as_deref(),
            Some("a�")
        );
        let text = "文".repeat(MAX_TEXT_CHARS + 1);
        let loaded = load_plain_bytes(text.as_bytes(), path, "plain.txt").unwrap();
        assert_eq!(
            loaded.text_content().unwrap().chars().count(),
            MAX_TEXT_CHARS
        );
        let wire = serde_json::to_string(&active_wire(loaded, None)).unwrap();
        assert!(wire.contains("truncated at 100000 characters"));
    }

    #[test]
    fn text_character_cap_counts_swift_graphemes_not_utf8_or_scalars() {
        let cluster = "👩‍💻e\u{301}";
        let text = cluster.repeat(MAX_TEXT_CHARS / 2 + 1);
        let loaded =
            load_plain_bytes(text.as_bytes(), Path::new("unicode.txt"), "unicode.txt").unwrap();
        assert_eq!(
            loaded.text_content().unwrap(),
            cluster.repeat(MAX_TEXT_CHARS / 2)
        );
        assert_eq!(text_count(&loaded.text_content().unwrap()), MAX_TEXT_CHARS);
    }

    #[test]
    fn file_size_cap_rejects_before_decode() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("large.png");
        std::fs::File::create(&path)
            .unwrap()
            .set_len(MAX_FILE_BYTES + 1)
            .unwrap();
        let error = load_store_attachment(&path).unwrap_err().to_string();
        assert!(error.contains("too large (25 MB — limit is 25 MB)"));
    }

    #[test]
    fn spreadsheet_uses_actual_rows_cells_and_raw_numeric_values_like_corexlsx() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("table.XLSX");
        let workbook = zip_fixture(&[
            (
                "xl/workbook.xml",
                r#"<workbook xmlns:r="urn:rels"><sheets><sheet name="Data &amp; values" r:id="rId1"/></sheets></workbook>"#,
            ),
            (
                "xl/_rels/workbook.xml.rels",
                r#"<Relationships><Relationship Id="rId1" Target="worksheets/sheet1.xml"/></Relationships>"#,
            ),
            (
                "xl/sharedStrings.xml",
                r#"<sst><si><t>comma, quote &quot; and
newline</t></si><si><r><t>rich </t></r><r><t>string</t></r></si></sst>"#,
            ),
            (
                "xl/worksheets/sheet1.xml",
                r#"<worksheet><sheetData><row r="1"><c r="A1" t="s"><v>0</v></c><c r="D1" t="inlineStr"><is><t>inline</t></is></c><c r="E1"><v>42.00</v></c><c r="F1" s="1"><v>45123</v></c><c r="G1" t="d"><v>2026-10-28T12:30:45+02:00</v></c></row><row r="8"><c r="A8"><f>SUM(1,2)</f><v>3</v></c><c r="B8" t="s"><v>1</v></c></row></sheetData></worksheet>"#,
            ),
        ]);
        std::fs::write(&path, workbook).unwrap();
        let loaded = load_file(&path).unwrap();
        let expected = "--- Data & values (2 rows) ---\n\"comma, quote \"\" and\nnewline\",inline,42.00,45123,2026-10-28T10:30:45Z\n3,rich string";
        assert_eq!(loaded.text_content().as_deref(), Some(expected));
        assert_eq!(
            active_wire(loaded, None),
            super::super::client::WireContent::Text(format!(
                "[File: table.XLSX]\n{expected}\n\nquestion"
            ))
        );
    }

    #[test]
    fn spreadsheet_row_cap_is_per_sheet_and_warns_on_active_wire() {
        let mut rows = String::from("<worksheet><sheetData>");
        for index in 0..201 {
            rows.push_str(&format!("<row><c><v>{index}</v></c></row>"));
        }
        rows.push_str("</sheetData></worksheet>");
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("many.xlsm");
        std::fs::write(&path, zip_fixture(&[
            ("xl/workbook.xml", r#"<workbook xmlns:r="urn:rels"><sheets><sheet name="First" r:id="r1"/><sheet name="Second" r:id="r2"/></sheets></workbook>"#),
            ("xl/_rels/workbook.xml.rels", r#"<Relationships><Relationship Id="r1" Target="/xl/worksheets/a.xml"/><Relationship Id="r2" Target="worksheets/b.xml"/></Relationships>"#),
            ("xl/worksheets/a.xml", &rows), ("xl/worksheets/b.xml", &rows),
        ])).unwrap();
        let loaded = load_file(&path).unwrap();
        assert_eq!(
            loaded.note.as_deref(),
            Some("First: first 200 of 201 rows; Second: first 200 of 201 rows")
        );
        let body = loaded.text_content().unwrap();
        assert!(body.contains("--- First (201 rows) ---"));
        assert!(body.contains("--- Second (201 rows) ---"));
        assert!(!body.lines().any(|line| line == "200"));
        assert!(serde_json::to_string(&active_wire(loaded, None))
            .unwrap()
            .contains("first 200 of 201 rows"));
    }

    #[test]
    fn image_store_wire_does_not_send_routine_downscale_note_or_drop_refused_image() {
        let loaded = load_image_bytes(&test_png(3000, 10), "photo.png").unwrap();
        match active_wire(loaded, Some(false)) {
            super::super::client::WireContent::Parts(parts) => {
                assert!(parts[0]
                    .image_url
                    .as_ref()
                    .unwrap()
                    .url
                    .starts_with("data:image/jpeg;base64,"));
                assert!(!serde_json::to_string(&parts)
                    .unwrap()
                    .contains("downscaled"));
            }
            _ => panic!("image must survive recorded refusal"),
        }
    }

    #[test]
    fn old_pdf_without_extraction_never_sends_raw_bytes_as_text() {
        let loaded: Attachment = serde_json::from_value(serde_json::json!({
            "filename": "old.pdf", "mime": "application/pdf", "kind": "pdf",
            "data_base64": base64_encode(b"%PDF-secret raw bytes"),
        }))
        .unwrap();
        let wire = serde_json::to_string(&active_wire(loaded.clone(), None)).unwrap();
        assert!(wire.contains("no extractable text"));
        assert!(!wire.contains("secret raw bytes"));
        assert!(loaded
            .wire_part(false)
            .text
            .unwrap()
            .contains("no extractable text"));
    }
}
