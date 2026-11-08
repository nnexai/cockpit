//! AppKit voice controls. Construction, updates, callbacks and destruction must
//! all run on the main thread. Choices come from the host, never the webview.
use std::{cell::{Cell, RefCell}, rc::{Rc, Weak}};

use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadOnly, Message};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSAutoresizingMaskOptions, NSButton, NSPopUpButton, NSTextField, NSView, NSWindow};
use objc2_foundation::{MainThreadMarker, NSObject, NSPoint, NSRect, NSRunLoop, NSRunLoopCommonModes, NSSize, NSString, NSTimer};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice { pub id: String, pub label: String }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection { pub session_id: String, pub root_id: String, pub project_id: Option<String>, pub space_id: Option<String> }
#[derive(Clone, Default)]
pub struct Choices { pub sessions: Vec<Choice>, pub roots: Vec<Choice>, pub projects: Vec<Choice>, pub spaces: Vec<Choice>, pub preferred_session_id: Option<String> }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase { Ready, Starting, Active { muted: bool }, Stopping }
pub struct DockState { pub status: String, pub phase: Phase }
#[derive(Clone, Debug)]
pub enum Action { SessionChanged(String), SelectionChanged, Start(Selection), SetMuted(bool), Stop }

const HEIGHT: f64 = 260.0;
type Callback = Rc<dyn Fn(Action)>;

struct Widgets {
    webview: Retained<NSView>,
    parent: Retained<NSView>,
    window: Retained<NSWindow>,
    container: Retained<NSView>,
    session: Retained<NSPopUpButton>,
    root: Retained<NSPopUpButton>,
    project: Retained<NSPopUpButton>,
    space: Retained<NSPopUpButton>,
    status: Retained<NSTextField>,
    primary: Retained<NSButton>,
    mute: Retained<NSButton>,
    target: Retained<DockTarget>,
    visibility_timer: Retained<NSTimer>,
    choices: RefCell<Choices>,
    // Each menu index maps to an ID. None is either the optional None item
    // or the unselected required placeholder.
    ids: RefCell<[Vec<Option<String>>; 4]>,
    phase: Cell<Phase>,
    updating: Cell<bool>,
    visibility_stop_sent: Cell<bool>,
    action: Callback,
}

#[derive(Default)]
struct TargetIvars { widgets: RefCell<Weak<Widgets>> }

define_class!(
    // SAFETY: NSObject has no subclassing requirements. All accesses are on
    // the AppKit main thread, and the weak reference avoids a retain cycle.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = TargetIvars]
    struct DockTarget;

    impl DockTarget {
        #[unsafe(method(sessionChanged:))]
        fn session_changed(&self, _sender: &AnyObject) {
            if let Some(w) = self.ivars().widgets.borrow().upgrade() { w.session_changed(); }
        }
        #[unsafe(method(selectionChanged:))]
        fn selection_changed(&self, _sender: &AnyObject) {
            if let Some(w) = self.ivars().widgets.borrow().upgrade() { w.selection_changed(); }
        }
        #[unsafe(method(primaryPressed:))]
        fn primary_pressed(&self, _sender: &AnyObject) {
            if let Some(w) = self.ivars().widgets.borrow().upgrade() { w.primary_pressed(); }
        }
        #[unsafe(method(mutePressed:))]
        fn mute_pressed(&self, _sender: &AnyObject) {
            if let Some(w) = self.ivars().widgets.borrow().upgrade() { w.mute_pressed(); }
        }
        #[unsafe(method(checkVisibility:))]
        fn check_visibility(&self, _sender: &AnyObject) {
            if let Some(w) = self.ivars().widgets.borrow().upgrade() { w.check_visibility(); }
        }
    }
);

impl DockTarget {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TargetIvars::default());
        // SAFETY: NSObject's init takes no arguments and returns the object.
        unsafe { msg_send![super(this), init] }
    }
}

impl Widgets {
    fn selected(&self, index: usize) -> Option<String> {
        let popup = [&self.session, &self.root, &self.project, &self.space][index];
        let index_in_menu = usize::try_from(popup.indexOfSelectedItem()).ok()?;
        self.ids.borrow()[index].get(index_in_menu).cloned().flatten()
    }
    fn selection(&self) -> Option<Selection> {
        let session_id = self.selected(0)?;
        let root_id = self.selected(1)?;
        let project_id = self.selected(2);
        let space_id = self.selected(3);
        let choices = self.choices.borrow();
        let contains = |items: &[Choice], id: &str| !id.is_empty() && items.iter().any(|item| item.id == id);
        if !contains(&choices.sessions, &session_id) || !contains(&choices.roots, &root_id)
            || project_id.as_deref().is_some_and(|id| !contains(&choices.projects, id))
            || space_id.as_deref().is_some_and(|id| !contains(&choices.spaces, id)) { return None; }
        Some(Selection { session_id, root_id, project_id, space_id })
    }
    fn controls_visible(&self) -> bool {
        if self.container.isHiddenOrHasHiddenAncestor() || !self.window.isVisible() || self.window.isMiniaturized() {
            return false;
        }
        let Some(window) = self.container.window() else { return false; };
        if !std::ptr::eq(&*window, &*self.window) { return false; }
        // SAFETY: Both views are retained by Widgets throughout this check.
        if !unsafe { self.container.superview() }.is_some_and(|parent| std::ptr::eq(&*parent, &*self.parent)) {
            return false;
        }
        // A host set_bounds that has not reserved the dock can put the
        // webview over it even though AppKit's visibleRect is unchanged.
        let dock = self.container.frame();
        let web = self.webview.frame();
        if dock.origin.x < web.origin.x + web.size.width && web.origin.x < dock.origin.x + dock.size.width
            && dock.origin.y < web.origin.y + web.size.height && web.origin.y < dock.origin.y + dock.size.height {
            return false;
        }
        let bounds = self.container.bounds();
        let visible = self.container.visibleRect();
        // All controls must fit, including the End call and mute buttons.
        bounds.size.width >= 315.0 && bounds.size.height >= HEIGHT
            && visible.origin.x <= bounds.origin.x && visible.origin.y <= bounds.origin.y
            && visible.origin.x + visible.size.width >= bounds.origin.x + bounds.size.width
            && visible.origin.y + visible.size.height >= bounds.origin.y + bounds.size.height
    }
    fn check_visibility(&self) {
        if !self.controls_visible() && matches!(self.phase.get(), Phase::Starting | Phase::Active { .. })
            && !self.visibility_stop_sent.replace(true) {
            self.phase.set(Phase::Stopping);
            self.refresh();
            (self.action)(Action::Stop);
        } else if self.phase.get() == Phase::Ready {
            self.refresh();
        }
    }
    fn refresh(&self) {
        let ready = self.phase.get() == Phase::Ready;
        for popup in [&self.session, &self.root, &self.project, &self.space] { popup.setEnabled(ready); }
        self.primary.setEnabled(match self.phase.get() {
            Phase::Ready => self.controls_visible() && self.selection().is_some(),
            Phase::Starting | Phase::Active { .. } => true,
            Phase::Stopping => false,
        });
        self.mute.setEnabled(matches!(self.phase.get(), Phase::Active { .. }));
    }
    fn session_changed(&self) {
        if self.updating.get() || self.phase.get() != Phase::Ready { return; }
        let Some(session) = self.selected(0) else { return; };
        if !self.choices.borrow().sessions.iter().any(|c| !c.id.is_empty() && c.id == session) { return; }
        self.choices.borrow_mut().roots.clear();
        self.refresh();
        (self.action)(Action::SessionChanged(session));
    }
    fn selection_changed(&self) {
        if self.updating.get() || self.phase.get() != Phase::Ready { return; }
        self.refresh();
        (self.action)(Action::SelectionChanged);
    }
    fn primary_pressed(&self) {
        match self.phase.get() {
            Phase::Ready => if self.controls_visible() { if let Some(selection) = self.selection() {
                self.phase.set(Phase::Starting);
                self.refresh();
                (self.action)(Action::Start(selection));
            } },
            Phase::Starting | Phase::Active { .. } => (self.action)(Action::Stop),
            Phase::Stopping => {}
        }
    }
    fn mute_pressed(&self) {
        if let Phase::Active { muted } = self.phase.get() {
            self.mute.setEnabled(false);
            (self.action)(Action::SetMuted(!muted));
        }
    }
}

/// A native sibling of the supplied WKWebView. Must remain on the main thread.
pub struct NativeVoiceDock(Rc<Widgets>);

impl NativeVoiceDock {
    pub fn attach(webview: &NSView, choices: Choices, initial: Option<Selection>, action: impl Fn(Action) + 'static) -> Self {
        let mtm = MainThreadMarker::new().expect("native voice dock requires the main thread");
        // SAFETY: The host keeps its webview attached for this call; the returned
        // retained parent remains alive for the dock's lifetime.
        let parent = unsafe { webview.superview() }.expect("webview must have a parent view");
        let window = webview.window().expect("webview must belong to a window");
        let target = DockTarget::new(mtm);
        let frame = webview.frame();
        let container = NSView::initWithFrame(NSView::alloc(mtm), rect(frame.origin.x, frame.origin.y, frame.size.width, HEIGHT));
        let mut mask = NSAutoresizingMaskOptions::ViewWidthSizable;
        if parent.isFlipped() { mask |= NSAutoresizingMaskOptions::ViewMinYMargin; }
        container.setAutoresizingMask(mask);
        parent.addSubview(&container);

        let session = popup(&container, &target, "Session", 222.0, sel!(sessionChanged:), mtm);
        let root = popup(&container, &target, "Supervisor", 190.0, sel!(selectionChanged:), mtm);
        let project = popup(&container, &target, "Project (optional)", 158.0, sel!(selectionChanged:), mtm);
        let space = popup(&container, &target, "Space (optional)", 126.0, sel!(selectionChanged:), mtm);
        let status = label(&container, "Select a session and supervisor before starting voice.", 92.0, 24.0, mtm);
        let _privacy = label(&container, "Audio is sent to OpenAI. Cockpit receives conversation text and handoff requests. OpenAI audio retention is not known to Cockpit.", 52.0, 38.0, mtm);
        let mute = button(&container, &target, "Mute microphone", 10.0, sel!(mutePressed:), mtm);
        let primary = button(&container, &target, "Start voice call", 165.0, sel!(primaryPressed:), mtm);
        // SAFETY: the timer retains its target; the selector has the timer
        // action signature. Common modes include live window resize/tracking.
        let visibility_timer = unsafe {
            let timer = NSTimer::timerWithTimeInterval_target_selector_userInfo_repeats(
                0.1, &target, sel!(checkVisibility:), None, true);
            NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes);
            timer
        };
        let widgets = Rc::new(Widgets { webview: webview.retain(), parent, window, container, session, root, project, space, status, primary, mute, target, visibility_timer,
            choices: RefCell::new(Choices::default()), ids: RefCell::new(Default::default()), phase: Cell::new(Phase::Ready), updating: Cell::new(false), visibility_stop_sent: Cell::new(false), action: Rc::new(action) });
        *widgets.target.ivars().widgets.borrow_mut() = Rc::downgrade(&widgets);
        let dock = Self(widgets);
        dock.reserve_layout();
        dock.set_choices(choices, initial);
        dock.0.mute.setHidden(true);
        dock
    }

    /// Call after the host explicitly sets the webview bounds. Ordinary parent
    /// resizes are handled by AppKit autoresizing.
    pub fn reserve_layout(&self) {
        let w = &self.0;
        let mut frame = w.webview.frame();
        let height = HEIGHT.min(frame.size.height.max(0.0));
        let dock_y = if w.parent.isFlipped() { frame.origin.y + frame.size.height - height } else { frame.origin.y };
        w.container.setFrame(rect(frame.origin.x, dock_y, frame.size.width, height));
        if !w.parent.isFlipped() { frame.origin.y += height; }
        frame.size.height -= height;
        w.webview.setFrame(frame);
    }

    pub fn set_choices(&self, choices: Choices, selection: Option<Selection>) {
        let w = &self.0;
        if w.phase.get() != Phase::Ready { return; }
        w.updating.set(true);
        let mut ids = w.ids.borrow_mut();
        ids[0] = populate(&w.session, &choices.sessions, selection.as_ref().map(|s| s.session_id.as_str()).or(choices.preferred_session_id.as_deref()), false);
        ids[1] = populate(&w.root, &choices.roots, selection.as_ref().map(|s| s.root_id.as_str()), false);
        ids[2] = populate(&w.project, &choices.projects, selection.as_ref().and_then(|s| s.project_id.as_deref()), true);
        ids[3] = populate(&w.space, &choices.spaces, selection.as_ref().and_then(|s| s.space_id.as_deref()), true);
        drop(ids);
        *w.choices.borrow_mut() = choices;
        w.updating.set(false);
        w.refresh();
    }
    pub fn set_state(&self, state: DockState) {
        let w = &self.0;
        w.status.setStringValue(&NSString::from_str(&state.status));
        w.phase.set(state.phase);
        if state.phase == Phase::Ready { w.visibility_stop_sent.set(false); }
        let title = match state.phase {
            Phase::Ready => "Start voice call",
            Phase::Starting => "Cancel",
            Phase::Active { .. } => "End call",
            Phase::Stopping => "Ending call…",
        };
        w.primary.setTitle(&NSString::from_str(title));
        if let Phase::Active { muted } = state.phase {
            w.mute.setTitle(&NSString::from_str(if muted { "Unmute microphone" } else { "Mute microphone" }));
            w.mute.setHidden(false);
        } else { w.mute.setHidden(true); }
        w.refresh();
        w.check_visibility();
    }
    pub fn detach(self) {}
}

impl Drop for NativeVoiceDock {
    fn drop(&mut self) {
        let w = &self.0;
        w.visibility_timer.invalidate();
        (w.action)(Action::Stop);
        let mut frame = w.webview.frame();
        let height = w.container.frame().size.height;
        if !w.parent.isFlipped() { frame.origin.y -= height; }
        frame.size.height += height;
        w.webview.setFrame(frame);
        w.container.removeFromSuperview();
    }
}

fn rect(x: f64, y: f64, width: f64, height: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(width, height))
}
fn label(parent: &NSView, text: &str, y: f64, height: f64, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let field = NSTextField::wrappingLabelWithString(&NSString::from_str(text), mtm);
    field.setFrame(rect(10.0, y, (parent.bounds().size.width - 20.0).max(0.0), height));
    field.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
    parent.addSubview(&field);
    field
}
fn popup(parent: &NSView, target: &DockTarget, title: &str, y: f64, action: objc2::runtime::Sel, mtm: MainThreadMarker) -> Retained<NSPopUpButton> {
    let caption = NSTextField::labelWithString(&NSString::from_str(title), mtm);
    caption.setFrame(rect(10.0, y + 3.0, 130.0, 22.0));
    parent.addSubview(&caption);
    let control = NSPopUpButton::initWithFrame_pullsDown(NSPopUpButton::alloc(mtm), rect(145.0, y, (parent.bounds().size.width - 155.0).max(0.0), 26.0), false);
    control.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
    // SAFETY: target is retained in Widgets; selectors are implemented above
    // with the NSControl action signature and run only on the main thread.
    unsafe { control.setTarget(Some(target)); control.setAction(Some(action)); }
    parent.addSubview(&control);
    control
}
fn button(parent: &NSView, target: &DockTarget, title: &str, x: f64, action: objc2::runtime::Sel, mtm: MainThreadMarker) -> Retained<NSButton> {
    // SAFETY: target outlives the button and implements the action selector.
    let control = unsafe { NSButton::buttonWithTitle_target_action(&NSString::from_str(title), Some(target), Some(action), mtm) };
    control.setFrame(rect(x, 12.0, 150.0, 28.0));
    parent.addSubview(&control);
    control
}
fn populate(popup: &NSPopUpButton, choices: &[Choice], selected: Option<&str>, optional: bool) -> Vec<Option<String>> {
    popup.removeAllItems();
    popup.addItemWithTitle(&NSString::from_str(if optional { "None" } else { "Select…" }));
    let mut ids = vec![None];
    for choice in choices.iter().filter(|choice| !choice.id.is_empty()) {
        popup.addItemWithTitle(&NSString::from_str(&format!("{} ({})", choice.label, choice.id)));
        ids.push(Some(choice.id.clone()));
    }
    if let Some(id) = selected.filter(|id| !id.is_empty()) {
        let index = ids.iter().position(|item| item.as_deref() == Some(id)).unwrap_or_else(|| {
            popup.addItemWithTitle(&NSString::from_str(&format!("Unavailable ({id})")));
            ids.push(Some(id.to_owned()));
            ids.len() - 1
        });
        popup.selectItemAtIndex(index as isize);
    } else { popup.selectItemAtIndex(0); }
    ids
}
