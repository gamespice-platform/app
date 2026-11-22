//! Deploy patches: JSON edits to files in the game folder that are not ours,
//! such as Minecraft's `launcher_profiles.json` (docs/loaders-and-versions.md,
//! "Deploy consumption", step 6).
//!
//! THE UNDO RULE
//! -------------
//! A patch is recorded in the ledger as one [`KIND`] row per file, holding the
//! value each pointer had before ([`AppliedPatch::previous`]) and the value we
//! wrote ([`AppliedPatch::applied`]). Undo puts `previous` back only where the
//! file still holds `applied`: the same "only if it is still ours" rule placed
//! files follow. A value the user changed afterwards is theirs and is kept.
//!
//! BYTE IDENTITY
//! -------------
//! When nobody touched the file after the patch (its sha256 is still the one
//! we wrote), undo writes the original bytes back. When something else
//! rewrote it (the launcher, the user), undo edits the document and, if the
//! result equals the original document, still writes the original bytes;
//! otherwise it re-serialises with the indentation the file had.
//!
//! USVFS
//! -----
//! A virtual deploy must not edit the real file, so [`write_overlay`] writes the
//! patched copy into a per-sandbox overlay folder that is mapped above every
//! mod. Nothing is recorded: the overlay is rebuilt from the real file on every
//! deploy and removed on purge.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::{AppError, AppResult};
use crate::plugins::jail::join_relative;

use super::ledger::LedgerEntry;

/// The ledger `kind` of a JSON patch row.
pub const KIND: &str = "jsonPatch";

/// The largest file a patch will read. `launcher_profiles.json` is a few KB.
const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;

/// One planned patch, placeholders already expanded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JsonPatchSpec {
    /// `patch:<manifestKey>`, the ledger's mod key.
    pub mod_key: String,
    /// Relative to the game folder, `/`-separated.
    pub path: String,
    /// The document to start from when the file does not exist.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ensure: Option<Value>,
    /// RFC 6901 pointer. Missing parent objects are created.
    pub pointer: String,
    pub value: Value,
}

/// What a patch did to one file: the ledger row's `patch` column.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JsonPatchRecord {
    /// The file did not exist and was created from `ensure`.
    pub created: bool,
    /// The bytes before the patch, when the file existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original: Option<String>,
    /// The document the file was created from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ensure: Option<Value>,
    /// sha256 of the bytes the patch wrote.
    #[serde(default)]
    pub written_sha256: String,
    pub patches: Vec<AppliedPatch>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppliedPatch {
    pub pointer: String,
    /// `None`: the pointer did not exist before.
    #[serde(default)]
    pub previous: Option<Value>,
    pub applied: Value,
}

/// What [`revert`] did.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevertReport {
    pub reverted: usize,
    /// `<path><pointer>` values the user changed since, left alone.
    pub kept: Vec<String>,
    pub errors: Vec<String>,
}

/// Apply the patches to the real files under `target` and return one ledger
/// row per file. Files that could not be patched are reported and skipped;
/// the others still apply.
pub fn apply(target: &Path, specs: &[JsonPatchSpec]) -> (Vec<LedgerEntry>, Vec<String>) {
    let mut rows = Vec::new();
    let mut errors = Vec::new();

    for (path, group) in by_path(specs) {
        match apply_file(target, path, &group) {
            Ok(row) => rows.push(row),
            Err(e) => errors.push(format!("{path}: {}", e.detail())),
        }
    }

    (rows, errors)
}

/// Write the patched copies of `target`'s files into `overlay`, for a virtual
/// deploy. The real files are only read.
pub fn write_overlay(target: &Path, overlay: &Path, specs: &[JsonPatchSpec]) -> Vec<String> {
    let mut errors = Vec::new();

    if overlay.exists() {
        if let Err(e) = std::fs::remove_dir_all(overlay) {
            errors.push(format!("{}: {e}", overlay.display()));

            return errors;
        }
    }

    for (path, group) in by_path(specs) {
        let result = (|| -> AppResult<()> {
            let (_, mut doc, _) = load(target, path, &group)?;

            for spec in &group {
                set_pointer(&mut doc, &spec.pointer, spec.value.clone())?;
            }

            let dest = join_relative(overlay, path)?;

            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }

            std::fs::write(&dest, render(&doc, "  ")?)?;

            Ok(())
        })();

        if let Err(e) = result {
            errors.push(format!("{path}: {}", e.detail()));
        }
    }

    errors
}

/// Undo the patch rows among `entries`. Rows of other kinds are ignored.
pub fn revert(target: &Path, entries: &[LedgerEntry]) -> RevertReport {
    let mut report = RevertReport::default();

    for entry in entries.iter().filter(|e| e.is_json_patch()) {
        let Some(record) = &entry.patch else {
            continue;
        };

        if let Err(e) = revert_file(target, &entry.path, record, &mut report) {
            report.errors.push(format!("{}: {}", entry.path, e.detail()));
        }
    }

    report
}

fn by_path(specs: &[JsonPatchSpec]) -> BTreeMap<&str, Vec<&JsonPatchSpec>> {
    let mut out: BTreeMap<&str, Vec<&JsonPatchSpec>> = BTreeMap::new();

    for spec in specs {
        out.entry(spec.path.as_str()).or_default().push(spec);
    }

    out
}

/// Read the file a patch group targets: its original text (if it existed),
/// the document, and the absolute path.
fn load(
    target: &Path,
    path: &str,
    group: &[&JsonPatchSpec],
) -> AppResult<(Option<String>, Value, std::path::PathBuf)> {
    let abs = join_relative(target, path)?;

    match std::fs::symlink_metadata(&abs) {
        Ok(meta) if meta.file_type().is_symlink() => Err(AppError::invalid(
            "it is a link; a deploy patch only edits plain files",
        )),
        Ok(meta) if !meta.is_file() => Err(AppError::invalid("it is not a file")),
        Ok(meta) if meta.len() > MAX_FILE_BYTES => Err(AppError::invalid("it is too large to patch")),
        Ok(_) => {
            let text = std::fs::read_to_string(&abs)
                .map_err(|e| AppError::invalid(format!("could not read it as text ({e})")))?;
            let doc = serde_json::from_str(&text)
                .map_err(|e| AppError::invalid(format!("it is not valid JSON ({e}); left alone")))?;

            Ok((Some(text), doc, abs))
        }
        Err(_) => {
            let ensure = group
                .iter()
                .find_map(|s| s.ensure.clone())
                .ok_or_else(|| AppError::invalid("it does not exist and the patch has no `ensure`"))?;

            Ok((None, ensure, abs))
        }
    }
}

fn apply_file(target: &Path, path: &str, group: &[&JsonPatchSpec]) -> AppResult<LedgerEntry> {
    let (original, mut doc, abs) = load(target, path, group)?;
    let created = original.is_none();
    let mut patches = Vec::new();

    for spec in group {
        let previous = set_pointer(&mut doc, &spec.pointer, spec.value.clone())?;

        patches.push(AppliedPatch {
            pointer: spec.pointer.clone(),
            previous,
            applied: spec.value.clone(),
        });
    }

    let indent = original.as_deref().map(indent_of).unwrap_or_else(|| "  ".into());
    let bytes = render(&doc, &indent)?;

    write_atomic(&abs, &bytes)?;

    Ok(LedgerEntry {
        path: path.to_string(),
        kind: KIND.to_string(),
        mod_key: group[0].mod_key.clone(),
        source: String::new(),
        size: 0,
        mtime_ms: 0,
        backup: None,
        patch: Some(JsonPatchRecord {
            created,
            original,
            ensure: if created { group.iter().find_map(|s| s.ensure.clone()) } else { None },
            written_sha256: sha256_hex(&bytes),
            patches,
        }),
    })
}

fn revert_file(
    target: &Path,
    path: &str,
    record: &JsonPatchRecord,
    report: &mut RevertReport,
) -> AppResult<()> {
    let abs = join_relative(target, path)?;

    let Ok(meta) = std::fs::symlink_metadata(&abs) else {
        // Gone: the user deleted it. Nothing of ours is left to undo.
        return Ok(());
    };

    if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
        report.kept.push(path.to_string());

        return Ok(());
    }

    let bytes = std::fs::read(&abs)?;

    // Untouched since the patch: put back exactly what was there.
    if sha256_hex(&bytes) == record.written_sha256 {
        match &record.original {
            Some(original) => write_atomic(&abs, original.as_bytes())?,
            None => std::fs::remove_file(&abs)?,
        }

        report.reverted += 1;

        return Ok(());
    }

    let text = String::from_utf8_lossy(&bytes).into_owned();
    let Ok(mut doc) = serde_json::from_str::<Value>(&text) else {
        report.kept.push(path.to_string());

        return Ok(());
    };

    for patch in record.patches.iter().rev() {
        if doc.pointer(&patch.pointer) != Some(&patch.applied) {
            report.kept.push(format!("{path}{}", patch.pointer));

            continue;
        }

        match &patch.previous {
            Some(previous) => {
                set_pointer(&mut doc, &patch.pointer, previous.clone())?;
            }
            None => remove_pointer(&mut doc, &patch.pointer),
        }
    }

    let original_doc = record
        .original
        .as_deref()
        .and_then(|o| serde_json::from_str::<Value>(o).ok());

    if record.created && record.ensure.as_ref() == Some(&doc) {
        std::fs::remove_file(&abs)?;
    } else if let (Some(original), true) = (&record.original, original_doc.as_ref() == Some(&doc)) {
        write_atomic(&abs, original.as_bytes())?;
    } else {
        write_atomic(&abs, &render(&doc, &indent_of(&text))?)?;
    }

    report.reverted += 1;

    Ok(())
}

/// Split an RFC 6901 pointer into its unescaped tokens.
fn tokens(pointer: &str) -> AppResult<Vec<String>> {
    if pointer.is_empty() {
        return Err(AppError::invalid("a deploy patch cannot replace the whole document"));
    }

    let rest = pointer
        .strip_prefix('/')
        .ok_or_else(|| AppError::invalid(format!("{pointer} is not a JSON pointer")))?;

    Ok(rest
        .split('/')
        .map(|t| t.replace("~1", "/").replace("~0", "~"))
        .collect())
}

/// Set `pointer` to `value`, creating missing parent objects, and return the
/// value that was there.
fn set_pointer(doc: &mut Value, pointer: &str, value: Value) -> AppResult<Option<Value>> {
    let tokens = tokens(pointer)?;
    let (last, parents) = tokens.split_last().expect("tokens is never empty");
    let mut cursor = doc;

    for token in parents {
        let map = cursor
            .as_object_mut()
            .ok_or_else(|| AppError::invalid(format!("{pointer}: a parent is not an object")))?;

        cursor = map
            .entry(token.clone())
            .or_insert_with(|| Value::Object(Default::default()));
    }

    let map = cursor
        .as_object_mut()
        .ok_or_else(|| AppError::invalid(format!("{pointer}: the parent is not an object")))?;

    Ok(map.insert(last.clone(), value))
}

fn remove_pointer(doc: &mut Value, pointer: &str) {
    let Ok(tokens) = tokens(pointer) else {
        return;
    };
    let (last, parents) = tokens.split_last().expect("tokens is never empty");
    let mut cursor = doc;

    for token in parents {
        match cursor.get_mut(token.as_str()) {
            Some(next) => cursor = next,
            None => return,
        }
    }

    if let Some(map) = cursor.as_object_mut() {
        map.remove(last.as_str());
    }
}

/// The indentation of the first indented line, so a re-serialised file keeps
/// the look it had. Two spaces when there is none.
fn indent_of(text: &str) -> String {
    text.lines()
        .skip(1)
        .find_map(|line| {
            let indent: String = line.chars().take_while(|c| *c == ' ' || *c == '\t').collect();

            (!indent.is_empty()).then_some(indent)
        })
        .unwrap_or_else(|| "  ".into())
}

fn render(doc: &Value, indent: &str) -> AppResult<Vec<u8>> {
    let mut out = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(indent.as_bytes());
    let mut ser = serde_json::Serializer::with_formatter(&mut out, formatter);

    doc.serialize(&mut ser)?;

    Ok(out)
}

fn write_atomic(abs: &Path, bytes: &[u8]) -> AppResult<()> {
    if let Some(parent) = abs.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let tmp = abs.with_file_name(format!(
        ".{}.tmcpatch",
        abs.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    ));

    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, abs)?;

    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn spec(pointer: &str, value: Value) -> JsonPatchSpec {
        JsonPatchSpec {
            mod_key: "patch:minecraft/version".into(),
            path: "launcher_profiles.json".into(),
            ensure: Some(json!({ "profiles": {}, "version": 3 })),
            pointer: pointer.into(),
            value,
        }
    }

    #[test]
    fn an_untouched_file_reverts_to_its_exact_bytes() {
        let tmp = tempfile::tempdir().unwrap();
        let original = "{\n    \"profiles\": {\"a\": {\"name\": \"A\"}},\n    \"version\": 3\n}\n";

        std::fs::write(tmp.path().join("launcher_profiles.json"), original).unwrap();

        let (rows, errors) = apply(tmp.path(), &[spec("/profiles/tmc-1", json!({ "name": "TMC" }))]);

        assert!(errors.is_empty(), "{errors:?}");

        let patched: Value =
            serde_json::from_slice(&std::fs::read(tmp.path().join("launcher_profiles.json")).unwrap()).unwrap();

        assert_eq!(patched["profiles"]["tmc-1"]["name"], "TMC");
        assert_eq!(patched["profiles"]["a"]["name"], "A");

        let report = revert(tmp.path(), &rows);

        assert_eq!(report.reverted, 1);
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("launcher_profiles.json")).unwrap(),
            original
        );
    }

    #[test]
    fn a_created_file_is_removed_again() {
        let tmp = tempfile::tempdir().unwrap();
        let (rows, errors) = apply(tmp.path(), &[spec("/profiles/tmc-1", json!(1))]);

        assert!(errors.is_empty());
        assert!(tmp.path().join("launcher_profiles.json").is_file());

        revert(tmp.path(), &rows);

        assert!(!tmp.path().join("launcher_profiles.json").exists());
    }

    #[test]
    fn a_value_the_user_changed_is_kept() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("launcher_profiles.json");
        let (rows, _) = apply(tmp.path(), &[spec("/profiles/tmc-1", json!({ "name": "TMC" }))]);
        let mut doc: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();

        doc["profiles"]["tmc-1"]["name"] = json!("Mine now");
        std::fs::write(&file, serde_json::to_vec_pretty(&doc).unwrap()).unwrap();

        let report = revert(tmp.path(), &rows);

        assert_eq!(report.kept, vec!["launcher_profiles.json/profiles/tmc-1".to_string()]);

        let after: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();

        assert_eq!(after["profiles"]["tmc-1"]["name"], "Mine now");
    }

    #[test]
    fn a_link_is_never_patched() {
        let tmp = tempfile::tempdir().unwrap();

        std::fs::write(tmp.path().join("real.json"), "{}").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(tmp.path().join("real.json"), tmp.path().join("launcher_profiles.json")).unwrap();
        #[cfg(not(unix))]
        return;

        let (rows, errors) = apply(tmp.path(), &[spec("/profiles/x", json!(1))]);

        assert!(rows.is_empty());
        assert_eq!(errors.len(), 1);
        assert_eq!(std::fs::read_to_string(tmp.path().join("real.json")).unwrap(), "{}");
    }

    #[test]
    fn the_overlay_leaves_the_real_file_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let game = tmp.path().join("game");
        let overlay = tmp.path().join("patched");

        std::fs::create_dir_all(&game).unwrap();
        std::fs::write(game.join("launcher_profiles.json"), "{\"profiles\":{}}").unwrap();

        let errors = write_overlay(&game, &overlay, &[spec("/profiles/tmc-1", json!(true))]);

        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(
            std::fs::read_to_string(game.join("launcher_profiles.json")).unwrap(),
            "{\"profiles\":{}}"
        );

        let doc: Value =
            serde_json::from_slice(&std::fs::read(overlay.join("launcher_profiles.json")).unwrap()).unwrap();

        assert_eq!(doc["profiles"]["tmc-1"], true);
    }
}
