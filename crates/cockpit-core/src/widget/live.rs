use super::*;

fn unavailable() -> InspectionError {
    InspectionError::new("widget_live_unavailable", "widget live feed is unavailable")
}

fn checked_json(value: &str, limit: usize) -> Result<(), InspectionError> {
    if value.len() > limit {
        return Err(InspectionError::new("widget_limit", "widget live JSON exceeds the byte limit"));
    }
    serde_json::from_str::<serde_json::Value>(value)
        .map_err(|_| InspectionError::new("widget_invalid_json", "widget live value must be valid JSON"))?;
    Ok(())
}

fn widget_mut<'a>(store: &'a mut Store, key: &WidgetKey, revision: u64) -> Result<&'a mut Widget, InspectionError> {
    store.check_running()?;
    let tab = store.locate(key).ok_or_else(unavailable)?;
    let widget = store.tabs.get_mut(&tab).expect("located tab").live.iter_mut()
        .find(|widget| widget.summary.key == *key).expect("located widget");
    if widget.summary.revision != revision { return Err(stale()); }
    Ok(widget)
}

const LIVE_LEASE_MS: u64 = 90_000;

fn check_binding(widget: &Widget, binding: &WidgetLiveBinding, credential_id: &str) -> Result<(), InspectionError> {
    match &widget.summary.live {
        Some(live) if live.status == WidgetLiveStatus::Live && live.binding == *binding
            && widget.live_credential_id.as_deref() == Some(credential_id)
            && widget.summary.source.as_ref().is_some_and(|source| source.status == WidgetSourceStatus::Present) => Ok(()),
        _ => Err(unavailable()),
    }
}

impl WidgetService {
    pub(super) fn expire_live_locked(&self, store: &mut Store) {
        let now = (self.clock)();
        let mut expired = Vec::new();
        for tab in store.tabs.values_mut() {
            for widget in &mut tab.live {
                if now.saturating_sub(widget.live_lease_at_ms) < LIVE_LEASE_MS { continue; }
                if let Some(live) = &mut widget.summary.live {
                    if live.status == WidgetLiveStatus::Live {
                        live.status = WidgetLiveStatus::Ended;
                        widget.live_credential_id = None;
                        widget.live_actions.clear();
                        expired.push(widget.summary.clone());
                    }
                }
            }
        }
        for summary in expired { store.emit_upsert(&self.events, summary); }
    }

    pub async fn resolve_live_key(&self, address: &WidgetAddress, id: &str, expected_source: &WidgetSourceSummary, expected_endpoint: &str) -> Result<WidgetKey, InspectionError> {
        validate_id(id)?;
        if address.source_pane_id.as_deref() != Some(expected_source.pane_id.as_str()) {
            return Err(unavailable());
        }
        let _operation = self.operation.lock().await;
        let (snapshot, paste) = self.fresh(address).await?;
        if snapshot.endpoint_identity != expected_endpoint { return Err(unavailable()); }
        let mut store = self.store.lock();
        store.check_running()?;
        self.retire_stale(&mut store, &snapshot);
        let resolved = target::resolve(&store, address, Some(id), &snapshot, &paste)?;
        let source = resolved.source.as_ref().ok_or_else(unavailable)?;
        if source.summary.pane_id != expected_source.pane_id
            || source.summary.tab_id != expected_source.tab_id
            || source.summary.space_id != expected_source.space_id
            || source.summary.terminal_id != expected_source.terminal_id
        {
            return Err(unavailable());
        }
        let widget = store.tabs.get(&resolved.key)
            .and_then(|tab| tab.live.iter().find(|widget| widget.summary.key.id == id))
            .ok_or_else(unavailable)?;
        check_owner(&widget.source_key, &resolved.source_key)?;
        Ok(resolved.key.widget_key(id))
    }

    // The caller must authenticate this binding before passing it to the service.
    pub fn attach_live(&self, key: &WidgetKey, revision: u64, binding: WidgetLiveBinding, credential_id: String) -> Result<(), InspectionError> {
        self.update_live(key, revision, true, |widget| {
            if !widget.summary.source.as_ref().is_some_and(|source| source.status == WidgetSourceStatus::Present) {
                return Err(unavailable());
            }
            if widget.summary.live.as_ref().is_some_and(|live| live.status == WidgetLiveStatus::Live) {
                return Err(unavailable());
            }
            widget.summary.live = Some(WidgetLive { binding, state: None, status: WidgetLiveStatus::Live });
            widget.live_credential_id = Some(credential_id);
            widget.live_lease_at_ms = (self.clock)();
            Ok(())
        })
    }

    pub fn publish_live(&self, key: &WidgetKey, revision: u64, binding: &WidgetLiveBinding, credential_id: &str, value_json: String) -> Result<(), InspectionError> {
        checked_json(&value_json, WIDGET_MAX_LIVE_STATE_BYTES)?;
        let now = (self.clock)();
        self.update_live(key, revision, false, |widget| {
            check_binding(widget, binding, credential_id)?;
            let live = widget.summary.live.as_mut().expect("checked live");
            let version = live.state.as_ref().map_or(Some(1), |state| state.version.checked_add(1))
                .ok_or_else(unavailable)?;
            live.state = Some(WidgetLiveState { version, value_json, updated_at_ms: now });
            widget.live_lease_at_ms = now;
            Ok(())
        })
    }

    pub fn end_live(&self, key: &WidgetKey, revision: u64, binding: &WidgetLiveBinding, credential_id: &str) -> Result<(), InspectionError> {
        self.update_live(key, revision, true, |widget| {
            check_binding(widget, binding, credential_id)?;
            widget.summary.live.as_mut().expect("checked live").status = WidgetLiveStatus::Ended;
            widget.live_credential_id = None;
            Ok(())
        })
    }

    fn update_live(&self, key: &WidgetKey, revision: u64, clear_actions: bool, update: impl FnOnce(&mut Widget) -> Result<(), InspectionError>) -> Result<(), InspectionError> {
        let mut store = self.store.lock();
        self.expire_live_locked(&mut store);
        let widget = widget_mut(&mut store, key, revision)?;
        let old_summary = widget.summary.clone();
        let old_credential = widget.live_credential_id.clone();
        let old_lease = widget.live_lease_at_ms;
        let old_size = summary_snapshot_bytes(&old_summary)?;
        update(widget)?;
        let new_size = match summary_snapshot_bytes(&widget.summary) {
            Ok(size) => size,
            Err(err) => { widget.summary = old_summary; widget.live_credential_id = old_credential; widget.live_lease_at_ms = old_lease; return Err(err); }
        };
        let old_widget_bytes = widget.snapshot_bytes;
        let new_widget_bytes = old_widget_bytes - old_size + new_size;
        let total = store.snapshot_bytes - old_widget_bytes + new_widget_bytes;
        if total + SNAPSHOT_ENVELOPE_BYTES > WIDGET_MAX_SNAPSHOT_BYTES {
            let widget = widget_mut(&mut store, key, revision)?;
            widget.summary = old_summary;
            widget.live_credential_id = old_credential;
            widget.live_lease_at_ms = old_lease;
            return Err(InspectionError::new("widget_limit", "owner widget snapshot byte limit reached"));
        }
        let widget = widget_mut(&mut store, key, revision)?;
        widget.snapshot_bytes = new_widget_bytes;
        let summary = widget.summary.clone();
        if clear_actions { widget.live_actions.clear(); }
        store.snapshot_bytes = total;
        store.emit_upsert(&self.events, summary);
        Ok(())
    }

    pub fn live_binding(&self, key: &WidgetKey, revision: u64) -> Result<(WidgetLiveBinding, String), InspectionError> {
        let mut store = self.store.lock();
        self.expire_live_locked(&mut store);
        let widget = widget_mut(&mut store, key, revision)?;
        match &widget.summary.live {
            Some(live) if live.status == WidgetLiveStatus::Live
                && widget.summary.source.as_ref().is_some_and(|source| source.status == WidgetSourceStatus::Present) =>
                Ok((live.binding.clone(), widget.live_credential_id.clone().ok_or_else(unavailable)?)),
            _ => Err(unavailable()),
        }
    }

    pub fn bound_send_live_action(&self, request: WidgetLiveActionRequest, binding: &WidgetLiveBinding, credential_id: &str) -> Result<WidgetLiveActionResponse, InspectionError> {
        checked_json(&request.value_json, WIDGET_MAX_LIVE_ACTION_BYTES)?;
        let mut store = self.store.lock();
        self.expire_live_locked(&mut store);
        let widget = widget_mut(&mut store, &request.key, request.revision)?;
        check_binding(widget, binding, credential_id)?;
        if request.binding != *binding || widget.summary.content.sha256 != request.content_sha256 {
            return Err(unavailable());
        }
        if widget.live_actions.len() >= WIDGET_MAX_LIVE_ACTIONS {
            return Err(InspectionError::new("widget_live_queue_full", "widget live action queue is full"));
        }
        widget.live_actions.push_back(WidgetLiveAction { value_json: request.value_json });
        let _ = self.live_actions.send(());
        Ok(WidgetLiveActionResponse {})
    }

    pub async fn poll_live_actions(&self, key: &WidgetKey, revision: u64, binding: &WidgetLiveBinding, credential_id: &str, timeout: Duration) -> Result<Vec<WidgetLiveAction>, InspectionError> {
        let mut actions = self.live_actions.subscribe();
        let mut events = self.events.subscribe();
        let deadline = tokio::time::Instant::now() + timeout.min(Duration::from_secs(30));
        let mut renewed = false;
        loop {
            {
                let mut store = self.store.lock();
                self.expire_live_locked(&mut store);
                let widget = widget_mut(&mut store, key, revision)?;
                check_binding(widget, binding, credential_id)?;
                if !renewed {
                    widget.live_lease_at_ms = (self.clock)();
                    renewed = true;
                }
                if !widget.live_actions.is_empty() { return Ok(widget.live_actions.drain(..).collect()); }
            }
            tokio::select! {
                _ = tokio::time::sleep_until(deadline) => {
                    let mut store = self.store.lock();
                    self.expire_live_locked(&mut store);
                    check_binding(widget_mut(&mut store, key, revision)?, binding, credential_id)?;
                    return Ok(Vec::new());
                },
                _ = actions.recv() => {},
                _ = events.recv() => {},
            }
        }
    }
}
