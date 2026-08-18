//! Time as an injected dependency.
//!
//! Expiry and revocation are the two controls that make a stolen token
//! survivable, and both are time comparisons. A test that cannot move time
//! cannot prove either one, so the clock is a seam rather than a call to
//! [`SystemTime::now`](std::time::SystemTime::now) buried in the verifier.

use std::time::{SystemTime, UNIX_EPOCH};

/// A whole-second instant on the Unix timeline, UTC.
///
/// Seconds, not milliseconds: every consumer here is an expiry or a revocation
/// boundary, and sub-second precision on those would be false precision.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Timestamp(i64);

impl Timestamp {
    #[must_use]
    pub const fn from_unix_seconds(seconds: i64) -> Self {
        Self(seconds)
    }

    #[must_use]
    pub const fn as_unix_seconds(self) -> i64 {
        self.0
    }

    /// Adds a whole number of seconds, saturating rather than wrapping.
    ///
    /// Saturating is deliberate: wrapping an expiry past the epoch boundary
    /// would turn a far-future expiry into a past one, silently invalidating a
    /// token instead of failing loudly.
    #[must_use]
    pub const fn saturating_add_seconds(self, seconds: i64) -> Self {
        Self(self.0.saturating_add(seconds))
    }

    /// Whether `self` is strictly before `other`.
    #[must_use]
    pub const fn is_before(self, other: Self) -> bool {
        self.0 < other.0
    }
}

/// A source of the current time.
pub trait Clock: Send + Sync {
    fn now(&self) -> Timestamp;
}

/// The operating system clock.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        // A clock before the Unix epoch is not a condition this system can act
        // on, and panicking in a request path is forbidden. Clamping to the
        // epoch makes every token appear expired, which fails closed.
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(Timestamp(0), |elapsed| {
                Timestamp(i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX))
            })
    }
}

/// A clock that reports whatever a test set it to.
#[derive(Clone, Copy, Debug)]
pub struct FixedClock(Timestamp);

impl FixedClock {
    #[must_use]
    pub const fn at_unix_seconds(seconds: i64) -> Self {
        Self(Timestamp::from_unix_seconds(seconds))
    }

    /// Moves the clock forward. Takes `&mut self` so a test cannot move time
    /// from behind a shared reference and surprise a concurrent reader.
    pub const fn advance_seconds(&mut self, seconds: i64) {
        self.0 = self.0.saturating_add_seconds(seconds);
    }
}

impl Clock for FixedClock {
    fn now(&self) -> Timestamp {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::{Clock, FixedClock, SystemClock, Timestamp};

    #[test]
    fn timestamps_order_on_the_timeline() {
        let earlier = Timestamp::from_unix_seconds(100);
        let later = Timestamp::from_unix_seconds(200);
        assert!(earlier.is_before(later));
        assert!(!later.is_before(earlier));
        assert!(!earlier.is_before(earlier));
    }

    #[test]
    fn adding_seconds_saturates_rather_than_wrapping() {
        let far_future = Timestamp::from_unix_seconds(i64::MAX);
        assert_eq!(far_future.saturating_add_seconds(1), far_future);
        // Wrapping here would turn the furthest possible expiry into an
        // already-expired one.
        assert!(!far_future.saturating_add_seconds(1).is_before(far_future));
    }

    #[test]
    fn a_fixed_clock_reports_and_advances_deterministically() {
        let mut clock = FixedClock::at_unix_seconds(1_000);
        assert_eq!(clock.now(), Timestamp::from_unix_seconds(1_000));
        clock.advance_seconds(60);
        assert_eq!(clock.now(), Timestamp::from_unix_seconds(1_060));
    }

    #[test]
    fn the_system_clock_is_after_the_repository_was_written() {
        // A loose lower bound: enough to catch a clock stuck at the epoch,
        // which would make every token appear expired.
        let now = SystemClock.now();
        assert!(
            now.as_unix_seconds() > 1_700_000_000,
            "system clock reported {now:?}"
        );
    }
}
