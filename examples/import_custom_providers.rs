//! Import ONLY custom provider metadata and its native Windows credentials.
//!
//! Usage: import_custom_providers --source-profile PATH [--expected-count 2] [--apply]
//!
//! DRY RUN is the default (metadata only; no credential reads or writes). The
//! installer MUST hold its supervisor/exclusive installation guard, close the
//! app, and verify zero remaining instances before invoking --apply. This helper
//! does not supervise processes or prove that the app is stopped. Keep the guard
//! held until it exits. It never reads settings, history, or legacy secrets.
//!
//! The destination is always ProjectDirs::from("com", "PopChat", "PopChat")'s
//! config_dir, ignoring POPCHAT_DATA_DIR. Supply the EXACT absolute source path
//! spelling previously used for POPCHAT_DATA_DIR: that spelling determines the
//! native credential namespace. Canonicalization is only used to reject a
//! source/destination alias, never to derive credential accounts.
//!
//! Credentials and JSON cannot form a perfectly atomic cross-resource
//! transaction. Newly created target credentials are rolled back on failure;
//! originals and a synced, uniquely named metadata backup are retained. A crash
//! can leave new credentials or the backup behind. No source key is deleted.

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Result};
use popchat_windows::chat::providers::{CapabilityException, ModelCapabilities, Provider};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

const MAPS: [&str; 9] = [
    "known_models",
    "selected_models",
    "settings_models",
    "known_model_efforts",
    "default_model_efforts",
    "selected_model_efforts",
    "model_capabilities",
    "capability_exceptions",
    "pdf_pass_through",
];

struct Options {
    source: PathBuf,
    expected_count: usize,
    apply: bool,
}

fn usage() -> &'static str {
    "Usage: import_custom_providers --source-profile PATH [--expected-count 2] [--apply]\nDefault: DRY RUN. --apply requires the installer supervisor guard and zero app instances.\nUse the exact absolute source-profile spelling used by the app. Destination: formal default profile."
}

fn parse_options(args: impl IntoIterator<Item = OsString>) -> Result<Option<Options>> {
    let mut args = args.into_iter();
    let mut source = None;
    let mut expected_count = None;
    let mut apply = false;
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--help" | "-h") => return Ok(None),
            Some("--source-profile") if source.is_none() => {
                source = Some(PathBuf::from(
                    args.next().ok_or_else(|| anyhow!("Missing source path"))?,
                ));
            }
            Some("--expected-count") if expected_count.is_none() => {
                expected_count = Some(
                    args.next()
                        .and_then(|v| v.to_str().and_then(|v| v.parse::<usize>().ok()))
                        .filter(|n| *n > 0)
                        .ok_or_else(|| anyhow!("Expected count must be a positive integer"))?,
                );
            }
            Some("--apply") if !apply => apply = true,
            _ => bail!("Invalid or repeated argument; use --help"),
        }
    }
    let source = source.ok_or_else(|| anyhow!("--source-profile is required"))?;
    if !source.is_absolute() {
        bail!("Source profile must be an absolute path with its original spelling");
    }
    Ok(Some(Options {
        source,
        expected_count: expected_count.unwrap_or(2),
        apply,
    }))
}

fn invalid_schema() -> anyhow::Error {
    anyhow!("Invalid provider-store schema; no data was imported")
}

fn validate_map<T: DeserializeOwned>(root: &Map<String, Value>, field: &str) -> Result<()> {
    let Some(value) = root.get(field) else {
        return Ok(());
    };
    let object = value.as_object().ok_or_else(invalid_schema)?;
    let mut ids = HashSet::new();
    for key in object.keys() {
        let id = Uuid::parse_str(key).map_err(|_| invalid_schema())?;
        if !ids.insert(id) {
            return Err(invalid_schema());
        }
    }
    // Validate values without replacing the original JSON (unknown fields stay).
    serde_json::from_value::<HashMap<Uuid, T>>(value.clone()).map_err(|_| invalid_schema())?;
    Ok(())
}

fn validate_store(value: &Value) -> Result<Vec<Provider>> {
    let root = value.as_object().ok_or_else(invalid_schema)?;
    let rows = root
        .get("providers")
        .and_then(Value::as_array)
        .ok_or_else(invalid_schema)?;
    let mut ids = HashSet::new();
    let mut providers = Vec::with_capacity(rows.len());
    for row in rows {
        if !row.is_object() {
            return Err(invalid_schema());
        }
        let provider: Provider =
            serde_json::from_value(row.clone()).map_err(|_| invalid_schema())?;
        if !ids.insert(provider.id) {
            bail!("Duplicate provider identities; no data was imported");
        }
        providers.push(provider);
    }
    if let Some(selection) = root.get("selected_id") {
        serde_json::from_value::<Option<Uuid>>(selection.clone()).map_err(|_| invalid_schema())?;
    }
    validate_map::<Vec<String>>(root, "known_models")?;
    validate_map::<String>(root, "selected_models")?;
    validate_map::<String>(root, "settings_models")?;
    validate_map::<HashMap<String, Vec<String>>>(root, "known_model_efforts")?;
    validate_map::<HashMap<String, String>>(root, "default_model_efforts")?;
    validate_map::<HashMap<String, String>>(root, "selected_model_efforts")?;
    validate_map::<HashMap<String, ModelCapabilities>>(root, "model_capabilities")?;
    validate_map::<HashMap<String, CapabilityException>>(root, "capability_exceptions")?;
    validate_map::<bool>(root, "pdf_pass_through")?;
    // Serde structs also accept sequences; the wire schema requires objects.
    for field in ["model_capabilities", "capability_exceptions"] {
        if let Some(map) = root.get(field).and_then(Value::as_object) {
            for models in map.values() {
                let models = models.as_object().ok_or_else(invalid_schema)?;
                if models.values().any(|model| !model.is_object()) {
                    return Err(invalid_schema());
                }
            }
        }
    }
    Ok(providers)
}

struct Merge {
    value: Value,
    ids: Vec<Uuid>,
    added: usize,
}

/// Pure merge: copies only non-preset rows and the allowlisted per-ID values.
fn merge_customs(source: &Value, destination: &Value, expected_count: usize) -> Result<Merge> {
    let source_providers = validate_store(source)?;
    let destination_providers = validate_store(destination)?;
    let customs: Vec<_> = source_providers
        .iter()
        .enumerate()
        .filter(|(_, provider)| !provider.is_preset)
        .collect();
    if customs.len() != expected_count {
        bail!("Custom provider count does not match expected count; no data was imported");
    }
    let mut merged = destination.clone();
    let mut added = 0;
    let mut ids = Vec::with_capacity(customs.len());
    for (index, provider) in customs {
        ids.push(provider.id);
        let row = &source["providers"][index];
        if let Some(existing) = destination_providers
            .iter()
            .position(|p| p.id == provider.id)
        {
            // Raw comparison also protects unknown provider fields from changes.
            if destination["providers"][existing] != *row {
                bail!("Provider identity conflict; existing data was preserved");
            }
        } else {
            merged["providers"]
                .as_array_mut()
                .unwrap()
                .push(row.clone());
            added += 1;
        }
        for field in MAPS {
            let Some(source_map) = source.get(field).and_then(Value::as_object) else {
                continue;
            };
            let Some((source_key, source_value)) = source_map
                .iter()
                .find(|(key, _)| Uuid::parse_str(key).ok() == Some(provider.id))
            else {
                continue;
            };
            let root = merged.as_object_mut().unwrap();
            let target_map = root
                .entry(field)
                .or_insert_with(|| Value::Object(Map::new()))
                .as_object_mut()
                .unwrap();
            if let Some((_, existing)) = target_map
                .iter()
                .find(|(key, _)| Uuid::parse_str(key).ok() == Some(provider.id))
            {
                if existing != source_value {
                    bail!("Per-provider metadata conflict; existing data was preserved");
                }
            } else {
                target_map.insert(source_key.clone(), source_value.clone());
            }
        }
    }
    validate_store(&merged)?;
    Ok(Merge {
        value: merged,
        ids,
        added,
    })
}

// No Debug implementation: these types contain secrets or credential accounts.
trait Credentials {
    fn get(&mut self, account: &str) -> Result<Option<String>>;
    fn set(&mut self, account: &str, value: &str) -> Result<()>;
    fn delete(&mut self, account: &str) -> Result<()>;
}

struct NativeCredentials;

#[cfg(windows)]
impl Credentials for NativeCredentials {
    fn get(&mut self, account: &str) -> Result<Option<String>> {
        let entry = keyring::Entry::new("PopChat", account)
            .map_err(|_| anyhow!("Native credential access failed"))?;
        match entry.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => bail!("Native credential read failed"),
        }
    }

    fn set(&mut self, account: &str, value: &str) -> Result<()> {
        keyring::Entry::new("PopChat", account)
            .and_then(|entry| entry.set_password(value))
            .map_err(|_| anyhow!("Native credential write failed"))
    }

    fn delete(&mut self, account: &str) -> Result<()> {
        let entry = keyring::Entry::new("PopChat", account)
            .map_err(|_| anyhow!("Native credential access failed"))?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => bail!("Native credential rollback failed"),
        }
    }
}

#[cfg(not(windows))]
impl Credentials for NativeCredentials {
    fn get(&mut self, _: &str) -> Result<Option<String>> {
        bail!("Native credential import is supported only on Windows")
    }
    fn set(&mut self, _: &str, _: &str) -> Result<()> {
        bail!("Native credential import is supported only on Windows")
    }
    fn delete(&mut self, _: &str) -> Result<()> {
        bail!("Native credential import is supported only on Windows")
    }
}

fn source_account(source: &Path, id: Uuid) -> String {
    let digest = Sha256::digest(source.to_string_lossy().as_bytes());
    let namespace: String = digest[..12]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("profile-{namespace}-{id}")
}

struct KeyPlan {
    entries: Vec<KeyEntry>,
    copied: usize,
    retained: usize,
    missing: usize,
}

struct KeyEntry {
    source_account: String,
    target_account: String,
    source: Option<String>,
    target: Option<String>,
}

fn read_key(credentials: &mut impl Credentials, account: &str) -> Result<Option<String>> {
    // Never attach native/backend error chains: they may contain secret values.
    credentials
        .get(account)
        .map_err(|_| anyhow!("Credential read failed; no secret details are logged"))
}

fn prepare_keys(
    credentials: &mut impl Credentials,
    source: &Path,
    ids: &[Uuid],
    legacy_exists: bool,
) -> Result<KeyPlan> {
    let mut plan = KeyPlan {
        entries: Vec::with_capacity(ids.len()),
        copied: 0,
        retained: 0,
        missing: 0,
    };
    for id in ids {
        let source_account = source_account(source, *id);
        let target_account = id.to_string();
        let source = read_key(credentials, &source_account)?;
        let target = read_key(credentials, &target_account)?;
        match (&source, &target) {
            (Some(source), Some(target)) if source != target => {
                bail!("Target credential conflict; existing credentials were preserved");
            }
            (Some(_), Some(_)) => plan.retained += 1,
            (Some(_), None) => plan.copied += 1,
            (None, _) if legacy_exists => {
                bail!("Source key is missing while a legacy secrets file exists; finish source key migration first");
            }
            (None, _) => plan.missing += 1,
        }
        plan.entries.push(KeyEntry {
            source_account,
            target_account,
            source,
            target,
        });
    }
    Ok(plan)
}

fn key_plan_for_mode(
    apply: bool,
    credentials: &mut impl Credentials,
    source: &Path,
    ids: &[Uuid],
    legacy_exists: bool,
) -> Result<Option<KeyPlan>> {
    if apply {
        prepare_keys(credentials, source, ids, legacy_exists).map(Some)
    } else {
        Ok(None)
    }
}

fn verify_key_snapshot(credentials: &mut impl Credentials, plan: &KeyPlan) -> Result<()> {
    for entry in &plan.entries {
        if read_key(credentials, &entry.source_account)? != entry.source
            || read_key(credentials, &entry.target_account)? != entry.target
        {
            bail!("Credentials changed during import; retry under the installer supervisor");
        }
    }
    Ok(())
}

fn rollback_keys(credentials: &mut impl Credentials, plan: &KeyPlan, created: &[usize]) -> bool {
    let mut complete = true;
    for &index in created.iter().rev() {
        let entry = &plan.entries[index];
        // Never remove a credential subsequently changed by another writer.
        match read_key(credentials, &entry.target_account) {
            Ok(None) => (),
            Ok(Some(value)) if Some(&value) == entry.source.as_ref() => {
                if credentials.delete(&entry.target_account).is_err()
                    || !matches!(read_key(credentials, &entry.target_account), Ok(None))
                {
                    complete = false;
                }
            }
            _ => complete = false,
        }
    }
    complete
}

/// Commit metadata only after fresh reads verified every newly created key.
fn apply_keys_and_commit(
    credentials: &mut impl Credentials,
    plan: &KeyPlan,
    commit: impl FnOnce() -> Result<()>,
) -> Result<()> {
    verify_key_snapshot(credentials, plan)?;
    let mut created = Vec::new();
    let result = (|| {
        for (index, entry) in plan.entries.iter().enumerate() {
            if let (Some(source), None) = (&entry.source, &entry.target) {
                if read_key(credentials, &entry.target_account)?.is_some() {
                    bail!("Target credential appeared during import; existing credential was preserved");
                }
                // Track before set: a backend may write and then report failure.
                created.push(index);
                credentials
                    .set(&entry.target_account, source)
                    .map_err(|_| anyhow!("Credential write failed"))?;
                if read_key(credentials, &entry.target_account)?.as_ref() != Some(source) {
                    bail!("Fresh credential verification failed");
                }
            }
        }
        for entry in &plan.entries {
            if let Some(source) = &entry.source {
                if read_key(credentials, &entry.target_account)?.as_ref() != Some(source) {
                    bail!("Final credential verification failed");
                }
            }
        }
        // Also sanitize commit errors, even for injected implementations.
        commit().map_err(|_| anyhow!("Metadata commit failed; original and backup were retained"))
    })();
    if let Err(error) = result {
        if !rollback_keys(credentials, plan, &created) {
            bail!("Import failed; credential rollback incomplete. Original metadata and backup were retained; manual review is required");
        }
        return Err(error);
    }
    Ok(())
}

struct Snapshots {
    source_path: PathBuf,
    destination_path: PathBuf,
    source: Vec<u8>,
    destination: Vec<u8>,
    legacy_path: PathBuf,
    legacy_exists: bool,
}

fn legacy_exists(path: &Path) -> Result<bool> {
    // Existence only, including dangling links. Never open or parse plaintext.
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => bail!("Could not check legacy secrets file existence"),
    }
}

impl Snapshots {
    fn verify(&self) -> Result<()> {
        let source = std::fs::read(&self.source_path)
            .map_err(|_| anyhow!("Could not recheck source metadata"))?;
        let destination = std::fs::read(&self.destination_path)
            .map_err(|_| anyhow!("Could not recheck destination metadata"))?;
        if source != self.source || destination != self.destination {
            bail!("Provider metadata changed during import; retry under the installer supervisor");
        }
        if legacy_exists(&self.legacy_path)? != self.legacy_exists {
            bail!("Legacy secrets file presence changed during import; retry under the installer supervisor");
        }
        Ok(())
    }

    fn backup(&self) -> Result<PathBuf> {
        let parent = self
            .destination_path
            .parent()
            .ok_or_else(|| anyhow!("Destination profile directory is unavailable"))?;
        let backup = parent.join(format!(
            "provider_store.before-custom-import-{}.json",
            Uuid::new_v4()
        ));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&backup)
            .map_err(|_| anyhow!("Could not create metadata backup"))?;
        file.write_all(&self.destination)
            .and_then(|_| file.sync_all())
            .map_err(|_| anyhow!("Could not sync metadata backup; import was not applied"))?;
        Ok(backup)
    }
}

fn parse_store(bytes: &[u8]) -> Result<Value> {
    serde_json::from_slice(bytes).map_err(|_| invalid_schema())
}

fn run(options: Options) -> Result<()> {
    if !cfg!(windows) {
        bail!("Native credential import is supported only on Windows");
    }
    let destination = directories::ProjectDirs::from("com", "PopChat", "PopChat")
        .ok_or_else(|| anyhow!("Formal default profile directory is unavailable"))?
        .config_dir()
        .to_owned();
    let source_canonical = std::fs::canonicalize(&options.source)
        .map_err(|_| anyhow!("Source profile directory is unavailable"))?;
    let destination_canonical = std::fs::canonicalize(&destination)
        .map_err(|_| anyhow!("Existing formal default profile directory is required"))?;
    if source_canonical == destination_canonical {
        bail!("Source and destination profiles must be different");
    }
    let source_path = options.source.join("provider_store.json");
    let destination_path = destination.join("provider_store.json");
    let source = std::fs::read(&source_path)
        .map_err(|_| anyhow!("Could not read source provider metadata"))?;
    let target = std::fs::read(&destination_path)
        .map_err(|_| anyhow!("Existing readable destination provider metadata is required"))?;
    let merge = merge_customs(
        &parse_store(&source)?,
        &parse_store(&target)?,
        options.expected_count,
    )?;
    let legacy_path = options.source.join("secrets.json");
    let snapshots = Snapshots {
        source_path,
        destination_path,
        source,
        destination: target,
        legacy_exists: legacy_exists(&legacy_path)?,
        legacy_path,
    };
    let mut credentials = NativeCredentials;
    let plan = key_plan_for_mode(
        options.apply,
        &mut credentials,
        &options.source,
        &merge.ids,
        snapshots.legacy_exists,
    )?;
    println!("Source profile: {}", options.source.display());
    println!("Destination profile: {}", destination.display());
    if let Some(plan) = plan {
        let bytes = serde_json::to_vec_pretty(&merge.value)
            .map_err(|_| anyhow!("Could not encode merged provider metadata"))?;
        snapshots.verify()?;
        let backup = snapshots.backup()?;
        println!("Metadata backup: {}", backup.display());
        snapshots.verify()?;
        apply_keys_and_commit(&mut credentials, &plan, || {
            snapshots.verify()?;
            // Avoid rewriting unchanged JSON on an idempotent repeat.
            if merge.value != parse_store(&snapshots.destination)? {
                popchat_windows::paths::atomic_write(&snapshots.destination_path, &bytes)
                    .map_err(|_| anyhow!("Atomic metadata replacement failed"))?;
            }
            Ok(())
        })?;
        println!(
            "Applied: imported={}, added={}, keys copied={}, retained={}, missing={}",
            merge.ids.len(),
            merge.added,
            plan.copied,
            plan.retained,
            plan.missing
        );
    } else {
        snapshots.verify()?;
        println!(
            "DRY RUN: import={}, added={}; metadata only, no credential access or writes",
            merge.ids.len(),
            merge.added
        );
    }
    Ok(())
}

fn main() {
    let result = match parse_options(std::env::args_os().skip(1)) {
        Ok(Some(options)) => run(options),
        Ok(None) => {
            println!("{}", usage());
            Ok(())
        }
        Err(error) => Err(error),
    };
    if let Err(error) = result {
        // Every public error is deliberately static and privacy-safe. Never
        // print Debug, causes, JSON, provider identities, or backend errors.
        eprintln!("Import aborted: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const CUSTOM_A: &str = "11111111-1111-4111-8111-111111111111";
    const CUSTOM_B: &str = "22222222-2222-4222-8222-222222222222";
    const PRESET: &str = "33333333-3333-4333-8333-333333333333";
    const EXISTING: &str = "44444444-4444-4444-8444-444444444444";
    const SECRET: &str = "fixture-only-sensitive-value";

    fn provider(id: &str, preset: bool) -> Value {
        json!({"id": id, "name": "fixture", "baseURL": "https://example.invalid/v1",
            "isPreset": preset, "defaultModel": "fixture-model", "kind": "openAICompatible"})
    }

    fn source() -> Value {
        json!({
            "providers": [provider(CUSTOM_A, false), provider(PRESET, true), provider(CUSTOM_B, false)],
            "selected_id": CUSTOM_B,
            "known_models": {CUSTOM_A: ["a"], CUSTOM_B: ["b"], PRESET: ["excluded"]},
            "selected_models": {CUSTOM_A: "a", PRESET: "excluded"},
            "settings_models": {CUSTOM_A: "a"},
            "known_model_efforts": {CUSTOM_A: {"a": ["low", "high"]}},
            "default_model_efforts": {CUSTOM_A: {"a": "high"}},
            "selected_model_efforts": {CUSTOM_A: {"a": "low"}},
            "model_capabilities": {CUSTOM_A: {"a": {"images": true, "files": false}}},
            "capability_exceptions": {CUSTOM_A: {"a": {"images": false, "source": "manual"}}},
            "pdf_pass_through": {CUSTOM_A: true},
            "source_only": "do not import"
        })
    }

    fn destination() -> Value {
        json!({
            "providers": [provider(EXISTING, false)], "selected_id": EXISTING,
            "known_models": {EXISTING: ["keep"]},
            "selected_models": {EXISTING: "keep"},
            "unknown_root": {"keep": [1, 2, 3]},
            "unrelated_map": {CUSTOM_A: "also keep"}
        })
    }

    #[test]
    fn merge_preserves_destination_and_only_imports_customs() {
        let source = source();
        let target = destination();
        let result = merge_customs(&source, &target, 2).unwrap();
        assert_eq!(result.added, 2);
        assert_eq!(result.ids.len(), 2);
        assert_eq!(result.value["providers"][0], target["providers"][0]);
        assert_eq!(result.value["providers"].as_array().unwrap().len(), 3);
        assert_eq!(result.value["selected_id"], target["selected_id"]);
        assert_eq!(result.value["unknown_root"], target["unknown_root"]);
        assert_eq!(result.value["unrelated_map"], target["unrelated_map"]);
        assert!(result.value.get("source_only").is_none());
        assert_eq!(result.value["known_models"][EXISTING], json!(["keep"]));
        assert_eq!(result.value["selected_models"][EXISTING], "keep");
        for field in MAPS {
            assert_eq!(result.value[field][CUSTOM_A], source[field][CUSTOM_A]);
            assert!(result.value[field].get(PRESET).is_none());
        }
        assert_eq!(source, self::source());
        assert_eq!(target, destination());
    }

    #[test]
    fn merge_is_idempotent_and_preserves_absent_legacy_maps() {
        let mut source = source();
        for field in MAPS {
            source.as_object_mut().unwrap().remove(field);
        }
        let first = merge_customs(&source, &destination(), 2).unwrap();
        let second = merge_customs(&source, &first.value, 2).unwrap();
        assert_eq!(second.added, 0);
        assert_eq!(first.value, second.value);
        for field in &MAPS[2..] {
            assert!(second.value.get(*field).is_none());
        }
        let full = merge_customs(&self::source(), &destination(), 2).unwrap();
        assert_eq!(
            merge_customs(&self::source(), &full.value, 2)
                .unwrap()
                .value,
            full.value
        );
    }

    #[test]
    fn merge_rejects_identity_and_every_map_conflict_without_mutation() {
        let source = source();
        let mut target = destination();
        target["providers"]
            .as_array_mut()
            .unwrap()
            .push(provider(CUSTOM_A, false));
        target["providers"][1]["name"] = json!("different");
        let before = target.clone();
        assert!(merge_customs(&source, &target, 2).is_err());
        assert_eq!(before, target);
        for field in MAPS {
            let mut target = destination();
            let different = match field {
                "known_models" => json!(["different"]),
                "selected_models" | "settings_models" => json!("different"),
                "known_model_efforts" => json!({"a": ["different"]}),
                "default_model_efforts" | "selected_model_efforts" => json!({"a": "different"}),
                "model_capabilities" => json!({"a": {"images": false}}),
                "capability_exceptions" => json!({"a": {"source": "learned"}}),
                "pdf_pass_through" => json!(false),
                _ => unreachable!(),
            };
            target[field] = json!({CUSTOM_A: different});
            let before = target.clone();
            assert!(merge_customs(&source, &target, 2).is_err(), "{field}");
            assert_eq!(before, target);
        }
    }

    #[test]
    fn schema_and_expected_count_are_strict() {
        let good = source();
        assert!(merge_customs(&good, &destination(), 1).is_err());
        for bad in [
            json!([]),
            json!({}),
            json!({"providers": null}),
            json!({"providers": [[]]}),
        ] {
            assert!(merge_customs(&bad, &destination(), 2).is_err());
            assert!(merge_customs(&good, &bad, 2).is_err());
        }
        let mut bad = good.clone();
        bad["providers"]
            .as_array_mut()
            .unwrap()
            .push(provider(CUSTOM_A, false));
        assert!(validate_store(&bad).is_err());
        for (field, value) in [
            ("selected_id", json!("invalid")),
            ("known_models", json!([])),
            ("known_models", json!({CUSTOM_A: "not an array"})),
            ("selected_models", json!({"not-a-uuid": "a"})),
            ("pdf_pass_through", json!({CUSTOM_A: "true"})),
            (
                "model_capabilities",
                json!({CUSTOM_A: {"a": [true, false]}}),
            ),
            (
                "capability_exceptions",
                json!({CUSTOM_A: {"a": {"source": "bad"}}}),
            ),
        ] {
            let mut bad = good.clone();
            bad[field] = value;
            assert!(validate_store(&bad).is_err(), "{field}");
        }
        let mut bad = good;
        bad["providers"][0]["isPreset"] = json!("false");
        assert!(validate_store(&bad).is_err());
    }

    #[derive(Default)]
    struct MemoryCredentials {
        values: HashMap<String, String>,
        writes: Vec<String>,
        deletes: Vec<String>,
        fail_set: Option<usize>,
        fail_delete: bool,
        corrupt_set: bool,
        fail_read: bool,
    }

    impl Credentials for MemoryCredentials {
        fn get(&mut self, account: &str) -> Result<Option<String>> {
            if self.fail_read {
                bail!("backend error includes {SECRET}");
            }
            Ok(self.values.get(account).cloned())
        }
        fn set(&mut self, account: &str, value: &str) -> Result<()> {
            self.writes.push(account.into());
            if self.fail_set == Some(self.writes.len()) {
                // Simulate a write that persisted despite a reported error.
                self.values.insert(account.into(), value.into());
                bail!("backend error includes {SECRET}");
            }
            self.values.insert(
                account.into(),
                if self.corrupt_set { "corrupt" } else { value }.into(),
            );
            Ok(())
        }
        fn delete(&mut self, account: &str) -> Result<()> {
            self.deletes.push(account.into());
            if self.fail_delete {
                bail!("backend error includes {SECRET}");
            }
            self.values.remove(account);
            Ok(())
        }
    }

    fn key_fixture() -> (PathBuf, Vec<Uuid>, MemoryCredentials) {
        let path = PathBuf::from("fixture-profile");
        let ids = vec![
            Uuid::parse_str(CUSTOM_A).unwrap(),
            Uuid::parse_str(CUSTOM_B).unwrap(),
        ];
        let mut credentials = MemoryCredentials::default();
        for id in &ids {
            credentials
                .values
                .insert(source_account(&path, *id), SECRET.into());
        }
        (path, ids, credentials)
    }

    fn safe_error(error: anyhow::Error) {
        let message = format!("{error:?} {error:#}");
        for private in [
            SECRET,
            CUSTOM_A,
            CUSTOM_B,
            "example.invalid",
            "fixture-profile",
        ] {
            assert!(!message.contains(private));
        }
    }

    #[test]
    fn key_prepare_is_read_only_and_apply_is_idempotent() {
        let (path, ids, mut credentials) = key_fixture();
        let original = credentials.values.clone();
        let plan = prepare_keys(&mut credentials, &path, &ids, false).unwrap();
        assert_eq!(plan.copied, 2);
        assert_eq!(credentials.values, original);
        assert!(credentials.writes.is_empty());
        apply_keys_and_commit(&mut credentials, &plan, || Ok(())).unwrap();
        assert_eq!(credentials.writes.len(), 2);
        assert!(credentials.deletes.is_empty());
        for id in &ids {
            assert_eq!(credentials.values[&source_account(&path, *id)], SECRET);
            assert_eq!(credentials.values[&id.to_string()], SECRET);
        }
        let repeat = prepare_keys(&mut credentials, &path, &ids, false).unwrap();
        assert_eq!((repeat.copied, repeat.retained, repeat.missing), (0, 2, 0));
        apply_keys_and_commit(&mut credentials, &repeat, || Ok(())).unwrap();
        assert_eq!(credentials.writes.len(), 2);
    }

    #[test]
    fn default_dry_run_never_accesses_credentials() {
        let (path, ids, mut credentials) = key_fixture();
        credentials.fail_read = true;
        let original = credentials.values.clone();
        assert!(
            key_plan_for_mode(false, &mut credentials, &path, &ids, true)
                .unwrap()
                .is_none()
        );
        assert_eq!(credentials.values, original);
        assert!(credentials.writes.is_empty());
        assert!(credentials.deletes.is_empty());
    }

    #[test]
    fn key_conflict_and_missing_legacy_key_abort_without_writes() {
        let (path, ids, mut credentials) = key_fixture();
        credentials
            .values
            .insert(CUSTOM_B.into(), "different".into());
        safe_error(
            prepare_keys(&mut credentials, &path, &ids, false)
                .err()
                .unwrap(),
        );
        assert!(credentials.writes.is_empty());
        credentials.values.remove(CUSTOM_B);
        credentials.values.remove(&source_account(&path, ids[0]));
        safe_error(
            prepare_keys(&mut credentials, &path, &ids, true)
                .err()
                .unwrap(),
        );
        let plan = prepare_keys(&mut credentials, &path, &ids, false).unwrap();
        assert_eq!((plan.copied, plan.retained, plan.missing), (1, 0, 1));
        assert!(credentials.writes.is_empty());
    }

    #[test]
    fn partial_write_failure_rolls_back_only_new_targets() {
        let (path, ids, mut credentials) = key_fixture();
        credentials
            .values
            .insert("unrelated-account".into(), SECRET.into());
        let original = credentials.values.clone();
        let plan = prepare_keys(&mut credentials, &path, &ids, false).unwrap();
        credentials.fail_set = Some(2);
        let error = apply_keys_and_commit(&mut credentials, &plan, || panic!("must not commit"))
            .unwrap_err();
        safe_error(error);
        assert_eq!(credentials.values, original);
        assert_eq!(credentials.deletes, [CUSTOM_B, CUSTOM_A]);
    }

    #[test]
    fn commit_failure_preserves_existing_and_source_keys() {
        let (path, ids, mut credentials) = key_fixture();
        credentials.values.insert(CUSTOM_A.into(), SECRET.into());
        let original = credentials.values.clone();
        let plan = prepare_keys(&mut credentials, &path, &ids, false).unwrap();
        safe_error(
            apply_keys_and_commit(&mut credentials, &plan, || bail!("{SECRET}")).unwrap_err(),
        );
        assert_eq!(credentials.values, original);
        assert_eq!(credentials.deletes, [CUSTOM_B]);
    }

    #[test]
    fn concurrent_target_change_aborts_and_never_overwrites() {
        let (path, ids, mut credentials) = key_fixture();
        let plan = prepare_keys(&mut credentials, &path, &ids, false).unwrap();
        credentials.values.insert(CUSTOM_B.into(), "changed".into());
        safe_error(
            apply_keys_and_commit(&mut credentials, &plan, || panic!("must not commit"))
                .unwrap_err(),
        );
        assert!(credentials.writes.is_empty());
        assert!(credentials.deletes.is_empty());
        assert_eq!(credentials.values[CUSTOM_B], "changed");
    }

    #[test]
    fn backend_errors_and_failed_verification_never_expose_secrets() {
        let (path, ids, mut credentials) = key_fixture();
        credentials.fail_read = true;
        safe_error(
            prepare_keys(&mut credentials, &path, &ids, false)
                .err()
                .unwrap(),
        );
        credentials.fail_read = false;
        let plan = prepare_keys(&mut credentials, &path, &ids, false).unwrap();
        credentials.corrupt_set = true;
        let error = apply_keys_and_commit(&mut credentials, &plan, || panic!("must not commit"))
            .unwrap_err();
        assert!(error.to_string().contains("rollback incomplete"));
        safe_error(error);
        // A different current value must never be deleted speculatively.
        assert!(credentials.deletes.is_empty());
    }

    #[test]
    fn rollback_failure_is_explicit_and_privacy_safe() {
        let (path, ids, mut credentials) = key_fixture();
        let plan = prepare_keys(&mut credentials, &path, &ids, false).unwrap();
        credentials.fail_delete = true;
        let error =
            apply_keys_and_commit(&mut credentials, &plan, || bail!("{SECRET}")).unwrap_err();
        assert!(error.to_string().contains("rollback incomplete"));
        safe_error(error);
    }

    #[test]
    fn snapshots_backup_and_atomic_commit_use_only_fixture_files() {
        let temp = tempfile::tempdir().unwrap();
        let source_path = temp.path().join("source.json");
        let destination_path = temp.path().join("provider_store.json");
        let source = serde_json::to_vec(&source()).unwrap();
        let destination = serde_json::to_vec(&destination()).unwrap();
        std::fs::write(&source_path, &source).unwrap();
        std::fs::write(&destination_path, &destination).unwrap();
        let snapshots = Snapshots {
            source_path,
            destination_path,
            source,
            destination,
            legacy_path: temp.path().join("secrets.json"),
            legacy_exists: false,
        };
        snapshots.verify().unwrap();
        let first = snapshots.backup().unwrap();
        let second = snapshots.backup().unwrap();
        assert_ne!(first, second);
        assert_eq!(std::fs::read(&first).unwrap(), snapshots.destination);
        assert_eq!(std::fs::read(&second).unwrap(), snapshots.destination);
        let merge = merge_customs(
            &parse_store(&snapshots.source).unwrap(),
            &parse_store(&snapshots.destination).unwrap(),
            2,
        )
        .unwrap();
        let (path, ids, mut credentials) = key_fixture();
        let original_keys = credentials.values.clone();
        let plan = prepare_keys(&mut credentials, &path, &ids, false).unwrap();
        std::fs::write(&snapshots.source_path, b"concurrent change").unwrap();
        safe_error(
            apply_keys_and_commit(&mut credentials, &plan, || {
                snapshots.verify()?;
                popchat_windows::paths::atomic_write(
                    &snapshots.destination_path,
                    &serde_json::to_vec(&merge.value).unwrap(),
                )?;
                Ok(())
            })
            .unwrap_err(),
        );
        assert_eq!(credentials.values, original_keys);
        assert_eq!(
            std::fs::read(&snapshots.destination_path).unwrap(),
            snapshots.destination
        );
        std::fs::write(&snapshots.source_path, &snapshots.source).unwrap();
        apply_keys_and_commit(&mut credentials, &plan, || {
            snapshots.verify()?;
            popchat_windows::paths::atomic_write(
                &snapshots.destination_path,
                &serde_json::to_vec(&merge.value).unwrap(),
            )?;
            Ok(())
        })
        .unwrap();
        assert_eq!(
            parse_store(&std::fs::read(&snapshots.destination_path).unwrap()).unwrap(),
            merge.value
        );
        assert_eq!(std::fs::read(first).unwrap(), snapshots.destination);
    }

    #[test]
    fn cli_defaults_to_dry_run_and_does_not_accept_destination_override() {
        let temp = tempfile::tempdir().unwrap();
        let args = vec![
            OsString::from("--source-profile"),
            temp.path().as_os_str().to_owned(),
        ];
        let options = parse_options(args.clone()).unwrap().unwrap();
        assert!(!options.apply);
        assert_eq!(options.expected_count, 2);
        let mut invalid = args.clone();
        invalid.push("--destination-profile".into());
        assert!(parse_options(invalid).is_err());
        let mut apply = args;
        apply.extend(["--apply".into(), "--expected-count".into(), "3".into()]);
        let options = parse_options(apply).unwrap().unwrap();
        assert!(options.apply);
        assert_eq!(options.expected_count, 3);
        assert!(parse_options(["--source-profile".into(), "relative".into()]).is_err());
        assert!(parse_options(["--help".into()]).unwrap().is_none());
    }

    #[test]
    fn account_namespace_matches_exact_path_digest() {
        let id = Uuid::parse_str(CUSTOM_A).unwrap();
        let path = Path::new("fixture-profile");
        let digest = Sha256::digest(b"fixture-profile");
        let prefix: String = digest[..12]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(source_account(path, id), format!("profile-{prefix}-{id}"));
        assert_ne!(
            source_account(path, id),
            source_account(Path::new("fixture-profile/"), id)
        );
    }
}
