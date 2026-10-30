//! Slash commands and the system prompt. Port of `ShortcutStore.swift` plus
//! the `/`-expansion logic in `ChatStore.send` and the system-prompt accessors.
//!
//! A user-defined prompt shortcut is invoked as `/name input` in the chat
//! field. `{input}` in the template marks where the typed remainder goes;
//! without a placeholder the input is appended on a new line. The display text
//! stays compact (`/name input`); the expanded template goes on the wire.
//!
//! Persistence (settings file) is intentionally NOT done here: the store
//! serializes to/from JSON so whatever settings layer exists can load and save
//! it without this module touching disk.

use serde::{Deserialize, Serialize};

/// Settings key under which the configured system prompt is stored.
pub const SYSTEM_PROMPT_KEY: &str = "systemPrompt";

/// Teaches the model the pasteable-block format the renderer recognizes.
/// Editable in Settings → Commands; an explicitly empty value means "send no
/// system prompt".
pub const DEFAULT_SYSTEM_PROMPT: &str =
    "You are a helpful assistant in a small desktop chat panel. Be concise and direct.\n\
\n\
When a reply includes content the user will copy and reuse verbatim — a prompt, template, \
code snippet, configuration, or other reusable text — wrap exactly that content in a \
pasteable block:\n\
\n\
<pasteable title=\"Short label\">\n\
the exact reusable content\n\
</pasteable>\n\
\n\
Pasteable rules:\n\
- The tags go on their own lines, with a short descriptive title.\n\
- Inside the tags, put only the content to copy — no commentary.\n\
- Keep explanations outside the block, in normal prose.\n\
- Use pasteable blocks only for content meant to be copied; use normal markdown code \
fences for illustrative code.";

pub const DEFAULT_TRANSLATE_TEMPLATE: &str =
    "Translate the following into natural, idiomatic English. Reply with the translation only:\n\n{input}";
pub const DEFAULT_EXPLAIN_TEMPLATE: &str =
    "Explain the following clearly and concisely, assuming a technical reader:\n\n{input}";

/// Resolves the stored system-prompt setting to what a request should carry:
/// missing → default, blank → `None` (send no system prompt).
pub fn effective_system_prompt(stored: Option<&str>) -> Option<String> {
    match stored {
        None => Some(DEFAULT_SYSTEM_PROMPT.to_string()),
        Some(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptShortcut {
    pub id: String,
    pub name: String,
    pub template: String,
}

impl PromptShortcut {
    pub fn expand(&self, input: &str) -> String {
        if self.template.contains("{input}") {
            self.template.replace("{input}", input)
        } else if input.is_empty() {
            self.template.clone()
        } else {
            format!("{}\n\n{input}", self.template)
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ShortcutStore {
    pub shortcuts: Vec<PromptShortcut>,
}

impl ShortcutStore {
    pub fn with_defaults() -> Self {
        Self {
            shortcuts: vec![
                PromptShortcut {
                    id: "default-translate".to_string(),
                    name: "translate".to_string(),
                    template: DEFAULT_TRANSLATE_TEMPLATE.to_string(),
                },
                PromptShortcut {
                    id: "default-explain".to_string(),
                    name: "explain".to_string(),
                    template: DEFAULT_EXPLAIN_TEMPLATE.to_string(),
                },
            ],
        }
    }

    pub fn from_json(json: &str) -> Option<Self> {
        // Accepts both the whole store and (for forward tolerance) a bare
        // array, which older builds wrote.
        serde_json::from_str(json).ok().or_else(|| {
            serde_json::from_str::<Vec<PromptShortcut>>(json)
                .ok()
                .map(|shortcuts| Self { shortcuts })
        })
    }

    pub fn to_json(&self) -> Option<String> {
        serde_json::to_string(self).ok()
    }

    /// Case-insensitive exact match (the send path).
    pub fn match_name(&self, name: &str) -> Option<&PromptShortcut> {
        self.shortcuts
            .iter()
            .find(|s| s.name.to_lowercase() == name.to_lowercase())
    }

    /// `/`-prefix autocomplete: names starting with `prefix` (leading `/`
    /// stripped, case-insensitive), in store order.
    pub fn complete(&self, prefix: &str) -> Vec<&PromptShortcut> {
        let needle = prefix.strip_prefix('/').unwrap_or(prefix).to_lowercase();
        self.shortcuts
            .iter()
            .filter(|s| s.name.to_lowercase().starts_with(&needle))
            .collect()
    }

    /// Splits `/name remainder` into `(name, input)`. Returns `None` when the
    /// text is not a slash invocation. Mirrors `ChatStore.send`: the name is
    /// the leading non-whitespace run, the input is the trimmed remainder.
    pub fn parse_invocation(text: &str) -> Option<(String, String)> {
        let body = text.strip_prefix('/')?;
        let name: String = body.chars().take_while(|c| !c.is_whitespace()).collect();
        let input = body[name.len()..].trim().to_string();
        Some((name, input))
    }

    /// Full send-path expansion: `Ok` carries the wire text, `Err` carries the
    /// user-facing error row text for unknown shortcuts.
    pub fn expand_command(&self, text: &str) -> Result<String, String> {
        match Self::parse_invocation(text) {
            Some((name, input)) => match self.match_name(&name) {
                Some(shortcut) => Ok(shortcut.expand(&input)),
                None => Err(format!(
                    "Unknown shortcut “/{name}”. Define it in Settings → Slash Commands, or escape the leading slash."
                )),
            },
            None => Ok(text.to_string()),
        }
    }

    pub fn add(&mut self) -> &PromptShortcut {
        let mut name = "new-shortcut".to_string();
        let mut counter = 2;
        while self.match_name(&name).is_some() {
            name = format!("new-shortcut-{counter}");
            counter += 1;
        }
        let id = format!("shortcut-{name}");
        self.shortcuts.push(PromptShortcut {
            id,
            name,
            template: "…{input}".to_string(),
        });
        self.shortcuts.last().expect("just pushed")
    }

    pub fn remove(&mut self, id: &str) {
        self.shortcuts.retain(|s| s.id != id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> ShortcutStore {
        ShortcutStore::with_defaults()
    }

    #[test]
    fn expand_replaces_placeholder() {
        let shortcut = PromptShortcut {
            id: "1".to_string(),
            name: "t".to_string(),
            template: "Say: {input}!".to_string(),
        };
        assert_eq!(shortcut.expand("hi"), "Say: hi!");
    }

    #[test]
    fn expand_without_placeholder_appends() {
        let shortcut = PromptShortcut {
            id: "1".to_string(),
            name: "t".to_string(),
            template: "Prefix".to_string(),
        };
        assert_eq!(shortcut.expand("body"), "Prefix\n\nbody");
        assert_eq!(shortcut.expand(""), "Prefix");
    }

    #[test]
    fn parse_invocation_splits_name_and_input() {
        assert_eq!(
            ShortcutStore::parse_invocation("/translate hello world"),
            Some(("translate".to_string(), "hello world".to_string()))
        );
        assert_eq!(
            ShortcutStore::parse_invocation("/explain"),
            Some(("explain".to_string(), String::new()))
        );
        assert_eq!(ShortcutStore::parse_invocation("no slash"), None);
    }

    #[test]
    fn expand_command_resolves_and_reports_unknown() {
        let store = store();
        assert_eq!(
            store.expand_command("/translate bonjour").unwrap(),
            "Translate the following into natural, idiomatic English. Reply with the translation only:\n\nbonjour"
        );
        let error = store.expand_command("/nope x").unwrap_err();
        assert!(error.contains("Unknown shortcut"), "got {error:?}");
        assert!(error.contains("/nope"), "got {error:?}");
        // Non-slash text passes through untouched.
        assert_eq!(store.expand_command("plain").unwrap(), "plain");
    }

    #[test]
    fn match_is_case_insensitive_and_complete_filters() {
        let store = store();
        assert!(store.match_name("TRANSLATE").is_some());
        let names: Vec<&str> = store
            .complete("/t")
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(names, ["translate"]);
        assert_eq!(store.complete("/").len(), 2);
        assert!(store.complete("/zzz").is_empty());
    }

    #[test]
    fn add_remove_roundtrip() {
        let mut store = store();
        let id = store.add().id.clone();
        assert_eq!(store.shortcuts.len(), 3);
        store.remove(&id);
        assert_eq!(store.shortcuts.len(), 2);
    }

    #[test]
    fn json_roundtrip() {
        let store = store();
        let json = store.to_json().unwrap();
        let restored = ShortcutStore::from_json(&json).unwrap();
        assert_eq!(restored.shortcuts, store.shortcuts);
    }

    #[test]
    fn effective_system_prompt_semantics() {
        assert_eq!(
            effective_system_prompt(None).as_deref(),
            Some(DEFAULT_SYSTEM_PROMPT)
        );
        assert_eq!(effective_system_prompt(Some("  \n ")), None);
        assert_eq!(
            effective_system_prompt(Some("  custom  ")).as_deref(),
            Some("custom")
        );
    }
}
