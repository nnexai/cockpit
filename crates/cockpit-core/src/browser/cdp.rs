use serde_json::Value;
use std::{path::Path, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

use super::BrowserService;
use super::process::SOCKET_TIMEOUT;
use super::receipts::{BrowserReceipt, read_regular};
use crate::InspectionError;

impl BrowserService {
    pub(super) async fn bind_inline_cdp(
        &self,
        receipt: &mut BrowserReceipt,
        allow_unrecorded: bool,
    ) -> Result<(), InspectionError> {
        let binding = wait_for_cdp_binding(Path::new(&receipt.profile_path)).await?;
        if (receipt.cdp_endpoint.is_none() || receipt.cdp_browser_identity.is_none())
            && !allow_unrecorded
        {
            return Err(InspectionError::new(
                "browser_cdp_unavailable",
                "Inline CDP ownership was not recorded for this browser incarnation",
            ));
        }
        if receipt
            .cdp_endpoint
            .as_deref()
            .is_some_and(|endpoint| endpoint != binding.endpoint)
            || receipt
                .cdp_browser_identity
                .as_deref()
                .is_some_and(|identity| identity != binding.browser_identity)
        {
            return Err(InspectionError::new(
                "browser_receipt_replaced",
                "Chromium CDP endpoint or browser identity changed",
            ));
        }
        receipt.cdp_endpoint = Some(binding.endpoint);
        receipt.cdp_browser_identity = Some(binding.browser_identity);
        Ok(())
    }

    pub(super) async fn resolve_inline_target(
        &self,
        receipt: &mut BrowserReceipt,
        previous: Option<&str>,
    ) -> Result<(), InspectionError> {
        let endpoint = receipt.cdp_endpoint.as_deref().ok_or_else(|| {
            InspectionError::new("browser_cdp_unavailable", "Inline CDP endpoint is absent")
        })?;
        let target_id = cdp_page_target(endpoint, previous).await?;
        receipt.target_id = Some(target_id);
        Ok(())
    }
}

#[derive(Debug)]
struct CdpBinding {
    endpoint: String,
    browser_identity: String,
}

async fn wait_for_cdp_binding(profile: &Path) -> Result<CdpBinding, InspectionError> {
    let deadline = tokio::time::Instant::now() + SOCKET_TIMEOUT;
    loop {
        match cdp_binding(profile).await {
            Ok(binding) => return Ok(binding),
            Err(error) if error.code == "browser_cdp_unavailable" => {
                if tokio::time::Instant::now() >= deadline {
                    return Err(error);
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            Err(error) => return Err(error),
        }
    }
}

pub(super) fn cdp_record(profile: &Path) -> Result<(u16, String), InspectionError> {
    let port_file = profile.join("DevToolsActivePort");
    let bytes = read_regular(&port_file, 512).map_err(|_| {
        InspectionError::new(
            "browser_cdp_unavailable",
            "Chromium did not publish a bounded DevToolsActivePort record",
        )
    })?;
    let record = std::str::from_utf8(&bytes).map_err(|_| {
        InspectionError::new(
            "browser_cdp_unavailable",
            "Chromium DevToolsActivePort record is not UTF-8",
        )
    })?;
    let mut lines = record.lines();
    let port = lines
        .next()
        .and_then(|value| value.parse::<u16>().ok())
        .filter(|port| *port != 0)
        .ok_or_else(|| {
            InspectionError::new(
                "browser_cdp_unavailable",
                "Chromium DevToolsActivePort does not contain a valid port",
            )
        })?;
    let browser_path = lines
        .next()
        .filter(|path| path.starts_with("/devtools/browser/") && path.len() <= 512)
        .ok_or_else(|| {
            InspectionError::new(
                "browser_cdp_unavailable",
                "Chromium DevToolsActivePort does not contain a browser identity",
            )
        })?
        .to_owned();
    if lines.next().is_some() {
        return Err(InspectionError::new(
            "browser_cdp_unavailable",
            "Chromium DevToolsActivePort record has unexpected fields",
        ));
    }
    Ok((port, browser_path))
}

async fn cdp_binding(profile: &Path) -> Result<CdpBinding, InspectionError> {
    let (port, browser_path) = cdp_record(profile)?;
    let endpoint = format!("http://127.0.0.1:{port}");
    let mut stream = tokio::time::timeout(SOCKET_TIMEOUT, TcpStream::connect(("127.0.0.1", port)))
        .await
        .map_err(|_| InspectionError::new("browser_cdp_unavailable", "Chromium CDP timed out"))?
        .map_err(|_| {
            InspectionError::new("browser_cdp_unavailable", "Chromium CDP is not reachable")
        })?;
    let request = format!(
        "GET /json/version HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    );
    tokio::time::timeout(SOCKET_TIMEOUT, stream.write_all(request.as_bytes()))
        .await
        .map_err(|_| {
            InspectionError::new("browser_cdp_unavailable", "Chromium CDP write timed out")
        })?
        .map_err(|_| {
            InspectionError::new("browser_cdp_unavailable", "Chromium CDP write failed")
        })?;
    let mut response = Vec::with_capacity(2048);
    let mut buffer = [0_u8; 4096];
    let header_end = loop {
        let count = tokio::time::timeout(SOCKET_TIMEOUT, stream.read(&mut buffer))
            .await
            .map_err(|_| {
                InspectionError::new("browser_cdp_unavailable", "Chromium CDP read timed out")
            })?
            .map_err(|_| {
                InspectionError::new("browser_cdp_unavailable", "Chromium CDP read failed")
            })?;
        if count == 0 {
            return Err(InspectionError::new(
                "browser_cdp_unavailable",
                "Chromium CDP returned truncated HTTP",
            ));
        }
        if response.len() + count > 64 * 1024 {
            return Err(InspectionError::new(
                "browser_cdp_unavailable",
                "Chromium CDP version response exceeds bounds",
            ));
        }
        response.extend_from_slice(&buffer[..count]);
        if let Some(position) = response.windows(4).position(|window| window == b"\r\n\r\n") {
            break position;
        }
    };
    let headers = std::str::from_utf8(&response[..header_end]).map_err(|_| {
        InspectionError::new(
            "browser_cdp_unavailable",
            "Chromium CDP returned invalid HTTP headers",
        )
    })?;
    let content_length = headers
        .lines()
        .find_map(|line| {
            line.strip_prefix("Content-Length:")
                .or_else(|| line.strip_prefix("content-length:"))
        })
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|length| *length <= 64 * 1024)
        .ok_or_else(|| {
            InspectionError::new(
                "browser_cdp_unavailable",
                "Chromium CDP response lacks a bounded Content-Length",
            )
        })?;
    let body_start = header_end + 4;
    while response.len() < body_start + content_length {
        let count = tokio::time::timeout(SOCKET_TIMEOUT, stream.read(&mut buffer))
            .await
            .map_err(|_| {
                InspectionError::new(
                    "browser_cdp_unavailable",
                    "Chromium CDP body read timed out",
                )
            })?
            .map_err(|_| {
                InspectionError::new("browser_cdp_unavailable", "Chromium CDP body read failed")
            })?;
        if count == 0 {
            return Err(InspectionError::new(
                "browser_cdp_unavailable",
                "Chromium CDP returned truncated body",
            ));
        }
        if response.len() + count > body_start + content_length {
            return Err(InspectionError::new(
                "browser_cdp_unavailable",
                "Chromium CDP body exceeds Content-Length",
            ));
        }
        response.extend_from_slice(&buffer[..count]);
    }
    let body = &response[body_start..body_start + content_length];
    let value: Value = serde_json::from_slice(body).map_err(|_| {
        InspectionError::new(
            "browser_cdp_unavailable",
            "Chromium CDP returned invalid JSON",
        )
    })?;
    let websocket = value
        .get("webSocketDebuggerUrl")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            InspectionError::new(
                "browser_cdp_unavailable",
                "Chromium CDP version response lacks browser websocket identity",
            )
        })?;
    let websocket = url::Url::parse(websocket).map_err(|_| {
        InspectionError::new(
            "browser_cdp_unavailable",
            "Chromium CDP websocket URL is invalid",
        )
    })?;
    if websocket.scheme() != "ws"
        || websocket.host_str() != Some("127.0.0.1")
        || websocket.port() != Some(port)
        || websocket.path() != browser_path
    {
        return Err(InspectionError::new(
            "browser_cdp_unavailable",
            "Chromium CDP websocket identity is not the loopback profile binding",
        ));
    }
    Ok(CdpBinding {
        endpoint,
        browser_identity: browser_path,
    })
}

async fn cdp_page_targets(endpoint: &str) -> Result<Vec<Value>, InspectionError> {
    let url = url::Url::parse(endpoint).map_err(|_| {
        InspectionError::new("browser_cdp_unavailable", "Inline CDP endpoint is invalid")
    })?;
    if url.scheme() != "http" || url.host_str() != Some("127.0.0.1") {
        return Err(InspectionError::new(
            "browser_cdp_unavailable",
            "Inline CDP endpoint is not loopback",
        ));
    }
    let port = url.port().ok_or_else(|| {
        InspectionError::new("browser_cdp_unavailable", "Inline CDP endpoint has no port")
    })?;
    let mut stream = tokio::time::timeout(SOCKET_TIMEOUT, TcpStream::connect(("127.0.0.1", port)))
        .await
        .map_err(|_| {
            InspectionError::new(
                "browser_cdp_unavailable",
                "Chromium CDP target listing timed out",
            )
        })?
        .map_err(|_| {
            InspectionError::new(
                "browser_cdp_unavailable",
                "Chromium CDP target listing is unreachable",
            )
        })?;
    let request =
        format!("GET /json/list HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
    tokio::time::timeout(SOCKET_TIMEOUT, stream.write_all(request.as_bytes()))
        .await
        .map_err(|_| {
            InspectionError::new(
                "browser_cdp_unavailable",
                "Chromium CDP target-list write timed out",
            )
        })?
        .map_err(|_| {
            InspectionError::new(
                "browser_cdp_unavailable",
                "Chromium CDP target-list write failed",
            )
        })?;

    let mut response = Vec::with_capacity(4096);
    let mut buffer = [0_u8; 4096];
    let header_end = loop {
        let count = tokio::time::timeout(SOCKET_TIMEOUT, stream.read(&mut buffer))
            .await
            .map_err(|_| {
                InspectionError::new(
                    "browser_cdp_unavailable",
                    "Chromium CDP target-list read timed out",
                )
            })?
            .map_err(|_| {
                InspectionError::new(
                    "browser_cdp_unavailable",
                    "Chromium CDP target-list read failed",
                )
            })?;
        if count == 0 {
            return Err(InspectionError::new(
                "browser_cdp_unavailable",
                "Chromium CDP returned truncated target-list HTTP",
            ));
        }
        if response.len() + count > 64 * 1024 {
            return Err(InspectionError::new(
                "browser_cdp_unavailable",
                "Chromium CDP target-list response exceeds bounds",
            ));
        }
        response.extend_from_slice(&buffer[..count]);
        if let Some(position) = response.windows(4).position(|window| window == b"\r\n\r\n") {
            break position;
        }
    };
    let headers = std::str::from_utf8(&response[..header_end]).map_err(|_| {
        InspectionError::new(
            "browser_cdp_unavailable",
            "Chromium CDP target-list returned invalid HTTP headers",
        )
    })?;
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case("content-length")
                .then_some(value.trim())
        })
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|length| *length <= 256 * 1024)
        .ok_or_else(|| {
            InspectionError::new(
                "browser_cdp_unavailable",
                "Chromium CDP target-list response lacks a bounded Content-Length",
            )
        })?;
    let body_start = header_end + 4;
    while response.len() < body_start + content_length {
        let count = tokio::time::timeout(SOCKET_TIMEOUT, stream.read(&mut buffer))
            .await
            .map_err(|_| {
                InspectionError::new(
                    "browser_cdp_unavailable",
                    "Chromium CDP target-list body read timed out",
                )
            })?
            .map_err(|_| {
                InspectionError::new(
                    "browser_cdp_unavailable",
                    "Chromium CDP target-list body read failed",
                )
            })?;
        if count == 0 {
            return Err(InspectionError::new(
                "browser_cdp_unavailable",
                "Chromium CDP returned truncated target-list body",
            ));
        }
        if response.len() + count > body_start + content_length {
            return Err(InspectionError::new(
                "browser_cdp_unavailable",
                "Chromium CDP target-list body exceeds Content-Length",
            ));
        }
        response.extend_from_slice(&buffer[..count]);
    }
    let body = &response[body_start..body_start + content_length];
    let targets: Vec<Value> = serde_json::from_slice(body).map_err(|_| {
        InspectionError::new(
            "browser_cdp_unavailable",
            "Chromium CDP target list is invalid",
        )
    })?;
    Ok(targets)
}

async fn cdp_page_target(
    endpoint: &str,
    previous: Option<&str>,
) -> Result<String, InspectionError> {
    let targets = cdp_page_targets(endpoint).await?;
    let pages: Vec<String> = targets
        .into_iter()
        .filter(|target| target.get("type").and_then(Value::as_str) == Some("page"))
        .filter_map(|target| target.get("id").and_then(Value::as_str).map(str::to_owned))
        .collect();
    let selected: Vec<String> = match previous {
        Some(previous) => pages
            .into_iter()
            .filter(|target| target != previous)
            .collect(),
        None => pages,
    };
    if selected.len() != 1 {
        return Err(InspectionError::new(
            "browser_target_unresolved",
            "CDP could not identify one stable page target for this browser operation",
        ));
    }
    Ok(selected.into_iter().next().expect("one target"))
}
