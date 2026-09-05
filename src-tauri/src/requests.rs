use std::io::{self, Write};

use cockpit_protocol::v1::ErrorResponse;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::{MAX_MUTATION_REQUEST_BYTES, stream_error};

struct RequestBudget(usize);

impl Write for RequestBudget {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 = self
            .0
            .checked_sub(bytes.len())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "request limit exceeded"))?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) fn decode_request<T: DeserializeOwned>(
    value: Value,
    domain: &str,
) -> Result<T, ErrorResponse> {
    let invalid = || {
        stream_error(
            &format!("invalid_{domain}_request"),
            &format!("Expected a bounded JSON {domain} request with valid fields"),
        )
    };
    serde_json::to_writer(RequestBudget(MAX_MUTATION_REQUEST_BYTES), &value)
        .map_err(|_| invalid())?;
    serde_json::from_value(value).map_err(|_| invalid())
}
