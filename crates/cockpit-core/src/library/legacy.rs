//! Read-only recovery of legacy companion relevance into Library selections.
use super::{refs, store::{Store, SpaceContextRecord, bounded_write, error}};
use crate::{InspectionError, project_store::{open_dir_nofollow_absolute, read_json_bounded}};
use cap_fs_ext::DirExt;
use cap_std::fs::Dir;
use cockpit_protocol::{library::{LibraryItemRef, SpaceTarget}, projects::ProjectConfiguration};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, io::ErrorKind, path::Path};

const MAX_RECORD: u64 = 4 * 1024 * 1024;
const MAX_ENTRIES: usize = 4096;

fn corrupt(message: impl Into<String>) -> InspectionError {
    error("library_corrupt", message)
}

pub(super) fn json_names(dir: &Dir, limit: usize) -> Result<Vec<String>, InspectionError> {
    let mut names = Vec::new();
    for (count, entry) in dir.entries().map_err(|e| corrupt(e.to_string()))?.enumerate() {
        if count >= limit { return Err(corrupt("legacy record directory exceeds migration bound")); }
        let entry = entry.map_err(|e| corrupt(e.to_string()))?;
        let name = entry.file_name().into_string().map_err(|_| corrupt("non-UTF-8 legacy record name"))?;
        if name.ends_with(".json") { names.push(name); }
    }
    names.sort();
    Ok(names)
}

pub(super) fn upgrade_entry(entry: &mut Value) -> Result<(), InspectionError> {
    if entry.is_null() { return Ok(()); }
    let references = entry.get_mut("summary").and_then(|s| s.get_mut("refs"))
        .and_then(Value::as_array_mut).ok_or_else(|| corrupt("legacy item has no reference set"))?;
    upgrade_refs(references)
}

fn upgrade_refs(references: &mut [Value]) -> Result<(), InspectionError> {
    for reference in references {
        if reference.get("kind").and_then(Value::as_str) == Some("space") {
            let object = reference.as_object_mut().ok_or_else(|| corrupt("invalid Space reference"))?;
            if let Some(old) = object.remove("companion_root_id") {
                let old = old.as_str().filter(|id| !id.is_empty())
                    .ok_or_else(|| corrupt("legacy Space reference has no ownership identity"))?;
                if object.contains_key("space_context_id") {
                    return Err(corrupt("ambiguous legacy Space reference"));
                }
                object.insert("space_context_id".into(), format!("legacy:{old}").into());
            }
            if object.get("space_context_id").and_then(Value::as_str).is_none_or(str::is_empty) {
                return Err(corrupt("Space reference has no ownership identity"));
            }
        }
        // Unknown ownership must block the schema flip, never be discarded.
        serde_json::from_value::<LibraryItemRef>(reference.clone())
            .map_err(|e| corrupt(format!("unsupported Library reference: {e}")))?;
    }
    Ok(())
}

fn strings(value: Option<&Value>) -> BTreeSet<String> {
    value.and_then(Value::as_array).into_iter().flatten()
        .filter_map(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned).collect()
}

fn optional_dir(parent: &Dir, name: &str) -> Result<Option<Dir>, InspectionError> {
    match parent.open_dir_nofollow(name) {
        Ok(dir) => Ok(Some(dir)),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(corrupt(e.to_string())),
    }
}

fn attempts(store: &Store) -> Result<Vec<Value>, InspectionError> {
    let mut records = Vec::new();
    // Released versions used space-adds; accept the earlier long-form name too.
    for name in ["space-adds", "space-add-attempts"] {
        if let Some(dir) = optional_dir(&store.meta, name)? {
            for name in json_names(&dir, MAX_ENTRIES)? {
                records.push(read_json_bounded(&dir, &name, MAX_RECORD)?);
            }
        }
    }
    Ok(records)
}

pub(super) fn upgrade_operations(store: &Store) -> Result<BTreeSet<String>, InspectionError> {
    let mut saved = BTreeSet::new();
    for name in json_names(&store.operations, MAX_ENTRIES)? {
        let mut operation: Value = read_json_bounded(&store.operations, &name, MAX_RECORD)?;
        let ids = strings(operation.get("item_ids"));
        if operation.get("target").is_some_and(|target| !target.is_null()) {
            saved.extend(ids.iter().cloned());
        }
        let retire = operation.get("finished").and_then(Value::as_bool) == Some(false)
            && (matches!(operation["kind"].as_str(), Some("space_add" | "space_update"))
                || operation.get("target").is_some_and(|target| !target.is_null()));
        let mut changed = false;
        if operation["kind"] == "space_update" {
            operation["kind"] = "space_add".into();
            changed = true;
        }
        if let Some(space) = operation.get_mut("space").filter(|s| !s.is_null()) {
            let old = space.as_object_mut().ok_or_else(|| corrupt("invalid legacy Space result"))?;
            let legacy = old.contains_key("written") || old.contains_key("copy_mode")
                || old.contains_key("skipped_edited") || old.contains_key("companion_root_id");
            if legacy {
                let mut selected = strings(old.get("item_ids"));
                selected.extend(ids.iter().cloned());
                // Saved Library IDs remain in operation.item_ids. An interrupted
                // copy is not evidence that a live direct selection succeeded.
                if retire { selected.clear(); }
                let space_id = old.get("space_id").and_then(Value::as_str)
                    .ok_or_else(|| corrupt("legacy Space result has no workspace identity"))?.to_owned();
                *space = serde_json::json!({"space_id": space_id, "item_ids": selected});
                changed = true;
            }
        }
        if retire {
            let failure = serde_json::json!({
                "code": "space_copy_retired",
                "message": "This interrupted Space copy operation was cancelled because companion copying has been retired. Saved Library items are preserved; select them directly in the Space."
            });
            let phases = operation.get_mut("phases").and_then(Value::as_array_mut)
                .ok_or_else(|| corrupt("unfinished legacy operation has no phases"))?;
            let mut space_phase = false;
            for phase in phases.iter_mut() {
                if phase["phase"] == "space" {
                    space_phase = true;
                    phase["state"] = "cancelled".into();
                    phase["message"] = "Companion copying has been retired".into();
                    phase["error"] = failure.clone();
                } else if matches!(phase["state"].as_str(), Some("pending" | "running")) {
                    phase["state"] = "cancelled".into();
                    phase["error"] = failure.clone();
                }
            }
            if !space_phase {
                phases.push(serde_json::json!({
                    "phase": "space", "state": "cancelled", "done": 0, "total": null,
                    "message": "Companion copying has been retired", "error": failure
                }));
            }
            if let Some(space) = operation.get_mut("space").filter(|space| !space.is_null()) {
                space["item_ids"] = serde_json::json!([]);
            }
            operation["cancel_requested"] = true.into();
            operation["finished"] = true.into();
            changed = true;
        }
        changed |= upgrade_embedded_refs(&mut operation)?;
        if changed { bounded_write(&store.operations, &name, &operation, MAX_RECORD)?; }
    }
    for attempt in attempts(store)? {
        if let Some(id) = attempt.get("item_id").and_then(Value::as_str).filter(|s| !s.is_empty()) {
            saved.insert(id.to_owned());
        }
        saved.extend(strings(attempt.get("saved_item_ids")));
        saved.extend(strings(attempt.get("item_ids")));
    }
    Ok(saved)
}

fn upgrade_embedded_refs(value: &mut Value) -> Result<bool, InspectionError> {
    let mut changed = false;
    match value {
        Value::Object(object) => {
            if let Some(references) = object.get_mut("refs") {
                let references = references.as_array_mut().ok_or_else(|| corrupt("invalid embedded references"))?;
                let before = references.clone();
                upgrade_refs(references)?;
                changed |= before != *references;
            }
            for child in object.values_mut() {
                changed |= upgrade_embedded_refs(child)?;
            }
        }
        Value::Array(children) => {
            for child in children { changed |= upgrade_embedded_refs(child)?; }
        }
        _ => {}
    }
    Ok(changed)
}

pub(super) fn protect_saved_entry(entry: &mut Value, saved: &BTreeSet<String>) -> Result<(), InspectionError> {
    if saved.contains(entry["summary"]["item_id"].as_str().unwrap_or_default()) {
        let references = entry["summary"]["refs"].as_array_mut()
            .ok_or_else(|| corrupt("Library references are not an array"))?;
        if !references.iter().any(|reference| reference["kind"] == "manual") {
            references.push(serde_json::json!({"kind": "manual"}));
        }
        entry["summary"]["purge_after"] = Value::Null;
    }
    Ok(())
}

fn companion_ref(dir: &Dir, path: &Path, id: &str) -> Result<String, InspectionError> {
    let metadata = dir.dir_metadata().map_err(|e| corrupt(e.to_string()))?;
    let mut digest = Sha256::new();
    digest.update(id.as_bytes());
    digest.update([0]);
    digest.update(path.to_string_lossy().as_bytes());
    digest.update([0]);
    #[cfg(unix)] {
        use cap_fs_ext::MetadataExt;
        digest.update(metadata.dev().to_le_bytes());
        digest.update(metadata.ino().to_le_bytes());
    }
    #[cfg(not(unix))] let _ = metadata;
    Ok(format!("legacy:companion-{:x}", digest.finalize()))
}

fn known_companions(configuration: &ProjectConfiguration, target: &SpaceTarget, endpoint: &str)
    -> Result<(BTreeSet<String>, BTreeSet<String>), InspectionError> {
    let mut ids = BTreeSet::new();
    let mut references = BTreeSet::new();
    let root = match open_dir_nofollow_absolute(Path::new(&configuration.companion_root)) {
        Ok(root) => root,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok((ids, references)),
        // Unsafe or unavailable roots remain opaque legacy ownership.
        Err(_) => return Ok((ids, references)),
    };
    let state = match open_dir_nofollow_absolute(Path::new(&configuration.state_root)) {
        Ok(state) => state,
        Err(_) => return Ok((ids, references)),
    };
    for (count, entry) in root.entries().map_err(|e| corrupt(e.to_string()))?.enumerate() {
        if count >= MAX_ENTRIES { return Err(corrupt("legacy companion directory exceeds migration bound")); }
        let entry = entry.map_err(|e| corrupt(e.to_string()))?;
        let Some(id) = entry.file_name().to_str().map(str::to_owned) else { continue; };
        if uuid::Uuid::parse_str(&id).is_err() { continue; }
        let Ok(dir) = root.open_dir_nofollow(&id) else { continue; };
        let Ok(association) = read_json_bounded::<Value>(&dir, "manifest.json", MAX_RECORD) else { continue; };
        if association["schema_version"] != 1 || association["ownership"] != "cockpit"
            || association["cockpit_operation_id"] != id
            || association["herdr_session_identity"] != endpoint
            || association["herdr_workspace_id"] != target.space_id { continue; }
        let Ok(record) = read_json_bounded::<Value>(&state, &format!("{id}.json"), MAX_RECORD) else { continue; };
        let Some(operation) = record.get("operation") else { continue; };
        if operation["operation_id"] != id || operation["session_id"] != target.session_id
            || operation["plan"]["session_id"] != target.session_id
            || operation["plan"]["endpoint_identity"] != endpoint
            || operation["workspace_id"] != target.space_id
            || operation["plan"]["checkout_path"] != association["checkout_path"] { continue; }
        references.insert(companion_ref(&dir, &Path::new(&configuration.companion_root).join(&id), &id)?);
        let manifest = match read_json_bounded::<Value>(&dir, "context-manifest.json", MAX_RECORD) {
            Ok(manifest) => manifest,
            Err(_) => continue,
        };
        if !matches!(manifest["schema_version"].as_u64(), Some(1 | 2))
            || manifest["companion_id"] != id
            || manifest["owner_workspace_id"] != target.space_id
            || manifest["owner_worktree_path"] != association["checkout_path"]
            || manifest["primary_repository_identity"] != association["repository_key"] { continue; }
        for entry in manifest.get("entries").and_then(Value::as_array).into_iter().flatten() {
            if let Some(id) = entry.get("library_item_id").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                ids.insert(id.to_owned());
            }
        }
        for entry in manifest.get("library_copies").and_then(Value::as_array).into_iter().flatten() {
            if let Some(id) = entry.get("item_id").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                ids.insert(id.to_owned());
            }
        }
        for follow in manifest.get("library_follows").and_then(Value::as_array).into_iter().flatten() {
            ids.extend(strings(follow.get("known_page_item_ids")));
        }
        for key in ["intended_entry", "previous_entry"] {
            if let Some(id) = manifest["pending_source_intent"][key]["library_item_id"].as_str().filter(|s| !s.is_empty()) {
                ids.insert(id.to_owned());
            }
        }
        if let Some(id) = manifest["pending_library_remove"]["library_item_id"].as_str().filter(|s| !s.is_empty()) {
            ids.insert(id.to_owned());
        }
    }
    Ok((ids, references))
}

/// Caller has proven the live target and holds the exclusive Library lock.
/// Only Library metadata changes; all companion data is left untouched.
pub(super) fn migrate_target(store: &Store, configuration: &ProjectConfiguration,
    target: &SpaceTarget, endpoint: &str, context_id: &str) -> Result<(), InspectionError> {
    if let Some(record) = store.index()?.space_contexts.iter().find(|r| r.space_context_id == context_id) {
        if record.session_id != target.session_id || record.space_id != target.space_id {
            return Err(corrupt("Space selection identity differs from live target"));
        }
        if record.legacy_migrated { return Ok(()); }
    }
    let (mut recovered, known_refs) = known_companions(configuration, target, endpoint)?;
    if !known_refs.is_empty() {
        for attempt in attempts(store)? {
            if attempt["target"]["session_id"] != target.session_id
                || attempt["target"]["space_id"] != target.space_id { continue; }
            if let Some(id) = attempt["item_id"].as_str().filter(|s| !s.is_empty()) { recovered.insert(id.to_owned()); }
            recovered.extend(strings(attempt.get("saved_item_ids")));
            recovered.extend(strings(attempt.get("item_ids")));
        }
    }
    store.mutate_index_locked(|index| {
        let reference = LibraryItemRef::Space { space_context_id: context_id.to_owned() };
        let mut selected = BTreeSet::new();
        for entry in &mut index.items {
            let known = entry.summary.refs.iter().any(|r| matches!(r,
                LibraryItemRef::Space { space_context_id } if known_refs.contains(space_context_id)));
            if known || recovered.contains(&entry.summary.item_id) {
                selected.insert(entry.summary.item_id.clone());
                entry.summary.refs.retain(|r| !matches!(r,
                    LibraryItemRef::Space { space_context_id } if known_refs.contains(space_context_id)));
                refs::insert_ref(&mut entry.summary, reference.clone());
            }
        }
        if let Some(record) = index.space_contexts.iter_mut().find(|r| r.space_context_id == context_id) {
            selected.extend(record.item_ids.iter().cloned());
            record.item_ids = selected.into_iter().collect();
            record.legacy_migrated = true;
        } else {
            index.space_contexts.push(SpaceContextRecord {
                space_context_id: context_id.to_owned(), session_id: target.session_id.clone(),
                space_id: target.space_id.clone(), item_ids: selected.into_iter().collect(),
                repository_paths: vec![], legacy_migrated: true,
            });
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::{asset_entry, tests::{asset, fixture}};

    #[test]
    fn opaque_reference_upgrade_is_idempotent_and_unknown_ownership_fails_closed() {
        let mut entry = serde_json::json!({"summary": {"refs": [
            {"kind": "space", "companion_root_id": "unknown-root"}, {"kind": "manual"}
        ]}});
        upgrade_entry(&mut entry).unwrap();
        assert_eq!(entry["summary"]["refs"][0]["space_context_id"], "legacy:unknown-root");
        let upgraded = entry.clone();
        upgrade_entry(&mut entry).unwrap();
        assert_eq!(entry, upgraded);
        let mut unknown = serde_json::json!({"summary": {"refs": [{"kind": "future-owner", "id": "keep"}]}});
        let original = unknown.clone();
        assert_eq!(upgrade_entry(&mut unknown).unwrap_err().code, "library_corrupt");
        assert_eq!(unknown, original);
    }

    #[test]
    fn operation_migration_preserves_saved_ids_not_copy_paths_and_embedded_ownership() {
        let f = fixture();
        let store = f.service.open().unwrap();
        let operation = serde_json::json!({
            "kind": "space_update", "item_ids": ["saved-a", "saved-b"],
            "finished": true,
            "target": {"session_id": "session", "space_id": "workspace"},
            "space": {"space_id": "workspace", "written": ["sources/not-an-id.md"],
                "copy_mode": "copy", "skipped_edited": [], "companion_root_id": "old"},
            "report": {"rows": [{"item": {"refs": [{"kind": "space", "companion_root_id": "opaque"}]}}]}
        });
        bounded_write(&store.operations, "legacy.json", &operation, MAX_RECORD).unwrap();
        let saved = upgrade_operations(&store).unwrap();
        assert_eq!(saved, BTreeSet::from(["saved-a".to_owned(), "saved-b".to_owned()]));
        let migrated: Value = read_json_bounded(&store.operations, "legacy.json", MAX_RECORD).unwrap();
        assert_eq!(migrated["kind"], "space_add");
        assert_eq!(migrated["space"], serde_json::json!({"space_id": "workspace", "item_ids": ["saved-a", "saved-b"]}));
        assert_eq!(migrated["report"]["rows"][0]["item"]["refs"][0]["space_context_id"], "legacy:opaque");
        upgrade_operations(&store).unwrap();
        assert_eq!(read_json_bounded::<Value>(&store.operations, "legacy.json", MAX_RECORD).unwrap(), migrated);
        let mut no_saved = operation;
        no_saved["item_ids"] = serde_json::json!([]);
        bounded_write(&store.operations, "no-saved.json", &no_saved, MAX_RECORD).unwrap();
        upgrade_operations(&store).unwrap();
        let no_saved: Value = read_json_bounded(&store.operations, "no-saved.json", MAX_RECORD).unwrap();
        assert_eq!(no_saved["space"]["item_ids"], serde_json::json!([]));
    }

    #[test]
    fn unfinished_copy_operations_retire_without_claiming_selection_success() {
        let f = fixture();
        let store = f.service.open().unwrap();
        for kind in ["space_update", "space_add", "add"] {
            let name = format!("{kind}.json");
            let operation = serde_json::json!({
                "kind": kind, "finished": false, "cancel_requested": false,
                "item_ids": ["saved-a"], "target": {"session_id": "session", "space_id": "workspace"},
                "phases": [
                    {"phase": "library", "state": "done", "done": 1, "error": null},
                    {"phase": "space", "state": "running", "done": 0, "error": null}
                ],
                "space": {"space_id": "workspace", "written": ["copy.md"],
                    "skipped_edited": [], "copy_mode": null, "companion_root_id": null}
            });
            bounded_write(&store.operations, &name, &operation, MAX_RECORD).unwrap();
        }
        upgrade_operations(&store).unwrap();
        for kind in ["space_update", "space_add", "add"] {
            let name = format!("{kind}.json");
            let retired: Value = read_json_bounded(&store.operations, &name, MAX_RECORD).unwrap();
            assert_eq!(retired["finished"], true);
            assert_eq!(retired["cancel_requested"], true);
            assert_eq!(retired["item_ids"], serde_json::json!(["saved-a"]));
            assert_eq!(retired["space"]["item_ids"], serde_json::json!([]));
            assert_eq!(retired["phases"][0]["state"], "done");
            assert_eq!(retired["phases"][1]["state"], "cancelled");
            assert_eq!(retired["phases"][1]["error"]["code"], "space_copy_retired");
            assert!(retired["space"].get("written").is_none());
            upgrade_operations(&store).unwrap();
            assert_eq!(read_json_bounded::<Value>(&store.operations, &name, MAX_RECORD).unwrap(), retired);
        }
    }

    #[test]
    fn known_copies_and_pending_saves_select_once_without_touching_legacy_files() {
        let f = fixture();
        let store = f.service.open().unwrap();
        let configuration = &f.service.configuration;
        let target = SpaceTarget { session_id: "session".into(), space_id: "workspace".into() };
        let mut item_ids = Vec::new();
        for number in 1..=4 {
            let content = asset(number, "migration");
            let mut entry = asset_entry(&content, None);
            let stage = store.stage_asset(&mut entry, &content).unwrap();
            item_ids.push(entry.summary.item_id.clone());
            store.publish(stage, entry, None, None).unwrap();
        }
        store.meta.create_dir("space-adds").unwrap();
        let attempt_dir = store.meta.open_dir_nofollow("space-adds").unwrap();
        bounded_write(&attempt_dir, "pending.json", &serde_json::json!({
            "target": target, "item_id": item_ids[3], "state": "pending"
        }), MAX_RECORD).unwrap();
        let companion_id = uuid::Uuid::new_v4().to_string();
        let path = Path::new(&configuration.companion_root).join(&companion_id);
        std::fs::create_dir_all(&path).unwrap();
        std::fs::create_dir_all(&configuration.state_root).unwrap();
        let dir = open_dir_nofollow_absolute(&path).unwrap();
        let association = serde_json::json!({
            "schema_version": 1, "ownership": "cockpit", "cockpit_operation_id": companion_id,
            "herdr_session_identity": "endpoint", "herdr_workspace_id": "workspace",
            "checkout_path": "/checkout", "repository_key": "/repo"
        });
        let manifest = serde_json::json!({
            "schema_version": 2, "companion_id": companion_id, "owner_workspace_id": "workspace",
            "owner_worktree_path": "/checkout", "primary_repository_identity": "/repo",
            "entries": [{"library_item_id": item_ids[0]}],
            "library_copies": [{"item_id": item_ids[1]}],
            "pending_source_intent": {"intended_entry": {"library_item_id": item_ids[2]}}
        });
        bounded_write(&dir, "manifest.json", &association, MAX_RECORD).unwrap();
        bounded_write(&dir, "context-manifest.json", &manifest, MAX_RECORD).unwrap();
        std::fs::write(path.join("notes.md"), b"user notes").unwrap();
        let state = open_dir_nofollow_absolute(Path::new(&configuration.state_root)).unwrap();
        bounded_write(&state, &format!("{companion_id}.json"), &serde_json::json!({
            "operation": {"operation_id": companion_id, "session_id": "session", "workspace_id": "workspace",
                "plan": {"session_id": "session", "endpoint_identity": "endpoint", "checkout_path": "/checkout"}}
        }), MAX_RECORD).unwrap();
        let known = companion_ref(&dir, &path, &companion_id).unwrap();
        store.add_ref(&item_ids[0], LibraryItemRef::Space { space_context_id: known.clone() }).unwrap();
        store.add_ref(&item_ids[0], LibraryItemRef::Space { space_context_id: "legacy:unresolved".into() }).unwrap();
        let association_bytes = dir.read("manifest.json").unwrap();
        let manifest_bytes = dir.read("context-manifest.json").unwrap();
        let _lock = store.exclusive().unwrap();
        migrate_target(&store, configuration, &target, "endpoint", "space:fresh").unwrap();
        let index = store.index().unwrap();
        assert_eq!(index.space_contexts[0].item_ids.len(), 4);
        for entry in &index.items {
            assert!(entry.summary.refs.contains(&LibraryItemRef::Space { space_context_id: "space:fresh".into() }));
            assert!(!entry.summary.refs.contains(&LibraryItemRef::Space { space_context_id: known.clone() }));
            assert_eq!(entry.summary.purge_after, None);
        }
        assert!(index.items[0].summary.refs.contains(&LibraryItemRef::Space { space_context_id: "legacy:unresolved".into() }));
        store.mutate_index_locked(|index| {
            index.space_contexts[0].item_ids.clear();
            for entry in &mut index.items {
                entry.summary.refs.retain(|reference| !matches!(reference,
                    LibraryItemRef::Space { space_context_id } if space_context_id == "space:fresh"));
            }
            Ok(())
        }).unwrap();
        migrate_target(&store, configuration, &target, "endpoint", "space:fresh").unwrap();
        assert!(store.index().unwrap().space_contexts[0].item_ids.is_empty(), "migration cannot resurrect unselections");
        assert_eq!(dir.read("manifest.json").unwrap(), association_bytes);
        assert_eq!(dir.read("context-manifest.json").unwrap(), manifest_bytes);
        assert_eq!(dir.read("notes.md").unwrap(), b"user notes");
        migrate_target(&store, configuration, &target, "other-endpoint", "space:other").unwrap();
        assert!(store.index().unwrap().space_contexts[1].item_ids.is_empty(), "stale endpoint cannot adopt companions");
        #[cfg(unix)] {
            // Even exact identity JSON outside the companion grants no authority
            // when reached through a symlink.
            let outside = f.root.join("outside-association.json");
            std::fs::write(&outside, &association_bytes).unwrap();
            dir.remove_file("manifest.json").unwrap();
            std::os::unix::fs::symlink(&outside, path.join("manifest.json")).unwrap();
            let (ids, references) = known_companions(configuration, &target, "endpoint").unwrap();
            assert!(ids.is_empty());
            assert!(references.is_empty());
            assert_eq!(std::fs::read(&outside).unwrap(), association_bytes);
        }
    }
}
