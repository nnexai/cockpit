//! Isolated bundled voice webview. The main webview remains alive underneath.
//!
//! WebKit's permission callback does not identify the requesting frame. The
//! microphone grant therefore depends on this view loading only the bundled
//! voice document, denying navigation and popups, and never rendering untrusted
//! HTML or embedding script-capable frames. Its response CSP reinforces those
//! constraints. Do not reuse this view for arbitrary content.

use parking_lot::Mutex;
use serde::Serialize;
use tauri::{
    http::header::HeaderValue, Emitter, LogicalPosition, Runtime, State, Webview,
    WebviewBuilder, WebviewUrl, webview::{NewWindowResponse, PermissionKind, PermissionResponse},
};

pub const VOICE_CHILD_LABEL: &str = "voice-child";

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
    session_id: String,
) -> Result<(), String> {
    if webview.label() != "main" {
        return Err("only the main view can open Voice".into());
    }
    if session_id.trim().is_empty() {
        return Err("a selected session is required".into());
    }
    let mut inner = state.0.lock();
    if inner.child.is_some() { return Err("Voice is already open".into()); }
    let child = VoiceChild::create(webview).map_err(|e| e.to_string())?;
    // Make the context available before the new document can invoke its command.
    inner.session_id = Some(session_id);
    if let Err(error) = child.show() {
        inner.session_id = None;
        return Err(error.to_string());
    }
    inner.child = Some(child);
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

struct VoiceChild<R: Runtime> {
    main: Webview<R>,
    child: Option<Webview<R>>,
}

impl<R: Runtime> VoiceChild<R> {
    fn create(main: Webview<R>) -> tauri::Result<Self> {
        #[cfg(any(target_os = "windows", target_os = "android"))]
        let expected = "http://tauri.localhost/voice.html";
        #[cfg(not(any(target_os = "windows", target_os = "android")))]
        let expected = "tauri://localhost/voice.html";
        let expected = tauri::Url::parse(expected).expect("static voice URL is valid");
        let navigation_url = expected.clone();
        let window = main.window();
        let size = window.inner_size()?;
        let builder = WebviewBuilder::new(VOICE_CHILD_LABEL, WebviewUrl::CustomProtocol(expected))
            .auto_resize()
            .on_navigation(move |url| url == &navigation_url)
            .on_permission_request(|_, kind| match kind {
                #[cfg(any(target_os = "linux", target_os = "macos"))]
                PermissionKind::Microphone => PermissionResponse::Allow,
                _ => PermissionResponse::Deny,
            })
            .on_new_window(|_, _| NewWindowResponse::Deny)
            .on_web_resource_request(|_, response| {
                response.headers_mut().append(
                    "content-security-policy",
                    HeaderValue::from_static("frame-src 'none'; child-src 'none'; object-src 'none'; base-uri 'none'; script-src 'self'"),
                );
            });
        let child = window.add_child(builder, LogicalPosition::new(0.0, 0.0), size)?;
        if let Err(error) = child.hide() {
            let _ = child.close();
            return Err(error);
        }
        Ok(Self { main, child: Some(child) })
    }

    fn show(&self) -> tauri::Result<()> {
        let child = self.child.as_ref().expect("voice child already closed");
        child.set_position(LogicalPosition::new(0.0, 0.0))?;
        child.set_size(self.main.window().inner_size()?)?;
        self.main.hide()?;
        if let Err(error) = child.show() {
            let _ = self.main.show();
            return Err(error);
        }
        child.set_focus()
    }

    fn hide(&self) -> tauri::Result<()> {
        let child = self.child.as_ref().expect("voice child already closed");
        child.hide()?;
        if let Err(error) = self.main.show() {
            let _ = child.show();
            return Err(error);
        }
        self.main.set_focus()
    }

    fn close(mut self) -> tauri::Result<()> {
        self.hide()?;
        if let Some(child) = self.child.as_ref() { child.close()?; }
        self.child = None;
        Ok(())
    }
}

impl<R: Runtime> Drop for VoiceChild<R> {
    fn drop(&mut self) {
        if let Some(child) = self.child.take() {
            let _ = child.hide();
            let _ = self.main.show();
            let _ = child.close();
        }
    }
}
