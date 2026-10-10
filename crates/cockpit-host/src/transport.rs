pub mod browser_frame;
pub mod browser_relay;
pub mod error;
pub mod guard;
pub mod http_input;
pub mod limits;
pub mod native_input;
pub mod operations;
pub mod registry;
pub mod session_stream;
pub mod shutdown;
pub mod terminal;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    Gateway,
    Native,
}
