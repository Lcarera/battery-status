//! Alert rules (spec 5.1). No Windows code.

use crate::config::Limits;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AlertLevel {
    Low,
    Critical,
    Empty,
}

fn level_for(percent: u8, limits: &Limits) -> Option<AlertLevel> {
    if percent <= limits.empty {
        Some(AlertLevel::Empty)
    } else if percent <= limits.critical {
        Some(AlertLevel::Critical)
    } else if percent <= limits.low {
        Some(AlertLevel::Low)
    } else {
        None
    }
}

/// Remembers, for each device key, the most urgent level already alerted.
#[derive(Debug, Default)]
pub struct AlertTracker {
    alerted: HashMap<String, AlertLevel>,
}

impl AlertTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a level only when an alert must be sent now.
    pub fn update(&mut self, key: &str, percent: u8, limits: &Limits) -> Option<AlertLevel> {
        if percent > limits.reset {
            self.alerted.remove(key);
            return None;
        }
        let current = level_for(percent, limits)?;
        match self.alerted.get(key) {
            Some(&done) if done >= current => None,
            _ => {
                self.alerted.insert(key.to_string(), current);
                Some(current)
            }
        }
    }

    pub fn forget(&mut self, key: &str) {
        self.alerted.remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(t: &mut AlertTracker, key: &str, values: &[u8]) -> Vec<Option<AlertLevel>> {
        let l = Limits::default();
        values.iter().map(|&v| t.update(key, v, &l)).collect()
    }

    #[test]
    fn above_low_no_alert() {
        assert_eq!(run(&mut AlertTracker::new(), "a", &[50]), vec![None]);
    }

    #[test]
    fn crossing_low_alerts_once() {
        let r = run(&mut AlertTracker::new(), "a", &[21, 20, 19, 20]);
        assert_eq!(r, vec![None, Some(AlertLevel::Low), None, None]);
    }

    #[test]
    fn critical_and_empty_alert() {
        let r = run(&mut AlertTracker::new(), "a", &[20, 10, 5, 3]);
        assert_eq!(
            r,
            vec![Some(AlertLevel::Low), Some(AlertLevel::Critical), Some(AlertLevel::Empty), None]
        );
    }

    #[test]
    fn skip_to_critical_gives_one_alert() {
        let r = run(&mut AlertTracker::new(), "a", &[25, 8, 7]);
        assert_eq!(r, vec![None, Some(AlertLevel::Critical), None]);
    }

    #[test]
    fn no_reset_between_21_and_25() {
        let r = run(&mut AlertTracker::new(), "a", &[20, 24, 20]);
        assert_eq!(r, vec![Some(AlertLevel::Low), None, None]);
    }

    #[test]
    fn reset_above_25() {
        let r = run(&mut AlertTracker::new(), "a", &[20, 26, 20]);
        assert_eq!(r, vec![Some(AlertLevel::Low), None, Some(AlertLevel::Low)]);
    }

    #[test]
    fn first_value_low_alerts() {
        assert_eq!(run(&mut AlertTracker::new(), "a", &[15]), vec![Some(AlertLevel::Low)]);
    }

    #[test]
    fn forget_allows_alert_on_reconnect() {
        let mut t = AlertTracker::new();
        assert_eq!(run(&mut t, "a", &[15]), vec![Some(AlertLevel::Low)]);
        t.forget("a");
        assert_eq!(run(&mut t, "a", &[15]), vec![Some(AlertLevel::Low)]);
    }

    #[test]
    fn keys_are_independent() {
        let mut t = AlertTracker::new();
        assert_eq!(run(&mut t, "a", &[15]), vec![Some(AlertLevel::Low)]);
        assert_eq!(run(&mut t, "b", &[15]), vec![Some(AlertLevel::Low)]);
    }

    #[test]
    fn zero_percent_is_empty() {
        assert_eq!(run(&mut AlertTracker::new(), "a", &[0]), vec![Some(AlertLevel::Empty)]);
    }
}
