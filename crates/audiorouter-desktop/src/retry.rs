use std::time::{Duration, Instant};

pub struct RetryTask {
    attempts: u32,
    pending_until: Option<Instant>,
    next_attempt: Option<Instant>,
}

impl RetryTask {
    pub fn new(now: Instant) -> Self {
        Self {
            attempts: 0,
            pending_until: None,
            next_attempt: Some(now),
        }
    }

    pub fn begin(&mut self, now: Instant, limit: u8, timeout: Duration) -> bool {
        if self.pending_until.is_some() || !self.next_attempt.is_some_and(|next| next <= now) {
            return false;
        }
        if limit != 0 && self.attempts >= u32::from(limit) {
            self.next_attempt = None;
            return false;
        }
        self.attempts = self.attempts.saturating_add(1);
        self.pending_until = Some(now + timeout);
        self.next_attempt = None;
        true
    }

    pub fn failed(&mut self, now: Instant, interval: Duration) {
        if self.pending_until.take().is_some() {
            self.next_attempt = Some(now + interval);
        }
    }

    pub fn timed_out(&mut self, now: Instant, interval: Duration) {
        if self.pending_until.is_some_and(|deadline| deadline <= now) {
            self.failed(now, interval);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_for_failure_and_then_the_full_interval() {
        let now = Instant::now();
        let mut task = RetryTask::new(now);
        assert!(task.begin(now, 3, Duration::from_secs(5)));
        assert!(!task.begin(now + Duration::from_secs(30), 3, Duration::from_secs(5)));
        task.failed(now + Duration::from_secs(30), Duration::from_secs(2));
        assert!(!task.begin(now + Duration::from_secs(31), 3, Duration::from_secs(5)));
        assert!(task.begin(now + Duration::from_secs(32), 3, Duration::from_secs(5)));
    }

    #[test]
    fn honors_all_finite_limits_without_restarting_on_discovery() {
        let now = Instant::now();
        for limit in 1..=5 {
            let mut task = RetryTask::new(now);
            for attempt in 0..limit {
                let time = now + Duration::from_secs(u64::from(attempt));
                assert!(task.begin(time, limit, Duration::from_secs(1)));
                task.failed(time, Duration::from_secs(1));
            }
            assert!(!task.begin(
                now + Duration::from_secs(100),
                limit,
                Duration::from_secs(1)
            ));
            assert!(!task.begin(
                now + Duration::from_secs(200),
                limit,
                Duration::from_secs(1)
            ));
        }
    }

    #[test]
    fn zero_means_continuous_and_duplicate_failure_does_not_reset_wait() {
        let now = Instant::now();
        let mut task = RetryTask::new(now);
        for attempt in 0..300 {
            let time = now + Duration::from_secs(attempt);
            assert!(task.begin(time, 0, Duration::from_secs(1)));
            task.failed(time, Duration::from_secs(1));
            task.failed(time + Duration::from_secs(50), Duration::from_secs(1));
        }
    }
}
