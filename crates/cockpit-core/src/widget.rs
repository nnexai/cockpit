use crate::{
    InspectionError,
    browser::{BrowserHerdrAdapter, BrowserHerdrSnapshot},
    paste_adapter::CommentPasteAdapter,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use cockpit_protocol::{comment_paste::CommentPasteTarget, widget::*};
use parking_lot::Mutex;
use sha2::{Digest, Sha256};
use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::broadcast;
use uuid::Uuid;

mod choices;
mod preflight;
mod store;
mod target;
#[cfg(test)]
mod tests;
mod text;
use store::{
    SNAPSHOT_ENVELOPE_BYTES, Store, TabKey, TabWidgets, Tombstone, Waiter, Widget,
    summary_snapshot_bytes,
};

pub struct WidgetService {
    adapter: Arc<dyn BrowserHerdrAdapter>,
    paste: Arc<dyn CommentPasteAdapter>,
    store: Arc<Mutex<Store>>,
    // Sequence fresh snapshot phases without blocking synchronous window operations.
    operation: tokio::sync::Mutex<()>,
    events: broadcast::Sender<WidgetEvent>,
    clock: Arc<dyn Fn() -> u64 + Send + Sync>,
}

pub struct WidgetSubscription {
    pub window_id: String,
    pub snapshot: WidgetEvent,
    pub events: broadcast::Receiver<WidgetEvent>,
    pub guard: WidgetWindowGuard,
}
pub struct WidgetWindowGuard {
    store: Arc<Mutex<Store>>,
    id: String,
}
impl Drop for WidgetWindowGuard {
    fn drop(&mut self) {
        self.store.lock().windows.remove(&self.id);
    }
}
struct WaitGuard {
    store: Arc<Mutex<Store>>,
    id: String,
}
impl Drop for WaitGuard {
    fn drop(&mut self) {
        self.store.lock().waiters.remove(&self.id);
    }
}

impl WidgetService {
    pub fn new(adapter: Arc<dyn BrowserHerdrAdapter>, paste: Arc<dyn CommentPasteAdapter>) -> Self {
        let (events, _) = broadcast::channel(256);
        Self {
            adapter,
            paste,
            store: Arc::new(Mutex::new(Store::default())),
            operation: tokio::sync::Mutex::new(()),
            events,
            clock: Arc::new(|| {
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis()
                    .min(u128::from(u64::MAX)) as u64
            }),
        }
    }
    #[cfg(test)]
    pub fn with_clock(mut self, clock: Arc<dyn Fn() -> u64 + Send + Sync>) -> Self {
        self.clock = clock;
        self
    }

    async fn fresh(
        &self,
        address: &WidgetAddress,
    ) -> Result<(BrowserHerdrSnapshot, Vec<CommentPasteTarget>), InspectionError> {
        let snapshot = self
            .adapter
            .browser_snapshot(&address.session_id)
            .await
            .map_err(target::adapter_error)?;
        if snapshot.snapshot.session_id != address.session_id {
            return Err(InspectionError::new(
                "widget_target_not_found",
                "Herdr snapshot belongs to another session",
            ));
        }
        if address
            .endpoint_path
            .as_ref()
            .is_some_and(|path| path != &snapshot.endpoint_path)
        {
            return Err(InspectionError::new(
                "widget_target_not_found",
                "Herdr endpoint differs from Cockpit's",
            ));
        }
        let paste = self
            .paste
            .comment_paste_targets(&address.session_id)
            .await
            .map_err(target::adapter_error)?;
        Ok((snapshot, paste))
    }

    pub async fn show(
        &self,
        request: WidgetShowRequest,
    ) -> Result<WidgetShowResponse, InspectionError> {
        validate_id(&request.id)?;
        let prepared = prepare(request.content)?;
        let _operation = self.operation.lock().await;
        let (snapshot, paste) = self.fresh(&request.address).await?;
        let mut store = self.store.lock();
        store.check_running()?;
        self.retire_stale(&mut store, &snapshot);
        let resolved = target::resolve(
            &store,
            &request.address,
            Some(&request.id),
            &snapshot,
            &paste,
        )?;
        let now = (self.clock)();
        let existing = store.tabs.get(&resolved.key);
        if existing.is_some_and(|tab| {
            tab.space_id != resolved.space_id
                && (tab
                    .live
                    .iter()
                    .any(|widget| widget.summary.key.id == request.id)
                    || tab.tombstones.iter().any(|stone| stone.id == request.id))
        }) {
            return Err(InspectionError::new(
                "widget_target_changed",
                "stored widget's tab has moved to another Space",
            ));
        }
        let live_index = existing.and_then(|tab| {
            tab.live
                .iter()
                .position(|widget| widget.summary.key.id == request.id)
        });
        let tomb_index = existing.and_then(|tab| {
            tab.tombstones
                .iter()
                .position(|stone| stone.id == request.id)
        });
        if let Some(index) = live_index {
            let widget = &existing.expect("live tab").live[index];
            check_owner(&widget.source_key, &resolved.source_key)?;
            if widget.summary.content.sha256 == prepared.facts.sha256
                && widget.summary.kind == prepared.kind
                && !(request.clear_selection && widget.selection.is_some())
            {
                return Ok(show_response(
                    &store,
                    &resolved,
                    &widget.summary,
                    WidgetShowResult::Unchanged,
                ));
            }
        }
        if let Some(index) = tomb_index {
            let stone = &existing.expect("tombstone tab").tombstones[index];
            check_owner(&stone.source_key, &resolved.source_key)?;
            if !request.reopen {
                return Err(InspectionError::new(
                    "widget_dismissed",
                    format!(
                        "{} was removed by the user at {} (revision {}) and was not restored. Continue in the terminal, or pass --reopen if the user asked to see it again.",
                        request.id,
                        timestamp(stone.removed_at_ms),
                        stone.revision
                    ),
                ));
            }
        }
        let old_bytes = live_index
            .map(|index| html_bytes(&existing.expect("live tab").live[index]))
            .unwrap_or(0);
        let old_snapshot_bytes = live_index
            .map(|index| existing.expect("live tab").live[index].snapshot_bytes)
            .unwrap_or(0);
        let new_bytes = body_html_bytes(&prepared.body);
        if store.html_bytes - old_bytes + new_bytes > WIDGET_MAX_TOTAL_HTML_BYTES {
            return Err(InspectionError::new(
                "widget_limit",
                "owner HTML byte limit reached",
            ));
        }
        if live_index.is_none() {
            if existing.is_some_and(|tab| tab.live.len() >= WIDGET_MAX_LIVE_PER_TAB) {
                return Err(InspectionError::new(
                    "widget_limit",
                    "tab already has eight live widgets",
                ));
            }
        }
        let title = request
            .title
            .as_deref()
            .map(|title| text::sanitize(title, 80))
            .filter(|title| !title.is_empty())
            .or(prepared.title)
            .unwrap_or_else(|| request.id.clone());
        let mut warnings = prepared.warnings;
        if resolved.source.is_none() {
            warnings.push("unattributed: no source pane was provided".into());
        }
        if !resolved.focused {
            warnings.push(format!("not_focused_tab: target tab {} is not Herdr's focused tab; the user will see only that tab's marker", resolved.key.tab));
        }
        warnings = warnings
            .into_iter()
            .map(|warning| text::sanitize(&warning, 512))
            .take(32)
            .collect();
        let (revision, created_seq, created_at, selection, selection_facts, result, change) =
            if let Some(index) = live_index {
                let old = &store.tabs[&resolved.key].live[index];
                let keep = !request.clear_selection;
                (
                    old.summary.revision + 1,
                    old.summary.created_seq,
                    old.summary.created_at_ms,
                    if keep { old.selection.clone() } else { None },
                    if keep {
                        old.summary.selection.clone()
                    } else {
                        None
                    },
                    WidgetShowResult::Replaced,
                    WidgetChange::Replaced,
                )
            } else {
                let created_seq = store.created_seq + 1;
                let revision = tomb_index
                    .map(|index| store.tabs[&resolved.key].tombstones[index].revision + 1)
                    .unwrap_or(1);
                (
                    revision,
                    created_seq,
                    now,
                    None,
                    None,
                    if tomb_index.is_some() {
                        WidgetShowResult::Reopened
                    } else {
                        WidgetShowResult::Opened
                    },
                    if tomb_index.is_some() {
                        WidgetChange::Reopened
                    } else {
                        WidgetChange::Opened
                    },
                )
            };
        let summary = WidgetSummary {
            key: resolved.key.widget_key(&request.id),
            space_id: resolved.space_id.clone(),
            title,
            revision,
            created_seq,
            kind: prepared.kind,
            presentation: prepared.presentation,
            content: prepared.facts,
            warnings,
            source: resolved
                .source
                .as_ref()
                .map(|source| source.summary.clone()),
            arrival: resolved.arrival.clone(),
            resolved_from: resolved.from.clone(),
            change,
            created_at_ms: created_at,
            updated_at_ms: now,
            selection: selection_facts,
        };
        let snapshot_bytes = summary_snapshot_bytes(&summary)?;
        if store.snapshot_bytes - old_snapshot_bytes + snapshot_bytes + SNAPSHOT_ENVELOPE_BYTES
            > WIDGET_MAX_SNAPSHOT_BYTES
        {
            return Err(InspectionError::new(
                "widget_limit",
                "owner widget snapshot byte limit reached",
            ));
        }
        if live_index.is_none() {
            store.arrivals.retain(|_, arrivals| {
                while arrivals
                    .front()
                    .is_some_and(|at| now.saturating_sub(*at) >= 60_000)
                {
                    arrivals.pop_front();
                }
                !arrivals.is_empty()
            });
            let arrivals = store
                .arrivals
                .entry(resolved.source_key.clone())
                .or_default();
            if arrivals.len() >= WIDGET_MAX_ARRIVALS_PER_MINUTE {
                return Err(InspectionError::new(
                    "widget_rate_limited",
                    "source already created or reopened six widgets in the last minute",
                ));
            }
            arrivals.push_back(now);
            store.created_seq = created_seq;
        }
        let response = show_response(&store, &resolved, &summary, result);
        let widget = Widget {
            summary: summary.clone(),
            body: prepared.body,
            source_key: resolved.source_key,
            fingerprint: resolved.source.and_then(|source| source.fingerprint),
            selection,
            snapshot_bytes,
        };
        let tab = store
            .tabs
            .entry(resolved.key)
            .or_insert_with(|| TabWidgets {
                space_id: resolved.space_id,
                endpoint_path: resolved.endpoint_path,
                live: Vec::new(),
                tombstones: Default::default(),
            });
        if let Some(index) = live_index {
            tab.live[index] = widget;
        } else {
            if let Some(index) = tomb_index {
                tab.tombstones.remove(index);
            }
            tab.live.push(widget);
        }
        store.html_bytes = store.html_bytes - old_bytes + new_bytes;
        store.snapshot_bytes = store.snapshot_bytes - old_snapshot_bytes + snapshot_bytes;
        store.emit_upsert(&self.events, summary);
        Ok(response)
    }

    pub async fn close(
        &self,
        request: WidgetCloseRequest,
    ) -> Result<WidgetCloseResponse, InspectionError> {
        validate_id(&request.id)?;
        let _operation = self.operation.lock().await;
        let (snapshot, paste) = self.fresh(&request.address).await?;
        let mut store = self.store.lock();
        store.check_running()?;
        self.retire_stale(&mut store, &snapshot);
        let resolved = target::resolve(
            &store,
            &request.address,
            Some(&request.id),
            &snapshot,
            &paste,
        )?;
        let Some(tab) = store.tabs.get_mut(&resolved.key) else {
            return Ok(WidgetCloseResponse {
                id: request.id,
                result: WidgetCloseResult::AlreadyRemoved,
            });
        };
        if let Some(index) = tab
            .live
            .iter()
            .position(|widget| widget.summary.key.id == request.id)
        {
            check_owner(&tab.live[index].source_key, &resolved.source_key)?;
            let widget = tab.live.remove(index);
            store.html_bytes -= html_bytes(&widget);
            store.snapshot_bytes -= widget.snapshot_bytes;
            store.finish_waiters(&resolved.key, &request.id, Err(not_found()));
            store.emit_remove(&self.events, widget.summary.key, WidgetRemovalReason::Agent);
            Ok(WidgetCloseResponse {
                id: request.id,
                result: WidgetCloseResult::Closed,
            })
        } else {
            if let Some(stone) = tab.tombstones.iter().find(|stone| stone.id == request.id) {
                check_owner(&stone.source_key, &resolved.source_key)?;
            }
            Ok(WidgetCloseResponse {
                id: request.id,
                result: WidgetCloseResult::AlreadyRemoved,
            })
        }
    }

    pub async fn list(
        &self,
        request: WidgetListRequest,
    ) -> Result<WidgetListResponse, InspectionError> {
        let _operation = self.operation.lock().await;
        let (snapshot, paste) = self.fresh(&request.address).await?;
        let mut store = self.store.lock();
        store.check_running()?;
        self.retire_stale(&mut store, &snapshot);
        let resolved = target::resolve(&store, &request.address, None, &snapshot, &paste)?;
        let mut widgets = Vec::new();
        if let Some(tab) = store.tabs.get(&resolved.key) {
            widgets.extend(
                tab.live
                    .iter()
                    .filter(|widget| widget.source_key == resolved.source_key)
                    .map(|widget| WidgetListEntry {
                        id: widget.summary.key.id.clone(),
                        state: WidgetListState::Live,
                        revision: widget.summary.revision,
                        presentation: Some(widget.summary.presentation.clone()),
                        displayed: Some(store.displayed(&resolved.key, &widget.summary.arrival)),
                        removed_at_ms: None,
                    }),
            );
            widgets.extend(
                tab.tombstones
                    .iter()
                    .filter(|stone| stone.source_key == resolved.source_key)
                    .map(|stone| WidgetListEntry {
                        id: stone.id.clone(),
                        state: WidgetListState::RemovedByUser,
                        revision: stone.revision,
                        presentation: None,
                        displayed: None,
                        removed_at_ms: Some(stone.removed_at_ms),
                    }),
            );
        }
        Ok(WidgetListResponse { widgets })
    }

    pub async fn selection(
        &self,
        request: WidgetSelectionRequest,
    ) -> Result<WidgetSelectionResponse, InspectionError> {
        validate_id(&request.id)?;
        if request
            .wait_seconds
            .is_some_and(|seconds| seconds > WIDGET_MAX_WAIT_SECONDS)
        {
            return Err(InspectionError::new(
                "widget_usage",
                "selection wait exceeds 3600 seconds",
            ));
        }
        let operation = self.operation.lock().await;
        let (snapshot, paste) = self.fresh(&request.address).await?;
        let (key, mut events, guard) = {
            let mut store = self.store.lock();
            store.check_running()?;
            self.retire_stale(&mut store, &snapshot);
            let resolved = target::resolve(
                &store,
                &request.address,
                Some(&request.id),
                &snapshot,
                &paste,
            )?;
            if let Some(response) = read_selection(
                &mut store,
                &resolved.key,
                &request.id,
                &resolved.source_key,
                request.wait_seconds.is_none(),
                (self.clock)(),
                &self.events,
            )? {
                return Ok(response);
            }
            if store.waiters.len() >= WIDGET_MAX_SELECTION_WAITERS {
                return Err(InspectionError::new(
                    "widget_busy",
                    "eight selection calls are already waiting",
                ));
            }
            let id = Uuid::new_v4().to_string();
            store.waiters.insert(
                id.clone(),
                Waiter {
                    key: resolved.key.clone(),
                    id: request.id.clone(),
                    outcome: None,
                },
            );
            (
                resolved.key,
                self.events.subscribe(),
                WaitGuard {
                    store: self.store.clone(),
                    id,
                },
            )
        };
        drop(operation);
        let wait = async {
            loop {
                let _ = events.recv().await;
                let mut store = self.store.lock();
                if let Some(outcome) = store
                    .waiters
                    .get(&guard.id)
                    .and_then(|waiter| waiter.outcome.clone())
                {
                    return outcome;
                }
                let owner = store
                    .tabs
                    .get(&key)
                    .and_then(|tab| {
                        tab.live
                            .iter()
                            .find(|widget| widget.summary.key.id == request.id)
                    })
                    .map(|widget| widget.source_key.clone())
                    .ok_or_else(not_found)?;
                if let Some(response) = read_selection(
                    &mut store,
                    &key,
                    &request.id,
                    &owner,
                    false,
                    (self.clock)(),
                    &self.events,
                )? {
                    return Ok(response);
                }
            }
        };
        match tokio::time::timeout(Duration::from_secs(request.wait_seconds.unwrap_or(0)), wait)
            .await
        {
            Ok(result) => result,
            Err(_) => {
                let store = self.store.lock();
                if let Some(outcome) = store
                    .waiters
                    .get(&guard.id)
                    .and_then(|waiter| waiter.outcome.clone())
                {
                    return outcome;
                }
                Ok(selection_status(
                    &request.id,
                    store
                        .tabs
                        .get(&key)
                        .and_then(|tab| {
                            tab.live
                                .iter()
                                .find(|widget| widget.summary.key.id == request.id)
                        })
                        .map(|widget| widget.summary.revision),
                    WidgetSelectionStatus::Timeout,
                ))
            }
        }
    }

    pub fn subscribe(&self) -> WidgetSubscription {
        let mut store = self.store.lock();
        let id = Uuid::new_v4().to_string();
        if !store.shutdown {
            store.windows.insert(
                id.clone(),
                WidgetWindowReport {
                    session_id: None,
                    displayed_tab_id: None,
                    blocker: None,
                },
            );
        }
        let events = self.events.subscribe();
        let mut widgets: Vec<_> = store
            .tabs
            .values()
            .flat_map(|tab| tab.live.iter().map(|widget| widget.summary.clone()))
            .collect();
        widgets.sort_by_key(|widget| widget.created_seq);
        WidgetSubscription {
            window_id: id.clone(),
            snapshot: WidgetEvent::Snapshot {
                sequence: store.sequence,
                widgets,
            },
            events,
            guard: WidgetWindowGuard {
                store: self.store.clone(),
                id,
            },
        }
    }
    pub fn report_window(
        &self,
        window_id: &str,
        report: WidgetWindowReport,
    ) -> Result<(), InspectionError> {
        if report.session_id.as_ref().is_some_and(|id| id.len() > 256)
            || report
                .displayed_tab_id
                .as_ref()
                .is_some_and(|id| id.len() > 256)
            || (report.displayed_tab_id.is_some() && report.session_id.is_none())
        {
            return Err(InspectionError::new(
                "widget_usage",
                "invalid window report",
            ));
        }
        let mut store = self.store.lock();
        store.check_running()?;
        let entry = store
            .windows
            .get_mut(window_id)
            .ok_or_else(|| InspectionError::new("widget_usage", "window subscription is absent"))?;
        *entry = report;
        Ok(())
    }
    pub fn content(&self, request: WidgetContentRequest) -> Result<WidgetContent, InspectionError> {
        let store = self.store.lock();
        store.check_running()?;
        let tab = store.locate(&request.key).ok_or_else(not_found)?;
        let widget = store.tabs[&tab]
            .live
            .iter()
            .find(|widget| widget.summary.key == request.key)
            .expect("located widget");
        if widget.summary.revision != request.revision {
            return Err(stale());
        }
        Ok(WidgetContent {
            key: request.key,
            revision: request.revision,
            sha256: widget.summary.content.sha256.clone(),
            body: widget.body.clone(),
            selection: widget.selection.clone(),
        })
    }
    pub fn remove(
        &self,
        request: WidgetRemoveRequest,
    ) -> Result<WidgetRemoveResponse, InspectionError> {
        let mut store = self.store.lock();
        store.check_running()?;
        let Some(key) = store.locate(&request.key) else {
            return Ok(WidgetRemoveResponse {
                result: WidgetRemoveResult::AlreadyRemoved,
            });
        };
        let tab = store.tabs.get_mut(&key).expect("located tab");
        let index = tab
            .live
            .iter()
            .position(|widget| widget.summary.key == request.key)
            .expect("located widget");
        let widget = tab.live.remove(index);
        let now = (self.clock)();
        tab.tombstones.push_back(Tombstone {
            id: request.key.id.clone(),
            revision: widget.summary.revision,
            removed_at_ms: now,
            source_key: widget.source_key.clone(),
        });
        if tab.tombstones.len() > WIDGET_MAX_TOMBSTONES_PER_TAB {
            tab.tombstones.pop_front();
        }
        store.html_bytes -= html_bytes(&widget);
        store.snapshot_bytes -= widget.snapshot_bytes;
        let mut dismissed = selection_status(
            &request.key.id,
            Some(widget.summary.revision),
            WidgetSelectionStatus::Dismissed,
        );
        dismissed.removed_at_ms = Some(now);
        store.finish_waiters(&key, &request.key.id, Ok(dismissed));
        store.emit_remove(&self.events, request.key, WidgetRemovalReason::User);
        Ok(WidgetRemoveResponse {
            result: WidgetRemoveResult::Removed,
        })
    }
    pub fn select(
        &self,
        request: WidgetSelectRequest,
    ) -> Result<WidgetSelectResponse, InspectionError> {
        let mut store = self.store.lock();
        store.check_running()?;
        let key = store.locate(&request.key).ok_or_else(not_found)?;
        let has_waiters = store.waiters.values().any(|waiter| {
            waiter.key == key && waiter.id == request.key.id && waiter.outcome.is_none()
        });
        let widget = store
            .tabs
            .get_mut(&key)
            .expect("located tab")
            .live
            .iter_mut()
            .find(|widget| widget.summary.key == request.key)
            .expect("located widget");
        if widget.summary.revision != request.revision {
            return Err(stale());
        }
        let value_json = match (&widget.body, request.value) {
            (WidgetBody::Choices { spec }, WidgetSelectValue::Choice { choice_id }) => {
                choices::value(spec, &choice_id)?
            }
            (WidgetBody::Html { .. }, WidgetSelectValue::Page { value_json }) => {
                if value_json.len() > WIDGET_MAX_SELECTION_BYTES {
                    return Err(InspectionError::new(
                        "widget_too_large",
                        "selection exceeds 16 KiB",
                    ));
                }
                let value: serde_json::Value = serde_json::from_str(&value_json).map_err(|_| {
                    InspectionError::new("widget_usage", "page selection is not valid finite JSON")
                })?;
                let compact = serde_json::to_string(&value).map_err(|_| {
                    InspectionError::new("widget_usage", "page selection is not valid finite JSON")
                })?;
                if compact.len() > WIDGET_MAX_SELECTION_BYTES {
                    return Err(InspectionError::new(
                        "widget_too_large",
                        "selection exceeds 16 KiB",
                    ));
                }
                compact
            }
            _ => {
                return Err(InspectionError::new(
                    "widget_selection_unavailable",
                    "selection kind does not match widget content",
                ));
            }
        };
        let now = (self.clock)();
        widget.selection = Some(WidgetSelectionResponse {
            id: request.key.id.clone(),
            revision: Some(request.revision),
            status: WidgetSelectionStatus::Selected,
            value_json: Some(value_json),
            at_ms: Some(now),
            removed_at_ms: None,
        });
        widget.summary.selection = Some(WidgetSelectionFacts {
            revision: request.revision,
            at_ms: now,
            read_at_ms: has_waiters.then_some(now),
        });
        widget.summary.change = WidgetChange::Updated;
        widget.summary.updated_at_ms = now;
        let summary = widget.summary.clone();
        let selected = widget.selection.clone().expect("stored selection");
        store.finish_waiters(&key, &request.key.id, Ok(selected));
        store.emit_upsert(&self.events, summary);
        Ok(WidgetSelectResponse { at_ms: now })
    }

    fn retire_stale(&self, store: &mut Store, snapshot: &BrowserHerdrSnapshot) {
        let keys: Vec<_> = store
            .tabs
            .iter()
            .filter(|(key, tab)| {
                key.session == snapshot.snapshot.session_id
                    && (key.endpoint != snapshot.endpoint_identity
                        || tab.endpoint_path != snapshot.endpoint_path
                        || !snapshot.snapshot.tabs.iter().any(|tab| tab.id == key.tab)
                        || !snapshot
                            .snapshot
                            .panes
                            .iter()
                            .any(|pane| pane.tab_id == key.tab))
            })
            .map(|(key, _)| key.clone())
            .collect();
        for key in keys {
            let tab = store.tabs.remove(&key).expect("known tab");
            for widget in tab.live {
                store.html_bytes -= html_bytes(&widget);
                store.snapshot_bytes -= widget.snapshot_bytes;
                store.finish_waiters(
                    &key,
                    &widget.summary.key.id,
                    Ok(selection_status(
                        &widget.summary.key.id,
                        Some(widget.summary.revision),
                        WidgetSelectionStatus::Retired,
                    )),
                );
                store.emit_remove(
                    &self.events,
                    widget.summary.key,
                    WidgetRemovalReason::Retired,
                );
            }
        }
    }

    pub async fn reconcile(&self) {
        let _operation = self.operation.lock().await;
        let sessions: std::collections::HashSet<_> = self
            .store
            .lock()
            .tabs
            .keys()
            .map(|key| key.session.clone())
            .collect();
        for session in sessions {
            let snapshot = match self.adapter.browser_snapshot(&session).await {
                Ok(snapshot) if snapshot.snapshot.session_id == session => snapshot,
                _ => continue,
            };
            let paste = self.paste.comment_paste_targets(&session).await.ok();
            let mut store = self.store.lock();
            if store.shutdown {
                return;
            }
            self.retire_stale(&mut store, &snapshot);
            let keys: Vec<_> = store
                .tabs
                .keys()
                .filter(|key| key.session == session)
                .cloned()
                .collect();
            for key in keys {
                let mut updates = Vec::new();
                for widget in &mut store.tabs.get_mut(&key).expect("known tab").live {
                    let Some(source) = &mut widget.summary.source else {
                        continue;
                    };
                    let pane = snapshot
                        .snapshot
                        .panes
                        .iter()
                        .find(|pane| pane.id == source.pane_id);
                    let status = if pane.is_none() {
                        WidgetSourceStatus::Closed
                    } else if pane.is_some_and(|pane| pane.terminal_id != source.terminal_id) {
                        WidgetSourceStatus::Restarted
                    } else if let Some(paste) = &paste {
                        let current = paste.iter().find(|target| {
                            target.pane_id == source.pane_id
                                && target.endpoint_identity == snapshot.endpoint_identity
                                && target.session_id == session
                        });
                        if widget.fingerprint.as_ref()
                            != current.map(|target| &target.agent_fingerprint)
                        {
                            WidgetSourceStatus::Restarted
                        } else {
                            WidgetSourceStatus::Present
                        }
                    } else {
                        continue;
                    };
                    if source.status != status {
                        source.status = status;
                        widget.summary.change = WidgetChange::Updated;
                        widget.summary.updated_at_ms = (self.clock)();
                        updates.push(widget.summary.clone());
                    }
                }
                for update in updates {
                    store.emit_upsert(&self.events, update);
                }
            }
        }
    }
    pub fn shutdown(&self) {
        let mut store = self.store.lock();
        if store.shutdown {
            return;
        }
        store.shutdown = true;
        for waiter in store.waiters.values_mut() {
            if waiter.outcome.is_none() {
                waiter.outcome = Some(Ok(selection_status(
                    &waiter.id,
                    None,
                    WidgetSelectionStatus::Retired,
                )));
            }
        }
        let tabs = std::mem::take(&mut store.tabs);
        store.html_bytes = 0;
        store.snapshot_bytes = 0;
        for tab in tabs.into_values() {
            for widget in tab.live {
                store.emit_remove(
                    &self.events,
                    widget.summary.key,
                    WidgetRemovalReason::Retired,
                );
            }
        }
    }
}

struct Prepared {
    body: WidgetBody,
    kind: WidgetKind,
    presentation: WidgetPresentation,
    facts: WidgetContentFacts,
    title: Option<String>,
    warnings: Vec<String>,
}
fn prepare(input: WidgetContentInput) -> Result<Prepared, InspectionError> {
    let (body, bytes, sha256, from, name, title, warnings, kind, presentation) = match input {
        WidgetContentInput::Html {
            content_base64,
            sha256,
            from,
            name,
        } => {
            if content_base64.len() > WIDGET_MAX_HTML_BYTES.div_ceil(3) * 4 {
                return Err(InspectionError::new(
                    "widget_too_large",
                    "HTML exceeds 1 MiB",
                ));
            }
            let bytes = STANDARD
                .decode(content_base64)
                .map_err(|_| InspectionError::new("widget_usage", "invalid HTML base64"))?;
            if bytes.len() > WIDGET_MAX_HTML_BYTES {
                return Err(InspectionError::new(
                    "widget_too_large",
                    "HTML exceeds 1 MiB",
                ));
            }
            check_sha(&bytes, &sha256)?;
            let html = std::str::from_utf8(&bytes)
                .map_err(|_| InspectionError::new("widget_usage", "HTML is not UTF-8"))?;
            let sanitized = preflight::sanitize(html)?;
            (
                WidgetBody::Html {
                    document: sanitized.document,
                },
                bytes.len(),
                sha256,
                from,
                name,
                sanitized.title,
                sanitized.warnings,
                WidgetKind::Html,
                WidgetPresentation::Active,
            )
        }
        WidgetContentInput::Choices {
            spec_json,
            sha256,
            from,
            name,
        } => {
            if spec_json.len() > WIDGET_MAX_CHOICES_BYTES {
                return Err(InspectionError::new(
                    "widget_too_large",
                    "choices exceed 64 KiB",
                ));
            }
            check_sha(spec_json.as_bytes(), &sha256)?;
            let spec = choices::parse(&spec_json)?;
            (
                WidgetBody::Choices { spec },
                spec_json.len(),
                sha256,
                from,
                name,
                None,
                Vec::new(),
                WidgetKind::Choices,
                WidgetPresentation::Choices,
            )
        }
    };
    Ok(Prepared {
        body,
        kind,
        presentation,
        facts: WidgetContentFacts {
            sha256,
            bytes: bytes as u64,
            from,
            name: name
                .map(|name| text::sanitize(name.rsplit(['/', '\\']).next().unwrap_or(""), 255)),
        },
        title,
        warnings,
    })
}
fn check_sha(bytes: &[u8], expected: &str) -> Result<(), InspectionError> {
    if format!("{:x}", Sha256::digest(bytes)) != expected {
        return Err(InspectionError::new(
            "widget_usage",
            "content SHA-256 does not match",
        ));
    }
    Ok(())
}
fn validate_id(id: &str) -> Result<(), InspectionError> {
    if text::valid_id(id) {
        Ok(())
    } else {
        Err(InspectionError::new(
            "widget_usage",
            "id must match [a-z0-9][a-z0-9_-]{0,47}",
        ))
    }
}
fn check_owner(actual: &str, expected: &str) -> Result<(), InspectionError> {
    if actual == expected {
        Ok(())
    } else {
        Err(InspectionError::new(
            "widget_not_owner",
            "widget belongs to another source",
        ))
    }
}
fn body_html_bytes(body: &WidgetBody) -> usize {
    match body {
        WidgetBody::Html { document } => document.len(),
        WidgetBody::Choices { .. } => 0,
    }
}
fn html_bytes(widget: &Widget) -> usize {
    body_html_bytes(&widget.body)
}
fn not_found() -> InspectionError {
    InspectionError::new("widget_target_not_found", "widget is absent")
}
fn stale() -> InspectionError {
    InspectionError::new(
        "widget_stale",
        "widget revision differs from the requested revision",
    )
}
fn selection_status(
    id: &str,
    revision: Option<u64>,
    status: WidgetSelectionStatus,
) -> WidgetSelectionResponse {
    WidgetSelectionResponse {
        id: id.into(),
        revision,
        status,
        value_json: None,
        at_ms: None,
        removed_at_ms: None,
    }
}
fn read_selection(
    store: &mut Store,
    key: &TabKey,
    id: &str,
    owner: &str,
    return_none: bool,
    now: u64,
    events: &broadcast::Sender<WidgetEvent>,
) -> Result<Option<WidgetSelectionResponse>, InspectionError> {
    let tab = store.tabs.get_mut(key).ok_or_else(not_found)?;
    if let Some(stone) = tab.tombstones.iter().find(|stone| stone.id == id) {
        check_owner(&stone.source_key, owner)?;
        let mut response =
            selection_status(id, Some(stone.revision), WidgetSelectionStatus::Dismissed);
        response.removed_at_ms = Some(stone.removed_at_ms);
        return Ok(Some(response));
    }
    let widget = tab
        .live
        .iter_mut()
        .find(|widget| widget.summary.key.id == id)
        .ok_or_else(not_found)?;
    check_owner(&widget.source_key, owner)?;
    if let Some(response) = widget.selection.clone() {
        if let Some(selection) = &mut widget.summary.selection {
            selection.read_at_ms = Some(now);
        }
        widget.summary.change = WidgetChange::Updated;
        widget.summary.updated_at_ms = now;
        let summary = widget.summary.clone();
        store.emit_upsert(events, summary);
        return Ok(Some(response));
    }
    Ok(if return_none {
        Some(selection_status(
            id,
            Some(widget.summary.revision),
            WidgetSelectionStatus::None,
        ))
    } else {
        None
    })
}
fn show_response(
    store: &Store,
    resolved: &target::Resolved,
    summary: &WidgetSummary,
    result: WidgetShowResult,
) -> WidgetShowResponse {
    WidgetShowResponse {
        id: summary.key.id.clone(),
        revision: summary.revision,
        result,
        displayed: store.displayed(&resolved.key, &resolved.arrival),
        location: resolved.location.clone(),
        presentation: summary.presentation.clone(),
        content: summary.content.clone(),
        source: resolved.source.as_ref().map(|source| WidgetSourceFacts {
            pane_id: source.summary.pane_id.clone(),
            tab_id: source.summary.tab_id.clone(),
            space_id: source.summary.space_id.clone(),
        }),
        target: WidgetTargetFacts {
            session_id: resolved.key.session.clone(),
            space_id: resolved.space_id.clone(),
            tab_id: resolved.key.tab.clone(),
            resolved_from: resolved.from.clone(),
        },
        warnings: summary.warnings.clone(),
    }
}

// Civil-date conversion keeps the dismissed message identical to the CLI's UTC-seconds output.
fn timestamp(ms: u64) -> String {
    let seconds = ms / 1000;
    let days = (seconds / 86400) as i64;
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = y + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        seconds / 3600 % 24,
        seconds / 60 % 60,
        seconds % 60
    )
}
