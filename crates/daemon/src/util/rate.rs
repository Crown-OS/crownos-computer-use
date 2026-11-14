use std::time::Instant;

/// A token bucket: `per_second` actions a second, with a burst of the same size.
#[derive(Debug, Clone)]
pub(crate) struct RateLimiter {
    capacity: f64,
    tokens: f64,
    refilled: Instant,
}

impl RateLimiter {
    pub(crate) fn per_second(per_second: u32) -> Self {
        let capacity = f64::from(per_second.max(1));
        Self {
            capacity,
            tokens: capacity,
            refilled: Instant::now(),
        }
    }

    /// Whether one more action may happen now.
    pub(crate) fn allow(&mut self) -> bool {
        self.allow_at(Instant::now())
    }

    fn allow_at(&mut self, now: Instant) -> bool {
        let elapsed = now.saturating_duration_since(self.refilled);
        self.tokens = (self.tokens + elapsed.as_secs_f64() * self.capacity).min(self.capacity);
        self.refilled = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn a_burst_is_allowed_then_refilled_over_time() {
        let mut limiter = RateLimiter::per_second(20);
        let start = Instant::now();
        assert!((0..20).all(|_| limiter.allow_at(start)));
        assert!(!limiter.allow_at(start));
        assert!(limiter.allow_at(start + Duration::from_millis(60)));
        assert!(!limiter.allow_at(start + Duration::from_millis(60)));
    }
}
