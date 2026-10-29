use cockpit_host::transport::error::OperationError;
use cockpit_protocol::v1::ErrorResponse;
use serde::Serialize;
use tokio::sync::Mutex;

#[derive(Default)]
pub struct PowerState {
    inhibitor: Mutex<Option<Inhibitor>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PowerStatus {
    stay_awake_available: bool,
    stay_awake: bool,
    screen_off_available: bool,
}

fn power_error(message: impl Into<String>) -> ErrorResponse {
    OperationError::rejected("native_power_unavailable", message.into()).into()
}

#[tauri::command]
pub async fn cockpit_power_status(state: tauri::State<'_, PowerState>) -> Result<PowerStatus, ErrorResponse> {
    let inhibitor = state.inhibitor.lock().await;
    Ok(PowerStatus {
        stay_awake_available: inhibitor.is_some() || inhibitor_available().await,
        stay_awake: inhibitor.is_some(),
        screen_off_available: screen_off_available(),
    })
}

#[tauri::command]
pub async fn cockpit_power_set_stay_awake(
    enabled: bool,
    state: tauri::State<'_, PowerState>,
) -> Result<(), ErrorResponse> {
    let mut inhibitor = state.inhibitor.lock().await;
    if enabled && inhibitor.is_none() {
        *inhibitor = Some(Inhibitor::acquire().await?);
    } else if !enabled {
        if let Some(active) = inhibitor.as_mut() {
            active.release().await?;
        }
        *inhibitor = None;
    }
    Ok(())
}

#[tauri::command]
pub async fn cockpit_power_screen_off() -> Result<(), ErrorResponse> {
    if !screen_off_available() {
        return Err(power_error("Screen power control is unavailable on this desktop"));
    }
    #[cfg(target_os = "linux")]
    let mut command = {
        let mut command = tokio::process::Command::new("niri");
        command.args(["msg", "action", "power-off-monitors"]);
        command
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = tokio::process::Command::new("/usr/bin/pmset");
        command.arg("displaysleepnow");
        command
    };
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let output = command.output().await.map_err(|error| power_error(format!("Could not turn off the display: {error}")))?;
        if !output.status.success() {
            return Err(power_error(format!("Could not turn off the display: {}", String::from_utf8_lossy(&output.stderr).trim())));
        }
        Ok(())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    Err(power_error("Screen power control is unsupported on this platform"))
}

#[cfg(target_os = "linux")]
fn screen_off_available() -> bool {
    std::env::var_os("NIRI_SOCKET").is_some_and(|value| !value.is_empty())
        && std::env::var_os("PATH").is_some_and(|path| {
            std::env::split_paths(&path).any(|dir| dir.join("niri").is_file())
        })
}

#[cfg(target_os = "macos")]
fn screen_off_available() -> bool { std::path::Path::new("/usr/bin/pmset").is_file() }

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn screen_off_available() -> bool { false }

#[cfg(target_os = "linux")]
struct Inhibitor {
    connection: zbus::Connection,
    cookie: u32,
}

#[cfg(target_os = "linux")]
async fn inhibitor_available() -> bool {
    let Ok(connection) = zbus::Connection::session().await else { return false };
    let Ok(proxy) = zbus::fdo::DBusProxy::new(&connection).await else { return false };
    let Ok(name) = "org.freedesktop.ScreenSaver".try_into() else { return false };
    proxy.name_has_owner(name).await.unwrap_or(false)
}

#[cfg(target_os = "linux")]
impl Inhibitor {
    async fn acquire() -> Result<Self, ErrorResponse> {
        let connection = zbus::Connection::session().await.map_err(|error| power_error(format!("Could not connect to the session bus: {error}")))?;
        let proxy = zbus::Proxy::new(&connection, "org.freedesktop.ScreenSaver", "/org/freedesktop/ScreenSaver", "org.freedesktop.ScreenSaver")
            .await.map_err(|error| power_error(format!("ScreenSaver service unavailable: {error}")))?;
        let cookie: u32 = proxy.call("Inhibit", &("Cockpit", "Agents are working"))
            .await.map_err(|error| power_error(format!("Could not inhibit idle sleep: {error}")))?;
        drop(proxy);
        Ok(Self { connection, cookie })
    }

    async fn release(&mut self) -> Result<(), ErrorResponse> {
        let proxy = zbus::Proxy::new(&self.connection, "org.freedesktop.ScreenSaver", "/org/freedesktop/ScreenSaver", "org.freedesktop.ScreenSaver")
            .await.map_err(|error| power_error(format!("ScreenSaver service unavailable: {error}")))?;
        proxy.call::<_, _, ()>("UnInhibit", &(self.cookie,)).await
            .map_err(|error| power_error(format!("Could not release the sleep inhibitor: {error}")))
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::{c_char, c_void};
    type CFStringRef = *const c_void;
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithCString(allocator: *const c_void, string: *const c_char, encoding: u32) -> CFStringRef;
        fn CFRelease(value: *const c_void);
    }
    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        fn IOPMAssertionCreateWithName(kind: CFStringRef, level: u32, name: CFStringRef, id: *mut u32) -> i32;
        fn IOPMAssertionRelease(id: u32) -> i32;
    }
    pub fn acquire() -> Result<u32, String> {
        unsafe {
            let kind = CFStringCreateWithCString(std::ptr::null(), c"PreventUserIdleSystemSleep".as_ptr(), 0x08000100);
            let name = CFStringCreateWithCString(std::ptr::null(), c"Cockpit: agents are working".as_ptr(), 0x08000100);
            if kind.is_null() || name.is_null() {
                if !kind.is_null() { CFRelease(kind); }
                if !name.is_null() { CFRelease(name); }
                return Err("Could not allocate power assertion strings".into());
            }
            let mut id = 0;
            let result = IOPMAssertionCreateWithName(kind, 255, name, &mut id);
            CFRelease(kind);
            CFRelease(name);
            if result != 0 { return Err(format!("Could not create power assertion: {result}")); }
            Ok(id)
        }
    }
    pub fn release(id: u32) -> Result<(), String> {
        let result = unsafe { IOPMAssertionRelease(id) };
        if result == 0 { Ok(()) } else { Err(format!("Could not release power assertion: {result}")) }
    }
}

#[cfg(target_os = "macos")]
struct Inhibitor { id: Option<u32> }
#[cfg(target_os = "macos")]
async fn inhibitor_available() -> bool { true }
#[cfg(target_os = "macos")]
impl Inhibitor {
    async fn acquire() -> Result<Self, ErrorResponse> { Ok(Self { id: Some(macos::acquire().map_err(power_error)?) }) }
    async fn release(&mut self) -> Result<(), ErrorResponse> {
        if let Some(id) = self.id { macos::release(id).map_err(power_error)?; self.id = None; }
        Ok(())
    }
}
#[cfg(target_os = "macos")]
impl Drop for Inhibitor {
    fn drop(&mut self) { if let Some(id) = self.id.take() { let _ = macos::release(id); } }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
struct Inhibitor;
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
async fn inhibitor_available() -> bool { false }
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
impl Inhibitor {
    async fn acquire() -> Result<Self, ErrorResponse> { Err(power_error("Stay awake is unsupported on this platform")) }
    async fn release(&mut self) -> Result<(), ErrorResponse> { Ok(()) }
}
