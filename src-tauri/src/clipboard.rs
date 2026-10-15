use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use cockpit_host::transport::error::OperationError;
use cockpit_protocol::v1::ErrorResponse;
use parking_lot::Mutex;
use serde::Deserialize;
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

fn clipboard_error(code: &str, message: impl Into<String>) -> ErrorResponse {
    OperationError::rejected(code, message).into()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalFile {
    name: String,
    bytes: Vec<u8>,
}

/// Owns only copies created by this app run, never clipboard source files.
struct RunDirectory(PathBuf);

impl Drop for RunDirectory {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.0) {
            eprintln!("Could not remove terminal input staging directory: {error}");
        }
    }
}

#[derive(Default)]
struct TerminalFilesInner {
    directory: Option<RunDirectory>,
    closed: bool,
}

#[derive(Default)]
pub struct TerminalFilesState(Mutex<TerminalFilesInner>);

impl TerminalFilesState {
    fn stage(&self, files: &[TerminalFile]) -> Result<String, ErrorResponse> {
        if files.is_empty() {
            return Err(clipboard_error(
                "terminal_files_invalid",
                "No files were supplied",
            ));
        }
        for file in files {
            validate_basename(&file.name)?;
        }
        let mut inner = self.0.lock();
        if inner.closed {
            return Err(clipboard_error(
                "terminal_files_failed",
                "Cockpit is shutting down",
            ));
        }
        if inner.directory.is_none() {
            let parent = std::env::temp_dir().canonicalize().map_err(|error| {
                clipboard_error(
                    "terminal_files_failed",
                    format!("Could not locate the temporary directory: {error}"),
                )
            })?;
            let root = parent.join(format!("cockpit-terminal-{}", uuid::Uuid::new_v4()));
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder.create(&root).map_err(|error| {
                clipboard_error(
                    "terminal_files_failed",
                    format!("Could not create terminal input staging: {error}"),
                )
            })?;
            // Never sweep another run's directories on restart. A crash may leave
            // copies in OS temp, but cannot cause deletion of live-run/original files.
            inner.directory = Some(RunDirectory(root));
        }
        let root = &inner
            .directory
            .as_ref()
            .expect("staging directory initialized")
            .0;
        let mut paths = String::new();
        for file in files {
            // A separate directory preserves duplicate original basenames safely.
            let directory = root.join(uuid::Uuid::new_v4().to_string());
            fs::create_dir(&directory).map_err(|error| {
                clipboard_error(
                    "terminal_files_failed",
                    format!("Could not stage the file: {error}"),
                )
            })?;
            let path = directory.join(&file.name);
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .map_err(|error| {
                    clipboard_error(
                        "terminal_files_failed",
                        format!("Could not stage the file: {error}"),
                    )
                })?;
            output.write_all(&file.bytes).map_err(|error| {
                clipboard_error(
                    "terminal_files_failed",
                    format!("Could not write the staged file: {error}"),
                )
            })?;
            append_quoted_path(&mut paths, &path)?;
        }
        Ok(paths)
    }

    // Tauri exits the process directly, so explicitly drop the owner on RunEvent::Exit.
    pub fn close(&self) {
        let directory = {
            let mut inner = self.0.lock();
            inner.closed = true;
            inner.directory.take()
        };
        drop(directory);
    }
}

fn validate_basename(name: &str) -> Result<(), ErrorResponse> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains(['/', '\\'])
        || name.chars().any(char::is_control)
    {
        return Err(clipboard_error(
            "terminal_files_invalid",
            "A staged file must have a filename without directory components or control characters",
        ));
    }
    Ok(())
}

fn quote_paths(paths: &[PathBuf]) -> Result<String, ErrorResponse> {
    let mut text = String::new();
    for path in paths {
        append_quoted_path(&mut text, path)?;
    }
    Ok(text)
}

fn append_quoted_path(output: &mut String, path: &Path) -> Result<(), ErrorResponse> {
    let text = path
        .to_str()
        .filter(|text| path.is_absolute() && !text.chars().any(char::is_control))
        .ok_or_else(|| {
            clipboard_error(
                "clipboard_read_failed",
                "The file path must be absolute UTF-8 without control characters",
            )
        })?;
    if !output.is_empty() {
        output.push(' ');
    }
    output.push('\'');
    for (index, part) in text.split('\'').enumerate() {
        if index != 0 {
            output.push_str("'\\''");
        }
        output.push_str(part);
    }
    output.push('\'');
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn parse_file_url(value: &str) -> Result<PathBuf, ErrorResponse> {
    url::Url::parse(value)
        .ok()
        .filter(|url| url.scheme() == "file" && url.query().is_none() && url.fragment().is_none())
        .and_then(|url| url.to_file_path().ok())
        .filter(|path| path.is_absolute())
        .ok_or_else(|| {
            clipboard_error(
                "clipboard_read_failed",
                "The clipboard contains an invalid local file URL",
            )
        })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
enum TerminalClipboard {
    Text(String),
    Files(Vec<PathBuf>),
    Png(Vec<u8>),
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn terminal_clipboard_text(
    clipboard: TerminalClipboard,
    files: &TerminalFilesState,
) -> Result<String, ErrorResponse> {
    match clipboard {
        TerminalClipboard::Text(text) => Ok(text),
        TerminalClipboard::Files(paths) if !paths.is_empty() => quote_paths(&paths),
        TerminalClipboard::Files(_) => Err(clipboard_error(
            "clipboard_read_failed",
            "The clipboard file list is empty",
        )),
        TerminalClipboard::Png(bytes) if !bytes.is_empty() => files.stage(&[TerminalFile {
            name: "clipboard.png".into(),
            bytes,
        }]),
        TerminalClipboard::Png(_) => Err(clipboard_error(
            "clipboard_read_failed",
            "The clipboard image is empty",
        )),
    }
}

#[tauri::command]
pub async fn cockpit_terminal_files_stage(
    files: Vec<TerminalFile>,
    state: tauri::State<'_, TerminalFilesState>,
) -> Result<String, ErrorResponse> {
    state.stage(&files)
}

/// Rich terminal paste is intentionally separate from ClipboardAccess.readText.
#[tauri::command]
pub async fn cockpit_clipboard_read_terminal<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: tauri::State<'_, TerminalFilesState>,
) -> Result<String, ErrorResponse> {
    #[cfg(target_os = "linux")]
    {
        let _ = app;
        terminal_clipboard_text(read_linux_terminal_clipboard().await?, &state)
    }
    #[cfg(target_os = "macos")]
    {
        let (send, receive) = tokio::sync::oneshot::channel();
        app.run_on_main_thread(move || {
            let _ = send.send(read_macos_terminal_clipboard());
        })
        .map_err(|error| {
            clipboard_error(
                "clipboard_unavailable",
                format!("Could not access the native clipboard: {error}"),
            )
        })?;
        let clipboard = receive.await.map_err(|_| {
            clipboard_error(
                "clipboard_unavailable",
                "The native clipboard read was interrupted",
            )
        })??;
        terminal_clipboard_text(clipboard, &state)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (app, state);
        Err(clipboard_error(
            "clipboard_unavailable",
            "The native clipboard adapter is unavailable on this platform",
        ))
    }
}

#[cfg(target_os = "linux")]
async fn linux_clipboard_output(args: &[&str]) -> Result<Vec<u8>, ErrorResponse> {
    let output = tokio::time::timeout(
        CLIPBOARD_TIMEOUT,
        TokioCommand::new("wl-paste")
            .args(args)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| {
        clipboard_error(
            "clipboard_read_timeout",
            "The native clipboard read exceeded 2 seconds",
        )
    })?
    .map_err(|error| {
        clipboard_error(
            "clipboard_unavailable",
            format!("Could not start wl-paste: {error}"),
        )
    })?;
    if !output.status.success() {
        return Err(clipboard_error(
            "clipboard_read_failed",
            format!("wl-paste exited with {}", output.status),
        ));
    }
    Ok(output.stdout)
}

#[cfg(target_os = "linux")]
fn linux_terminal_type(types: &str) -> Option<&'static str> {
    if types.lines().any(|mime| mime == "text/uri-list") {
        Some("text/uri-list")
    } else if types.lines().any(|mime| mime == "image/png") {
        Some("image/png")
    } else {
        None
    }
}

#[cfg(target_os = "linux")]
async fn read_linux_terminal_clipboard() -> Result<TerminalClipboard, ErrorResponse> {
    let types = linux_clipboard_output(&["--list-types"]).await?;
    let types = String::from_utf8(types).map_err(|error| {
        clipboard_error(
            "clipboard_read_failed",
            format!("The clipboard types were not valid UTF-8: {error}"),
        )
    })?;
    match linux_terminal_type(&types) {
        Some("text/uri-list") => {
            let bytes =
                linux_clipboard_output(&["--no-newline", "--type", "text/uri-list"]).await?;
            let list = String::from_utf8(bytes).map_err(|error| {
                clipboard_error(
                    "clipboard_read_failed",
                    format!("The clipboard file list was not valid UTF-8: {error}"),
                )
            })?;
            let paths = list
                .lines()
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
                .map(parse_file_url)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(TerminalClipboard::Files(paths))
        }
        Some("image/png") => {
            let bytes = linux_clipboard_output(&["--no-newline", "--type", "image/png"]).await?;
            Ok(TerminalClipboard::Png(bytes))
        }
        _ => read_linux_clipboard().await.map(TerminalClipboard::Text),
    }
}

#[cfg(target_os = "macos")]
fn read_macos_terminal_clipboard() -> Result<TerminalClipboard, ErrorResponse> {
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSPasteboard};
    use objc2_foundation::{NSDictionary, NSString};

    objc2::rc::autoreleasepool(|_| {
        let board = NSPasteboard::generalPasteboard();
        let types = board.types();
        let file_type = NSString::from_str("public.file-url");
        let png_type = NSString::from_str("public.png");
        let tiff_type = NSString::from_str("public.tiff");
        let has_type = |kind: &NSString| {
            types
                .as_ref()
                .is_some_and(|types| types.containsObject(kind))
        };
        if has_type(&file_type) {
            let items = board.pasteboardItems().ok_or_else(|| {
                clipboard_error(
                    "clipboard_read_failed",
                    "Could not read the clipboard files",
                )
            })?;
            let mut paths = Vec::new();
            for index in 0..items.count() {
                let item = items.objectAtIndex(index);
                if item.types().containsObject(&file_type) {
                    let value = item.stringForType(&file_type).ok_or_else(|| {
                        clipboard_error(
                            "clipboard_read_failed",
                            "Could not read a clipboard file URL",
                        )
                    })?;
                    paths.push(parse_file_url(&value.to_string())?);
                }
            }
            return Ok(TerminalClipboard::Files(paths));
        }
        if has_type(&png_type) {
            let data = board.dataForType(&png_type).ok_or_else(|| {
                clipboard_error(
                    "clipboard_read_failed",
                    "Could not read the clipboard image",
                )
            })?;
            return Ok(TerminalClipboard::Png(data.to_vec()));
        }
        if has_type(&tiff_type) {
            let data = board.dataForType(&tiff_type).ok_or_else(|| {
                clipboard_error(
                    "clipboard_read_failed",
                    "Could not read the clipboard image",
                )
            })?;
            let image = NSBitmapImageRep::imageRepWithData(&data).ok_or_else(|| {
                clipboard_error(
                    "clipboard_read_failed",
                    "Could not decode the clipboard image",
                )
            })?;
            // SAFETY: An empty dictionary contains no incorrectly typed properties.
            let png = unsafe {
                image.representationUsingType_properties(
                    NSBitmapImageFileType::PNG,
                    &NSDictionary::new(),
                )
            }
            .ok_or_else(|| {
                clipboard_error(
                    "clipboard_read_failed",
                    "Could not convert the clipboard image to PNG",
                )
            })?;
            return Ok(TerminalClipboard::Png(png.to_vec()));
        }
        let text_type = NSString::from_str("public.utf8-plain-text");
        if let Some(text) = board.stringForType(&text_type) {
            return Ok(TerminalClipboard::Text(text.to_string()));
        }
        if types.as_ref().is_none_or(|types| types.count() == 0) {
            return Ok(TerminalClipboard::Text(String::new()));
        }
        Err(clipboard_error(
            "clipboard_read_failed",
            "The clipboard does not contain readable files, PNG/TIFF images, or text",
        ))
    })
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
    let bytes = linux_clipboard_output(&["--no-newline", "--type", "text/plain"]).await?;
    String::from_utf8(bytes).map_err(|error| {
        clipboard_error(
            "clipboard_read_failed",
            format!("The clipboard was not valid UTF-8: {error}"),
        )
    })
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

#[cfg(test)]
mod terminal_input_tests {
    use super::*;

    #[test]
    fn quotes_absolute_paths_without_executing_them() {
        assert_eq!(
            quote_paths(&[
                PathBuf::from("/tmp/O'Brien image.png"),
                PathBuf::from("/tmp/终端.txt")
            ])
            .unwrap(),
            "'/tmp/O'\\''Brien image.png' '/tmp/终端.txt'"
        );
        assert!(quote_paths(&[PathBuf::from("relative.txt")]).is_err());
        assert!(quote_paths(&[PathBuf::from("/tmp/nul\0name")]).is_err());
    }

    #[test]
    fn rejects_untrusted_directory_components() {
        for name in [
            "",
            ".",
            "..",
            "../escape",
            "/absolute",
            "a/b",
            "a\\b",
            "nul\0name",
        ] {
            assert!(validate_basename(name).is_err(), "{name:?}");
        }
        assert!(validate_basename("O'Brien 终端.png").is_ok());
    }

    #[test]
    fn terminal_paths_reject_controls_before_appending_or_staging() {
        let state = TerminalFilesState::default();
        for control in (0..=0x9f)
            .filter_map(char::from_u32)
            .filter(|value| value.is_control())
        {
            let name = format!("before{control}after.txt");
            let path = PathBuf::from(format!("/tmp/{name}"));
            let mut output = "'/tmp/safe.txt'".to_string();
            assert!(
                append_quoted_path(&mut output, &path).is_err(),
                "{control:?}"
            );
            assert_eq!(output, "'/tmp/safe.txt'");
            assert!(
                state
                    .stage(&[TerminalFile {
                        name,
                        bytes: Vec::new(),
                    }])
                    .is_err(),
                "{control:?}"
            );
        }
        assert!(state.0.lock().directory.is_none());
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn clipboard_file_urls_cannot_paste_terminal_controls() {
        let state = TerminalFilesState::default();
        for encoded in ["%00", "%09", "%0A", "%0D", "%1B", "%7F", "%C2%85", "%C2%9B"] {
            let path = parse_file_url(&format!("file:///tmp/before{encoded}after.txt")).unwrap();
            assert!(
                terminal_clipboard_text(TerminalClipboard::Files(vec![path]), &state).is_err(),
                "{encoded}"
            );
        }
    }

    #[test]
    fn staging_preserves_duplicate_basenames_and_cleans_only_owned_copies() {
        let state = TerminalFilesState::default();
        let text = state
            .stage(&[
                TerminalFile {
                    name: "same name.txt".into(),
                    bytes: b"first".to_vec(),
                },
                TerminalFile {
                    name: "same name.txt".into(),
                    bytes: b"second".to_vec(),
                },
            ])
            .unwrap();
        assert!(!text.ends_with('\n'));
        let root = state.0.lock().directory.as_ref().unwrap().0.clone();
        let mut contents = fs::read_dir(&root)
            .unwrap()
            .map(|entry| {
                let file = entry.unwrap().path().join("same name.txt");
                fs::read(file).unwrap()
            })
            .collect::<Vec<_>>();
        contents.sort();
        assert_eq!(contents, [b"first".to_vec(), b"second".to_vec()]);
        state.close();
        assert!(!root.exists());
        assert!(
            state
                .stage(&[TerminalFile {
                    name: "later.txt".into(),
                    bytes: Vec::new()
                }])
                .is_err()
        );
    }

    #[test]
    fn traversal_is_rejected_before_creating_a_directory() {
        let state = TerminalFilesState::default();
        assert!(
            state
                .stage(&[TerminalFile {
                    name: "../escape".into(),
                    bytes: vec![1]
                }])
                .is_err()
        );
        assert!(state.0.lock().directory.is_none());
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn empty_text_is_valid_but_empty_native_files_and_images_are_errors() {
        let state = TerminalFilesState::default();
        assert_eq!(
            terminal_clipboard_text(TerminalClipboard::Text(String::new()), &state).unwrap(),
            ""
        );
        assert!(terminal_clipboard_text(TerminalClipboard::Files(Vec::new()), &state).is_err());
        assert!(terminal_clipboard_text(TerminalClipboard::Png(Vec::new()), &state).is_err());
        assert!(state.0.lock().directory.is_none());
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn accepts_only_local_absolute_file_urls() {
        assert_eq!(
            parse_file_url("file:///tmp/O%27Brien%20image.png").unwrap(),
            PathBuf::from("/tmp/O'Brien image.png")
        );
        for value in [
            "https://example.com/file",
            "file://remote/tmp/file",
            "file:///tmp/file?query",
            "not a URL",
        ] {
            assert!(parse_file_url(value).is_err(), "{value}");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn terminal_prefers_files_then_images_even_with_text_available() {
        assert_eq!(
            linux_terminal_type("text/plain\nimage/png\ntext/uri-list\n"),
            Some("text/uri-list")
        );
        assert_eq!(
            linux_terminal_type("text/plain\nimage/png\n"),
            Some("image/png")
        );
        assert_eq!(linux_terminal_type("text/plain\n"), None);
    }
}
