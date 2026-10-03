use cockpit_protocol::widget::*;
use std::collections::{HashMap, VecDeque};
use tokio::sync::broadcast;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct TabKey {
    pub endpoint: String,
    pub session: String,
    pub tab: String,
}
impl TabKey {
    pub fn widget_key(&self, id: &str) -> WidgetKey {
        WidgetKey {
            session_id: self.session.clone(),
            tab_id: self.tab.clone(),
            id: id.into(),
        }
    }
}

pub(super) struct SourceRecord {
    pub summary: WidgetSourceSummary,
    pub fingerprint: Option<String>,
}
impl SourceRecord {
    pub fn key(&self) -> String {
        self.fingerprint
            .clone()
            .unwrap_or_else(|| format!("pane:{}", self.summary.pane_id))
    }
}

pub(super) struct Widget {
    pub summary: WidgetSummary,
    pub body: WidgetBody,
    pub source_key: String,
    pub fingerprint: Option<String>,
    pub selection: Option<WidgetSelectionResponse>,
    // Includes comma and headroom for future selection facts, numeric widths,
    // source status and change updates, so reads/selects never grow the budget.
    pub snapshot_bytes: usize,
}

pub(super) struct Tombstone {
    pub id: String,
    pub revision: u64,
    pub removed_at_ms: u64,
    pub source_key: String,
}

pub(super) struct TabWidgets {
    pub space_id: String,
    pub endpoint_path: String,
    pub live: Vec<Widget>,
    pub tombstones: VecDeque<Tombstone>,
}

pub(super) struct Waiter {
    pub key: TabKey,
    pub id: String,
    pub outcome: Option<Result<WidgetSelectionResponse, crate::InspectionError>>,
}

#[derive(Default)]
pub(super) struct Store {
    pub tabs: HashMap<TabKey, TabWidgets>,
    pub windows: HashMap<String, WidgetWindowReport>,
    pub arrivals: HashMap<String, VecDeque<u64>>,
    pub waiters: HashMap<String, Waiter>,
    pub sequence: u64,
    pub created_seq: u64,
    pub html_bytes: usize,
    pub snapshot_bytes: usize,
    pub shutdown: bool,
}
impl Store {
    pub fn emit_upsert(&mut self, events: &broadcast::Sender<WidgetEvent>, summary: WidgetSummary) {
        self.sequence += 1;
        let _ = events.send(WidgetEvent::Upserted {
            sequence: self.sequence,
            widget: summary,
        });
    }
    pub fn emit_remove(
        &mut self,
        events: &broadcast::Sender<WidgetEvent>,
        key: WidgetKey,
        reason: WidgetRemovalReason,
    ) {
        self.sequence += 1;
        let _ = events.send(WidgetEvent::Removed {
            sequence: self.sequence,
            key,
            reason,
        });
    }
    pub fn finish_waiters(
        &mut self,
        key: &TabKey,
        id: &str,
        outcome: Result<WidgetSelectionResponse, crate::InspectionError>,
    ) {
        for waiter in self
            .waiters
            .values_mut()
            .filter(|waiter| waiter.key == *key && waiter.id == id)
        {
            if waiter.outcome.is_none() {
                waiter.outcome = Some(outcome.clone());
            }
        }
    }
    pub fn displayed(&self, key: &TabKey, arrival: &WidgetArrival) -> WidgetDisplayed {
        if self.windows.is_empty() {
            return WidgetDisplayed::NoWindow;
        }
        if *arrival == WidgetArrival::CrossSource {
            return WidgetDisplayed::WhenOpened;
        }
        let displayed = self.windows.values().filter(|report| {
            report.session_id.as_deref() == Some(&key.session)
                && report.displayed_tab_id.as_deref() == Some(&key.tab)
        });
        let mut blocked = false;
        for report in displayed {
            if report.blocker.is_none() {
                return WidgetDisplayed::Now;
            }
            blocked = true;
        }
        if blocked {
            WidgetDisplayed::WhenVisible
        } else {
            WidgetDisplayed::WhenTabSelected
        }
    }
    pub fn locate(&self, key: &WidgetKey) -> Option<TabKey> {
        self.tabs
            .iter()
            .find(|(tab, widgets)| {
                tab.session == key.session_id
                    && tab.tab == key.tab_id
                    && widgets
                        .live
                        .iter()
                        .any(|widget| widget.summary.key.id == key.id)
            })
            .map(|(tab, _)| tab.clone())
    }
    pub fn check_running(&self) -> Result<(), crate::InspectionError> {
        if self.shutdown {
            Err(crate::InspectionError::new(
                "widget_retired",
                "widget owner has shut down",
            ))
        } else {
            Ok(())
        }
    }
}

// Covers the Snapshot tag, maximum u64 sequence, array delimiters and closing
// envelope. Per-widget accounting includes its comma even for the first entry.
pub(super) const SNAPSHOT_ENVELOPE_BYTES: usize = 64;
const SUMMARY_GROWTH_BYTES: usize = 256;

pub(super) fn summary_snapshot_bytes(
    summary: &WidgetSummary,
) -> Result<usize, crate::InspectionError> {
    struct Counter(usize);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self
                .0
                .checked_add(bytes.len())
                .filter(|size| *size <= WIDGET_MAX_SNAPSHOT_BYTES)
                .ok_or_else(|| std::io::Error::other("widget snapshot exceeds 8 MiB"))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter(0);
    serde_json::to_writer(&mut counter, summary).map_err(|_| {
        crate::InspectionError::new("widget_limit", "owner widget snapshot byte limit reached")
    })?;
    Ok(counter.0 + SUMMARY_GROWTH_BYTES + 1)
}
