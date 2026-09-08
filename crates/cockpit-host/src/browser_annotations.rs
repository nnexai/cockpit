use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, HeaderValue, Method, Request, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use cockpit_core::InspectionError;
use cockpit_core::browser::{ANNOTATION_EXTENSION_ID, BrowserService};
use cockpit_protocol::browser_feedback::BrowserCaptureSubmission;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

#[derive(Clone)]
struct AnnotationState {
    service: Arc<BrowserService>,
    authority: String,
    origin: String,
}

pub(crate) struct AnnotationServer {
    stop: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
}

impl AnnotationServer {
    pub(crate) async fn start(service: Arc<BrowserService>) -> Result<Self, InspectionError> {
        let port = service.feedback_port()?.unwrap_or(0);
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
            .await
            .map_err(|error| {
                InspectionError::new(
                    "browser_annotation_bind",
                    format!("Cannot bind the private annotation endpoint: {error}"),
                )
            })?;
        let address = listener
            .local_addr()
            .map_err(|error| InspectionError::new("browser_annotation_bind", error.to_string()))?;
        service.configure_feedback_endpoint(format!("http://{address}"))?;
        service.persist_feedback_port(address.port())?;
        let state = AnnotationState {
            service,
            authority: address.to_string(),
            origin: format!("chrome-extension://{ANNOTATION_EXTENSION_ID}"),
        };
        let router = Router::new()
            .route("/status", get(status))
            .route("/capture", post(capture))
            .layer(DefaultBodyLimit::max(6 * 1024 * 1024))
            .layer(middleware::from_fn_with_state(state.clone(), guard))
            .with_state(state);
        let (stop, stopped) = oneshot::channel();
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, router)
                .with_graceful_shutdown(async {
                    let _ = stopped.await;
                })
                .await;
        });
        Ok(Self {
            stop: Some(stop),
            task: Some(task),
        })
    }

    pub(crate) async fn shutdown(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(mut task) = self.task.take() {
            if tokio::time::timeout(Duration::from_secs(5), &mut task)
                .await
                .is_err()
            {
                task.abort();
                let _ = task.await;
            }
        }
    }
}

impl Drop for AnnotationServer {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

fn credentials(headers: &HeaderMap) -> Result<(&str, &str), InspectionError> {
    let key = headers
        .get("x-cockpit-association")
        .and_then(|value| value.to_str().ok());
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    match (key, token) {
        (Some(key), Some(token)) => Ok((key, token)),
        _ => Err(InspectionError::new(
            "browser_annotation_unauthorized",
            "Annotation pairing is required",
        )),
    }
}

async fn guard(
    State(state): State<AnnotationState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        != Some(state.authority.as_str())
    {
        return (
            StatusCode::FORBIDDEN,
            "Invalid annotation endpoint authority",
        )
            .into_response();
    }
    let origin = request.headers().get(header::ORIGIN);
    if origin.is_some_and(|origin| origin.as_bytes() != state.origin.as_bytes()) {
        return (
            StatusCode::FORBIDDEN,
            "Annotation endpoint is restricted to the Cockpit extension",
        )
            .into_response();
    }
    let mut response = if request.method() == Method::OPTIONS {
        if origin.is_none() {
            return StatusCode::FORBIDDEN.into_response();
        }
        StatusCode::NO_CONTENT.into_response()
    } else {
        let authorized = credentials(request.headers())
            .and_then(|(key, token)| state.service.authorize_annotation(key, token));
        if let Err(error) = authorized {
            return failure(error);
        }
        next.run(request).await
    };
    let headers = response.headers_mut();
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_str(&state.origin).expect("fixed extension origin is a header value"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, POST, OPTIONS"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("authorization, content-type, x-cockpit-association"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

async fn status(State(state): State<AnnotationState>, headers: HeaderMap) -> Response {
    match credentials(&headers).and_then(|(key, token)| state.service.extension_status(key, token))
    {
        Ok(status) => Json(status).into_response(),
        Err(error) => failure(error),
    }
}

async fn capture(
    State(state): State<AnnotationState>,
    headers: HeaderMap,
    Json(submission): Json<BrowserCaptureSubmission>,
) -> Response {
    match credentials(&headers)
        .and_then(|(key, token)| state.service.save_extension_capture(key, token, submission))
    {
        Ok(saved) => Json(saved).into_response(),
        Err(error) => failure(error),
    }
}

fn failure(error: InspectionError) -> Response {
    let status = if error.code == "browser_annotation_unauthorized" {
        StatusCode::UNAUTHORIZED
    } else if error.code.contains("limit") {
        StatusCode::PAYLOAD_TOO_LARGE
    } else {
        StatusCode::BAD_REQUEST
    };
    (
        status,
        Json(serde_json::json!({"code": error.code, "message": error.message})),
    )
        .into_response()
}
