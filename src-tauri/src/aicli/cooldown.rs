//! Two-level cooldown policy; a clock is injected for deterministic acceptance.
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone, Hash, PartialEq, Eq)]
enum Key {
    Line(String),
    Model(String, String),
}
struct Failure {
    count: u32,
    last: Instant,
}

pub struct Cooldown {
    pub lines: Mutex<HashMap<String, Instant>>,
    pub models: Mutex<HashMap<(String, String), Instant>>,
    failures: Mutex<HashMap<Key, Failure>>,
    clock: Arc<dyn Fn() -> Instant + Send + Sync>,
}
impl Default for Cooldown {
    fn default() -> Self {
        Self::with_clock(Arc::new(Instant::now))
    }
}
impl Cooldown {
    pub fn with_clock(clock: Arc<dyn Fn() -> Instant + Send + Sync>) -> Self {
        Self {
            lines: Default::default(),
            models: Default::default(),
            failures: Default::default(),
            clock,
        }
    }
    pub fn fail(&self, line: &str, model: Option<&str>, retry_after: Option<Duration>) -> Instant {
        let now = (self.clock)();
        let key = match model {
            Some(model) => Key::Model(line.into(), model.into()),
            None => Key::Line(line.into()),
        };
        let mut failures = self.failures.lock().unwrap_or_else(|e| e.into_inner());
        let failure = failures.entry(key).or_insert(Failure {
            count: 0,
            last: now,
        });
        if now.saturating_duration_since(failure.last) >= Duration::from_secs(1800) {
            failure.count = 0;
        }
        failure.count = failure.count.saturating_add(1);
        failure.last = now;
        let cap = if model.is_some() { 1800 } else { 600 };
        let backoff = Duration::from_secs((60u64 << (failure.count - 1).min(10)).min(cap));
        let delay = backoff.max(
            retry_after
                .unwrap_or_default()
                .min(Duration::from_secs(3600)),
        );
        let until = now + delay;
        match model {
            Some(model) => {
                self.models
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert((line.into(), model.into()), until);
            }
            None => {
                self.lines
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(line.into(), until);
            }
        }
        until
    }
    pub fn success(&self, line: &str, model: Option<&str>) {
        let mut failures = self.failures.lock().unwrap_or_else(|e| e.into_inner());
        failures.remove(&Key::Line(line.into()));
        self.lines
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(line);
        if let Some(model) = model {
            failures.remove(&Key::Model(line.into(), model.into()));
            self.models
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&(line.into(), model.into()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Cooldown, Arc<Mutex<Instant>>) {
        let now = Arc::new(Mutex::new(Instant::now()));
        let clock = now.clone();
        (
            Cooldown::with_clock(Arc::new(move || *clock.lock().unwrap())),
            now,
        )
    }
    #[test]
    fn line_backoff_sequence_and_cap() {
        let (cooldown, now) = fixture();
        for seconds in [
            60, 120, 240, 480, 600, 600, 600, 600, 600, 600, 600, 600, 600,
        ] {
            assert_eq!(
                cooldown.fail("main", None, None) - *now.lock().unwrap(),
                Duration::from_secs(seconds)
            );
        }
        assert!(cooldown.models.lock().unwrap().is_empty());
    }
    #[test]
    fn model_backoff_sequence_and_cap_are_independent() {
        let (cooldown, now) = fixture();
        for seconds in [60, 120, 240, 480, 960, 1800, 1800] {
            assert_eq!(
                cooldown.fail("main", Some("a"), None) - *now.lock().unwrap(),
                Duration::from_secs(seconds)
            );
        }
        assert_eq!(
            cooldown.fail("main", Some("b"), None) - *now.lock().unwrap(),
            Duration::from_secs(60)
        );
        assert_eq!(
            cooldown.fail("global", Some("a"), None) - *now.lock().unwrap(),
            Duration::from_secs(60)
        );
        assert!(cooldown.lines.lock().unwrap().is_empty());
    }
    #[test]
    fn retry_after_extends_only_and_is_capped() {
        let (cooldown, now) = fixture();
        for (after, expected) in [(10, 60), (300, 300), (100, 240), (99999, 3600)] {
            assert_eq!(
                cooldown.fail("main", Some("a"), Some(Duration::from_secs(after)))
                    - *now.lock().unwrap(),
                Duration::from_secs(expected)
            );
        }
    }
    #[test]
    fn thirty_minutes_without_failure_resets_count() {
        let (cooldown, now) = fixture();
        cooldown.fail("main", None, None);
        *now.lock().unwrap() += Duration::from_secs(1799);
        assert_eq!(
            cooldown.fail("main", None, None) - *now.lock().unwrap(),
            Duration::from_secs(120)
        );
        *now.lock().unwrap() += Duration::from_secs(1800);
        assert_eq!(
            cooldown.fail("main", None, None) - *now.lock().unwrap(),
            Duration::from_secs(60)
        );
    }
    #[test]
    fn success_clears_corresponding_entries_and_counts_only() {
        let (cooldown, now) = fixture();
        cooldown.fail("main", None, None);
        cooldown.fail("main", Some("a"), None);
        cooldown.fail("main", Some("b"), None);
        cooldown.fail("global", None, None);
        cooldown.success("main", Some("a"));
        assert!(!cooldown.lines.lock().unwrap().contains_key("main"));
        assert!(cooldown.lines.lock().unwrap().contains_key("global"));
        assert!(!cooldown
            .models
            .lock()
            .unwrap()
            .contains_key(&("main".into(), "a".into())));
        assert!(cooldown
            .models
            .lock()
            .unwrap()
            .contains_key(&("main".into(), "b".into())));
        assert_eq!(
            cooldown.fail("main", None, None) - *now.lock().unwrap(),
            Duration::from_secs(60)
        );
        assert_eq!(
            cooldown.fail("main", Some("a"), None) - *now.lock().unwrap(),
            Duration::from_secs(60)
        );
    }
}
