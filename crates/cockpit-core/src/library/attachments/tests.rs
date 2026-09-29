use super::*;
use crate::library::{tests::{self as base, finished}, space::tests::{companion_named, target}};
use crate::sources::{SourceProvider, SourceService, SourceFetchRequest, ProviderResolution, ConfluencePage, DownloadedAttachment, SpacePageListing, SpacePage, confluence_page_url};
use cockpit_protocol::{projects::ProjectProvider, sources::SourceCapability};
use std::sync::{Mutex, atomic::{AtomicUsize, Ordering}};

const SITE: &str = "https://acme.atlassian.net/wiki";
static NEXT_PAGE_ID: AtomicUsize = AtomicUsize::new(100_000);
#[derive(Clone, Copy)]
enum Behavior {
    Normal, Escape, Symlink, WrongId, WrongDestination, Oversize, Fail,
    #[cfg(unix)]
    ChunkedWriter,
    #[cfg(unix)]
    ExtraFileWriter,
    #[cfg(unix)]
    WaitingChild,
}
struct Provider {
    page_id: String,
    attachments: Mutex<Vec<(SourceAttachment, Vec<u8>)>>,
    behavior: Mutex<Behavior>,
    calls: AtomicUsize,
    version: AtomicUsize,
    entered: tokio::sync::Notify,
    release: tokio::sync::Semaphore,
    block: std::sync::atomic::AtomicBool,
    #[cfg(unix)]
    child_pid: AtomicUsize,
    #[cfg(unix)]
    descendant_pid: AtomicUsize,
}
#[async_trait::async_trait]
impl SourceProvider for Provider {
    fn provider_id(&self) -> &str { "confluence" }
    fn capabilities(&self) -> Vec<SourceCapability> { vec![] }
    async fn resolve_input(&self, input: &str) -> Result<ProviderResolution, InspectionError> {
        if input == "SD" { return Ok(ProviderResolution::ConfluenceSpace { space_key: "SD".into() }); }
        Ok(ProviderResolution::ConfluencePage(ConfluencePage {
            page_id: self.page_id.clone(), space_key: "SD".into(), title: "Release checklist".into(),
            version: Some(self.version.load(Ordering::SeqCst) as u64), source_url: format!("{SITE}/spaces/SD/pages/{}/Release", self.page_id), canonical_url: confluence_page_url(SITE, &self.page_id),
        }))
    }
    async fn fetch(&self, _: &SourceFetchRequest) -> Result<Vec<SourceAsset>, InspectionError> {
        let version = self.version.load(Ordering::SeqCst).to_string();
        Ok(vec![SourceAsset {
            source: SourceRef { provider_id: "confluence".into(), provider_instance: SITE.into(), resource_type: "page".into(), canonical_id: self.page_id.clone() },
            title: "Release checklist".into(), source_url: Some(format!("{SITE}/spaces/SD/pages/{}/Release", self.page_id)), original_url: None, source_revision: Some(version.clone()), complete: true,
            body: format!("Body version {version}"), fields: vec![], diagnostics: vec![], container: Some(SourceContainer { id: "SD".into(), label: "Software Development".into() }),
            attachments: self.attachments.lock().unwrap_or_else(|e| e.into_inner()).iter().map(|(a,_)| { let mut a = a.clone(); a.source_revision = Some(version.clone()); a }).collect(),
        }])
    }
    async fn list_space_pages(&self, _: &str, _: u32, _: &std::sync::atomic::AtomicBool) -> Result<SpacePageListing, InspectionError> {
        Ok(SpacePageListing { space_name: "Software Development".into(), homepage_id: Some(self.page_id.clone()), total: Some(1), complete: true,
            pages: vec![SpacePage { page_id: self.page_id.clone(), title: "Release checklist".into(), version: self.version.load(Ordering::SeqCst) as u64, ancestors: vec![], position: None }] })
    }
    async fn download_attachment(&self, _: &str, a: &AttachmentRef, _: &[AttachmentRef], dest: &Dir, dest_path: &Path, _budget: crate::process::StagingBudget) -> Result<DownloadedAttachment, InspectionError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert!(dest_path.is_absolute());
        assert!(dest_path.to_string_lossy().contains("/.cockpit/staging/"));
        assert!(dest_path.file_name().unwrap().to_string_lossy().starts_with("dl-"));
        assert_eq!(dest.entries().unwrap().count(), 0);
        #[cfg(unix)] { use cap_std::fs::MetadataExt; assert_eq!(dest.dir_metadata().unwrap().mode() & 0o777, 0o700); }
        if self.block.load(Ordering::SeqCst) {
            self.entered.notify_one();
            self.release.acquire().await.unwrap().forget();
        }
        let behavior = *self.behavior.lock().unwrap_or_else(|e| e.into_inner());
        #[cfg(unix)]
        if matches!(behavior, Behavior::ChunkedWriter | Behavior::ExtraFileWriter | Behavior::WaitingChild) {
            self.run_child(dest, dest_path, _budget, behavior).await?;
        }
        let bytes = self.attachments.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|(p,_)| p.id == a.id).unwrap().1.clone();
        match behavior {
            Behavior::Escape => return Ok(DownloadedAttachment { attachment_id: a.id.clone(), file_name: "../evil".into() }),
            Behavior::Symlink => { #[cfg(unix)] std::os::unix::fs::symlink("/etc/passwd", dest_path.join("payload")).unwrap(); },
            Behavior::WrongDestination => {
                let replacement = dest_path.with_extension("moved");
                std::fs::rename(dest_path, &replacement).unwrap();
                std::fs::create_dir(dest_path).unwrap();
            }
            Behavior::Fail => return Err(error("source_fetch_failed", "download failed")),
            Behavior::Oversize => { dest.write("payload", [bytes.as_slice(), b"too much"].concat()).unwrap(); },
            _ => dest.write("payload", &bytes).unwrap(),
        }
        Ok(DownloadedAttachment { attachment_id: if matches!(behavior, Behavior::WrongId) { "wrong".into() } else { a.id.clone() }, file_name: "payload".into() })
    }
}

#[cfg(unix)]
impl Provider {
    async fn run_child(&self, dest: &Dir, dest_path: &Path, budget: crate::process::StagingBudget, behavior: Behavior) -> Result<(), InspectionError> {
        let pid_path = dest_path.with_extension("pids");
        let mut command = tokio::process::Command::new("/bin/sh");
        command.arg("-c").arg(r#"
            exec >/dev/null 2>&1
            printf a > "$1/payload"
            sleep 30 &
            worker=$!
            printf '%s %s' "$$" "$worker" > "$2"
            case "$3" in
                bytes)
                    printf a > "$1/sibling"
                    for chunk in 1 2 3 4 5; do
                        printf a >> "$1/payload"
                        sleep 0.02
                    done
                    ;;
                files) : > "$1/unpredicted" ;;
            esac
            wait "$worker"
        "#).arg("attachment-writer").arg(dest_path).arg(&pid_path).arg(match behavior {
            Behavior::ChunkedWriter => "bytes",
            Behavior::ExtraFileWriter => "files",
            _ => "wait",
        });
        let run = crate::process::run_bounded_staging_command(
            command, 1024, 1024, std::time::Duration::from_secs(30), "Attachment CLI", dest, budget,
        );
        tokio::pin!(run);
        // Record both owned processes even if the monitor wins before this
        // readiness poll; their pid file is outside the watched flat directory.
        let result = tokio::select! {
            result = &mut run => Some(result),
            _ = async {
                loop {
                    if read_child_pids(&pid_path).is_some() { break; }
                    tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                }
            } => None,
        };
        let (parent, child) = read_child_pids(&pid_path).expect("CLI wrote its pids");
        self.child_pid.store(parent as usize, Ordering::SeqCst);
        self.descendant_pid.store(child as usize, Ordering::SeqCst);
        self.entered.notify_one();
        let result = match result {
            Some(result) => result,
            None => run.await,
        };
        if matches!(behavior, Behavior::ChunkedWriter | Behavior::ExtraFileWriter) {
            assert_eq!(result.as_ref().unwrap_err().code, "source_attachment_size");
        }
        result?;
        Ok(())
    }
}

#[cfg(unix)]
fn read_child_pids(path: &Path) -> Option<(u32, u32)> {
    let value = std::fs::read_to_string(path).ok()?;
    let (parent, child) = value.split_once(' ')?;
    Some((parent.parse().ok()?, child.parse().ok()?))
}

#[cfg(unix)]
async fn assert_child_reaped(provider: &Provider) {
    use nix::{errno::Errno, sys::{signal::kill, wait::{waitpid, WaitPidFlag}}, unistd::Pid};
    let parent = provider.child_pid.load(Ordering::SeqCst) as i32;
    let child = provider.descendant_pid.load(Ordering::SeqCst);
    assert!(parent > 0 && child > 0, "CLI started");
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while kill(Pid::from_raw(parent), None) != Err(Errno::ESRCH) {
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
    }).await.expect("CLI was not killed and reaped");
    assert_eq!(waitpid(Pid::from_raw(parent), Some(WaitPidFlag::WNOHANG)), Err(Errno::ECHILD));
    // A killed grandchild may briefly be a zombie until the OS adopter reaps
    // it. It must never remain running after the download worker has stopped.
    #[cfg(target_os = "linux")]
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            match std::fs::read_to_string(format!("/proc/{child}/stat")) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
                Ok(stat) if stat.rsplit_once(") ").is_some_and(|(_, rest)| rest.starts_with("Z ") || rest.starts_with("X ")) => break,
                _ => tokio::time::sleep(std::time::Duration::from_millis(1)).await,
            }
        }
    }).await.expect("CLI descendant survived process-group cleanup");
}
fn attachment(id: &str, title: &str, bytes: &[u8]) -> (SourceAttachment, Vec<u8>) {
    (SourceAttachment { id: id.into(), title: title.into(), media_type: None, size: Some(bytes.len() as u64), source_url: None, source_revision: Some("1".into()), path: None, not_downloaded: Some("not_requested".into()) }, bytes.to_vec())
}
fn fixture(attachments: Vec<(SourceAttachment, Vec<u8>)>, file_cap: u64, page_cap: u64) -> (base::Fixture, Arc<Provider>) {
    let mut f = base::fixture();
    let mut config = f.service.configuration.clone();
    config.providers = vec![ProjectProvider { id: "confluence".into(), base_url: SITE.into(), executable: "confluence".into(), login: None }];
    config.limits.library_attachment_bytes = file_cap; config.limits.library_item_attachment_bytes = page_cap;
    let provider = Arc::new(Provider {
        page_id: NEXT_PAGE_ID.fetch_add(1, Ordering::Relaxed).to_string(),
        attachments: Mutex::new(attachments), behavior: Mutex::new(Behavior::Normal),
        calls: AtomicUsize::new(0), version: AtomicUsize::new(1),
        entered: tokio::sync::Notify::new(), release: tokio::sync::Semaphore::new(0),
        block: std::sync::atomic::AtomicBool::new(false),
        #[cfg(unix)]
        child_pid: AtomicUsize::new(0),
        #[cfg(unix)]
        descendant_pid: AtomicUsize::new(0),
    });
    f.service = LibraryService::new(config.clone(), Arc::new(SourceService::new(&config, vec![provider.clone()]).unwrap()));
    (f, provider)
}
fn add(download: bool, follow: bool) -> LibraryAddRequest {
    LibraryAddRequest { input: if follow { "SD" } else { "42" }.into(), provider_id: Some("confluence".into()), hydrate_references: false, follow, follow_mode: None, download_attachments: download, refresh_existing: false, label: None, target: None }
}
async fn item(service: &LibraryService) -> LibraryItemSummary { service.listing(None).await.unwrap().items.remove(0) }
async fn save(f: &base::Fixture) -> LibraryItemSummary {
    let op = finished(&f.service, f.service.start_add(add(false, false)).await.unwrap()).await;
    assert_eq!(op.phases[0].state, LibraryPhaseState::Done, "{op:?}"); item(&f.service).await
}
async fn action(service: &LibraryService, old: &LibraryItemSummary, ids: &[&str], action: LibraryAttachmentAction) -> LibraryOperation {
    finished(service, service.start_attachments(LibraryAttachmentRequest { item_id: old.item_id.clone(), attachment_ids: ids.iter().map(|s| (*s).into()).collect(), action }).await.unwrap()).await
}
fn document(f: &base::Fixture, item: &LibraryItemSummary) -> Vec<u8> { std::fs::read(Path::new(&f.service.configuration.library_root).join(item.document_path.as_ref().unwrap())).unwrap() }
fn bytes(f: &base::Fixture, item: &LibraryItemSummary, index: usize) -> Vec<u8> { std::fs::read(Path::new(&f.service.configuration.library_root).join(&item.item_path).join(item.attachments[index].relative_path.as_ref().unwrap())).unwrap() }
fn staging_empty(f: &base::Fixture) { assert_eq!(std::fs::read_dir(Path::new(&f.service.configuration.library_root).join(".cockpit/staging")).unwrap().count(), 0); }

#[tokio::test]
async fn default_no_download_then_safe_names_explicit_bytes_and_offline_removal() {
    let names = ["../x", "a/b.png", "con", "-rf.png", ".hidden", " pad.txt ", "Report.PDF", "report.pdf"];
    let assets = names.iter().enumerate().map(|(n,s)| attachment(&n.to_string(), s, format!("bytes-{n}").as_bytes())).collect();
    let (f,p) = fixture(assets, 1024, 10000);
    let old = save(&f).await;
    assert_eq!(p.calls.load(Ordering::SeqCst), 0);
    let ids: Vec<_> = old.attachments.iter().map(|a| a.attachment_id.as_str()).collect();
    let op = action(&f.service, &old, &ids, LibraryAttachmentAction::Download).await;
    assert_eq!(op.phases[0].state, LibraryPhaseState::Done, "{op:?}");
    let downloaded = item(&f.service).await;
    let mut paths = BTreeSet::new();
    for (n,a) in downloaded.attachments.iter().enumerate() {
        assert_eq!(a.original_name, names[n]); assert!(single(&a.stored_name)); assert!(!a.stored_name.starts_with(['.', '-']));
        assert!(paths.insert(a.stored_name.to_ascii_lowercase()));
        assert_eq!(bytes(&f, &downloaded, n), format!("bytes-{n}").as_bytes());
    }
    assert_ne!(old.revision, downloaded.revision);
    assert!(String::from_utf8(document(&f, &downloaded)).unwrap().contains("Body version 1"));
    *p.behavior.lock().unwrap_or_else(|e| e.into_inner()) = Behavior::Fail;
    let removed = action(&f.service, &downloaded, &ids, LibraryAttachmentAction::RemoveDownloaded).await;
    assert_eq!(removed.phases[0].state, LibraryPhaseState::Done, "{removed:?}");
    let after = item(&f.service).await;
    assert_eq!(after.revision, old.revision);
    assert!(after.attachments.iter().all(|a| a.state == LibraryAttachmentState::NotDownloaded && a.relative_path.is_none()));
    assert_eq!(p.calls.load(Ordering::SeqCst), names.len()); staging_empty(&f);
}

#[tokio::test]
async fn unsafe_provider_outputs_keep_published_page_unchanged() {
    for behavior in [Behavior::Escape, Behavior::Symlink, Behavior::WrongId, Behavior::WrongDestination] {
        let (f,p) = fixture(vec![attachment("a", "pic.png", b"data")], 100, 100);
        let old = save(&f).await; let before = document(&f, &old);
        *p.behavior.lock().unwrap_or_else(|e| e.into_inner()) = behavior;
        let op = action(&f.service, &old, &["a"], LibraryAttachmentAction::Download).await;
        assert_eq!(op.phases[0].error.as_ref().unwrap().code, "source_capability_unavailable", "{op:?}");
        assert_eq!(item(&f.service).await.revision, old.revision); assert_eq!(document(&f, &old), before);
        assert!(!f.root.join("evil").exists()); staging_empty(&f);
    }
}

#[tokio::test]
async fn metadata_disk_and_page_caps_discard_or_skip_without_excess_calls() {
    let (f,p) = fixture(vec![attachment("a", "a.png", b"12345"), attachment("b", "b.png", b"12345"), attachment("c", "c.png", b"123456789")], 8, 8);
    let old = save(&f).await;
    let op = action(&f.service, &old, &["a", "b", "c"], LibraryAttachmentAction::Download).await;
    assert_eq!(op.phases[0].state, LibraryPhaseState::Partial);
    let after = item(&f.service).await;
    assert_eq!(after.attachments.iter().map(|a| a.state).collect::<Vec<_>>(), [LibraryAttachmentState::Downloaded, LibraryAttachmentState::OverLimit, LibraryAttachmentState::OverLimit]);
    assert_eq!(p.calls.load(Ordering::SeqCst), 1);
    assert_eq!(bytes(&f, &after, 0), b"12345"); staging_empty(&f);
    let (f,p) = fixture(vec![attachment("a", "a.png", b"12345")], 8, 100);
    let old = save(&f).await; *p.behavior.lock().unwrap_or_else(|e| e.into_inner()) = Behavior::Oversize;
    let op = action(&f.service, &old, &["a"], LibraryAttachmentAction::Download).await;
    assert_eq!(op.phases[0].state, LibraryPhaseState::Partial);
    let after = item(&f.service).await;
    assert_eq!(after.attachments[0].state, LibraryAttachmentState::Failed);
    assert!(after.attachments[0].relative_path.is_none()); staging_empty(&f);
}

#[tokio::test]
async fn predicted_overmatch_skips_cli_and_svg_is_not_media() {
    let (f,p) = fixture(vec![attachment("a", "x*y.png", b"12345"), attachment("b", "xzy.png", b"12345")], 8, 8);
    let old = save(&f).await;
    action(&f.service, &old, &["a"], LibraryAttachmentAction::Download).await;
    assert_eq!(p.calls.load(Ordering::SeqCst), 0); assert_eq!(item(&f.service).await.attachments[0].state, LibraryAttachmentState::Failed);
    let (f,_) = fixture(vec![attachment("a", "active.svg", b"<svg xmlns=\"http://www.w3.org/2000/svg\"><script>alert(1)</script></svg>")], 1024, 1024);
    let old = save(&f).await; action(&f.service, &old, &["a"], LibraryAttachmentAction::Download).await;
    let after = item(&f.service).await;
    let e = f.service.media(LibraryMediaRequest { path: format!("{}/{}", after.item_path, after.attachments[0].relative_path.as_ref().unwrap()), expected_revision: None }).await.unwrap_err();
    assert_eq!(e.code, "context_media_type_refused");
}

#[tokio::test]
async fn journal_failure_rolls_back_whole_attachment_revision() {
    let (f,_) = fixture(vec![attachment("a", "a.png", b"data")], 100, 100);
    let old = save(&f).await; let before = document(&f, &old);
    let store = f.service.open().unwrap();
    *store.fault.lock().unwrap_or_else(|e| e.into_inner()) = Some("journal");
    let op = action(&f.service, &old, &["a"], LibraryAttachmentAction::Download).await;
    assert_eq!(op.phases[0].state, LibraryPhaseState::Failed);
    let after = item(&f.service).await;
    assert_eq!(after.revision, old.revision); assert_eq!(document(&f, &after), before);
    assert!(after.attachments[0].relative_path.is_none()); staging_empty(&f);
}

#[tokio::test]
async fn follow_opt_in_is_saved_and_refresh_downloads_only_when_enabled() {
    let (f,p) = fixture(vec![attachment("a", "a.txt", b"data")], 100, 100);
    finished(&f.service, f.service.start_add(add(false, true)).await.unwrap()).await;
    assert_eq!(p.calls.load(Ordering::SeqCst), 0);
    finished(&f.service, f.service.start_add(add(true, true)).await.unwrap()).await;
    assert!(f.service.listing(None).await.unwrap().follows[0].include_attachments);
    assert_eq!(p.calls.load(Ordering::SeqCst), 1);
    p.version.store(2, Ordering::SeqCst);
    finished(&f.service, f.service.start_refresh(LibraryRefreshRequest::All).await.unwrap()).await;
    assert_eq!(p.calls.load(Ordering::SeqCst), 2);
    assert_eq!(item(&f.service).await.attachments[0].state, LibraryAttachmentState::Downloaded);
    finished(&f.service, f.service.start_add(add(false, true)).await.unwrap()).await;
    p.version.store(3, Ordering::SeqCst);
    finished(&f.service, f.service.start_refresh(LibraryRefreshRequest::All).await.unwrap()).await;
    assert!(!f.service.listing(None).await.unwrap().follows[0].include_attachments);
    assert_eq!(p.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn space_shared_layout_manifest_and_edited_obsolete_attachments() {
    let (f,_) = fixture(vec![attachment("a", "release-flow.png", b"first"), attachment("b", "edited.png", b"second")], 100, 100);
    let (projects, adapter, root) = companion_named(&f, "space").await;
    let service = f.service.clone().with_projects(projects, adapter);
    let old = save(&f).await;
    action(&service, &old, &["a", "b"], LibraryAttachmentAction::Download).await;
    let saved = item(&service).await;
    let add = finished(&service, service.start_space_add(SpaceAddRequest { target: target(), item_ids: vec![saved.item_id.clone()], follow_ids: vec![] }).await.unwrap()).await;
    assert_eq!(add.phases[0].state, LibraryPhaseState::Done, "{add:?}");
    let row = service.space_listing(target()).await.unwrap().rows.remove(0);
    let doc = row.paths.iter().find(|p| p.ends_with(".md")).unwrap();
    let parent = Path::new(doc).parent().unwrap();
    let a = parent.join("_files/release-flow.png"); let b = parent.join("_files/edited.png");
    assert_eq!(std::fs::read(root.join(doc)).unwrap(), document(&f, &saved));
    assert_eq!(std::fs::read(root.join(&a)).unwrap(), bytes(&f, &saved, 0));
    let manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("context-manifest.json")).unwrap()).unwrap();
    let file = manifest["entries"].as_array().unwrap().iter().find(|e| e["library_file"] == "_files/release-flow.png").unwrap();
    assert_eq!(file["content_hash"], store::hash(b"first"));
    assert!(String::from_utf8(document(&f, &saved)).unwrap().contains("path: \"_files/release-flow.png\""));
    action(&service, &saved, &["a", "b"], LibraryAttachmentAction::RemoveDownloaded).await;
    assert_ne!(item(&service).await.revision, saved.revision);
    assert_eq!(service.space_listing(target()).await.unwrap().rows[0].state, SpaceCopyState::LibraryNewer);
    std::fs::write(root.join(&b), b"edited locally").unwrap();
    let updated = finished(&service, service.start_space_update(SpaceUpdateRequest { target: target(), scope: SpaceUpdateScope::All {}, replace_edited: vec![] }).await.unwrap()).await;
    assert!(!root.join(&a).exists()); assert_eq!(std::fs::read(root.join(&b)).unwrap(), b"edited locally");
    assert_eq!(updated.space.unwrap().skipped_edited, vec![b.to_string_lossy().into_owned()]);
}

#[tokio::test]
async fn cancellation_discards_staged_download_without_changing_page() {
    let (f,p) = fixture(vec![attachment("a", "a.png", b"data")], 100, 100);
    let old = save(&f).await;
    let before = document(&f, &old);
    p.block.store(true, Ordering::SeqCst);
    let op = f.service.start_attachments(LibraryAttachmentRequest {
        item_id: old.item_id.clone(), attachment_ids: vec!["a".into()], action: LibraryAttachmentAction::Download,
    }).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), p.entered.notified()).await.unwrap();
    f.service.cancel(&op.operation_id).await.unwrap();
    let cancelled = finished(&f.service, op).await;
    assert_eq!(cancelled.phases[0].state, LibraryPhaseState::Cancelled);
    assert_eq!(item(&f.service).await.revision, old.revision);
    assert_eq!(document(&f, &old), before);
    staging_empty(&f);
}

#[cfg(unix)]
#[tokio::test]
async fn in_flight_staging_budget_kills_chunked_cli_and_removes_only_owned_staging() {
    // Two advertised matches total six bytes. The writer grows them to seven
    // bytes, with neither file exceeding the eight-byte per-file cap, then
    // stays alive. Both output pipes close before any excess bytes are written.
    let (f,p) = fixture(vec![attachment("a", "x*y.png", b"aaa"), attachment("b", "xzy.png", b"bbb")], 8, 8);
    let old = save(&f).await;
    *p.behavior.lock().unwrap_or_else(|e| e.into_inner()) = Behavior::ChunkedWriter;
    let staging = Path::new(&f.service.configuration.library_root).join(".cockpit/staging");
    let orphan = staging.join("unrelated-orphan");
    std::fs::create_dir(&orphan).unwrap();
    std::fs::write(orphan.join("keep"), b"unrelated").unwrap();
    let op = tokio::time::timeout(std::time::Duration::from_secs(5),
        action(&f.service, &old, &["a"], LibraryAttachmentAction::Download)
    ).await.expect("oversized CLI was allowed to continue");
    assert_eq!(op.phases[0].state, LibraryPhaseState::Partial, "{op:?}");
    let after = item(&f.service).await;
    assert_eq!(after.attachments[0].state, LibraryAttachmentState::Failed);
    assert!(after.attachments[0].relative_path.is_none());
    assert_child_reaped(&p).await;
    assert_eq!(std::fs::read(orphan.join("keep")).unwrap(), b"unrelated");
    assert_eq!(std::fs::read_dir(&staging).unwrap().count(), 1);
}

#[cfg(unix)]
#[tokio::test]
async fn unpredicted_file_kills_cli_before_completion() {
    let (f,p) = fixture(vec![attachment("a", "a.png", b"data")], 100, 100);
    let old = save(&f).await;
    *p.behavior.lock().unwrap_or_else(|e| e.into_inner()) = Behavior::ExtraFileWriter;
    let op = tokio::time::timeout(std::time::Duration::from_secs(5),
        action(&f.service, &old, &["a"], LibraryAttachmentAction::Download)
    ).await.expect("file count breach did not terminate the CLI");
    assert_eq!(op.phases[0].state, LibraryPhaseState::Partial, "{op:?}");
    assert_eq!(item(&f.service).await.attachments[0].state, LibraryAttachmentState::Failed);
    assert_child_reaped(&p).await;
    staging_empty(&f);
}

#[cfg(unix)]
#[tokio::test]
async fn cancellation_kills_running_cli_group_and_reaps_without_publication() {
    let (f,p) = fixture(vec![attachment("a", "a.png", b"data")], 100, 100);
    let old = save(&f).await;
    let before = document(&f, &old);
    *p.behavior.lock().unwrap_or_else(|e| e.into_inner()) = Behavior::WaitingChild;
    let op = f.service.start_attachments(LibraryAttachmentRequest {
        item_id: old.item_id.clone(), attachment_ids: vec!["a".into()], action: LibraryAttachmentAction::Download,
    }).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), p.entered.notified()).await.unwrap();
    f.service.cancel(&op.operation_id).await.unwrap();
    let cancelled = tokio::time::timeout(std::time::Duration::from_secs(5), finished(&f.service, op)).await.unwrap();
    assert_eq!(cancelled.phases[0].state, LibraryPhaseState::Cancelled);
    assert_child_reaped(&p).await;
    assert_eq!(item(&f.service).await.revision, old.revision);
    assert_eq!(document(&f, &old), before);
    staging_empty(&f);
}

#[tokio::test]
async fn failed_follow_attachment_replacement_keeps_previous_body_and_bytes() {
    let (f,p) = fixture(vec![attachment("a", "a.png", b"data")], 100, 100);
    finished(&f.service, f.service.start_add(add(true, true)).await.unwrap()).await;
    let old = item(&f.service).await;
    let before = document(&f, &old);
    *p.behavior.lock().unwrap_or_else(|e| e.into_inner()) = Behavior::Fail;
    p.version.store(2, Ordering::SeqCst);
    let op = finished(&f.service, f.service.start_refresh(LibraryRefreshRequest::All).await.unwrap()).await;
    assert_eq!(op.phases[0].state, LibraryPhaseState::Partial);
    assert_eq!(item(&f.service).await.revision, old.revision);
    assert_eq!(document(&f, &old), before);
    assert_eq!(bytes(&f, &old, 0), b"data");
    staging_empty(&f);
}

#[tokio::test]
async fn opted_in_follow_refresh_retries_failed_attachment_downloads() {
    let (f,p) = fixture(vec![attachment("a", "a.png", b"data")], 100, 100);
    *p.behavior.lock().unwrap_or_else(|e| e.into_inner()) = Behavior::Fail;
    let first = finished(&f.service, f.service.start_add(add(true, true)).await.unwrap()).await;
    assert_eq!(first.phases[0].state, LibraryPhaseState::Partial, "{first:?}");
    assert_eq!(item(&f.service).await.attachments[0].state, LibraryAttachmentState::Failed);
    assert_eq!(p.calls.load(Ordering::SeqCst), 1);

    *p.behavior.lock().unwrap_or_else(|e| e.into_inner()) = Behavior::Normal;
    let retry = finished(&f.service, f.service.start_refresh(LibraryRefreshRequest::All).await.unwrap()).await;
    assert_eq!(retry.phases[0].state, LibraryPhaseState::Done, "{retry:?}");
    let recovered = item(&f.service).await;
    assert_eq!(recovered.attachments[0].state, LibraryAttachmentState::Downloaded);
    assert_eq!(bytes(&f, &recovered, 0), b"data");
    assert_eq!(p.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn confirmed_replace_redownloads_locally_edited_attachments() {
    for edited in [b"edit".as_slice(), b"edited attachment".as_slice()] {
        let (f,p) = fixture(vec![attachment("a", "a.png", b"data")], 100, 100);
        let saved = save(&f).await;
        let downloaded = action(&f.service, &saved, &["a"], LibraryAttachmentAction::Download).await;
        assert_eq!(downloaded.phases[0].state, LibraryPhaseState::Done, "{downloaded:?}");
        let current = item(&f.service).await;
        let path = Path::new(&f.service.configuration.library_root)
            .join(&current.item_path)
            .join(current.attachments[0].relative_path.as_deref().unwrap());
        std::fs::write(&path, edited).unwrap();

        let conflict_op = finished(
            &f.service,
            f.service.start_refresh(LibraryRefreshRequest::Items { item_ids: vec![current.item_id.clone()] }).await.unwrap(),
        ).await;
        assert_eq!(conflict_op.report.unwrap().conflict, 1);
        let conflicted = item(&f.service).await;
        assert_eq!(conflicted.state, LibraryItemState::Conflict);
        assert!(conflicted.conflict.iter().any(|file| file.path == "_files/a.png"));

        let replaced = finished(
            &f.service,
            f.service.start_replace(LibraryReplaceRequest {
                item_id: current.item_id,
                confirmed: conflicted.conflict,
            }).await.unwrap(),
        ).await;
        assert_eq!(replaced.phases[0].state, LibraryPhaseState::Done, "{replaced:?}");
        let repaired = item(&f.service).await;
        assert_eq!(repaired.attachments[0].state, LibraryAttachmentState::Downloaded);
        assert_eq!(bytes(&f, &repaired, 0), b"data");
        assert_eq!(p.calls.load(Ordering::SeqCst), 2, "replacement must fetch remote attachment bytes");
    }
}

#[tokio::test]
async fn confirmed_replacement_reserves_only_the_new_attachment_bytes() {
    let (f, _) = fixture(vec![attachment("a", "a.png", b"123456")], 10, 10);
    let saved = save(&f).await;
    let downloaded = action(&f.service, &saved, &["a"], LibraryAttachmentAction::Download).await;
    assert_eq!(downloaded.phases[0].state, LibraryPhaseState::Done, "{downloaded:?}");
    let current = item(&f.service).await;
    let path = Path::new(&f.service.configuration.library_root)
        .join(&current.item_path)
        .join(current.attachments[0].relative_path.as_deref().unwrap());
    std::fs::write(&path, b"edited").unwrap();


    let conflict = finished(
        &f.service,
        f.service.start_refresh(LibraryRefreshRequest::Items { item_ids: vec![current.item_id.clone()] }).await.unwrap(),
    ).await;
    assert_eq!(conflict.report.unwrap().conflict, 1);
    let conflicted = item(&f.service).await;
    let replaced = finished(
        &f.service,
        f.service.start_replace(LibraryReplaceRequest {
            item_id: current.item_id,
            confirmed: conflicted.conflict,
        }).await.unwrap(),
    ).await;
    assert_eq!(replaced.phases[0].state, LibraryPhaseState::Done, "{replaced:?}");
    let repaired = item(&f.service).await;
    assert_eq!(repaired.attachments[0].state, LibraryAttachmentState::Downloaded);
    assert_eq!(bytes(&f, &repaired, 0), b"123456");
}

#[tokio::test]
async fn unknown_size_reserves_full_file_allowance_from_remaining_page_budget() {
    let mut unknown = attachment("b", "b.png", b"data");
    unknown.0.size = None;
    let (f,p) = fixture(vec![attachment("a", "a.png", b"abc"), unknown], 8, 10);
    let old = save(&f).await;
    let op = action(&f.service, &old, &["a", "b"], LibraryAttachmentAction::Download).await;
    assert_eq!(op.phases[0].state, LibraryPhaseState::Partial, "{op:?}");
    let after = item(&f.service).await;
    assert_eq!(after.attachments[0].state, LibraryAttachmentState::Downloaded);
    assert_eq!(bytes(&f, &after, 0), b"abc");
    assert_eq!(after.attachments[1].state, LibraryAttachmentState::Failed);
    assert!(after.attachments[1].relative_path.is_none());
    assert_eq!(p.calls.load(Ordering::SeqCst), 1, "unknown size must reserve eight bytes, not fit in the remaining seven");
    staging_empty(&f);
}
