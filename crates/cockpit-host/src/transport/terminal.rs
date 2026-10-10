use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use cockpit_protocol::v1::{
    TerminalCommand, TerminalOpenRequest, TerminalOwnershipState, TerminalStreamMessage,
};

use super::{
    error::OperationError,
    limits::{SerializationLimitError, TERMINAL_COMMAND_BYTES, check_serialized_size},
};

pub fn localize_terminal_message(
    message: TerminalStreamMessage,
    stream_id: &str,
) -> TerminalStreamMessage {
    match message {
        TerminalStreamMessage::MouseMode {
            session_id,
            pane_id,
            enabled,
            ..
        } => TerminalStreamMessage::MouseMode {
            session_id,
            pane_id,
            stream_id: stream_id.to_owned(),
            enabled,
        },
        TerminalStreamMessage::Ownership {
            session_id,
            pane_id,
            state,
            message,
            ..
        } => TerminalStreamMessage::Ownership {
            session_id,
            pane_id,
            stream_id: stream_id.to_owned(),
            state,
            message,
        },
        TerminalStreamMessage::Frame {
            session_id,
            pane_id,
            seq,
            encoding,
            width,
            height,
            full,
            bytes,
            ..
        } => TerminalStreamMessage::Frame {
            session_id,
            pane_id,
            stream_id: stream_id.to_owned(),
            seq,
            encoding,
            width,
            height,
            full,
            bytes,
        },
        TerminalStreamMessage::Closed {
            session_id,
            pane_id,
            reason,
            ..
        } => TerminalStreamMessage::Closed {
            session_id,
            pane_id,
            stream_id: stream_id.to_owned(),
            reason,
        },
        TerminalStreamMessage::Disconnected {
            session_id,
            pane_id,
            code,
            message,
            ..
        } => TerminalStreamMessage::Disconnected {
            session_id,
            pane_id,
            stream_id: stream_id.to_owned(),
            code,
            message,
        },
        TerminalStreamMessage::Error {
            session_id,
            pane_id,
            code,
            message,
            ..
        } => TerminalStreamMessage::Error {
            session_id,
            pane_id,
            stream_id: stream_id.to_owned(),
            code,
            message,
        },
    }
}

pub fn terminal_error(
    request: &TerminalOpenRequest,
    stream_id: &str,
    code: &str,
    message: &str,
) -> TerminalStreamMessage {
    TerminalStreamMessage::Error {
        session_id: request.session_id.clone(),
        pane_id: request.pane_id.clone(),
        stream_id: stream_id.to_owned(),
        code: code.to_owned(),
        message: message.to_owned(),
    }
}

pub fn decimal_sequence(value: &str) -> Option<u64> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

pub fn validate_terminal_message(
    message: &TerminalStreamMessage,
    request: &TerminalOpenRequest,
    herdr_stream_id: &str,
    last_seq: &mut Option<u64>,
    has_baseline: &mut bool,
) -> Result<(), (&'static str, &'static str)> {
    let (session_id, pane_id, stream_id) = match message {
        TerminalStreamMessage::MouseMode {
            session_id,
            pane_id,
            stream_id,
            ..
        }
        | TerminalStreamMessage::Ownership {
            session_id,
            pane_id,
            stream_id,
            ..
        }
        | TerminalStreamMessage::Frame {
            session_id,
            pane_id,
            stream_id,
            ..
        }
        | TerminalStreamMessage::Closed {
            session_id,
            pane_id,
            stream_id,
            ..
        }
        | TerminalStreamMessage::Disconnected {
            session_id,
            pane_id,
            stream_id,
            ..
        }
        | TerminalStreamMessage::Error {
            session_id,
            pane_id,
            stream_id,
            ..
        } => (session_id, pane_id, stream_id),
    };
    if session_id != &request.session_id || pane_id != &request.pane_id {
        return Err(("terminal_frame_invalid", "Terminal frame resource mismatch"));
    }
    if stream_id != herdr_stream_id {
        return Err(("terminal_frame_invalid", "Terminal frame stream mismatch"));
    }
    if let TerminalStreamMessage::Frame {
        seq,
        encoding,
        width: _,
        height: _,
        full,
        bytes,
        ..
    } = message
    {
        if encoding != "ansi" || BASE64.decode(bytes.as_bytes()).is_err() {
            return Err(("terminal_frame_invalid", "Invalid terminal frame"));
        }
        let Some(number) = decimal_sequence(seq) else {
            return Err(("terminal_sequence_error", "Invalid terminal sequence"));
        };
        if !*has_baseline && !*full {
            return Err((
                "terminal_sequence_error",
                "Incremental terminal frame has no full-frame baseline",
            ));
        }
        if let Some(previous) = *last_seq
            && number != previous.saturating_add(1)
        {
            return Err((
                "terminal_sequence_error",
                "Terminal sequence is not consecutive",
            ));
        }
        *last_seq = Some(number);
        *has_baseline = true;
    }
    Ok(())
}

/// Native command validation runs before looking up the local terminal stream.
pub fn command(command: TerminalCommand) -> Result<TerminalCommand, OperationError> {
    command
        .validate()
        .map_err(|message| OperationError::rejected("invalid_terminal_command", message))?;
    match check_serialized_size(&command, TERMINAL_COMMAND_BYTES) {
        Ok(()) => Ok(command),
        Err(SerializationLimitError::Invalid(_)) => Err(OperationError::rejected(
            "invalid_terminal_command",
            "Invalid terminal command",
        )),
        Err(SerializationLimitError::TooLarge) => Err(OperationError::rejected(
            "terminal_command_too_large",
            "Terminal command exceeds the 96 KiB limit",
        )),
    }
}

pub fn ended(message: &TerminalStreamMessage) -> bool {
    matches!(
        message,
        TerminalStreamMessage::Closed { .. }
            | TerminalStreamMessage::Disconnected { .. }
            | TerminalStreamMessage::Error { .. }
            | TerminalStreamMessage::Ownership {
                state: TerminalOwnershipState::Lost | TerminalOwnershipState::Conflict,
                ..
            }
    )
}
