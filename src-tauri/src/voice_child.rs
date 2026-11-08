//! A dedicated, in-window webview for the bundled voice page.
//!
//! The owner must give `voice-child` a narrowly scoped IPC capability (not the
//! main webview's capability), and must deny media requests on the main webview.
//! The voice document must also ship with `frame-src 'none'` in its CSP. The
//! additional response CSP below is defense in depth, not proof of the origin
//! of a native media permission request.
//!
//! In particular, WebKitGTK does not expose the requesting frame/origin in its
//! permission callback. Until trusted-origin media permission is established,
//! this webview denies all permissions, including microphone. Never replace
//! that denial with a grant based only on the webview label or its current URL.

use tauri::{
    http::HeaderValue,
    webview::{NewWindowResponse, PermissionResponse, WebviewBuilder},
    LogicalPosition, Runtime, Webview, WebviewUrl,
};
use parking_lot::Mutex;
use serde::Serialize;
use tauri::{Emitter, State};
use std::sync::Arc;
use crate::native_voice::{controller::NativeVoiceController, presentation::NativeVoicePresentation};
use tauri::Manager;

#[tauri::command]
pub fn cockpit_native_voice_snapshot<R: Runtime>(
    webview: Webview<R>,
    controller: State<'_, Arc<NativeVoiceController>>,
) -> Result<NativeVoicePresentation, String> {
    if webview.label() != VOICE_CHILD_LABEL {
        return Err("Voice presentation is only available to the voice view".into());
    }
    Ok(controller.snapshot())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
thread_local! {
    static DOCK: std::cell::RefCell<Option<(Arc<std::sync::atomic::AtomicBool>, crate::native_voice::controls::NativeVoiceDock)>> = const { std::cell::RefCell::new(None) };
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
async fn voice_choices<R: Runtime>(
    app: &tauri::AppHandle<R>, session_id: &str,
) -> Result<(crate::native_voice::controls::Choices, Option<crate::native_voice::controls::Selection>), String> {
    use crate::native_voice::controls::{Choice, Choices, Selection};
    use cockpit_host::transport::{operations::{orchestration::orchestration_liaison_lookup, projects::repositories, session::{sessions, session_snapshot}}, Transport};
    use cockpit_protocol::orchestration::{LiaisonLookupRequest, LiaisonLookupResponse};
    let service = app.state::<cockpit_core::CockpitService>();
    let runtime = app.state::<Arc<cockpit_host::OrchestrationRuntime>>();
    let sessions = sessions(&service, Transport::Native, ()).await.map_err(|e| format!("{e:?}"))?;
    if !sessions.sessions.iter().any(|s| s.id == session_id) {
        return Err("The selected session is no longer available".into());
    }
    let snapshot = session_snapshot(&service, Transport::Native, session_id.to_owned()).await.map_err(|e| format!("{e:?}"))?;
    let repositories = repositories(&service, Transport::Native, ()).await.map_err(|e| format!("{e:?}"))?;
    let lookup = orchestration_liaison_lookup(&runtime, Transport::Native, LiaisonLookupRequest { session_id: session_id.to_owned(), selected_root_id: None }).await.map_err(|e| format!("{e:?}"))?;
    let (preferred, roots) = match lookup {
        LiaisonLookupResponse::Resolved { preferred_root_id, roots, .. } => (preferred_root_id, roots),
        LiaisonLookupResponse::Unavailable { message, .. } => return Err(message),
    };
    let root = preferred.filter(|id| roots.iter().any(|r| &r.root_id == id))
        .or_else(|| (roots.len() == 1).then(|| roots[0].root_id.clone()));
    let initial = root.map(|root_id| Selection { session_id: session_id.to_owned(), root_id, project_id: None, space_id: None });
    let choices = Choices {
        preferred_session_id: Some(session_id.to_owned()),
        sessions: sessions.sessions.into_iter().map(|s| Choice { id: s.id, label: s.label }).collect(),
        roots: roots.into_iter().map(|r| Choice { label: format!("{} · {:?}", r.root_id, r.connection), id: r.root_id }).collect(),
        projects: repositories.repositories.into_iter().map(|r| Choice { id: r.repository_id, label: r.name }).collect(),
        spaces: snapshot.spaces.into_iter().map(|s| Choice { id: s.id, label: s.label }).collect(),
    };
    Ok((choices, initial))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
async fn attach_dock<R: Runtime>(
    child: &Webview<R>,
    choices: crate::native_voice::controls::Choices,
    initial: Option<crate::native_voice::controls::Selection>,
    controller: Arc<NativeVoiceController>,
    alive: Arc<std::sync::atomic::AtomicBool>,
) -> Result<(), String> {
    let app = child.app_handle().clone();
    let (attached, result) = tokio::sync::oneshot::channel::<Result<(), String>>();
    child.with_webview(move |platform| {
        use crate::native_voice::controls::{Action, DockState, NativeVoiceDock, Phase};
        if !alive.load(std::sync::atomic::Ordering::SeqCst) {
            let _ = attached.send(Err("Voice was closed before controls attached".into()));
            return;
        }
        #[cfg(target_os = "linux")]
        let parent = {
            use gtk::prelude::*;
            match platform.inner().parent().and_then(|p| p.downcast::<gtk::Box>().ok()) {
                Some(parent) => parent,
                None => { let _ = attached.send(Err("Cannot attach native Voice controls to the window".into())); return; }
            }
        };
        #[cfg(target_os = "macos")]
        let parent = match unsafe { platform.inner().cast::<objc2_app_kit::NSView>().as_ref() } {
            Some(view) if unsafe { view.superview() }.is_some() => view,
            _ => { let _ = attached.send(Err("Cannot attach native Voice controls to the window".into())); return; }
        };
        let generation = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let pending = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let action_alive = alive.clone();
        let action_pending = pending.clone();
        let action_controller = controller.clone();
        #[cfg(target_os = "macos")]
        let platform_app = app.clone();
        let action = move |action| match action {
            Action::SessionChanged(session_id) => {
                action_pending.store(true, std::sync::atomic::Ordering::SeqCst);
                let alive = action_alive.clone();
                let pending = action_pending.clone();
                let serial = generation.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                let generation = generation.clone();
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let result = voice_choices(&app, &session_id).await;
                    let _ = app.run_on_main_thread(move || {
                        if !alive.load(std::sync::atomic::Ordering::SeqCst) || generation.load(std::sync::atomic::Ordering::SeqCst) != serial { return; }
                        DOCK.with(|slot| if let Some((_, dock)) = slot.borrow().as_ref() {
                            match result {
                                Ok((choices, initial)) => { pending.store(false, std::sync::atomic::Ordering::SeqCst); dock.set_choices(choices, initial); dock.set_state(DockState { status: "Ready".into(), phase: Phase::Ready }); }
                                Err(error) => dock.set_state(DockState { status: error, phase: Phase::Ready }),
                            }
                        });
                    });
                });
            }
            Action::SelectionChanged => {}
            Action::Start(selection) => {
                let scope = if selection.project_id.is_some() || selection.space_id.is_some() {
                    Some(cockpit_protocol::orchestration::HandoffBinding {
                        repository_id: selection.project_id, workspace_id: selection.space_id, branch: None, base_ref: None,
                    })
                } else { None };
                if let Err(error) = action_controller.start(selection.session_id, Some(selection.root_id), scope) {
                    DOCK.with(|slot| if let Some((_, dock)) = slot.borrow().as_ref() {
                        dock.set_state(DockState { status: error, phase: Phase::Ready });
                    });
                }
            }
            Action::SetMuted(muted) => { let _ = action_controller.set_muted(muted); }
            Action::Stop => action_controller.stop(),
        };
        let dock = NativeVoiceDock::attach(&parent, choices, initial, action);
        DOCK.with(|slot| *slot.borrow_mut() = Some((alive.clone(), dock)));
        let _ = attached.send(Ok(()));
        #[cfg(target_os = "linux")]
        gtk::glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
            if !alive.load(std::sync::atomic::Ordering::SeqCst) { return gtk::glib::ControlFlow::Break; }
            if pending.load(std::sync::atomic::Ordering::SeqCst) { return gtk::glib::ControlFlow::Continue; }
            let snapshot = controller.snapshot();
            DOCK.with(|slot| {
                let slot = slot.borrow();
                let Some((_, dock)) = slot.as_ref() else { return gtk::glib::ControlFlow::Break; };
                let phase = match snapshot.status.as_str() {
                    "starting" => Phase::Starting,
                    "active" => Phase::Active { muted: snapshot.muted },
                    "stopping" => Phase::Stopping,
                    _ => Phase::Ready,
                };
                dock.set_state(DockState { status: snapshot.error.unwrap_or(snapshot.status), phase });
                gtk::glib::ControlFlow::Continue
            })
        });
        #[cfg(target_os = "macos")]
        {
            let mut updates = controller.subscribe();
            let app = platform_app.clone();
            tauri::async_runtime::spawn(async move {
                while updates.changed().await.is_ok() {
                    if !alive.load(std::sync::atomic::Ordering::SeqCst) { break; }
                    if pending.load(std::sync::atomic::Ordering::SeqCst) { continue; }
                    let snapshot = updates.borrow_and_update().clone();
                    let alive = alive.clone();
                    let _ = app.run_on_main_thread(move || {
                        if !alive.load(std::sync::atomic::Ordering::SeqCst) { return; }
                        DOCK.with(|slot| if let Some((_, dock)) = slot.borrow().as_ref() {
                            let phase = match snapshot.status.as_str() {
                                "starting" => Phase::Starting,
                                "active" => Phase::Active { muted: snapshot.muted },
                                "stopping" => Phase::Stopping,
                                _ => Phase::Ready,
                            };
                            dock.set_state(DockState { status: snapshot.error.unwrap_or(snapshot.status), phase });
                        });
                    });
                }
            });
        }
    }).map_err(|e| e.to_string())?;
    result.await.map_err(|_| "Native Voice controls could not be attached".to_string())?
}

pub struct VoiceViewState<R: Runtime>(Mutex<VoiceViewInner<R>>);

struct VoiceViewInner<R: Runtime> {
    child: Option<VoiceChild<R>>,
    session_id: Option<String>,
}

impl<R: Runtime> Default for VoiceViewState<R> {
    fn default() -> Self {
        Self(Mutex::new(VoiceViewInner { child: None, session_id: None }))
    }
}

#[derive(Serialize)]
pub struct VoiceViewContext {
    session_id: Option<String>,
}

#[tauri::command]
pub async fn cockpit_voice_view_open<R: Runtime>(
    webview: Webview<R>,
    state: State<'_, VoiceViewState<R>>,
    controller: State<'_, Arc<NativeVoiceController>>,
    session_id: String,
) -> Result<(), String> {
    if webview.label() != "main" {
        return Err("only the main view can open Voice".into());
    }
    if session_id.trim().is_empty() {
        return Err("a selected session is required".into());
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    return Err("Native Voice controls are not available on this platform".into());
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    let (choices, initial) = voice_choices(webview.app_handle(), &session_id).await?;
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    let (child_webview, alive) = {
        let mut inner = state.0.lock();
        if inner.child.is_some() { return Err("Voice is already open".into()); }
        let child = VoiceChild::create(webview, controller.inner().clone()).map_err(|e| e.to_string())?;
        child.show().map_err(|e| e.to_string())?;
        let child_webview = child.child.as_ref().expect("new voice child").clone();
        let alive = child.alive.clone();
        inner.session_id = Some(session_id);
        inner.child = Some(child);
        (child_webview, alive)
    };
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    if let Err(error) = attach_dock(&child_webview, choices, initial, controller.inner().clone(), alive.clone()).await {
        let mut inner = state.0.lock();
        if inner.child.as_ref().is_some_and(|child| Arc::ptr_eq(&child.alive, &alive)) {
            inner.session_id = None;
            if let Some(child) = inner.child.take() { let _ = child.close(); }
        }
        return Err(error);
    }
    Ok(())
}

#[tauri::command]
pub fn cockpit_voice_view_context<R: Runtime>(
    webview: Webview<R>,
    state: State<'_, VoiceViewState<R>>,
) -> Result<VoiceViewContext, String> {
    if webview.label() != VOICE_CHILD_LABEL {
        return Err("Voice context is only available to the voice view".into());
    }
    Ok(VoiceViewContext { session_id: state.0.lock().session_id.clone() })
}

#[tauri::command]
pub fn cockpit_voice_view_close<R: Runtime>(
    webview: Webview<R>,
    state: State<'_, VoiceViewState<R>>,
    view: String,
) -> Result<(), String> {
    if webview.label() != VOICE_CHILD_LABEL {
        return Err("only the voice view can close Voice".into());
    }
    if view != "workbench" && view != "inbox" {
        return Err("invalid destination".into());
    }
    let mut inner = state.0.lock();
    inner.child.as_ref().ok_or("Voice is not open")?.main
        .emit_to(tauri::EventTarget::webview("main"), "cockpit-voice-return", &view)
        .map_err(|e| e.to_string())?;
    let child = inner.child.take().expect("checked above");
    inner.session_id = None;
    child.close().map_err(|e| e.to_string())
}


pub const VOICE_CHILD_LABEL: &str = "voice-child";

/// Owns the voice webview; the existing main webview is never destroyed.
///
/// Explicitly call `close` when finished so failures are observable. Dropping
/// the owner also attempts to restore the main view and close the child.
pub struct VoiceChild<R: Runtime> {
    main: Webview<R>,
    child: Option<Webview<R>>,
    controller: Arc<NativeVoiceController>,
    alive: Arc<std::sync::atomic::AtomicBool>,
}

impl<R: Runtime> VoiceChild<R> {
    /// Create a hidden child in the main webview's existing native window.
    /// The Tauri `unstable` feature is required. Production bundled assets
    /// are required: an external development server is not an allowed origin.
    pub fn create(main: Webview<R>, controller: Arc<NativeVoiceController>) -> tauri::Result<Self> {
        #[cfg(any(target_os = "windows", target_os = "android"))]
        let expected = "http://tauri.localhost/voice.html";
        #[cfg(not(any(target_os = "windows", target_os = "android")))]
        let expected = "tauri://localhost/voice.html";
        let expected = tauri::Url::parse(expected).expect("static voice URL is valid");
        let navigation_url = expected.clone();
        let navigation_controller = controller.clone();
        let navigated = std::sync::atomic::AtomicBool::new(false);
        let window = main.window();
        let size = window.inner_size()?;
        let builder = WebviewBuilder::new(
            VOICE_CHILD_LABEL,
            WebviewUrl::CustomProtocol(expected),
        )
        .auto_resize()
        .on_navigation(move |url| {
            let allowed = url == &navigation_url;
            if !allowed || navigated.swap(true, std::sync::atomic::Ordering::SeqCst) { navigation_controller.stop(); }
            allowed
        })
        .on_new_window(|_, _| NewWindowResponse::Deny)
        .on_permission_request(|_, _| PermissionResponse::Deny)
        .on_web_resource_request(|_, response| {
            // Multiple CSP headers are intersected, so this cannot weaken an
            // existing application policy. The bundled page must also carry
            // this policy itself; external dev-server responses bypass this hook.
            response.headers_mut().append(
                "content-security-policy",
                HeaderValue::from_static("frame-src 'none'; child-src 'none'; object-src 'none'"),
            );
        });
        let child = window.add_child(builder, LogicalPosition::new(0.0, 0.0), size)?;
        if let Err(error) = child.hide() {
            let _ = child.close();
            return Err(error);
        }
        Ok(Self { main, child: Some(child), controller, alive: Arc::new(std::sync::atomic::AtomicBool::new(true)) })
    }

    /// Replace the main webview without destroying its document or UI state.
    pub fn show(&self) -> tauri::Result<()> {
        let child = self.child.as_ref().expect("voice child already closed");
        // GTK uses a box rather than honoring child bounds, so only one child
        // may be visible at a time. Refresh bounds for platforms that use them.
        child.set_position(LogicalPosition::new(0.0, 0.0))?;
        child.set_size(self.main.window().inner_size()?)?;
        self.main.hide()?;
        if let Err(error) = child.show() {
            let _ = self.main.show();
            return Err(error);
        }
        child.set_focus()
    }

    /// Hide the voice webview and restore the unchanged main webview.
    pub fn hide(&self) -> tauri::Result<()> {
        let child = self.child.as_ref().expect("voice child already closed");
        child.hide()?;
        if let Err(error) = self.main.show() {
            let _ = child.show();
            return Err(error);
        }
        self.main.set_focus()
    }

    /// Restore the main webview and destroy the voice webview. This also
    /// terminates the voice document; the owner must stop its audio/session.
    pub fn close(mut self) -> tauri::Result<()> {
        self.stop_local();
        self.hide()?;
        if let Some(child) = self.child.as_ref() {
            child.close()?;
        }
        self.child = None;
        Ok(())
    }

    fn stop_local(&self) {
        self.alive.store(false, std::sync::atomic::Ordering::SeqCst);
        self.controller.stop();
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        if let Some(child) = self.child.as_ref() {
            let alive = self.alive.clone();
            let _ = child.with_webview(move |_| {
                DOCK.with(|slot| {
                    let mut slot = slot.borrow_mut();
                    if slot.as_ref().is_some_and(|(owner, _)| Arc::ptr_eq(owner, &alive)) {
                        if let Some((_, dock)) = slot.take() { dock.detach(); }
                    }
                });
            });
        }
    }
}

impl<R: Runtime> Drop for VoiceChild<R> {
    fn drop(&mut self) {
        self.stop_local();
        if let Some(child) = self.child.take() {
            let _ = child.hide();
            let _ = self.main.show();
            let _ = child.close();
        }
    }
}
