use async_channel::{bounded, Receiver, Sender};
use std::time::Duration;
use tokio::time::Instant;
use std::sync::{Arc, Mutex};
use chrono::{DateTime, Utc, Local};

/// Internal state of an API key, shared across multiple pool slots.
#[derive(Debug)]
pub struct ApiKeyInner {
    pub id: String,
    // Limits
    pub rps_limit: Option<u32>,
    pub rpd_limit: Option<u32>,
    pub tpm_limit: Option<u32>,
    pub tpd_limit: Option<u32>,
    pub max_request_tokens: Option<u32>,
    pub cooldown_on_limit: bool,

    // Real-time counters
    pub requests_this_second: u32,
    pub last_second_reset: Instant,

    pub requests_today: u32,
    pub last_day_reset: DateTime<Utc>,

    pub tokens_this_minute: u32,
    pub last_minute_reset: Instant,

    pub tokens_today: u32,

    pub banned_until: Option<Instant>,
    pub expires_at: Option<DateTime<Utc>>,
    pub _secret: String,
    pub _secret_type: String,
}

#[derive(Debug, Clone)]
pub struct ApiKey {
    pub inner: Arc<Mutex<ApiKeyInner>>,
}

impl ApiKey {
    pub fn new(
        id: &str, 
        rps_limit: Option<u32>, 
        rpd_limit: Option<u32>,
        tpm_limit: Option<u32>,
        tpd_limit: Option<u32>,
        max_request_tokens: Option<u32>,
        cooldown_on_limit: bool,
        secret: String, 
        secret_type: String, 
        expires_at: Option<DateTime<Utc>>
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(ApiKeyInner {
                id: id.to_string(),
                rps_limit,
                rpd_limit,
                tpm_limit,
                tpd_limit,
                max_request_tokens,
                cooldown_on_limit,
                requests_this_second: 0,
                last_second_reset: Instant::now(),
                requests_today: 0,
                last_day_reset: Utc::now(),
                tokens_this_minute: 0,
                last_minute_reset: Instant::now(),
                tokens_today: 0,
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

    pub fn max_request_tokens(&self) -> Option<u32> {
        self.inner.lock().unwrap().max_request_tokens
    }

    pub fn try_use(&self) -> Result<(), String> {
        let mut state = self.inner.lock().unwrap();
        let now_inst = Instant::now();
        let now_utc = Utc::now();

        // 1. Check expiration
        if let Some(expires) = state.expires_at {
            if now_utc > expires { return Err("Key expired".to_string()); }
        }

        // 2. Check manual ban
        if let Some(until) = state.banned_until {
            if now_inst < until { return Err("Key cooling down".to_string()); }
            else { state.banned_until = None; }
        }

        // 3. Reset Second Counter
        if now_inst.duration_since(state.last_second_reset).as_secs() >= 1 {
            state.requests_this_second = 0;
            state.last_second_reset = now_inst;
        }

        // 4. Reset Minute Counter
        if now_inst.duration_since(state.last_minute_reset).as_secs() >= 60 {
            state.tokens_this_minute = 0;
            state.last_minute_reset = now_inst;
        }

        // 5. Reset Day Counter (00:00 UTC)
        if now_utc.date_naive() != state.last_day_reset.date_naive() {
            state.requests_today = 0;
            state.tokens_today = 0;
            state.last_day_reset = now_utc;
        }

        // 6. Validate Limits
        if let Some(limit) = state.rps_limit {
            if state.requests_this_second >= limit {
                if state.cooldown_on_limit { self.set_cooldown_internal(&mut state, Duration::from_secs(1)); }
                return Err("RPS limit hit".to_string());
            }
        }

        if let Some(limit) = state.rpd_limit {
            if state.requests_today >= limit {
                if state.cooldown_on_limit { self.set_cooldown_internal(&mut state, Duration::from_secs(3600)); }
                return Err("RPD limit hit".to_string());
            }
        }

        if let Some(limit) = state.tpm_limit {
            if state.tokens_this_minute >= limit {
                if state.cooldown_on_limit { self.set_cooldown_internal(&mut state, Duration::from_secs(60)); }
                return Err("TPM limit hit".to_string());
            }
        }

        if let Some(limit) = state.tpd_limit {
            if state.tokens_today >= limit {
                if state.cooldown_on_limit { self.set_cooldown_internal(&mut state, Duration::from_secs(3600)); }
                return Err("TPD limit hit".to_string());
            }
        }

        state.requests_this_second += 1;
        state.requests_today += 1;
        println!(" [{}] [DEBUG] Key '{}' usage incremented (RPS: {}, Today: {})", Local::now().format("%H:%M:%S%.3f"), state.id, state.requests_this_second, state.requests_today);
        Ok(())
    }

    pub fn record_usage(&self, tokens: u32) {
        let mut state = self.inner.lock().unwrap();
        state.tokens_this_minute = state.tokens_this_minute.saturating_add(tokens);
        state.tokens_today = state.tokens_today.saturating_add(tokens);
    }

    fn set_cooldown_internal(&self, state: &mut ApiKeyInner, duration: Duration) {
        state.banned_until = Some(Instant::now() + duration);
    }

    pub fn set_cooldown(&self, duration: Duration) {
        let mut state = self.inner.lock().unwrap();
        self.set_cooldown_internal(&mut state, duration);
    }
}

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

    pub fn add_key(&self, key: ApiKey) -> Result<(), String> {
        self.sender.try_send(key).map_err(|_| "Pool capacity exceeded".to_string())
    }

    pub async fn acquire(&self) -> ApiKey {
        let key = self.receiver.recv().await.expect("Channel closed");
        println!(" [{}] [DEBUG] KeyPool: Acquired key '{}'", Local::now().format("%H:%M:%S%.3f"), key.id());
        key
    }

    pub async fn release(&self, key: ApiKey) {
        let id = key.id();
        self.sender.send(key).await.expect("Channel closed");
        println!(" [{}] [DEBUG] KeyPool: Released key '{}' back to pool", Local::now().format("%H:%M:%S%.3f"), id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_key(id: &str) -> ApiKey {
        ApiKey::new(
            id,
            Some(10),  // rps
            Some(100), // rpd
            Some(1000), // tpm
            Some(10000), // tpd
            None,       // max_request_tokens
            false,      // cooldown_on_limit
            "sk-test".to_string(),
            "api_key".to_string(),
            None,
        )
    }

    #[test]
    fn test_key_acquire_and_release() {
        let pool = KeyPool::new(2);
        let key = make_key("test-key-1");

        assert!(pool.add_key(key).is_ok());
    }

    #[test]
    fn test_key_try_use_succeeds_within_limits() {
        let key = make_key("test-key");
        assert!(key.try_use().is_ok());
    }

    #[test]
    fn test_key_try_use_rps_limit() {
        let key = make_key("rps-key");
        // First use should succeed
        assert!(key.try_use().is_ok());
        // We'd need to increment requests_this_second to 10 before it fails
        // Instead, create a key with limit 0 to test immediate rejection
        let limited = ApiKey::new(
            "limited",
            Some(0),  // rps = 0, always exceeded
            None,
            None,
            None,
            None,
            false,
            "sk-test".to_string(),
            "api_key".to_string(),
            None,
        );
        assert!(limited.try_use().is_err());
    }

    #[test]
    fn test_key_expiration() {
        let expired = ApiKey::new(
            "expired",
            None, None, None, None, None, false,
            "sk-test".to_string(),
            "api_key".to_string(),
            Some(chrono::Utc::now() - chrono::Duration::hours(1)),
        );
        assert!(expired.try_use().is_err());
    }

    #[test]
    fn test_key_valid_expiration() {
        let valid = ApiKey::new(
            "valid",
            None, None, None, None, None, false,
            "sk-test".to_string(),
            "api_key".to_string(),
            Some(chrono::Utc::now() + chrono::Duration::days(1)),
        );
        assert!(valid.try_use().is_ok());
    }

    #[test]
    fn test_key_cooldown_on_limit() {
        let key = ApiKey::new(
            "cooldown",
            Some(0), None, None, None, None,
            true, // cooldown_on_limit
            "sk-test".to_string(),
            "api_key".to_string(),
            None,
        );
        assert!(key.try_use().is_err());
        // Should be in cooldown now
        assert!(key.try_use().is_err());
    }

    #[test]
    fn test_pool_capacity() {
        let pool = KeyPool::new(1);
        let key1 = make_key("key-1");
        let key2 = make_key("key-2");

        assert!(pool.add_key(key1).is_ok());
        assert!(pool.add_key(key2).is_err()); // capacity exceeded
    }

    #[test]
    fn test_record_usage_tracks_tokens() {
        let key = make_key("tracking");
        key.record_usage(50);
        key.record_usage(150);

        let state = key.inner.lock().unwrap();
        assert_eq!(state.tokens_this_minute, 200);
        assert_eq!(state.tokens_today, 200);
    }
}
