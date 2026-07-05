//! Retry policy primitives.

use std::time::Duration;

#[derive(Debug, Clone)]
pub enum RetryPolicy {
    None,
    FixedDelay {
        max_attempts: u32,
        delay: Duration,
    },
    ExponentialBackoff {
        max_attempts: u32,
        initial_delay: Duration,
        max_delay: Duration,
        multiplier: f64,
    },
    Custom {
        max_attempts: u32,
        delay_fn: fn(attempt: u32) -> Duration,
    },
}

impl RetryPolicy {
    ///
    /// ## Example
    ///
    /// ```rust
    /// use corework::retry::RetryPolicy;
    /// use std::time::Duration;
    ///
    /// let policy = RetryPolicy::fixed_delay(3, Duration::from_secs(1));
    /// ```
    pub fn fixed_delay(max_attempts: u32, delay: Duration) -> Self {
        Self::FixedDelay {
            max_attempts,
            delay,
        }
    }

    ///
    /// ## Example
    ///
    /// ```rust
    /// use corework::retry::RetryPolicy;
    /// use std::time::Duration;
    ///
    /// let policy = RetryPolicy::exponential_backoff(
    ///     3,
    ///     Duration::from_millis(100),
    ///     Duration::from_secs(5),
    ///     2.0
    /// );
    /// ```
    pub fn exponential_backoff(
        max_attempts: u32,
        initial_delay: Duration,
        max_delay: Duration,
        multiplier: f64,
    ) -> Self {
        Self::ExponentialBackoff {
            max_attempts,
            initial_delay,
            max_delay,
            multiplier,
        }
    }

    pub fn custom(max_attempts: u32, delay_fn: fn(attempt: u32) -> Duration) -> Self {
        Self::Custom {
            max_attempts,
            delay_fn,
        }
    }

    pub fn max_attempts(&self) -> u32 {
        match self {
            Self::None => 0,
            Self::FixedDelay { max_attempts, .. } => *max_attempts,
            Self::ExponentialBackoff { max_attempts, .. } => *max_attempts,
            Self::Custom { max_attempts, .. } => *max_attempts,
        }
    }

    pub fn delay_for_attempt(&self, attempt: u32) -> Option<Duration> {
        match self {
            Self::None => None,
            Self::FixedDelay {
                max_attempts,
                delay,
            } => {
                if attempt >= *max_attempts {
                    None
                } else {
                    Some(*delay)
                }
            }
            Self::ExponentialBackoff {
                max_attempts,
                initial_delay,
                max_delay,
                multiplier,
            } => {
                if attempt >= *max_attempts {
                    None
                } else {
                    let delay_ms =
                        initial_delay.as_millis() as f64 * multiplier.powi(attempt as i32);
                    let delay = Duration::from_millis(delay_ms as u64);
                    Some(delay.min(*max_delay))
                }
            }
            Self::Custom {
                max_attempts,
                delay_fn,
            } => {
                if attempt >= *max_attempts {
                    None
                } else {
                    Some(delay_fn(attempt))
                }
            }
        }
    }

    pub fn should_retry(&self, attempt: u32) -> bool {
        attempt < self.max_attempts()
    }
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self::exponential_backoff(3, Duration::from_millis(100), Duration::from_secs(30), 2.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_retry() {
        let policy = RetryPolicy::None;
        assert_eq!(policy.max_attempts(), 0);
        assert!(!policy.should_retry(0));
        assert_eq!(policy.delay_for_attempt(0), None);
    }

    #[test]
    fn test_fixed_delay() {
        let policy = RetryPolicy::fixed_delay(3, Duration::from_secs(1));
        assert_eq!(policy.max_attempts(), 3);
        assert!(policy.should_retry(0));
        assert!(policy.should_retry(2));
        assert!(!policy.should_retry(3));
        assert_eq!(policy.delay_for_attempt(0), Some(Duration::from_secs(1)));
        assert_eq!(policy.delay_for_attempt(2), Some(Duration::from_secs(1)));
        assert_eq!(policy.delay_for_attempt(3), None);
    }

    #[test]
    fn test_exponential_backoff() {
        let policy = RetryPolicy::exponential_backoff(
            3,
            Duration::from_millis(100),
            Duration::from_secs(5),
            2.0,
        );
        assert_eq!(policy.max_attempts(), 3);

        assert_eq!(
            policy.delay_for_attempt(0),
            Some(Duration::from_millis(100))
        );
        assert_eq!(
            policy.delay_for_attempt(1),
            Some(Duration::from_millis(200))
        );
        assert_eq!(
            policy.delay_for_attempt(2),
            Some(Duration::from_millis(400))
        );
        assert_eq!(policy.delay_for_attempt(3), None);
    }
}
