use std::io::{self, Write};

use cockpit_protocol::v1::ErrorResponse;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::{MAX_MUTATION_REQUEST_BYTES, stream_error};

const MAX_LIBRARY_CONFIRMATION_REQUEST_BYTES: usize = 13 * 1024 * 1024;

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
    decode_request_with_limit(value, domain, MAX_MUTATION_REQUEST_BYTES)
}

pub(super) fn decode_confirmation_request<T: DeserializeOwned>(
    value: Value,
    domain: &str,
) -> Result<T, ErrorResponse> {
    decode_request_with_limit(value, domain, MAX_LIBRARY_CONFIRMATION_REQUEST_BYTES)
}

fn decode_request_with_limit<T: DeserializeOwned>(
    value: Value,
    domain: &str,
    max_bytes: usize,
) -> Result<T, ErrorResponse> {
    let invalid = || {
        stream_error(
            &format!("invalid_{domain}_request"),
            &format!("Expected a bounded JSON {domain} request with valid fields"),
        )
    };
    serde_json::to_writer(RequestBudget(max_bytes), &value)
        .map_err(|_| invalid())?;
    serde_json::from_value(value).map_err(|_| invalid())
}

#[cfg(test)]
mod tests {
    use super::decode_confirmation_request;
    use serde_json::{Value, json};

    #[test]
    fn large_library_confirmation_request_fits_its_budget() {
        let confirmed = (0..512)
            .map(|index| {
                json!({
                    "path": format!("sources/file-{index}-{}", "x".repeat(200)),
                    "current_hash": format!("sha256:{}", "0".repeat(64)),
                })
            })
            .collect::<Vec<_>>();
        let request = json!({"confirmed": confirmed});
        assert!(serde_json::to_vec(&request).unwrap().len() > 64 * 1024);
        assert_eq!(
            decode_confirmation_request::<Value>(request.clone(), "library remove").unwrap(),
            request
        );
    }
}
