use std::collections::VecDeque;
use std::time::{Duration, Instant};

pub struct RateLimiter {
    window: Duration,
    max_events: usize,
    timestamps: VecDeque<Instant>,
}

impl RateLimiter {
    pub fn new(window: Duration, max_events: usize) -> Self {
        Self {
            window,
            max_events,
            timestamps: VecDeque::new(),
        }
    }

    // Check if action is allowed
    pub fn check(&mut self) -> bool {
        self.cleanup_old();

        if self.timestamps.len() < self.max_events {
            self.timestamps.push_back(Instant::now());
            true
        } else {
            false
        }
    }

    // Remove timestamps outside window
    fn cleanup_old(&mut self) {
        let now = Instant::now();
        // Handle potential clock drift/monotonicity issues gracefully
        let cutoff = now.checked_sub(self.window).unwrap_or(now);
        while let Some(&ts) = self.timestamps.front() {
            if ts < cutoff {
                self.timestamps.pop_front();
            } else {
                break;
            }
        }
    }

    // Get current rate
    #[allow(dead_code)]
    pub fn current_rate(&mut self) -> usize {
        self.cleanup_old();
        self.timestamps.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn test_rate_limit_allows() {
        let mut limiter = RateLimiter::new(Duration::from_secs(1), 5);
        for _ in 0..5 {
            assert!(limiter.check());
        }
        assert!(!limiter.check());
    }

    #[test]
    fn test_rate_limit_window_cleanup() {
        let mut limiter = RateLimiter::new(Duration::from_millis(100), 1);
        assert!(limiter.check());
        assert!(!limiter.check());
        thread::sleep(Duration::from_millis(150));
        assert!(limiter.check());
    }
}
