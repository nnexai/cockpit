use std::io::{self, Write};

use cockpit_protocol::v1::ErrorResponse;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::{MAX_MUTATION_REQUEST_BYTES, stream_error};

const MAX_LIBRARY_REQUEST_BYTES: usize = 13 * 1024 * 1024;

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

pub(super) fn decode_library_request<T: DeserializeOwned>(
    value: Value,
    domain: &str,
) -> Result<T, ErrorResponse> {
    decode_request_with_limit(value, domain, MAX_LIBRARY_REQUEST_BYTES)
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
    use super::decode_library_request;
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
            decode_library_request::<Value>(request.clone(), "library remove").unwrap(),
            request
        );
    }

    #[test]
    fn library_selection_requests_share_browser_byte_budget() {
        use cockpit_protocol::library::{SpaceAddRequest, SpaceRemoveRequest, SpaceRepositoriesRequest};
        let target = json!({"session_id": "session", "space_id": "space"});
        let selected = json!({"target": target, "item_ids": vec!["x".repeat(512); 5000]});
        assert_eq!(decode_library_request::<SpaceAddRequest>(selected.clone(), "library").unwrap().item_ids.len(), 5000);
        assert_eq!(decode_library_request::<SpaceRemoveRequest>(selected, "library").unwrap().item_ids.len(), 5000);
        let repositories = json!({"target": target, "repository_paths": vec![format!("/{}", "x".repeat(4094)); 64]});
        assert_eq!(decode_library_request::<SpaceRepositoriesRequest>(repositories, "library").unwrap().repository_paths.len(), 64);
        assert!(decode_library_request::<SpaceAddRequest>(json!({"target": target, "item_ids": [], "follow_ids": []}), "library").is_err());
        assert!(decode_library_request::<SpaceRemoveRequest>(json!({"target": target, "logical_id": "old", "confirmed": []}), "library").is_err());
    }
}
