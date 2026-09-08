//! CLI and browser gateway host for Cockpit.
//!
//! This crate deliberately owns all HTTP and command-line concerns. The application
//! behavior remains in `cockpit-core`, allowing native and browser transports to
//! consume the same status operation.

mod browser_annotations;
pub mod browser_runtime;
pub mod server;

pub use browser_runtime::BrowserRuntime;
pub use server::{
    ServerConfig, ServerError, build_router, router, serve, validate_bind, validate_static_root,
};
