use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{oneshot, Mutex};

/// Tracks in-flight request/response pairs sent over UDP, keyed by a
/// correlation ID. The central UDP listener uses `resolve` to deliver
/// a response to whichever caller is waiting on it via `wait_for`.
#[derive(Clone, Default)]
pub struct PendingRequests<T> {
    inner: Arc<Mutex<HashMap<String, oneshot::Sender<T>>>>,
}

impl<T> PendingRequests<T> {
    pub fn new() -> Self {
        Self { inner: Arc::new(Mutex::new(HashMap::new())) }
    }

    /// Registers a pending request and returns a receiver that will fire
    /// once `resolve` is called with the same request_id.
    pub async fn wait_for(&self, request_id: String) -> oneshot::Receiver<T> {
        let (tx, rx) = oneshot::channel();
        self.inner.lock().await.insert(request_id, tx);
        rx
    }

    /// Called by the central listener when a response arrives. Delivers it
    /// to the waiting caller, if any. Silently does nothing if no one is
    /// waiting (e.g. it already timed out, or this is an unsolicited reply).
    pub async fn resolve(&self, request_id: &str, value: T) {
        if let Some(tx) = self.inner.lock().await.remove(request_id) {
            let _ = tx.send(value); // ignore error: caller may have timed out already
        }
    }

    /// Cleans up a registration that timed out, so the map doesn't grow
    /// unboundedly from requests that never got a reply.
    pub async fn cancel(&self, request_id: &str) {
        self.inner.lock().await.remove(request_id);
    }
}