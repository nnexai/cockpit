//! Native GTK controls. All methods and callbacks run on the GTK main thread.
//! The caller supplies choices obtained from the host, never from the webview.
use std::{cell::{Cell, RefCell}, rc::Rc};

use gtk::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    pub session_id: String,
    pub root_id: String,
    pub project_id: Option<String>,
    pub space_id: Option<String>,
}

#[derive(Clone, Default)]
pub struct Choices {
    pub sessions: Vec<Choice>,
    pub preferred_session_id: Option<String>,
    pub roots: Vec<Choice>,
    pub projects: Vec<Choice>,
    pub spaces: Vec<Choice>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Ready,
    Starting,
    Active { muted: bool },
    Stopping,
}

pub struct DockState {
    pub status: String,
    pub phase: Phase,
}

#[derive(Clone, Debug)]
pub enum Action {
    SessionChanged(String),
    SelectionChanged,
    Start(Selection),
    SetMuted(bool),
    Stop,
}

type Callback = Rc<dyn Fn(Action)>;

struct Widgets {
    parent: gtk::Box,
    container: gtk::Box,
    session: gtk::ComboBoxText,
    root: gtk::ComboBoxText,
    project: gtk::ComboBoxText,
    space: gtk::ComboBoxText,
    status: gtk::Label,
    primary: gtk::Button,
    mute: gtk::Button,
    choices: RefCell<Choices>,
    phase: Cell<Phase>,
    updating: Cell<bool>,
    action: Callback,
}

impl Widgets {
    fn selection(&self) -> Option<Selection> {
        let session_id = self.session.active_id()?.to_string();
        let root_id = self.root.active_id()?.to_string();
        let project_id = self.project.active_id().and_then(|s| (!s.is_empty()).then(|| s.to_string()));
        let space_id = self.space.active_id().and_then(|s| (!s.is_empty()).then(|| s.to_string()));
        let choices = self.choices.borrow();
        let contains = |items: &[Choice], id: &str| items.iter().any(|item| item.id == id && !id.is_empty());
        if !contains(&choices.sessions, &session_id) || !contains(&choices.roots, &root_id)
            || project_id.as_deref().is_some_and(|id| !contains(&choices.projects, id))
            || space_id.as_deref().is_some_and(|id| !contains(&choices.spaces, id)) {
            return None;
        }
        Some(Selection { session_id, root_id, project_id, space_id })
    }

    fn refresh_sensitivity(&self) {
        let ready = self.phase.get() == Phase::Ready;
        for widget in [&self.session, &self.root, &self.project, &self.space] {
            widget.set_sensitive(ready);
        }
        self.primary.set_sensitive(match self.phase.get() {
            Phase::Ready => self.selection().is_some(),
            Phase::Starting | Phase::Active { .. } => true,
            Phase::Stopping => false,
        });
        self.mute.set_sensitive(matches!(self.phase.get(), Phase::Active { .. }));
    }

    fn controls_visible(&self) -> bool {
        fn fully_visible(widget: &gtk::Widget) -> bool {
            if !widget.is_mapped() { return false; }
            let bounds = widget.allocation();
            if bounds.width() <= 0 || bounds.height() <= 0 { return false; }
            let mut ancestor = widget.parent();
            while let Some(parent) = ancestor {
                if !parent.is_mapped() { return false; }
                let Some((x, y)) = widget.translate_coordinates(&parent, 0, 0) else { return false; };
                let area = parent.allocation();
                if x < 0 || y < 0 || x + bounds.width() > area.width() || y + bounds.height() > area.height() {
                    return false;
                }
                ancestor = parent.parent();
            }
            true
        }
        let primary: &gtk::Widget = self.primary.upcast_ref();
        let container: &gtk::Widget = self.container.upcast_ref();
        let (minimum, _) = primary.preferred_size();
        let size = primary.allocation();
        fully_visible(container) && fully_visible(primary)
            && size.width() >= minimum.width && size.height() >= minimum.height
            && (!matches!(self.phase.get(), Phase::Active { .. })
                || fully_visible(self.mute.upcast_ref()))
    }

    fn stop_for_hidden_controls(&self) {
        if matches!(self.phase.get(), Phase::Starting | Phase::Active { .. }) {
            self.phase.set(Phase::Stopping);
            self.status.set_text("Voice controls are not fully visible. Stopping the call.");
            self.refresh_sensitivity();
            (self.action)(Action::Stop);
        }
    }

    fn stop_if_unavailable(&self) {
        if !self.controls_visible() { self.stop_for_hidden_controls(); }
    }
}

/// A dock is a real sibling of the WebKit widget in the window's GTK box.
/// Keep it on the UI thread; dropping or detaching it removes the widgets.
pub struct NativeVoiceDock(Rc<Widgets>);

impl NativeVoiceDock {
    pub fn attach(parent: &gtk::Box, choices: Choices, initial: Option<Selection>, action: impl Fn(Action) + 'static) -> Self {
        let container = gtk::Box::new(gtk::Orientation::Vertical, 6);
        container.set_border_width(10);
        if let Some(accessible) = container.accessible() {
            accessible.set_name("Voice audio controls");
        }
        let selectors = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let session = selector(&selectors, "Session");
        let root = selector(&selectors, "Supervisor");
        let project = selector(&selectors, "Project (optional)");
        let space = selector(&selectors, "Space (optional)");
        container.pack_start(&selectors, false, false, 0);
        let status = gtk::Label::new(Some("Select a session and supervisor before starting voice."));
        status.set_xalign(0.0);
        status.set_line_wrap(true);
        container.pack_start(&status, false, false, 0);
        let privacy = gtk::Label::new(Some("Audio is sent to OpenAI. Cockpit receives conversation text and handoff requests. OpenAI audio retention is not known to Cockpit."));
        privacy.set_xalign(0.0);
        privacy.set_line_wrap(true);
        container.pack_start(&privacy, false, false, 0);
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let mute = gtk::Button::with_label("Mute microphone");
        let primary = gtk::Button::with_label("Start voice call");
        actions.pack_end(&primary, false, false, 0);
        actions.pack_end(&mute, false, false, 0);
        container.pack_start(&actions, false, false, 0);
        parent.pack_end(&container, false, false, 0);
        let widgets = Rc::new(Widgets { parent: parent.clone(), container, session, root, project, space, status, primary, mute,
            choices: RefCell::new(Choices::default()), phase: Cell::new(Phase::Ready), updating: Cell::new(false), action: Rc::new(action) });
        let weak = Rc::downgrade(&widgets);
        widgets.container.connect_unmap(move |_| {
            if let Some(w) = weak.upgrade() { w.stop_for_hidden_controls(); }
        });
        for widget in [&widgets.container, &widgets.parent] {
            let weak = Rc::downgrade(&widgets);
            widget.connect_size_allocate(move |_, _| {
                let weak = weak.clone();
                gtk::glib::idle_add_local_once(move || {
                    if let Some(w) = weak.upgrade() { w.stop_if_unavailable(); }
                });
            });
        }
        let weak = Rc::downgrade(&widgets);
        widgets.session.connect_changed(move |session| {
            if let Some(w) = weak.upgrade() {
                if w.updating.get() { return; }
                let Some(id) = session.active_id().map(|id| id.to_string()) else { return; };
                if !w.choices.borrow().sessions.iter().any(|choice| choice.id == id) { return; }
                // Roots belong to a session. Never allow a root from the previous
                // session to be used while the new host choices are loading.
                w.updating.set(true);
                w.root.remove_all();
                w.choices.borrow_mut().roots.clear();
                w.updating.set(false);
                w.refresh_sensitivity();
                (w.action)(Action::SessionChanged(id));
            }
        });
        for combo in [&widgets.root, &widgets.project, &widgets.space] {
            let weak = Rc::downgrade(&widgets);
            combo.connect_changed(move |_| {
                if let Some(w) = weak.upgrade() {
                    if w.updating.get() { return; }
                    w.refresh_sensitivity();
                    if w.selection().is_some() { (w.action)(Action::SelectionChanged); }
                }
            });
        }
        let weak = Rc::downgrade(&widgets);
        widgets.primary.connect_clicked(move |_| {
            if let Some(w) = weak.upgrade() {
                match w.phase.get() {
                    Phase::Ready => if let Some(selection) = w.selection() {
                        if !w.controls_visible() {
                            w.status.set_text("Voice controls are not fully visible. Enlarge the window to start.");
                            return;
                        }
                        // Freeze the visible selection before handing control to the controller.
                        w.phase.set(Phase::Starting);
                        w.refresh_sensitivity();
                        (w.action)(Action::Start(selection));
                    },
                    Phase::Starting | Phase::Active { .. } => (w.action)(Action::Stop),
                    Phase::Stopping => {},
                }
            }
        });
        let weak = Rc::downgrade(&widgets);
        widgets.mute.connect_clicked(move |_| {
            if let Some(w) = weak.upgrade() {
                if let Phase::Active { muted } = w.phase.get() {
                    // A pending request must not permit repeated unmute activations.
                    w.mute.set_sensitive(false);
                    (w.action)(Action::SetMuted(!muted));
                }
            }
        });
        let dock = Self(widgets);
        dock.set_choices(choices, initial);
        dock.0.container.show_all();
        dock.0.mute.hide();
        dock
    }

    /// Replace host-provided choices and selection atomically. A missing selected
    /// ID remains visible as unavailable and cannot be silently downgraded.
    pub fn set_choices(&self, choices: Choices, selection: Option<Selection>) {
        let w = &self.0;
        if w.phase.get() != Phase::Ready { return; }
        w.updating.set(true);
        let session_id = selection.as_ref().map(|s| s.session_id.as_str()).or(choices.preferred_session_id.as_deref());
        populate(&w.session, &choices.sessions, session_id, false);
        populate(&w.root, &choices.roots, selection.as_ref().map(|s| s.root_id.as_str()), false);
        populate(&w.project, &choices.projects, selection.as_ref().and_then(|s| s.project_id.as_deref()), true);
        populate(&w.space, &choices.spaces, selection.as_ref().and_then(|s| s.space_id.as_deref()), true);
        *w.choices.borrow_mut() = choices;
        w.updating.set(false);
        w.refresh_sensitivity();
    }

    pub fn set_state(&self, state: DockState) {
        let w = &self.0;
        w.status.set_text(&state.status);
        w.phase.set(state.phase);
        match state.phase {
            Phase::Ready => { w.primary.set_label("Start voice call"); w.mute.hide(); }
            Phase::Starting => { w.primary.set_label("Cancel"); w.mute.hide(); }
            Phase::Active { muted } => {
                w.primary.set_label("End call");
                w.mute.set_label(if muted { "Unmute microphone" } else { "Mute microphone" });
                w.mute.show();
            }
            Phase::Stopping => { w.primary.set_label("Ending call…"); w.mute.hide(); }
        }
        w.refresh_sensitivity();
        let weak = Rc::downgrade(w);
        gtk::glib::idle_add_local_once(move || {
            if let Some(w) = weak.upgrade() { w.stop_if_unavailable(); }
        });
    }

    pub fn detach(self) {}
}

impl Drop for NativeVoiceDock {
    fn drop(&mut self) {
        (self.0.action)(Action::Stop);
        self.0.parent.remove(&self.0.container);
    }
}

fn selector(parent: &gtk::Box, text: &str) -> gtk::ComboBoxText {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    let combo = gtk::ComboBoxText::new();
    label.set_mnemonic_widget(Some(&combo));
    row.pack_start(&label, false, false, 0);
    row.pack_start(&combo, true, true, 0);
    parent.pack_start(&row, false, false, 0);
    combo
}

fn populate(combo: &gtk::ComboBoxText, choices: &[Choice], selected: Option<&str>, optional: bool) {
    combo.remove_all();
    if optional { combo.append(Some(""), "None"); }
    for choice in choices {
        if !choice.id.is_empty() { combo.append(Some(&choice.id), &format!("{} ({})", choice.label, choice.id)); }
    }
    if let Some(id) = selected.filter(|id| !id.is_empty()) {
        if !choices.iter().any(|choice| choice.id == id) {
            combo.append(Some(id), &format!("Unavailable ({id})"));
        }
        combo.set_active_id(Some(id));
    } else if optional {
        combo.set_active_id(Some(""));
    }
}
