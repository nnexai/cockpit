use cockpit_host::transport::error::OperationError;
use cockpit_protocol::v1::ErrorResponse;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use tokio::{io::AsyncWriteExt, process::Command as TokioCommand};

#[cfg(any(target_os = "linux", target_os = "macos"))]
const CLIPBOARD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

#[tauri::command]
pub async fn cockpit_clipboard_write(text: String) -> Result<(), ErrorResponse> {
    #[cfg(target_os = "linux")]
    {
        write_linux_clipboard(&text).await
    }
    #[cfg(target_os = "macos")]
    {
        write_macos_clipboard(&text).await
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = text;
        Err(OperationError::rejected(
            "clipboard_unavailable",
            "The native clipboard adapter is unavailable on this platform",
        )
        .into())
    }
}

#[tauri::command]
pub async fn cockpit_clipboard_read() -> Result<String, ErrorResponse> {
    #[cfg(target_os = "linux")]
    {
        read_linux_clipboard().await
    }
    #[cfg(target_os = "macos")]
    {
        read_macos_clipboard().await
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Err(OperationError::rejected(
            "clipboard_unavailable",
            "The native clipboard adapter is unavailable on this platform",
        )
        .into())
    }
}

#[cfg(target_os = "linux")]
async fn write_linux_clipboard(text: &str) -> Result<(), ErrorResponse> {
    let mut process = TokioCommand::new("wl-copy")
        .stdin(std::process::Stdio::piped())
        .spawn()
        .map_err(|error| {
            ErrorResponse::from(OperationError::rejected(
                "clipboard_unavailable",
                format!("Could not start wl-copy: {error}"),
            ))
        })?;
    let result = tokio::time::timeout(CLIPBOARD_TIMEOUT, async {
        let mut stdin = process.stdin.take().ok_or_else(|| {
            ErrorResponse::from(OperationError::rejected(
                "clipboard_unavailable",
                "wl-copy stdin was unavailable",
            ))
        })?;
        stdin.write_all(text.as_bytes()).await.map_err(|error| {
            ErrorResponse::from(OperationError::rejected(
                "clipboard_write_failed",
                format!("Could not write the clipboard: {error}"),
            ))
        })?;
        drop(stdin);
        let status = process.wait().await.map_err(|error| {
            ErrorResponse::from(OperationError::rejected(
                "clipboard_write_failed",
                format!("Could not finish the clipboard write: {error}"),
            ))
        })?;
        if status.success() {
            Ok(())
        } else {
            Err(OperationError::rejected(
                "clipboard_write_failed",
                format!("wl-copy exited with {status}"),
            )
            .into())
        }
    })
    .await;
    match result {
        Ok(result) => result,
        Err(_) => {
            let _ = process.kill().await;
            Err(OperationError::rejected(
                "clipboard_write_timeout",
                "The native clipboard write exceeded 2 seconds",
            )
            .into())
        }
    }
}

#[cfg(target_os = "linux")]
async fn read_linux_clipboard() -> Result<String, ErrorResponse> {
    let output = tokio::time::timeout(
        CLIPBOARD_TIMEOUT,
        TokioCommand::new("wl-paste")
            .args(["--no-newline", "--type", "text/plain"])
            .output(),
    )
    .await
    .map_err(|_| {
        ErrorResponse::from(OperationError::rejected(
            "clipboard_read_timeout",
            "The native clipboard read exceeded 2 seconds",
        ))
    })?
    .map_err(|error| {
        ErrorResponse::from(OperationError::rejected(
            "clipboard_unavailable",
            format!("Could not start wl-paste: {error}"),
        ))
    })?;
    if output.status.success() {
        String::from_utf8(output.stdout).map_err(|error| {
            ErrorResponse::from(OperationError::rejected(
                "clipboard_read_failed",
                format!("The clipboard was not valid UTF-8: {error}"),
            ))
        })
    } else {
        Err(OperationError::rejected(
            "clipboard_read_failed",
            format!("wl-paste exited with {}", output.status),
        )
        .into())
    }
}

#[cfg(target_os = "macos")]
async fn write_macos_clipboard(text: &str) -> Result<(), ErrorResponse> {
    let mut process = TokioCommand::new("pbcopy")
        .stdin(std::process::Stdio::piped())
        .spawn()
        .map_err(|error| {
            ErrorResponse::from(OperationError::rejected(
                "clipboard_unavailable",
                format!("Could not start pbcopy: {error}"),
            ))
        })?;
    let result = tokio::time::timeout(CLIPBOARD_TIMEOUT, async {
        let mut stdin = process.stdin.take().ok_or_else(|| {
            ErrorResponse::from(OperationError::rejected(
                "clipboard_unavailable",
                "pbcopy stdin was unavailable",
            ))
        })?;
        stdin.write_all(text.as_bytes()).await.map_err(|error| {
            ErrorResponse::from(OperationError::rejected(
                "clipboard_write_failed",
                format!("Could not write the clipboard: {error}"),
            ))
        })?;
        drop(stdin);
        let status = process.wait().await.map_err(|error| {
            ErrorResponse::from(OperationError::rejected(
                "clipboard_write_failed",
                format!("Could not finish the clipboard write: {error}"),
            ))
        })?;
        if status.success() {
            Ok(())
        } else {
            Err(OperationError::rejected(
                "clipboard_write_failed",
                format!("pbcopy exited with {status}"),
            )
            .into())
        }
    })
    .await;
    match result {
        Ok(result) => result,
        Err(_) => {
            let _ = process.kill().await;
            Err(OperationError::rejected(
                "clipboard_write_timeout",
                "The native clipboard write exceeded 2 seconds",
            )
            .into())
        }
    }
}

#[cfg(target_os = "macos")]
async fn read_macos_clipboard() -> Result<String, ErrorResponse> {
    let output = tokio::time::timeout(CLIPBOARD_TIMEOUT, TokioCommand::new("pbpaste").output())
        .await
        .map_err(|_| {
            ErrorResponse::from(OperationError::rejected(
                "clipboard_read_timeout",
                "The native clipboard read exceeded 2 seconds",
            ))
        })?
        .map_err(|error| {
            ErrorResponse::from(OperationError::rejected(
                "clipboard_unavailable",
                format!("Could not start pbpaste: {error}"),
            ))
        })?;
    if output.status.success() {
        String::from_utf8(output.stdout).map_err(|error| {
            ErrorResponse::from(OperationError::rejected(
                "clipboard_read_failed",
                format!("The clipboard was not valid UTF-8: {error}"),
            ))
        })
    } else {
        Err(OperationError::rejected(
            "clipboard_read_failed",
            format!("pbpaste exited with {}", output.status),
        )
        .into())
    }
}

#[cfg(all(test, target_os = "linux"))]
mod clipboard_tests {
    use super::{read_linux_clipboard, write_linux_clipboard};

    #[tokio::test]
    #[ignore = "requires a run-owned Wayland display; run explicitly for native integration proof"]
    async fn native_clipboard_roundtrip_preserves_unicode_and_newlines() {
        let expected = "Cockpit αβγ\nline two\n✓ 终端";
        write_linux_clipboard(expected)
            .await
            .expect("wl-copy should accept text");
        let actual = read_linux_clipboard()
            .await
            .expect("wl-paste should return text");
        assert_eq!(actual, expected);
    }
}
