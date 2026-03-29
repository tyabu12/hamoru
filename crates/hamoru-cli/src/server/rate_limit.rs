//! Token-bucket rate limiter for `hamoru serve`.
//!
//! Uses DashMap for per-key sharded concurrency (D2). Each key gets an
//! independent bucket that refills at `requests_per_minute / 60` tokens
//! per second. When auth is disabled, all requests share a single
//! `"__global__"` bucket.
//!
//! A background eviction task removes entries unused for >1 hour.

use std::time::Instant;

use dashmap::DashMap;

/// Per-key token bucket rate limiter.
pub struct RateLimiter {
    buckets: DashMap<String, TokenBucketEntry>,
    requests_per_minute: u32,
}

struct TokenBucketEntry {
    tokens: f64,
    last_refill: Instant,
    last_used: Instant,
}

/// Global key used when auth is disabled.
pub const GLOBAL_KEY: &str = "__global__";

impl RateLimiter {
    /// Creates a new rate limiter with the given per-minute limit.
    pub fn new(requests_per_minute: u32) -> Self {
        Self {
            buckets: DashMap::new(),
            requests_per_minute,
        }
    }

    /// Attempts to consume one token for the given key.
    ///
    /// Returns `Ok(())` if allowed, or `Err(retry_after_secs)` if the
    /// bucket is exhausted.
    pub fn check(&self, key: &str) -> Result<(), u64> {
        let rpm = self.requests_per_minute;
        let refill_rate = f64::from(rpm) / 60.0;
        let now = Instant::now();

        let mut entry = self.buckets.entry(key.to_string()).or_insert_with(|| {
            TokenBucketEntry {
                tokens: f64::from(rpm),
                last_refill: now,
                last_used: now,
            }
        });

        // Refill tokens based on elapsed time
        let elapsed = now.duration_since(entry.last_refill).as_secs_f64();
        entry.tokens = (entry.tokens + elapsed * refill_rate).min(f64::from(rpm));
        entry.last_refill = now;
        entry.last_used = now;

        if entry.tokens >= 1.0 {
            entry.tokens -= 1.0;
            Ok(())
        } else {
            // Calculate retry-after: time until 1 token is available
            let deficit = 1.0 - entry.tokens;
            let wait_secs = deficit / refill_rate;
            // Clamp to [1, 60] seconds
            let retry_after = (wait_secs.ceil() as u64).clamp(1, 60);
            Err(retry_after)
        }
    }

    /// Evicts entries that have been unused for longer than `max_age`.
    pub fn evict_stale(&self, max_age: std::time::Duration) {
        let now = Instant::now();
        self.buckets
            .retain(|_, entry| now.duration_since(entry.last_used) < max_age);
    }

    /// Returns the number of tracked keys (for diagnostics).
    pub fn key_count(&self) -> usize {
        self.buckets.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn allows_requests_under_limit() {
        let limiter = RateLimiter::new(60);
        // First request should always succeed
        assert!(limiter.check("key-0").is_ok());
    }

    #[test]
    fn rejects_burst_over_limit() {
        let limiter = RateLimiter::new(2); // 2 req/min for easy testing
        assert!(limiter.check("key-0").is_ok());
        assert!(limiter.check("key-0").is_ok());
        // Third request should be rejected
        let result = limiter.check("key-0");
        assert!(result.is_err());
        let retry_after = result.unwrap_err();
        assert!(retry_after >= 1);
        assert!(retry_after <= 60);
    }

    #[test]
    fn different_keys_are_independent() {
        let limiter = RateLimiter::new(1);
        assert!(limiter.check("key-0").is_ok());
        assert!(limiter.check("key-0").is_err()); // exhausted
        assert!(limiter.check("key-1").is_ok()); // different key, should work
    }

    #[test]
    fn evicts_stale_entries() {
        let limiter = RateLimiter::new(60);
        limiter.check("key-0").unwrap();
        limiter.check("key-1").unwrap();
        assert_eq!(limiter.key_count(), 2);

        // Evict with zero max_age — everything is stale
        limiter.evict_stale(Duration::ZERO);
        assert_eq!(limiter.key_count(), 0);
    }

    #[test]
    fn keeps_recent_entries_during_eviction() {
        let limiter = RateLimiter::new(60);
        limiter.check("recent").unwrap();

        // Evict with generous max_age — nothing should be removed
        limiter.evict_stale(Duration::from_secs(3600));
        assert_eq!(limiter.key_count(), 1);
    }

    #[test]
    fn global_key_works() {
        let limiter = RateLimiter::new(60);
        assert!(limiter.check(GLOBAL_KEY).is_ok());
        assert_eq!(limiter.key_count(), 1);
    }

    #[test]
    fn retry_after_is_clamped() {
        let limiter = RateLimiter::new(1); // 1 req/min = very slow refill
        limiter.check("key-0").unwrap();
        let retry = limiter.check("key-0").unwrap_err();
        // With 1 RPM, wait should be ~60s, clamped to 60
        assert_eq!(retry, 60);
    }
}
