//! Rate limits: token buckets by who is asking and for what.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Mutex;
use std::time::{Duration, Instant};

struct Bucket {
    tokens: f64,
    at: Instant,
}

/// Allows `burst` at once and `burst` per `per` after that, for each key.
pub struct Limiter<K> {
    burst: f64,
    per: Duration,
    buckets: Mutex<HashMap<K, Bucket>>,
}

impl<K: Hash + Eq> Limiter<K> {
    pub fn new(burst: u32, per: Duration) -> Limiter<K> {
        Limiter {
            burst: f64::from(burst),
            per,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// Takes one from `key`'s allowance, or says how long until there is one.
    pub fn take(&self, key: K) -> Result<(), Duration> {
        self.take_at(key, Instant::now())
    }

    fn take_at(&self, key: K, now: Instant) -> Result<(), Duration> {
        let rate = self.burst / self.per.as_secs_f64();
        let mut buckets = self.buckets.lock().unwrap_or_else(|e| e.into_inner());
        let bucket = buckets.entry(key).or_insert(Bucket {
            tokens: self.burst,
            at: now,
        });
        let gained = now.saturating_duration_since(bucket.at).as_secs_f64() * rate;
        bucket.tokens = (bucket.tokens + gained).min(self.burst);
        bucket.at = now;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            Ok(())
        } else {
            Err(Duration::from_secs_f64((1.0 - bucket.tokens) / rate))
        }
    }

    /// Forgets the keys whose allowance is whole again.
    pub fn sweep(&self) {
        let now = Instant::now();
        let mut buckets = self.buckets.lock().unwrap_or_else(|e| e.into_inner());
        buckets.retain(|_, b| now.saturating_duration_since(b.at) < self.per);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_burst_then_a_steady_trickle() {
        let limiter = Limiter::new(2, Duration::from_secs(10));
        let start = Instant::now();
        assert!(limiter.take_at("a", start).is_ok());
        assert!(limiter.take_at("a", start).is_ok());
        let wait = limiter.take_at("a", start).unwrap_err();
        assert_eq!(wait, Duration::from_secs(5));
        assert!(limiter.take_at("b", start).is_ok());
        assert!(limiter.take_at("a", start + Duration::from_secs(5)).is_ok());
        assert!(
            limiter
                .take_at("a", start + Duration::from_secs(6))
                .is_err()
        );
    }
}
