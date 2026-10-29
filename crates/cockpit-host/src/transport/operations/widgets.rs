use cockpit_protocol::widget::*;

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

pub async fn widget_shelf_list(runtime: &BrowserRuntime, _transport: Transport, request: WidgetShelfListRequest) -> Result<WidgetShelfListResponse, OperationError> {
    runtime.widget_shelf_list(request).await.map_err(Into::into)
}

pub async fn widget_shelf_reopen(runtime: &BrowserRuntime, _transport: Transport, request: WidgetShelfReopenRequest) -> Result<WidgetSummary, OperationError> {
    runtime.widget_shelf_reopen(request).await.map_err(Into::into)
}

pub async fn widget_select(
    runtime: &BrowserRuntime,
    _transport: Transport,
    request: WidgetSelectRequest,
) -> Result<WidgetSelectResponse, OperationError> {
    runtime.widget_select(request).await.map_err(Into::into)
}
