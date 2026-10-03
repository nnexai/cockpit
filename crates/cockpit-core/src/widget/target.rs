use super::{
    store::{SourceRecord, Store, TabKey},
    text,
};
use crate::{InspectionError, browser::BrowserHerdrSnapshot};
use cockpit_protocol::{comment_paste::CommentPasteTarget, widget::*};

pub(super) struct Resolved {
    pub key: TabKey,
    pub endpoint_path: String,
    pub space_id: String,
    pub location: String,
    pub source: Option<SourceRecord>,
    pub source_key: String,
    pub arrival: WidgetArrival,
    pub from: WidgetResolvedFrom,
    pub focused: bool,
}

pub(super) fn adapter_error(error: InspectionError) -> InspectionError {
    if error.code.starts_with("widget_") {
        error
    } else {
        InspectionError::new("widget_herdr_unavailable", error.message)
    }
}

pub(super) fn resolve(
    store: &Store,
    address: &WidgetAddress,
    id: Option<&str>,
    snapshot: &BrowserHerdrSnapshot,
    paste: &[CommentPasteTarget],
) -> Result<Resolved, InspectionError> {
    let missing = |message| InspectionError::new("widget_target_not_found", message);
    if snapshot.snapshot.session_id != address.session_id {
        return Err(missing("Herdr snapshot belongs to another session"));
    }
    if address
        .endpoint_path
        .as_ref()
        .is_some_and(|path| path != &snapshot.endpoint_path)
    {
        return Err(missing("Herdr endpoint differs from Cockpit's"));
    }
    let source = address
        .source_pane_id
        .as_ref()
        .map(|id| {
            let pane = snapshot
                .snapshot
                .panes
                .iter()
                .find(|pane| pane.id == *id)
                .ok_or_else(|| missing("source pane is absent"))?;
            let tab = snapshot
                .snapshot
                .tabs
                .iter()
                .find(|tab| tab.id == pane.tab_id)
                .ok_or_else(|| missing("source tab is absent"))?;
            let agent = paste.iter().find(|target| target.pane_id == pane.id);
            if agent.is_some_and(|agent| {
                agent.endpoint_identity != snapshot.endpoint_identity
                    || agent.session_id != address.session_id
                    || agent.terminal_id != pane.terminal_id
                    || agent.tab_id != pane.tab_id
                    || agent.workspace_id != tab.space_id
            }) {
                return Err(InspectionError::new(
                    "widget_herdr_unavailable",
                    "source identity changed between fresh Herdr snapshots",
                ));
            }
            Ok(SourceRecord {
                summary: WidgetSourceSummary {
                    pane_id: pane.id.clone(),
                    tab_id: pane.tab_id.clone(),
                    space_id: tab.space_id.clone(),
                    terminal_id: pane.terminal_id.clone(),
                    agent_label: agent.map(|agent| text::sanitize(&agent.agent_label, 80)),
                    fingerprint_prefix: agent.and_then(|agent| {
                        let digest = agent.agent_fingerprint.strip_prefix("sha256:")?;
                        if digest.len() != 64
                            || !digest
                                .bytes()
                                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                        {
                            return None;
                        }
                        digest.get(..12).map(str::to_owned)
                    }),
                    status: WidgetSourceStatus::Present,
                },
                fingerprint: agent.map(|agent| agent.agent_fingerprint.clone()),
            })
        })
        .transpose()?;
    let source_key = source
        .as_ref()
        .map(SourceRecord::key)
        .unwrap_or_else(|| "unattributed".into());
    let (tab_id, from) = match &address.locator {
        WidgetLocator::CurrentPane => (
            source
                .as_ref()
                .ok_or_else(|| missing("current pane requires a source pane"))?
                .summary
                .tab_id
                .clone(),
            WidgetResolvedFrom::CurrentPane,
        ),
        WidgetLocator::Pane { pane_id } => (
            snapshot
                .snapshot
                .panes
                .iter()
                .find(|pane| pane.id == *pane_id)
                .ok_or_else(|| missing("target pane is absent"))?
                .tab_id
                .clone(),
            WidgetResolvedFrom::Pane,
        ),
        WidgetLocator::Tab { tab_id } => (tab_id.clone(), WidgetResolvedFrom::Tab),
        WidgetLocator::Space { space_id } => {
            if !snapshot
                .snapshot
                .spaces
                .iter()
                .any(|space| space.id == *space_id)
            {
                return Err(missing("target Space is absent"));
            }
            let stored = id.and_then(|id| {
                store
                    .tabs
                    .iter()
                    .find(|(key, widgets)| {
                        key.endpoint == snapshot.endpoint_identity
                            && key.session == address.session_id
                            && widgets.space_id == *space_id
                            && (widgets.live.iter().any(|widget| {
                                widget.summary.key.id == id && widget.source_key == source_key
                            }) || widgets.tombstones.iter().any(|tombstone| {
                                tombstone.id == id && tombstone.source_key == source_key
                            }))
                    })
                    .map(|(key, _)| key.tab.clone())
            });
            if let Some(tab) = stored {
                (tab, WidgetResolvedFrom::Stored)
            } else {
                let mut tabs = snapshot
                    .snapshot
                    .tabs
                    .iter()
                    .filter(|tab| tab.space_id == *space_id && tab.focused);
                let tab = tabs.next().ok_or_else(|| {
                    InspectionError::new(
                        "widget_target_no_focused_tab",
                        "Space has no uniquely focused tab",
                    )
                })?;
                if tabs.next().is_some() {
                    return Err(InspectionError::new(
                        "widget_target_no_focused_tab",
                        "Space has no uniquely focused tab",
                    ));
                }
                (tab.id.clone(), WidgetResolvedFrom::SpaceFocusedTab)
            }
        }
    };
    let tab = snapshot
        .snapshot
        .tabs
        .iter()
        .find(|tab| tab.id == tab_id)
        .ok_or_else(|| missing("target tab is absent"))?;
    if !snapshot
        .snapshot
        .panes
        .iter()
        .any(|pane| pane.tab_id == tab_id)
    {
        return Err(missing("target tab has no panes"));
    }
    let space = snapshot
        .snapshot
        .spaces
        .iter()
        .find(|space| space.id == tab.space_id)
        .ok_or_else(|| missing("target Space is absent"))?;
    if address
        .space_check
        .as_ref()
        .is_some_and(|id| id != &tab.space_id)
    {
        return Err(InspectionError::new(
            "widget_target_mismatch",
            "target tab is not in the requested Space",
        ));
    }
    let arrival = if source
        .as_ref()
        .is_some_and(|source| source.summary.tab_id == tab_id)
    {
        WidgetArrival::OwnTab
    } else {
        WidgetArrival::CrossSource
    };
    Ok(Resolved {
        key: TabKey {
            endpoint: snapshot.endpoint_identity.clone(),
            session: address.session_id.clone(),
            tab: tab_id,
        },
        endpoint_path: snapshot.endpoint_path.clone(),
        space_id: tab.space_id.clone(),
        location: format!(
            "Space {} · tab {}",
            text::sanitize(&space.label, 80),
            tab.number
        ),
        source,
        source_key,
        arrival,
        from,
        focused: tab.focused,
    })
}
