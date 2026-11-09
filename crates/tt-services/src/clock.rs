use tt_domain::Millis;

/// Source of "now" (unix ms, UTC).
pub trait Clock: Send + Sync {
    fn now(&self) -> Millis;
}

/// Wall clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Millis {
        chrono::Utc::now().timestamp_millis()
    }
}

/// Manually driven clock for tests.
#[cfg(any(test, feature = "test-util"))]
#[derive(Debug, Default)]
pub struct FixedClock(std::sync::atomic::AtomicI64);

#[cfg(any(test, feature = "test-util"))]
impl FixedClock {
    pub fn new(t: Millis) -> Self {
        Self(std::sync::atomic::AtomicI64::new(t))
    }

    pub fn set(&self, t: Millis) {
        self.0.store(t, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn advance(&self, ms: i64) {
        self.0.fetch_add(ms, std::sync::atomic::Ordering::SeqCst);
    }
}

#[cfg(any(test, feature = "test-util"))]
impl Clock for FixedClock {
    fn now(&self) -> Millis {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_clock_set_and_advance() {
        let c = FixedClock::new(1_000);
        assert_eq!(c.now(), 1_000);
        c.advance(500);
        assert_eq!(c.now(), 1_500);
        c.set(42);
        assert_eq!(c.now(), 42);
    }

    #[test]
    fn system_clock_is_recent() {
        // 2020-01-01T00:00:00Z
        assert!(SystemClock.now() > 1_577_836_800_000);
    }
}
