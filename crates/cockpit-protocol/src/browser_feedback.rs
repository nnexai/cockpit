use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserPoint {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserViewport {
    pub width: f64,
    pub height: f64,
    pub scroll_x: f64,
    pub scroll_y: f64,
    pub device_pixel_ratio: f64,
    pub visual_scale: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserElementEvidence {
    pub tag: String,
    pub text: String,
    pub role: Option<String>,
    pub name: Option<String>,
    pub locators: Vec<String>,
    pub excerpt: String,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum BrowserAnnotationKind {
    Freehand,
    Element,
    Region,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserAnnotation {
    pub id: String,
    pub kind: BrowserAnnotationKind,
    pub comment: String,
    pub color: String,
    pub points: Vec<BrowserPoint>,
    pub bounds: Option<BrowserRect>,
    pub element: Option<BrowserElementEvidence>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserPageEvidence {
    pub url: String,
    pub title: String,
    #[ts(type = "number")]
    pub tab_id: i64,
    pub document_id: String,
    pub captured_at: String,
    pub viewport: BrowserViewport,
    pub image_width: u32,
    pub image_height: u32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserCaptureSubmission {
    pub association_key: String,
    pub browser_instance: String,
    pub capture_id: String,
    pub page: BrowserPageEvidence,
    pub annotations: Vec<BrowserAnnotation>,
    pub png_base64: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserCaptureContext {
    pub association_key: String,
    pub session_id: String,
    pub space_id: String,
    pub space_label: String,
    pub playwright_session: String,
    pub working_directory: String,
    pub invocation: String,
    pub browser_instance: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserFeedbackCapture {
    pub id: String,
    pub context: BrowserCaptureContext,
    pub page: BrowserPageEvidence,
    pub annotations: Vec<BrowserAnnotation>,
    pub pending_ids: Vec<String>,
    pub image_path: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserCaptureSaved {
    pub capture_id: String,
    pub annotation_ids: Vec<String>,
    pub image_path: String,
    pub pending_count: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserFeedbackResponse {
    pub captures: Vec<BrowserFeedbackCapture>,
    pub pending_count: usize,
    #[ts(type = "number")]
    pub retention_seconds: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserFeedbackAck {
    pub acknowledged_ids: Vec<String>,
    pub remaining: usize,
}
