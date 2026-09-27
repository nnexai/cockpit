//! Explicit attachment acquisition and complete-item D2 republication.
use super::{LibraryService, SaveOptions, operations, store::{self, LibraryIndexEntry, MarkerFile, Stage, Store, error}};
use crate::{InspectionError, sources::{AttachmentRef, SourceAsset, SourceAttachment, SourceContainer, SourceRef, FrontmatterField, content_revision, confluence_attachment_pattern, confluence_glob_matches}};
use cap_fs_ext::{DirExt, OpenOptionsFollowExt, OpenOptionsSyncExt};
use cap_std::fs::{Dir, OpenOptions};
use cockpit_protocol::library::*;
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, io::{Read, Write}, path::Path, sync::Arc};

fn io(e: std::io::Error) -> InspectionError { error("library_unavailable", e.to_string()) }
fn unsafe_download() -> InspectionError {
    error("source_capability_unavailable", "this confluence-cli version cannot download attachments safely")
}
fn single(name: &str) -> bool {
    !name.is_empty() && name.len() <= 255 && !name.contains(['/', '\\'])
        && Path::new(name).components().count() == 1
        && matches!(Path::new(name).components().next(), Some(std::path::Component::Normal(_)))
}
fn open_regular(dir: &Dir, name: &str) -> Result<cap_std::fs::File, InspectionError> {
    if !single(name) { return Err(unsafe_download()); }
    let mut opts = OpenOptions::new();
    opts.read(true).follow(cap_fs_ext::FollowSymlinks::No).nonblock(true);
    let file = dir.open_with(name, &opts).map_err(|_| unsafe_download())?;
    let metadata = file.metadata().map_err(|_| unsafe_download())?;
    #[cfg(unix)]
    {
        use cap_std::fs::MetadataExt;
        if metadata.nlink() != 1 { return Err(unsafe_download()); }
    }
    if !metadata.is_file() { return Err(unsafe_download()); }
    Ok(file)
}
fn stored_name(title: &str, id: &str, used: &mut BTreeSet<String>) -> String {
    let mut base = crate::context_assets::readable_name(title);
    let stem = base.split('.').next().unwrap_or_default().to_ascii_lowercase();
    if matches!(stem.as_str(), "con" | "prn" | "aux" | "nul")
        || (stem.len() == 4 && (stem.starts_with("com") || stem.starts_with("lpt")) && stem.ends_with(|c: char| c.is_ascii_digit()))
    { base.insert_str(0, "attachment-"); }
    if used.insert(base.to_ascii_lowercase()) { return base; }
    let suffix = format!("{:x}", Sha256::digest(id.as_bytes()));
    let (stem, ext) = base.rsplit_once('.').map(|(s,e)| (s, format!(".{e}"))).unwrap_or((&base, String::new()));
    let mut n = 0u32;
    loop {
        let candidate = format!("{stem}-{}{}{ext}", &suffix[..12], if n == 0 { String::new() } else { format!("-{n}") });
        if used.insert(candidate.to_ascii_lowercase()) { return candidate; }
        n += 1;
    }
}

pub(super) struct Prepared {
    pub stage: Stage,
    pub files: Vec<MarkerFile>,
    pub partial: bool,
    pub reason: Option<String>,
}
impl Prepared {
    pub fn revision(&self, asset: &SourceAsset) -> String {
        let content = content_revision(asset);
        if self.files.is_empty() && !asset.attachments.iter().any(|a| matches!(a.not_downloaded.as_deref(), Some("over_limit" | "failed"))) {
            return content;
        }
        let mut hash = Sha256::new();
        hash.update(content.as_bytes());
        for a in &asset.attachments {
            hash.update(serde_json::to_vec(&(&a.id, &a.path, &a.not_downloaded)).expect("attachment state"));
        }
        for f in &self.files {
            hash.update(serde_json::to_vec(&(&f.path, &f.hash, f.bytes)).expect("attachment file"));
        }
        format!("sha256:{:x}", hash.finalize())
    }
}
pub(super) fn summaries(asset: &SourceAsset) -> Vec<LibraryAttachment> {
    asset.attachments.iter().map(|a| LibraryAttachment {
        attachment_id: a.id.clone(), original_name: a.title.clone(),
        stored_name: a.path.as_deref().and_then(|p| p.strip_prefix("attachments/")).unwrap_or(&a.title).into(),
        media_type: a.media_type.clone(), bytes: a.size, version: a.source_revision.clone(),
        state: if a.path.is_some() { LibraryAttachmentState::Downloaded } else { match a.not_downloaded.as_deref() {
            Some("over_limit") => LibraryAttachmentState::OverLimit,
            Some("failed") => LibraryAttachmentState::Failed,
            _ => LibraryAttachmentState::NotDownloaded,
        } }, relative_path: a.path.clone(),
    }).collect()
}

impl LibraryService {
    pub async fn start_attachments(&self, request: LibraryAttachmentRequest) -> Result<LibraryOperation, InspectionError> {
        let handle = operations::runtime()?;
        let store = self.open()?;
        let lease = store.lease(&request.item_id)?;
        let old = self.entry(&store, &request.item_id)?.ok_or_else(|| error("library_item_not_found", "Library item does not exist"))?;
        let provider = old.summary.provider_id.as_deref().unwrap_or_default();
        if old.summary.resource_type.as_deref() != Some("page") || !self.configuration.providers.iter().any(|p| p.id == provider && crate::repositories::is_confluence_executable(&p.executable)) {
            return Err(error("source_capability_unavailable", "Only Confluence pages support attachment downloads"));
        }
        let mut selected = BTreeSet::new();
        if request.attachment_ids.is_empty() || request.attachment_ids.len() > 256 || request.attachment_ids.iter().any(|id| !selected.insert(id) || !old.summary.attachments.iter().any(|a| &a.attachment_id == id)) {
            return Err(error("source_provider_contract", "Select existing, distinct attachment ids"));
        }
        let (record, operation_lease) = operations::create(&store, LibraryOperationKind::Attachments, Some(1))?;
        let service = self.clone();
        let id = record.operation_id.clone();
        let worker = store.clone();
        operations::spawn(handle, store, id.clone(), operation_lease, async move {
            let _lease = lease;
            if operations::cancelled(&worker, &id)? { return Ok(()); }
            let asset = {
                let _lock = worker.shared()?;
                worker.check_confirmation(&old, None)?;
                read_asset(&worker, &old)?
            };
            service.save_asset_with(&worker, &id, asset, Some(old), SaveOptions {
                attachment_request: Some(&request), ..SaveOptions::default()
            }).await
        });
        Ok(record)
    }

    pub(super) async fn prepare_attachments(
        &self, store: &Arc<Store>, operation: &str, asset: &mut SourceAsset,
        old: Option<&LibraryIndexEntry>, download_all: bool, request: Option<&LibraryAttachmentRequest>,
        confirmed: Option<&[LibraryConflictFile]>,
    ) -> Result<Option<Prepared>, InspectionError> {
        let stage = store.stage()?;
        let mut prepared = Prepared { stage, files: vec![], partial: false, reason: None };
        let mut ids = BTreeSet::new();
        if asset.attachments.iter().any(|a| !ids.insert(a.id.clone())) {
            return Err(error("source_provider_contract", "Attachment ids are not unique"));
        }
        let refs: Vec<_> = asset.attachments.iter().map(|a| AttachmentRef { id: a.id.clone(), title: a.title.clone(), bytes: a.size }).collect();
        let mut used = old.into_iter().flat_map(|e| &e.summary.attachments)
            .filter_map(|a| a.relative_path.as_deref().and_then(|p| p.strip_prefix("attachments/")))
            .map(str::to_ascii_lowercase).collect::<BTreeSet<_>>();
        let mut total = asset.attachments.iter().filter_map(|a| {
            let removing = request.is_some_and(|r| r.action == LibraryAttachmentAction::RemoveDownloaded && r.attachment_ids.contains(&a.id));
            old.and_then(|e| e.summary.attachments.iter().find(|p| !removing && p.attachment_id == a.id && p.version == a.source_revision && p.bytes == a.size && p.original_name == a.title)
                .and_then(|p| p.relative_path.as_ref()).and_then(|path| e.inventory.iter().find(|f| &f.path == path)).map(|f| f.bytes))
        }).fold(0u64, u64::saturating_add);
        let per_file = self.configuration.limits.library_attachment_bytes;
        let per_page = self.configuration.limits.library_item_attachment_bytes;
        let mut failures = Vec::new();
        // Download directories never enter the item stage which D2 publishes.
        let mut downloads = None;
        for (index, a) in asset.attachments.iter_mut().enumerate() {
            if operations::cancelled(store, operation)? { return Ok(None); }
            a.path = None;
            let previous = old.and_then(|e| e.summary.attachments.iter().find(|p| p.attachment_id == a.id));
            let selected = request.is_some_and(|r| r.attachment_ids.contains(&a.id));
            let removing = selected && request.is_some_and(|r| r.action == LibraryAttachmentAction::RemoveDownloaded);
            let previous_path = previous.and_then(|p| p.relative_path.as_deref());
            let replacing_confirmed_file = previous_path.is_some_and(|path| {
                confirmed.is_some_and(|files| files.iter().any(|file| file.path == path))
            });
            let downloading = download_all || (selected && !removing) || replacing_confirmed_file;
            let unchanged = previous.is_some_and(|p| p.version == a.source_revision && p.bytes == a.size && p.original_name == a.title);
            a.not_downloaded = Some(a.not_downloaded.take().filter(|_| previous.is_none()).unwrap_or_else(|| "not_requested".into()));
            if removing { continue; }
            let previous_file = previous.filter(|_| unchanged && !replacing_confirmed_file).and_then(|p| p.relative_path.as_deref());
            if let (Some(old), Some(path)) = (old, previous_file) {
                let expected = old.inventory.iter().find(|f| f.path == path).ok_or_else(|| error("library_corrupt", "Attachment is absent from the item inventory"))?;
                let name = path.strip_prefix("attachments/").filter(|name| single(name)).ok_or_else(unsafe_download)?;
                let _lock = store.shared()?;
                let root = store.item_dir(&old.summary.item_path)?;
                let parent = root.open_dir_nofollow("attachments").map_err(io)?;
                let file = open_regular(&parent, name)?;
                let copied = copy_file(file, &prepared.stage.dir, name, expected.bytes, Some(expected.bytes))?;
                if copied.hash != expected.hash { return Err(error("library_conflict", "Library attachment changed while copying")); }
                a.path = Some(copied.path.clone()); a.not_downloaded = None;
                prepared.files.push(copied);
                continue;
            }
            if !downloading {
                if unchanged {
                    a.not_downloaded = Some(match previous.map(|p| p.state) {
                        Some(LibraryAttachmentState::OverLimit) => "over_limit",
                        Some(LibraryAttachmentState::Failed) => "failed",
                        _ => "not_requested",
                    }.into());
                }
                continue;
            }
            if a.size.is_some_and(|bytes| bytes > per_file || bytes > per_page.saturating_sub(total)) || total >= per_page {
                if previous.is_some_and(|p| p.state == LibraryAttachmentState::Downloaded) {
                    return Err(error("source_attachment_size", "Attachment replacement exceeds limits; the previous page and attachments were retained"));
                }
                a.not_downloaded = Some("over_limit".into()); prepared.partial = true;
                failures.push(format!("{}: over limit", a.title));
                continue;
            }
            let pattern = confluence_attachment_pattern(&a.title);
            let mut matches = refs.iter().filter(|r| confluence_glob_matches(&pattern, &r.title));
            // Unknown sizes reserve the full per-file allowance. Checked
            // addition refuses overflowing match sets rather than truncating
            // the allowance passed to the in-flight monitor.
            let budget = matches.try_fold(crate::process::StagingBudget { bytes: 0, max_files: 0 }, |budget, r| {
                Some(crate::process::StagingBudget {
                    bytes: budget.bytes.checked_add(r.bytes.unwrap_or(per_file))?,
                    max_files: budget.max_files + 1,
                })
            });
            let remaining = per_page.saturating_sub(total);
            let Some(budget) = budget.filter(|budget| budget.bytes <= per_file && budget.bytes <= remaining) else {
                if previous.is_some_and(|p| p.state == LibraryAttachmentState::Downloaded) {
                    return Err(error("source_attachment_size", "Attachment replacement matches siblings over the limit; the previous page and attachments were retained"));
                }
                a.not_downloaded = Some("failed".into()); prepared.partial = true;
                failures.push(format!("{}: name matches other attachments over the limit", a.title));
                continue;
            };
            if downloads.is_none() { downloads = Some(store.stage()?); }
            let downloads = downloads.as_ref().expect("download stage initialized");
            let dl = format!("dl-{index}");
            rustix::fs::mkdirat(&downloads.dir, &dl, rustix::fs::Mode::from_raw_mode(0o700)).map_err(|e| io(e.into()))?;
            let dest = downloads.dir.open_dir_nofollow(&dl).map_err(io)?;
            let dest_path = store.path.join(".cockpit/staging").join(&downloads.name).join(&dl);
            let siblings: Vec<_> = refs.iter().filter(|r| r.id != a.id).cloned().collect();
            let result = tokio::select! {
                biased;
                cancelled = wait_for_cancellation(store, operation) => {
                    cancelled?;
                    return Ok(None);
                }
                result = self.sources.download_attachment(&asset.source.provider_id, &asset.source.canonical_id, &refs[index], &siblings, &dest, &dest_path, budget) => result,
            };
            if operations::cancelled(store, operation)? { return Ok(None); }
            let actual = crate::context_assets::open_absolute_dir_nofollow(&dest_path).map_err(|_| unsafe_download())?;
            #[cfg(unix)]
            {
                use cap_std::fs::MetadataExt;
                let before = dest.dir_metadata().map_err(io)?;
                let after = actual.dir_metadata().map_err(io)?;
                if (before.dev(), before.ino()) != (after.dev(), after.ino()) { return Err(unsafe_download()); }
            }
            let copied = match result {
                Ok(result) => {
                    let file = open_regular(&dest, &result.file_name)?;
                    if let Some(name) = previous.and_then(|p| p.relative_path.as_deref()).and_then(|p| p.strip_prefix("attachments/")) {
                        used.remove(&name.to_ascii_lowercase());
                    }
                    let name = stored_name(&a.title, &a.id, &mut used);
                    copy_file(file, &prepared.stage.dir, &name, per_file.min(per_page.saturating_sub(total)), a.size)
                }
                Err(e) if e.code == "source_capability_unavailable" => return Err(e),
                Err(e) => Err(e),
            };
            match copied {
                Ok(file) => {
                    total += file.bytes;
                    a.path = Some(file.path.clone()); a.not_downloaded = None;
                    prepared.files.push(file);
                }
                Err(e) => {
                    if previous.is_some_and(|p| p.state == LibraryAttachmentState::Downloaded) {
                        return Err(error("source_attachment_failed", "Attachment replacement failed; the previous page and attachments were retained"));
                    }
                    a.not_downloaded = Some("failed".into()); prepared.partial = true;
                    // Provider diagnostics may include CLI paths/config; persist only a stable code.
                    failures.push(format!("{}: {}", a.title, e.code));
                }
            }
            downloads.dir.remove_dir_all(&dl).map_err(io)?;
        }
        if !failures.is_empty() { prepared.reason = Some(failures.join("; ")); }
        Ok(Some(prepared))
    }
}

// Operation cancellation is persisted so other hosts can request it. Poll the
// record while the provider runs; dropping its future kills its owned child
// before the operation-owned Stage guards remove the download directories.
async fn wait_for_cancellation(store: &Store, operation: &str) -> Result<(), InspectionError> {
    loop {
        if operations::cancelled(store, operation)? { return Ok(()); }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

/// Copy only the verified open regular file, never rename a CLI-controlled path.
/// The temporary output is removed on every failure, including growth mid-copy.
fn copy_file(mut file: cap_std::fs::File, root: &Dir, name: &str, limit: u64, size: Option<u64>) -> Result<MarkerFile, InspectionError> {
    let before = file.metadata().map_err(io)?;
    if before.len() > limit || size.is_some_and(|n| before.len() != n) {
        return Err(error("source_attachment_size", "Attachment size differs from metadata or exceeds the limit"));
    }
    match root.create_dir("attachments") { Ok(()) => {}, Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}, Err(e) => return Err(io(e)) }
    let parent = root.open_dir_nofollow("attachments").map_err(io)?;
    let mut opts = OpenOptions::new(); opts.write(true).create_new(true).follow(cap_fs_ext::FollowSymlinks::No);
    let mut out = parent.open_with(name, &opts).map_err(io)?;
    let result = (|| {
        let mut hash = Sha256::new(); let mut bytes = 0u64; let mut buf = [0u8; 32768];
        loop {
            let n = file.read(&mut buf).map_err(io)?; if n == 0 { break; }
            bytes += n as u64;
            if bytes > limit || bytes > before.len() { return Err(error("source_attachment_size", "Attachment grew while reading")); }
            out.write_all(&buf[..n]).map_err(io)?; hash.update(&buf[..n]);
        }
        let after = file.metadata().map_err(io)?;
        if bytes != before.len() || after.len() != before.len() || after.modified().ok() != before.modified().ok() {
            return Err(error("source_attachment_size", "Attachment changed while reading"));
        }
        out.sync_all().map_err(io)?;
        Ok(MarkerFile { path: format!("attachments/{name}"), hash: format!("sha256:{:x}", hash.finalize()), bytes })
    })();
    drop(out);
    if result.is_err() { let _ = parent.remove_file(name); }
    result
}

/// Read only Cockpit's generated format, verified against the trusted inventory.
/// No provider call is needed to remove bytes or download a listed attachment.
fn read_asset(store: &Store, entry: &LibraryIndexEntry) -> Result<SourceAsset, InspectionError> {
    let bad = || error("library_corrupt", "Library document has invalid generated metadata");
    let root = store.item_dir(&entry.summary.item_path)?;
    let expected = entry.inventory.iter().find(|f| f.path == "document.md").ok_or_else(bad)?;
    let mut file = open_regular(&root, "document.md")?;
    if file.metadata().map_err(io)?.len() != expected.bytes { return Err(bad()); }
    let mut bytes = Vec::new(); Read::by_ref(&mut file).take(expected.bytes + 1).read_to_end(&mut bytes).map_err(io)?;
    if store::hash(&bytes) != expected.hash { return Err(error("library_conflict", "Library document changed")); }
    let text = String::from_utf8(bytes).map_err(|_| bad())?;
    let (header, body) = text.strip_prefix("---\n").and_then(|s| s.split_once("\n---\n")).ok_or_else(bad)?;
    let body = body.strip_prefix(&format!("\n# {}\n\n", entry.summary.title)).and_then(|s| s.strip_suffix('\n')).ok_or_else(bad)?.to_owned();
    let mut values = serde_json::Map::new();
    let mut fields = vec![];
    let mut container = serde_json::Map::new();
    let mut attachments: Vec<SourceAttachment> = vec![];
    let mut section = "";
    for line in header.lines() {
        if line == "container:" { section = "container"; continue; }
        if line == "attachments:" { section = "attachments"; continue; }
        if let Some(id) = line.strip_prefix("  - id: ") {
            attachments.push(SourceAttachment { id: serde_json::from_str(id).map_err(|_| bad())?, title: String::new(), media_type: None, size: None, source_url: None, source_revision: None, path: None, not_downloaded: None });
            continue;
        }
        let (key, value) = line.trim_start().split_once(": ").ok_or_else(bad)?;
        let value: serde_json::Value = serde_json::from_str(value).map_err(|_| bad())?;
        if line.starts_with("    ") && section == "attachments" {
            let a = attachments.last_mut().ok_or_else(bad)?;
            let string = || value.as_str().map(str::to_owned).ok_or_else(bad);
            match key {
                "title" => a.title = string()?, "media_type" => a.media_type = Some(string()?),
                "size" => a.size = Some(value.as_u64().ok_or_else(bad)?),
                "source_url" => a.source_url = Some(string()?), "source_revision" => a.source_revision = Some(string()?),
                "path" => a.path = Some(string()?), "not_downloaded" => a.not_downloaded = Some(string()?),
                _ => return Err(bad()),
            }
        } else if line.starts_with("  ") && section == "container" {
            container.insert(key.into(), value);
        } else {
            section = "";
            if !matches!(key, "library_item_id" | "library_revision" | "schema_version" | "provider" | "resource_type" | "canonical_id" | "provider_instance" | "source_url" | "original_url" | "complete" | "source_revision" | "content_hash" | "generated") {
                fields.push(FrontmatterField { key: key.into(), value: serde_json::from_value(value.clone()).map_err(|_| bad())? });
            }
            values.insert(key.into(), value);
        }
    }
    let string = |key: &str| values.get(key).and_then(|v| v.as_str()).map(str::to_owned).ok_or_else(bad);
    let optional = |key: &str| string(key).map(|s| (!s.is_empty()).then_some(s));
    let asset = SourceAsset {
        source: SourceRef { provider_id: string("provider")?, provider_instance: string("provider_instance")?, resource_type: string("resource_type")?, canonical_id: string("canonical_id")? },
        title: entry.summary.title.clone(), source_url: optional("source_url")?, original_url: optional("original_url")?, source_revision: optional("source_revision")?,
        complete: values.get("complete").and_then(|v| v.as_bool()).ok_or_else(bad)?, diagnostics: entry.summary.diagnostics.clone(),
        body, fields, attachments,
        container: if container.is_empty() { None } else { Some(serde_json::from_value::<SourceContainer>(container.into()).map_err(|_| bad())?) },
    };
    if content_revision(&asset) != string("content_hash")? || super::item_id(&asset.source) != entry.summary.item_id { return Err(bad()); }
    Ok(asset)
}

#[cfg(test)]
mod tests;
