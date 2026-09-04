//! CLI and browser gateway host for Cockpit.
//!
//! This crate deliberately owns all HTTP and command-line concerns. The application
//! behavior remains in `cockpit-core`, allowing native and browser transports to
//! consume the same status operation.

pub mod server;

pub use server::{
    ServerConfig, ServerError, build_router, router, serve, validate_bind, validate_static_root,
};
