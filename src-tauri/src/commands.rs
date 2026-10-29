use cockpit_core::CockpitService;
use cockpit_host::{BrowserRuntime, OrchestrationRuntime, transport::Transport};
use cockpit_protocol::v1::ErrorResponse;
use std::sync::Arc;
use tauri::State;

macro_rules! native_command {
    (service, $command:ident, ($($arg:ident: $ty:ty),*), $out:ty, $call:path, $input:expr) => {
        #[tauri::command]
        pub async fn $command(
            $($arg: $ty,)*
            service: State<'_, CockpitService>,
        ) -> Result<$out, ErrorResponse> {
            let input = ($input)(($($arg,)* )).map_err(ErrorResponse::from)?;
            $call(service.inner(), Transport::Native, input)
                .await
                .map_err(ErrorResponse::from)
        }
    };
    (browser, $command:ident, ($($arg:ident: $ty:ty),*), $out:ty, $call:path, $input:expr) => {
        #[tauri::command]
        pub async fn $command(
            $($arg: $ty,)*
            runtime: State<'_, Arc<BrowserRuntime>>,
        ) -> Result<$out, ErrorResponse> {
            let input = ($input)(($($arg,)* )).map_err(ErrorResponse::from)?;
            $call(runtime.inner(), Transport::Native, input)
                .await
                .map_err(ErrorResponse::from)
        }
    };
    (orchestration, $command:ident, ($($arg:ident: $ty:ty),*), $out:ty, $call:path, $input:expr) => {
        #[tauri::command]
        pub async fn $command(
            $($arg: $ty,)*
            runtime: State<'_, Arc<OrchestrationRuntime>>,
        ) -> Result<$out, ErrorResponse> {
            let input = ($input)(($($arg,)* )).map_err(ErrorResponse::from)?;
            $call(runtime.inner(), Transport::Native, input)
                .await
                .map_err(ErrorResponse::from)
        }
    };
}

macro_rules! native_commands {
    ($({
        id: $id:ident, ctx: $ctx:ident, out: $out:ty, call: $call:path,
        native: $command:ident ($($arg:ident: $ty:ty),*) => $input:expr,
        http: $($http:tt)*
    })*) => {
        $(native_command!($ctx, $command, ($($arg: $ty),*), $out, $call, $input);)*

        pub fn invoke_handler<R: tauri::Runtime>()
            -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static
        {
            tauri::generate_handler![
                $($command,)*
                crate::power::cockpit_power_status,
                crate::power::cockpit_power_set_stay_awake,
                crate::power::cockpit_power_screen_off,
                crate::streams::cockpit_session_subscribe,
                crate::streams::cockpit_scratch_page_generation,
                crate::streams::cockpit_terminal_open,
                crate::streams::cockpit_terminal_command,
                crate::streams::cockpit_stream_cancel,
                crate::streams::cockpit_browser_view_subscribe,
                crate::streams::cockpit_widget_subscribe,
                crate::streams::cockpit_widget_report,
                crate::clipboard::cockpit_clipboard_read,
                crate::clipboard::cockpit_clipboard_read_terminal,
                crate::clipboard::cockpit_terminal_files_stage,
                crate::clipboard::cockpit_clipboard_write,
            ]
        }
    };
}

cockpit_host::cockpit_operations!(native_commands);

#[cfg(test)]
mod tests {
    #[test]
    fn liaison_lookup_is_granted_only_to_the_main_window() {
        let capability: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities/default.json")).unwrap();
        assert_eq!(capability["windows"], serde_json::json!(["main"]));
        assert!(capability.get("webviews").is_none());
        let permissions = capability["permissions"].as_array().unwrap();
        assert_eq!(
            permissions
                .iter()
                .filter(|permission| {
                    permission.as_str() == Some("allow-orchestration-liaison-lookup")
                })
                .count(),
            1
        );
    }
}
