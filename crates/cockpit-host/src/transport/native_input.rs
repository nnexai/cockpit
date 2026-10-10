use std::io::{self, Write};

use cockpit_protocol::notes::NotesRequest;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

use super::{
    error::OperationError,
    limits::{NOTES_BYTES, widget_request_size},
};

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

fn decode<T: DeserializeOwned>(
    value: Value,
    limit: usize,
    domain: &'static str,
) -> Result<T, OperationError> {
    // Never include serde's rejection text: credential bodies may contain secrets.
    let invalid = || {
        OperationError::rejected(
            &format!("invalid_{domain}_request"),
            format!("Expected a bounded JSON {domain} request with valid fields"),
        )
    };
    serde_json::to_writer(RequestBudget(limit), &value).map_err(|_| invalid())?;
    serde_json::from_value(value).map_err(|_| invalid())
}

pub fn none() -> impl FnOnce(()) -> Result<(), OperationError> {
    |()| Ok(())
}

pub fn typed<T>() -> impl FnOnce((T,)) -> Result<T, OperationError> {
    |(value,)| Ok(value)
}

pub fn typed2<A, B>() -> impl FnOnce((A, B)) -> Result<(A, B), OperationError> {
    |value| Ok(value)
}

pub fn value<T: DeserializeOwned>(
    limit: usize,
    domain: &'static str,
) -> impl FnOnce((Value,)) -> Result<T, OperationError> {
    move |(value,)| decode(value, limit, domain)
}

pub fn session_value<T: DeserializeOwned>(
    limit: usize,
    domain: &'static str,
) -> impl FnOnce((String, Value)) -> Result<(String, T), OperationError> {
    move |(session, value)| Ok((session, decode(value, limit, domain)?))
}

pub fn pair_value<T: DeserializeOwned>(
    limit: usize,
    domain: &'static str,
) -> impl FnOnce((String, String, Value)) -> Result<(String, String, T), OperationError> {
    move |(session, resource, value)| Ok((session, resource, decode(value, limit, domain)?))
}

pub fn notes() -> impl FnOnce((Value,)) -> Result<NotesRequest, OperationError> {
    |(value,)| {
        serde_json::to_writer(RequestBudget(NOTES_BYTES), &value).map_err(|_| {
            OperationError::rejected("notes_too_large", "Notes request exceeds the 4 MiB limit")
        })?;
        serde_json::from_value(value).map_err(|_| {
            OperationError::rejected(
                "notes_usage",
                "Expected a bounded JSON Notes request with valid fields",
            )
        })
    }
}

pub fn widget<T: Serialize>() -> impl FnOnce((T,)) -> Result<T, OperationError> {
    |(request,)| {
        widget_request_size(&request)?;
        Ok(request)
    }
}

pub fn view_id() -> impl FnOnce((String,)) -> Result<String, OperationError> {
    |(id,)| {
        if id.trim().is_empty() {
            return Err(OperationError::rejected(
                "invalid_browser_view_id",
                "Browser view id is required",
            ));
        }
        Ok(id)
    }
}

pub fn validated<T>(
    validate: fn(&T) -> Result<(), OperationError>,
) -> impl FnOnce((T,)) -> Result<T, OperationError> {
    move |(value,)| {
        validate(&value)?;
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::value;
    use crate::transport::limits::LIBRARY_BYTES;
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
            value::<Value>(LIBRARY_BYTES, "library remove")((request.clone(),)).unwrap(),
            request,
        );
    }

    #[test]
    fn library_selection_requests_share_browser_byte_budget() {
        use cockpit_protocol::library::{
            SpaceAddRequest, SpaceRemoveRequest, SpaceRepositoriesRequest,
        };
        let target = json!({"session_id": "session", "space_id": "space"});
        let selected = json!({"target": target, "item_ids": vec!["x".repeat(512); 5000]});
        assert_eq!(
            value::<SpaceAddRequest>(LIBRARY_BYTES, "library")((selected.clone(),))
                .unwrap()
                .item_ids
                .len(),
            5000
        );
        assert_eq!(
            value::<SpaceRemoveRequest>(LIBRARY_BYTES, "library")((selected,))
                .unwrap()
                .item_ids
                .len(),
            5000
        );
        let repositories = json!({"target": target, "repository_paths": vec![format!("/{}", "x".repeat(4094)); 64]});
        assert_eq!(
            value::<SpaceRepositoriesRequest>(LIBRARY_BYTES, "library")((repositories,))
                .unwrap()
                .repository_paths
                .len(),
            64
        );
        assert!(
            value::<SpaceAddRequest>(LIBRARY_BYTES, "library")((
                json!({"target": target, "item_ids": [], "follow_ids": []}),
            ))
            .is_err()
        );
        assert!(
            value::<SpaceRemoveRequest>(LIBRARY_BYTES, "library")((
                json!({"target": target, "logical_id": "old", "confirmed": []}),
            ))
            .is_err()
        );
    }
}
