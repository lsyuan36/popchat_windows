//! Composer-independent draft attachments. Loading is synchronous: the host
//! runs it on a worker and appends successful results on the UI thread.

use std::path::Path;

use crate::chat::{attach, store};

#[derive(Debug, Clone, Default)]
pub struct AttachmentDraft {
    pub attachments: Vec<store::Attachment>,
}

impl AttachmentDraft {
    pub fn new() -> Self {
        Self::default()
    }

    /// Load one file, leaving the existing draft intact on failure.
    pub fn load_path(&mut self, path: &Path) -> anyhow::Result<()> {
        self.attachments.push(attach::load_store_attachment(path)?);
        Ok(())
    }

    /// Keep each readable file; report failures individually, never silently.
    pub fn load_paths<P: AsRef<Path>>(
        &mut self,
        paths: impl IntoIterator<Item = P>,
    ) -> Vec<anyhow::Error> {
        paths
            .into_iter()
            .filter_map(|path| self.load_path(path.as_ref()).err())
            .collect()
    }

    /// `false` means the clipboard has no image (the text-paste path may proceed).
    pub fn load_clipboard_image(&mut self) -> anyhow::Result<bool> {
        match attach::load_clipboard_store_attachment()? {
            Some(attachment) => {
                self.attachments.push(attachment);
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// Image bytes supplied by the host; useful for paste/drop integrations
    /// without opening the system clipboard inside this helper.
    pub fn load_image_bytes(&mut self, bytes: &[u8], suggested_name: &str) -> anyhow::Result<()> {
        self.attachments
            .push(attach::into_store_attachment(attach::load_image_bytes(
                bytes,
                suggested_name,
            )?));
        Ok(())
    }

    pub fn remove(&mut self, index: usize) -> Option<store::Attachment> {
        (index < self.attachments.len()).then(|| self.attachments.remove(index))
    }

    pub fn clear(&mut self) {
        self.attachments.clear();
    }

    /// Transfer ownership only when the host accepts a send.
    pub fn take(&mut self) -> Vec<store::Attachment> {
        std::mem::take(&mut self.attachments)
    }

    /// Filename chips with visible warning notes and remove buttons. Does not
    /// invent a picker; the host opens its native dialog and supplies paths.
    pub fn show(&mut self, ui: &mut egui::Ui) {
        let mut remove = None;
        ui.horizontal_wrapped(|ui| {
            for (index, attachment) in self.attachments.iter().enumerate() {
                ui.push_id(attachment.id, |ui| {
                    ui.group(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(&attachment.filename);
                            if super::icons::hover_help(ui.small_button("×"), "移除附件").clicked()
                            {
                                remove = Some(index);
                            }
                        });
                        if let Some(note) = &attachment.note {
                            if attachment.note_kind == store::NoteKind::Warning {
                                ui.colored_label(ui.visuals().warn_fg_color, format!("⚠ {note}"));
                            } else {
                                ui.weak(note);
                            }
                        }
                    });
                });
            }
        });
        if let Some(index) = remove {
            self.remove(index);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draft_load_remove_clear_and_take_are_local() {
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("first.txt");
        let second = directory.path().join("second.txt");
        std::fs::write(&first, "first").unwrap();
        std::fs::write(&second, "second").unwrap();
        let mut draft = AttachmentDraft::new();
        let errors = draft.load_paths([first, directory.path().join("missing.txt"), second]);
        assert_eq!(errors.len(), 1);
        assert_eq!(draft.attachments.len(), 2);
        assert!(draft.remove(99).is_none());
        assert_eq!(draft.remove(0).unwrap().filename, "first.txt");
        assert_eq!(draft.take()[0].filename, "second.txt");
        assert!(draft.attachments.is_empty());
        draft.attachments.push(store::Attachment::new(
            "a",
            store::AttachmentContent::Text("a".into()),
        ));
        draft.clear();
        assert!(draft.attachments.is_empty());
    }

    #[test]
    fn failed_image_paste_does_not_modify_draft() {
        let mut draft = AttachmentDraft::new();
        assert!(draft.load_image_bytes(b"invalid", "Pasted image").is_err());
        assert!(draft.attachments.is_empty());
    }
}
