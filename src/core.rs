use async_channel::{bounded, Receiver, Sender};
use std::time::Duration;
use tokio::time::Instant;
use std::sync::{Arc, Mutex};

/// Internal state of an API key, shared across multiple pool slots.
#[derive(Debug)]
pub struct ApiKeyInner {
    pub id: String,
    pub tier_limit: u32,
    pub requests_this_second: u32,
    pub last_reset: Instant,
    pub banned_until: Option<Instant>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub _secret: String,
    pub _secret_type: String,
}

/// Represents a handle to an API key with shared rate limiting state.
#[derive(Debug, Clone)]
pub struct ApiKey {
    pub inner: Arc<Mutex<ApiKeyInner>>,
}

impl ApiKey {
    /// Creates a new ApiKey instance with shared state.
    pub fn new(id: &str, tier_limit: u32, secret: String, secret_type: String, expires_at: Option<chrono::DateTime<chrono::Utc>>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(ApiKeyInner {
                id: id.to_string(),
                tier_limit,
                requests_this_second: 0,
                last_reset: Instant::now(),
                banned_until: None,
                expires_at,
                _secret: secret,
                _secret_type: secret_type,
            })),
        }
    }

    pub fn id(&self) -> String {
        self.inner.lock().unwrap().id.clone()
    }

    /// Attempts to use the API key, checking against rate limits and bans.
    pub fn try_use(&self) -> Result<(), &'static str> {
        let mut state = self.inner.lock().unwrap();
        let now = Instant::now();

        if let Some(until) = state.banned_until {
            if now < until {
                return Err("Key is currently cooling down");
            } else {
                state.banned_until = None;
            }
        }

        if let Some(expires) = state.expires_at {
            if chrono::Utc::now() > expires {
                return Err("Key has expired");
            }
        }

        if now.duration_since(state.last_reset).as_secs() >= 1 {
            state.requests_this_second = 0;
            state.last_reset = now;
        }

        if state.requests_this_second >= state.tier_limit {
            return Err("Key tier limit exceeded");
        }

        state.requests_this_second += 1;
        Ok(())
    }

    pub fn _set_cooldown(&self, duration: Duration) {
        let mut state = self.inner.lock().unwrap();
        state.banned_until = Some(Instant::now() + duration);
    }
}

/// A pool of API keys managed via an asynchronous channel.
#[derive(Clone)]
pub struct KeyPool {
    pub sender: Sender<ApiKey>,
    pub receiver: Receiver<ApiKey>,
}

impl KeyPool {
    pub fn new(capacity: usize) -> Self {
        let (sender, receiver) = bounded(capacity);
        Self { sender, receiver }
    }

    pub async fn add_key(&self, key: ApiKey) {
        self.sender.send(key).await.expect("Channel closed");
    }

    pub async fn acquire(&self) -> ApiKey {
        self.receiver.recv().await.expect("Channel closed")
    }

    pub async fn release(&self, key: ApiKey) {
        self.sender.send(key).await.expect("Channel closed");
    }
}
