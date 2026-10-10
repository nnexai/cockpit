use std::net::SocketAddr;

use cockpit_core::InspectionError;
use cockpit_protocol::browser_view::{
    BROWSER_VIEW_FRAME_MAX_HEIGHT, BROWSER_VIEW_FRAME_MAX_JPEG_BYTES, BROWSER_VIEW_FRAME_MAX_WIDTH,
    BROWSER_VIEW_FRAME_V2_HEADER_BYTES, BROWSER_VIEW_FRAME_V2_MAGIC, BROWSER_VIEW_FRAME_V2_VERSION,
    BrowserViewFrameDescriptor,
};
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{Message as FrameMessage, client::IntoClientRequest, protocol::WebSocketConfig},
};

pub const MAX_FRAME: usize =
    BROWSER_VIEW_FRAME_V2_HEADER_BYTES as usize + BROWSER_VIEW_FRAME_MAX_JPEG_BYTES as usize;

#[derive(Debug)]
pub struct FramePacket {
    pub descriptor: BrowserViewFrameDescriptor,
    pub jpeg: Vec<u8>,
}

pub fn decode_frame(
    payload: &[u8],
    target_id: &str,
    expected_epoch: u64,
) -> Result<FramePacket, InspectionError> {
    let header_len = BROWSER_VIEW_FRAME_V2_HEADER_BYTES as usize;
    if payload.len() < header_len {
        return Err(InspectionError::new(
            "browser_frame_invalid",
            "Browser frame header is truncated",
        ));
    }
    let read_u16 =
        |offset: usize| u16::from_be_bytes(payload[offset..offset + 2].try_into().unwrap());
    let read_u32 =
        |offset: usize| u32::from_be_bytes(payload[offset..offset + 4].try_into().unwrap());
    let read_u64 =
        |offset: usize| u64::from_be_bytes(payload[offset..offset + 8].try_into().unwrap());
    if read_u32(0) != BROWSER_VIEW_FRAME_V2_MAGIC
        || read_u16(4) != BROWSER_VIEW_FRAME_V2_VERSION
        || read_u16(6) != BROWSER_VIEW_FRAME_V2_HEADER_BYTES
    {
        return Err(InspectionError::new(
            "browser_frame_invalid",
            "Browser frame envelope is not IBFV v2",
        ));
    }

    let epoch = read_u64(8);
    let frame_sequence = read_u64(16);
    let document_generation = read_u64(24);
    let viewport_revision = read_u64(32);
    let jpeg_len = read_u32(80) as usize;
    if epoch != expected_epoch
        || frame_sequence == 0
        || document_generation == 0
        || viewport_revision == 0
        || jpeg_len == 0
        || jpeg_len > BROWSER_VIEW_FRAME_MAX_JPEG_BYTES as usize
        || payload.len() != header_len + jpeg_len
    {
        return Err(InspectionError::new(
            "browser_frame_invalid",
            "Browser frame identity, sequence, generation, or length is invalid",
        ));
    }
    if read_u32(84) != 0 || payload[88..header_len].iter().any(|byte| *byte != 0) {
        return Err(InspectionError::new(
            "browser_frame_invalid",
            "Browser frame flags or reserved bytes are non-zero",
        ));
    }
    if target_id.is_empty() {
        return Err(InspectionError::new(
            "browser_frame_invalid",
            "Browser frame target identity is unavailable",
        ));
    }

    let viewport_css_width = f32::from_bits(read_u32(48));
    let viewport_css_height = f32::from_bits(read_u32(52));
    let viewport_offset_x = f32::from_bits(read_u32(56));
    let viewport_offset_y = f32::from_bits(read_u32(60));
    let scroll_x = f32::from_bits(read_u32(64));
    let scroll_y = f32::from_bits(read_u32(68));
    if !viewport_css_width.is_finite()
        || !viewport_css_height.is_finite()
        || !viewport_offset_x.is_finite()
        || !viewport_offset_y.is_finite()
        || !scroll_x.is_finite()
        || !scroll_y.is_finite()
        || viewport_css_width <= 0.0
        || viewport_css_height <= 0.0
        || viewport_css_width > BROWSER_VIEW_FRAME_MAX_WIDTH as f32
        || viewport_css_height > BROWSER_VIEW_FRAME_MAX_HEIGHT as f32
    {
        return Err(InspectionError::new(
            "browser_frame_invalid",
            "Browser frame geometry is non-finite or outside bounds",
        ));
    }

    let image_width = read_u32(40);
    let image_height = read_u32(44);
    let jpeg = &payload[header_len..];
    if jpeg.len() < 2
        || jpeg[0] != 0xff
        || jpeg[1] != 0xd8
        || jpeg[jpeg.len() - 2] != 0xff
        || jpeg[jpeg.len() - 1] != 0xd9
    {
        return Err(InspectionError::new(
            "browser_frame_invalid",
            "Browser frame payload is not a complete JPEG",
        ));
    }
    let descriptor = BrowserViewFrameDescriptor {
        target_id: target_id.to_owned(),
        stream_epoch: epoch,
        frame_sequence,
        document_generation,
        viewport_revision,
        image_width,
        image_height,
        viewport_css_width: viewport_css_width as f64,
        viewport_css_height: viewport_css_height as f64,
        viewport_offset_x: viewport_offset_x as f64,
        viewport_offset_y: viewport_offset_y as f64,
        scroll_x: scroll_x as f64,
        scroll_y: scroll_y as f64,
        capture_timestamp_micros: read_u64(72),
        jpeg_length: jpeg_len as u32,
    };
    descriptor
        .validate()
        .map_err(|message| InspectionError::new("browser_frame_invalid", message))?;
    Ok(FramePacket {
        descriptor,
        jpeg: jpeg.to_vec(),
    })
}

pub struct FrameConnection {
    stream: WebSocketStream<TcpStream>,
}
impl FrameConnection {
    pub async fn send_credit(
        &mut self,
        kind: &str,
        frame_sequence: u64,
    ) -> Result<(), InspectionError> {
        self.stream
            .send(FrameMessage::Text(
                serde_json::json!({"type": kind, "frame_sequence": frame_sequence})
                    .to_string()
                    .into(),
            ))
            .await
            .map_err(|_| frame_error("Could not return browser frame credit"))
    }

    pub async fn connect(endpoint: &str, grant: &str) -> Result<Self, InspectionError> {
        let authority_path = endpoint
            .strip_prefix("ws://")
            .ok_or_else(|| frame_error("Browser frame endpoint is not loopback WebSocket"))?;
        let authority = authority_path.split('/').next().unwrap_or_default();
        let address: SocketAddr = authority
            .parse()
            .map_err(|_| frame_error("Browser frame endpoint is invalid"))?;
        if !address.ip().is_loopback() {
            return Err(frame_error("Browser frame endpoint is not loopback"));
        }
        let mut request = endpoint
            .into_client_request()
            .map_err(|_| frame_error("Browser frame endpoint is invalid"))?;
        request.headers_mut().insert(
            "Origin",
            format!("http://{authority}")
                .parse()
                .map_err(|_| frame_error("Browser frame origin is invalid"))?,
        );
        let config = WebSocketConfig::default()
            .max_message_size(Some(MAX_FRAME))
            .max_frame_size(Some(MAX_FRAME));
        let connection = async {
            let tcp = TcpStream::connect(address)
                .await
                .map_err(|_| frame_error("Could not connect browser frame endpoint"))?;
            let (mut stream, _) =
                tokio_tungstenite::client_async_with_config(request, tcp, Some(config))
                    .await
                    .map_err(|_| frame_error("Browser frame handshake failed"))?;
            stream
                .send(FrameMessage::Text(
                    serde_json::json!({"grant": grant}).to_string().into(),
                ))
                .await
                .map_err(|_| frame_error("Could not authorize browser frame endpoint"))?;
            Ok(Self { stream })
        };
        tokio::time::timeout(std::time::Duration::from_secs(5), connection)
            .await
            .map_err(|_| frame_error("Browser frame connection timed out"))?
    }

    pub async fn recv(&mut self) -> Result<Option<Vec<u8>>, InspectionError> {
        loop {
            match self.stream.next().await {
                Some(Ok(FrameMessage::Binary(payload))) => return Ok(Some(payload.to_vec())),
                None | Some(Ok(FrameMessage::Close(_))) => return Ok(None),
                Some(Ok(FrameMessage::Ping(_))) => {
                    self.stream
                        .flush()
                        .await
                        .map_err(|_| frame_error("Browser frame heartbeat failed"))?;
                }
                Some(Ok(FrameMessage::Pong(_))) => {}
                _ => return Err(frame_error("Invalid browser frame message")),
            }
        }
    }
}
fn frame_error(message: &str) -> InspectionError {
    InspectionError::new("browser_frame_unavailable", message)
}
