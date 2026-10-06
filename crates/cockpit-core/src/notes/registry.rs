use super::fs;
use crate::InspectionError;
use cap_std::fs::Dir;
use cockpit_protocol::notes::*;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};
use uuid::Uuid;
const MAX_REGISTRY: usize = 4 * 1024 * 1024;

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    label: Option<String>,
    created: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    endpoint: String,
    session_id: String,
    space_id: String,
    notes_id: String,
    bound: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Registry {
    schema: u32,
    #[serde(deserialize_with = "unique_notes")]
    notes: BTreeMap<String, Entry>,
    bindings: Vec<Binding>,
}
impl Default for Registry {
    fn default() -> Self {
        Self {
            schema: 1,
            notes: BTreeMap::new(),
            bindings: vec![],
        }
    }
}
fn unique_notes<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<BTreeMap<String, Entry>, D::Error> {
    struct Visitor;
    impl<'de> serde::de::Visitor<'de> for Visitor {
        type Value = BTreeMap<String, Entry>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a unique Notes map")
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut map: A,
        ) -> Result<Self::Value, A::Error> {
            let mut notes = BTreeMap::new();
            while let Some((id, entry)) = map.next_entry::<String, Entry>()? {
                if notes.len() >= 4096 || notes.insert(id, entry).is_some() {
                    return Err(serde::de::Error::custom(
                        "duplicate or excessive Notes entries",
                    ));
                }
            }
            Ok(notes)
        }
    }
    deserializer.deserialize_map(Visitor)
}
#[derive(Clone)]
pub(super) struct Authority {
    pub endpoint: String,
    pub session_id: String,
    pub space_id: String,
    pub label: String,
}
impl Authority {
    pub(super) fn info(&self) -> NotesSpaceInfo {
        NotesSpaceInfo {
            session_id: self.session_id.clone(),
            space_id: self.space_id.clone(),
            label: self.label.clone(),
        }
    }
    fn matches(&self, binding: &Binding) -> bool {
        self.endpoint == binding.endpoint
            && self.session_id == binding.session_id
            && self.space_id == binding.space_id
    }
}
fn state(root: &Dir, create: bool) -> Result<Option<Dir>, InspectionError> {
    match fs::child(root, ".cockpit", create) {
        Ok(dir) => Ok(Some(dir)),
        Err(e) if !create && e.code == "notes_not_found" => Ok(None),
        Err(e) => Err(e),
    }
}
fn timestamp_valid(value: &str) -> bool {
    value.len() <= 128
        && value.ends_with('Z')
        && time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339)
            .is_ok()
}
fn load(dir: Option<&Dir>) -> Result<(Registry, NotesDocument), InspectionError> {
    let absent = NotesDocument {
        content: String::new(),
        revision: "absent".into(),
    };
    let Some(dir) = dir else {
        return Ok((Registry::default(), absent));
    };
    let doc = fs::read(dir, "registry.json", MAX_REGISTRY)
        .map_err(|e| fs::error("notes_registry_corrupt", e.message))?;
    if doc.revision == "absent" {
        return Ok((Registry::default(), doc));
    }
    let reg: Registry = serde_json::from_str(&doc.content)
        .map_err(|e| fs::error("notes_registry_corrupt", e.to_string()))?;
    if reg.schema != 1
        || reg.bindings.len() > 4096
        || reg.notes.len() > 4096
        || reg.notes.iter().any(|(id, entry)| {
            fs::validate_uuid(id).is_err()
                || entry
                    .label
                    .as_ref()
                    .is_some_and(|v| v.len() > 4096 || v.chars().any(char::is_control))
                || entry.created.as_ref().is_some_and(|v| !timestamp_valid(v))
        })
        || reg.bindings.iter().any(|b| {
            fs::validate_uuid(&b.notes_id).is_err()
                || !reg.notes.contains_key(&b.notes_id)
                || !timestamp_valid(&b.bound)
                || b.endpoint.is_empty()
                || b.endpoint.len() > 4096
                || b.endpoint.chars().any(char::is_control)
                || [b.session_id.as_str(), b.space_id.as_str()]
                    .iter()
                    .any(|v| v.is_empty() || v.len() > 256 || v.chars().any(char::is_control))
        })
    {
        return Err(fs::error(
            "notes_registry_corrupt",
            "Notes registry failed validation",
        ));
    }
    let mut keys = std::collections::BTreeSet::new();
    let mut folders = std::collections::BTreeSet::new();
    if reg.bindings.iter().any(|b| {
        !keys.insert((&b.endpoint, &b.session_id, &b.space_id)) || !folders.insert(&b.notes_id)
    }) {
        return Err(fs::error(
            "notes_registry_corrupt",
            "Notes registry contains duplicate bindings",
        ));
    }
    Ok((reg, doc))
}
fn save(dir: &Dir, base: &NotesDocument, reg: &Registry) -> Result<(), InspectionError> {
    if reg.bindings.len() > 4096 || reg.notes.len() > 4096 {
        return Err(fs::error(
            "notes_too_large",
            "Notes registry limit exceeded",
        ));
    }
    let json = serde_json::to_string_pretty(reg)
        .map_err(|e| fs::error("notes_registry_corrupt", e.to_string()))?;
    fs::publish(dir, "registry.json", base, &json, MAX_REGISTRY)
}
pub(super) fn catalog(root: &Dir) -> Result<NotesResult, InspectionError> {
    let state = state(root, false)?;
    let (reg, _) = load(state.as_ref())?;
    let mut entries = Vec::new();
    for id in fs::entries(root, 4097)? {
        if fs::validate_uuid(&id).is_err() {
            continue;
        }
        let meta = root.symlink_metadata(&id).map_err(fs::io_error)?;
        if meta.file_type().is_symlink() {
            return Err(fs::error(
                "notes_unsafe_path",
                "Catalog folder is a symlink",
            ));
        }
        if !meta.is_dir() {
            continue;
        }
        let _ = fs::child(root, &id, false)?;
        if entries.len() >= 4096 {
            return Err(fs::error(
                "notes_too_large",
                "Notes catalog exceeds its folder limit",
            ));
        }
        let entry = reg.notes.get(&id);
        entries.push(NotesCatalogEntry {
            notes_id: id.clone(),
            label: entry.and_then(|e| e.label.clone()),
            created: entry.and_then(|e| e.created.clone()),
            bound: reg.bindings.iter().any(|b| b.notes_id == id),
        });
    }
    Ok(NotesResult::Catalog { entries })
}
pub(super) fn resolve(root: &Dir, authority: &Authority) -> Result<String, InspectionError> {
    let state = state(root, false)?;
    let (reg, _) = load(state.as_ref())?;
    let binding = reg
        .bindings
        .iter()
        .find(|b| authority.matches(b))
        .ok_or_else(|| {
            fs::error(
                "notes_unbound",
                "This Space has no Notes association; explicitly create or attach Notes",
            )
        })?;
    let _ = fs::child(root, &binding.notes_id, false)?;
    Ok(binding.notes_id.clone())
}
pub(super) fn bind(
    root: &Dir,
    authority: &Authority,
    attach: Option<&str>,
) -> Result<(String, bool), InspectionError> {
    let dir = state(root, true)?.expect("created registry directory");
    let _lock = fs::lock(&dir, "registry.lock")?;
    let (mut reg, base) = load(Some(&dir))?;
    if let Some(binding) = reg.bindings.iter().find(|b| authority.matches(b)) {
        if attach.is_none_or(|id| id == binding.notes_id) {
            let _ = fs::child(root, &binding.notes_id, false)?;
            return Ok((binding.notes_id.clone(), false));
        }
        return Err(fs::error(
            "notes_already_bound",
            "This Space is already associated with different Notes",
        ));
    }
    let id = match attach {
        Some(id) => {
            fs::validate_uuid(id)?;
            let _ = fs::child(root, id, false)?;
            id.to_owned()
        }
        None => {
            if reg.notes.len() >= 4096 {
                return Err(fs::error("notes_too_large", "Notes catalog limit exceeded"));
            }
            // Bound the whole catalog before creating anything.
            let NotesResult::Catalog { entries } = catalog(root)? else {
                unreachable!()
            };
            if entries.len() >= 4096 {
                return Err(fs::error("notes_too_large", "Notes catalog limit exceeded"));
            }
            Uuid::new_v4().to_string()
        }
    };
    let now = fs::now();
    reg.notes.entry(id.clone()).or_insert_with(|| Entry {
        label: Some(authority.label.clone()),
        created: if attach.is_none() {
            Some(now.clone())
        } else {
            None
        },
    });
    if let Some(entry) = reg.notes.get_mut(&id) {
        entry.label = Some(authority.label.clone());
    }
    reg.bindings.retain(|b| b.notes_id != id);
    reg.bindings.push(Binding {
        endpoint: authority.endpoint.clone(),
        session_id: authority.session_id.clone(),
        space_id: authority.space_id.clone(),
        notes_id: id.clone(),
        bound: now,
    });
    if reg.notes.len() > 4096 || reg.bindings.len() > 4096 {
        return Err(fs::error(
            "notes_too_large",
            "Notes registry limit exceeded",
        ));
    }
    // Validate the serialized registry before creating a durable folder.
    if serde_json::to_vec(&reg)
        .map_err(|e| fs::error("notes_registry_corrupt", e.to_string()))?
        .len()
        > MAX_REGISTRY
    {
        return Err(fs::error(
            "notes_too_large",
            "Notes registry exceeds its size limit",
        ));
    }
    if attach.is_none() {
        let _ = fs::child(root, &id, true)?;
    }
    save(&dir, &base, &reg)?;
    Ok((id, true))
}
fn metadata_token(dir: &Dir, name: &str) -> Result<String, InspectionError> {
    match dir.symlink_metadata(name) {
        Ok(m) => {
            if m.file_type().is_symlink() || !m.is_file() {
                return Err(fs::error(
                    "notes_unsafe_path",
                    "Notes document is not a regular file",
                ));
            }
            Ok(format!(
                "{}:{:?}",
                m.len(),
                m.modified().map_err(fs::io_error)?
            ))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok("absent".into()),
        Err(e) => Err(fs::io_error(e)),
    }
}
fn collection_token(
    dir: &Dir,
    name: &str,
    max: usize,
    nested: bool,
) -> Result<String, InspectionError> {
    let collection = match fs::child(dir, name, false) {
        Ok(d) => d,
        Err(e) if e.code == "notes_not_found" => return Ok("absent".into()),
        Err(e) => return Err(e),
    };
    let mut token = String::new();
    for entry in fs::entries(&collection, max)? {
        if entry.starts_with('.') {
            continue;
        }
        let meta = collection.symlink_metadata(&entry).map_err(fs::io_error)?;
        if meta.file_type().is_symlink() {
            return Err(fs::error(
                "notes_unsafe_path",
                "Notes collection contains a symlink",
            ));
        }
        if nested && meta.is_dir() {
            fs::validate_id(&entry, 64)?;
            token.push_str(&entry);
            token.push_str(&collection_token(&collection, &entry, 1000, false)?);
        } else if meta.is_file() && entry.ends_with(".md") {
            token.push_str(&entry);
            token.push(':');
            token.push_str(&metadata_token(&collection, &entry)?);
            token.push('\n');
        }
    }
    Ok(fs::revision(token.as_bytes()))
}
pub(super) fn target(
    folder: &Path,
    dir: &Dir,
    id: String,
    authority: Option<&Authority>,
) -> Result<NotesResult, InspectionError> {
    Ok(NotesResult::Target {
        info: NotesTargetInfo {
            notes_id: id,
            folder: folder.to_string_lossy().into_owned(),
            space: authority.map(Authority::info),
            change_tokens: NotesChangeTokens {
                scratchpad: fs::revision(metadata_token(dir, "scratchpad.md")?.as_bytes()),
                todos: fs::revision(metadata_token(dir, "todos.md")?.as_bytes()),
                decisions: collection_token(dir, "decisions", 4096, false)?,
                comments: collection_token(dir, "comments", 5000, true)?,
            },
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("cockpit-notes-registry-{}", Uuid::new_v4()));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn dir(&self) -> Dir {
            fs::root(&self.0, false).unwrap().1
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn authority(endpoint: &str) -> Authority {
        Authority {
            endpoint: endpoint.into(),
            session_id: "session".into(),
            space_id: "w1".into(),
            label: "Same label".into(),
        }
    }
    #[test]
    fn notes_binding_is_boot_scoped_and_attach_moves_single_association() {
        let f = Fixture::new();
        let dir = f.dir();
        let old = authority("boot-1");
        let fresh = authority("boot-2");
        let (id, changed) = bind(&dir, &old, None).unwrap();
        assert!(changed);
        assert_eq!(resolve(&dir, &old).unwrap(), id);
        assert_eq!(resolve(&dir, &fresh).unwrap_err().code, "notes_unbound");
        assert_eq!(bind(&dir, &fresh, Some(&id)).unwrap(), (id.clone(), true));
        assert_eq!(resolve(&dir, &fresh).unwrap(), id);
        assert_eq!(resolve(&dir, &old).unwrap_err().code, "notes_unbound");
        assert_eq!(bind(&dir, &fresh, Some(&id)).unwrap(), (id, false));
        assert!(f.0.join(".cockpit/registry.lock").is_file());
    }
    #[test]
    fn notes_corrupt_registry_is_never_replaced_or_used_for_creation() {
        let f = Fixture::new();
        let dir = f.dir();
        let state = fs::child(&dir, ".cockpit", true).unwrap();
        let id = Uuid::new_v4().to_string();
        let invalid = format!(
            r#"{{"schema":1,"notes":{{"{id}":{{"label":null,"created":null}},"{id}":{{"label":null,"created":null}}}},"bindings":[]}}"#
        );
        std::fs::write(f.0.join(".cockpit/registry.json"), &invalid).unwrap();
        assert_eq!(
            bind(&dir, &authority("boot"), None).unwrap_err().code,
            "notes_registry_corrupt"
        );
        assert_eq!(
            fs::read(&state, "registry.json", MAX_REGISTRY)
                .unwrap()
                .content,
            invalid
        );
        assert!(!f.0.join(id).exists());
    }
}
