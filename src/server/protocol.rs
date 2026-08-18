//! Wire protocol definitions, request/response models, and message framing for ChocoBase.

use serde::{Deserialize, Serialize};
use std::io;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::engine::ExecResult;

/// Change event action for realtime subscription.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ChangeAction {
    Insert,
    Update,
    Delete,
}

/// Realtime change event emitted upon committed database mutations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChangeEvent {
    pub table: String,
    pub action: ChangeAction,
    pub old_row: Option<Vec<crate::types::value::Value>>,
    pub new_row: Option<Vec<crate::types::value::Value>>,
    pub timestamp_ms: u64,
}

/// Client request sent to ChocoBase server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Request {
    /// Execute a SQL statement or query.
    Query { sql: String },
    /// Heartbeat ping to check connection liveness.
    Ping,
    /// Subscribe to live mutation events on a table (or all tables if None).
    Subscribe { table: Option<String> },
    /// Unsubscribe from live mutation events.
    Unsubscribe,
}

/// Server response returned to client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Response {
    /// Successful query execution result.
    Result(ExecResult),
    /// Execution or syntax error.
    Error(String),
    /// Heartbeat pong.
    Pong,
    /// Live change event notification.
    Event(ChangeEvent),
    /// Subscription acknowledged.
    Subscribed,
    /// Unsubscription acknowledged.
    Unsubscribed,
}

/// Serializes and writes a framed response to an async writer.
/// Uses newline-delimited JSON framing.
pub async fn write_response<W: AsyncWriteExt + Unpin>(
    writer: &mut W,
    response: &Response,
) -> io::Result<()> {
    let mut json =
        serde_json::to_vec(response).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    json.push(b'\n');
    writer.write_all(&json).await?;
    writer.flush().await?;
    Ok(())
}

/// Serializes and writes a framed request to an async writer.
pub async fn write_request<W: AsyncWriteExt + Unpin>(
    writer: &mut W,
    request: &Request,
) -> io::Result<()> {
    let mut json =
        serde_json::to_vec(request).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    json.push(b'\n');
    writer.write_all(&json).await?;
    writer.flush().await?;
    Ok(())
}

/// Reads and deserializes a framed request from an async reader buffer.
/// Returns `Ok(Some(Request))` on complete message, `Ok(None)` on EOF, or `Err`.
pub async fn read_request<R: AsyncReadExt + Unpin>(
    reader: &mut R,
    buffer: &mut Vec<u8>,
) -> io::Result<Option<Request>> {
    let mut chunk = [0u8; 1024];
    loop {
        if let Some(pos) = buffer.iter().position(|&b| b == b'\n') {
            let line = buffer.drain(..=pos).collect::<Vec<u8>>();
            let text = &line[..line.len().saturating_sub(1)];
            if text.is_empty() {
                continue;
            }
            let req: Request = serde_json::from_slice(text)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            return Ok(Some(req));
        }

        let n = reader.read(&mut chunk).await?;
        if n == 0 {
            if buffer.is_empty() {
                return Ok(None);
            }
            let text = buffer.drain(..).collect::<Vec<u8>>();
            let req: Request = serde_json::from_slice(&text)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            return Ok(Some(req));
        }
        buffer.extend_from_slice(&chunk[..n]);
    }
}

/// Reads and deserializes a framed response from an async reader buffer.
pub async fn read_response<R: AsyncReadExt + Unpin>(
    reader: &mut R,
    buffer: &mut Vec<u8>,
) -> io::Result<Option<Response>> {
    let mut chunk = [0u8; 1024];
    loop {
        if let Some(pos) = buffer.iter().position(|&b| b == b'\n') {
            let line = buffer.drain(..=pos).collect::<Vec<u8>>();
            let text = &line[..line.len().saturating_sub(1)];
            if text.is_empty() {
                continue;
            }
            let resp: Response = serde_json::from_slice(text)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            return Ok(Some(resp));
        }

        let n = reader.read(&mut chunk).await?;
        if n == 0 {
            if buffer.is_empty() {
                return Ok(None);
            }
            let text = buffer.drain(..).collect::<Vec<u8>>();
            let resp: Response = serde_json::from_slice(&text)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            return Ok(Some(resp));
        }
        buffer.extend_from_slice(&chunk[..n]);
    }
}
