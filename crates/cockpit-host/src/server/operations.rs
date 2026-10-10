use std::sync::Arc;

use axum::{
    Extension, Json, Router,
    extract::State,
    response::{IntoResponse, Response},
};
use cockpit_core::CockpitService;

use crate::{
    OrchestrationRuntime,
    browser_runtime::BrowserRuntime,
    transport::{
        Transport,
        error::{OperationError, StatusPolicy},
        guard::{Missing, require, require_origin},
    },
};

macro_rules! gateway_status {
    (service) => {
        StatusPolicy::Service
    };
    (notes) => {
        StatusPolicy::Notes
    };
    (bad_request) => {
        StatusPolicy::BadRequest
    };
}

// Passing the binding through every helper preserves macro hygiene when a guard
// replaces an optional runtime with its checked Arc.
macro_rules! gateway_guard_before {
    ($context:ident, $status:ident, first($missing:ident)) => {
        let $context = match require($context, Missing::$missing) {
            Ok(context) => context,
            Err(error) => return error.into_response($status),
        };
    };
    ($context:ident, $status:ident, after($missing:ident)) => {};
    ($context:ident, $status:ident, none) => {};
}

macro_rules! gateway_guard_after {
    ($context:ident, $status:ident, after($missing:ident)) => {
        let $context = match require($context, Missing::$missing) {
            Ok(context) => context,
            Err(error) => return error.into_response($status),
        };
    };
    ($context:ident, $status:ident, first($missing:ident)) => {};
    ($context:ident, $status:ident, none) => {};
}

macro_rules! gateway_handler {
    ({ id: $id:ident, ctx: service, $($row:tt)* }) => {
        gateway_handler_impl! {
            $id, (State: State<CockpitService>), context; $($row)*
        }
    };
    ({ id: $id:ident, ctx: browser, $($row:tt)* }) => {
        gateway_handler_impl! {
            $id, (Extension: Extension<Option<Arc<BrowserRuntime>>>), context;
            $($row)*
        }
    };
    ({ id: $id:ident, ctx: orchestration, $($row:tt)* }) => {
        gateway_handler_impl! {
            $id, (Extension: Extension<Option<Arc<OrchestrationRuntime>>>), context;
            $($row)*
        }
    };
}

macro_rules! gateway_handler_impl {
    (
        $id:ident, ($wrapper:ident : $context_type:ty), $context:ident;
        out: $out:ty, call: $call:path,
        native: $native:ident ($($native_arg:ident : $native_type:ty),* $(,)?)
            => $native_input:expr,
        http: none
    ) => {};
    (
        $id:ident, ($wrapper:ident : $context_type:ty), $context:ident;
        out: $out:ty, call: $call:path,
        native: $native:ident ($($native_arg:ident : $native_type:ty),* $(,)?)
            => $native_input:expr,
        http: $method:ident $path:literal ($($arg:ident : $extractor:ty),* $(,)?)
            => $input:expr;
        limit = $limit:expr; origin = $origin:ident;
        guard = $guard:ident $(($missing:ident))?; status = $status:ident
    ) => {
        async fn $id(
            $wrapper($context): $context_type,
            $($arg: $extractor,)*
        ) -> Response {
            let status = gateway_status!($status);
            gateway_guard_before!($context, status, $guard $(($missing))?);
            let input = match ($input)(($($arg,)*)) {
                Ok(input) => input,
                Err(error) => return error.into_response(status),
            };
            gateway_guard_after!($context, status, $guard $(($missing))?);
            let result: Result<$out, OperationError> =
                $call(&$context, Transport::Gateway, input).await;
            match result {
                Ok(output) => Json(output).into_response(),
                Err(error) => error.into_response(status),
            }
        }
    };
}

macro_rules! gateway_route {
    (
        $router:expr;
        { id: $id:ident, ctx: $ctx:ident, out: $out:ty, call: $call:path,
          native: $native:ident ($($native_arg:ident : $native_type:ty),* $(,)?)
            => $native_input:expr,
          http: none }
    ) => { $router };
    (
        $router:expr;
        { id: $id:ident, ctx: $ctx:ident, out: $out:ty, call: $call:path,
          native: $native:ident ($($native_arg:ident : $native_type:ty),* $(,)?)
            => $native_input:expr,
          http: $method:ident $path:literal ($($arg:ident : $extractor:ty),* $(,)?)
            => $input:expr; $($metadata:tt)* }
    ) => {
        gateway_route_http!($router, $path, $method, $id; $($metadata)*)
    };
}

macro_rules! gateway_route_http {
    (
        $router:expr, $path:literal, $method:ident, $id:ident;
        limit = none; origin = $origin:ident;
        guard = $guard:ident $(($missing:ident))?; status = $status:ident
    ) => {
        gateway_origin!($router, $path, ::axum::routing::$method($id), $origin)
    };
    (
        $router:expr, $path:literal, $method:ident, $id:ident;
        limit = $limit:expr; origin = $origin:ident;
        guard = $guard:ident $(($missing:ident))?; status = $status:ident
    ) => {
        gateway_origin!(
            $router,
            $path,
            ::axum::routing::$method($id).layer(::axum::extract::DefaultBodyLimit::max($limit)),
            $origin
        )
    };
}

macro_rules! gateway_origin {
    ($router:expr, $path:literal, $method:expr, none) => {
        $router.route($path, $method)
    };
    ($router:expr, $path:literal, $method:expr, route) => {
        // Router::route_layer also wraps the method-not-allowed fallback.
        // MethodRouter::route_layer would change origin rejection precedence.
        $router.merge(
            Router::new()
                .route($path, $method)
                .route_layer(::axum::middleware::from_fn(require_origin)),
        )
    };
    ($router:expr, $path:literal, $method:expr, method) => {
        $router.route(
            $path,
            $method.layer(::axum::middleware::from_fn(require_origin)),
        )
    };
}

macro_rules! gateway_routes {
    ($({ $($row:tt)* })*) => {
        $(gateway_handler!({ $($row)* });)*

        pub(super) fn operation_routes() -> Router<CockpitService> {
            let router = Router::new();
            $(let router = gateway_route!(router; { $($row)* });)*
            router
        }
    };
}

crate::cockpit_operations!(gateway_routes);
