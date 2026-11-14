use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{Mutex as AsyncMutex, oneshot};

use super::pipe::{Decoder, frame};
use crate::error::SourceError;

/// A page load or a screenshot of a heavy page can take a while; anything past this is a hang.
const REPLY_TIMEOUT: Duration = Duration::from_secs(20);
const READ_CHUNK: usize = 64 * 1024;

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>;

/// One DevTools connection: requests get ids, replies find their waiter, events are logged.
pub struct CdpClient {
    next_id: AtomicU64,
    pending: Pending,
    writer: AsyncMutex<Box<dyn AsyncWrite + Send + Unpin>>,
}

impl std::fmt::Debug for CdpClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("CdpClient").finish_non_exhaustive()
    }
}

impl CdpClient {
    /// Starts reading `reader` on a task; `writer` carries requests.
    pub fn new(
        reader: impl AsyncRead + Send + Unpin + 'static,
        writer: impl AsyncWrite + Send + Unpin + 'static,
    ) -> Self {
        let pending = Pending::default();
        tokio::spawn(read_loop(reader, Arc::clone(&pending)));
        Self {
            next_id: AtomicU64::new(1),
            pending,
            writer: AsyncMutex::new(Box::new(writer)),
        }
    }

    /// Sends `method` and waits for its result; a `session` targets one attached page.
    ///
    /// # Errors
    ///
    /// Fails on a protocol error from the browser, a closed pipe, or a reply that never comes.
    pub async fn send(
        &self,
        method: &str,
        params: Value,
        session: Option<&str>,
    ) -> Result<Value, SourceError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut message = json!({ "id": id, "method": method, "params": params });
        if let Some(session) = session {
            message["sessionId"] = Value::from(session);
        }
        let (sender, receiver) = oneshot::channel();
        if let Ok(mut pending) = self.pending.lock() {
            pending.insert(id, sender);
        }
        {
            let mut writer = self.writer.lock().await;
            writer.write_all(&frame(&message)).await?;
            writer.flush().await?;
        }
        let reply = tokio::time::timeout(REPLY_TIMEOUT, receiver)
            .await
            .map_err(|_| SourceError::Timeout)?
            .map_err(|_| SourceError::Gone)?;
        if let Some(error) = reply.get("error") {
            let text = error["message"].as_str().unwrap_or("unknown error");
            return Err(SourceError::Browser(format!("{method}: {text}")));
        }
        Ok(reply.get("result").cloned().unwrap_or(Value::Null))
    }
}

async fn read_loop(mut reader: impl AsyncRead + Unpin, pending: Pending) {
    let mut decoder = Decoder::default();
    let mut chunk = vec![0u8; READ_CHUNK];
    loop {
        let read = match reader.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
        };
        for message in decoder.push(chunk.get(..read).unwrap_or_default()) {
            match message.get("id").and_then(Value::as_u64) {
                Some(id) => {
                    let waiter = pending
                        .lock()
                        .ok()
                        .and_then(|mut pending| pending.remove(&id));
                    if let Some(waiter) = waiter {
                        let _ = waiter.send(message);
                    }
                }
                None => tracing::trace!(method = ?message.get("method"), "browser event"),
            }
        }
    }
    if let Ok(mut pending) = pending.lock() {
        pending.clear();
    }
}
