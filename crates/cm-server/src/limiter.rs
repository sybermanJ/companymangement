//! Failed-login limiter: after `MAX` failures for one email inside `WINDOW`,
//! further attempts are refused until the window passes. In memory, per
//! server instance; a shared store can replace it when running several instances.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const MAX: u32 = 5;
const WINDOW: Duration = Duration::from_secs(15 * 60);

#[derive(Default)]
pub struct LoginLimiter {
    inner: Mutex<HashMap<String, (u32, Instant)>>,
}

impl LoginLimiter {
    fn key(email: &str) -> String {
        email.trim().to_lowercase()
    }

    pub fn is_blocked(&self, email: &str) -> bool {
        let map = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        map.get(&Self::key(email))
            .is_some_and(|(n, t)| *n >= MAX && t.elapsed() < WINDOW)
    }

    pub fn record_failure(&self, email: &str) {
        let mut map = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if map.len() > 100_000 {
            map.retain(|_, (_, t)| t.elapsed() < WINDOW);
        }
        let e = map.entry(Self::key(email)).or_insert((0, Instant::now()));
        if e.1.elapsed() >= WINDOW {
            *e = (0, Instant::now());
        }
        e.0 += 1;
    }

    pub fn clear(&self, email: &str) {
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&Self::key(email));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_after_five_failures_and_clears_on_success() {
        let l = LoginLimiter::default();
        for _ in 0..4 {
            l.record_failure("A@x.co");
        }
        assert!(!l.is_blocked("a@x.co"));
        l.record_failure("a@x.co");
        assert!(l.is_blocked(" a@X.co "));
        l.clear("a@x.co");
        assert!(!l.is_blocked("a@x.co"));
    }
}
