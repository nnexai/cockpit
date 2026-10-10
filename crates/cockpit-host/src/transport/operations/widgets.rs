use cockpit_protocol::widget::{
    WidgetContent, WidgetContentRequest, WidgetRemoveRequest, WidgetRemoveResponse,
    WidgetSelectRequest, WidgetSelectResponse,
};

use crate::{
    BrowserRuntime,
    transport::{Transport, error::OperationError},
};

pub async fn widget_content(
    runtime: &BrowserRuntime,
    _transport: Transport,
    request: WidgetContentRequest,
) -> Result<WidgetContent, OperationError> {
    runtime.widget_content(request).await.map_err(Into::into)
}

pub async fn widget_remove(
    runtime: &BrowserRuntime,
    _transport: Transport,
    request: WidgetRemoveRequest,
) -> Result<WidgetRemoveResponse, OperationError> {
    runtime.widget_remove(request).await.map_err(Into::into)
}

pub async fn widget_select(
    runtime: &BrowserRuntime,
    _transport: Transport,
    request: WidgetSelectRequest,
) -> Result<WidgetSelectResponse, OperationError> {
    runtime.widget_select(request).await.map_err(Into::into)
}
