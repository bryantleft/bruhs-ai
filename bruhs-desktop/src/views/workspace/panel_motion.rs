//! Fraction of panel width visible during a reversible open or close.
use std::time::Instant;
#[derive(Default)]
pub(super) struct Motion {
    target: Option<bool>,
    from: f32,
    started: Option<Instant>,
}
impl Motion {
    fn value(&self, now: Instant, millis: u16) -> f32 {
        let end = if self.target == Some(true) { 1.0 } else { 0.0 };
        let Some(started) = self.started else {
            return end;
        };
        if millis == 0 {
            return end;
        }
        let fraction = (now.saturating_duration_since(started).as_secs_f32() * 1000.0
            / f32::from(millis))
        .clamp(0.0, 1.0);
        if fraction == 1.0 {
            return end;
        }
        let eased = fraction * (2.0 - fraction);
        self.from + (end - self.from) * eased
    }
    pub fn sample(&mut self, open: bool, now: Instant, millis: u16) -> f32 {
        if self.target != Some(open) {
            self.from = self.value(now, millis);
            self.started = self.target.map(|_| now);
            self.target = Some(open);
        }
        let value = self.value(now, millis);
        if value == if open { 1.0 } else { 0.0 } {
            self.started = None
        }
        value
    }
    pub fn running(&self) -> bool {
        self.started.is_some()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn panel_motion_reverses_in_place_and_reduced_motion_finishes_immediately() {
        let mut motion = Motion::default();
        let start = Instant::now();
        assert_eq!(motion.sample(false, start, 200), 0.0);
        assert_eq!(motion.sample(true, start, 200), 0.0);
        let mid = start + std::time::Duration::from_millis(100);
        let halfway = motion.sample(true, mid, 200);
        assert!(halfway > 0.0 && halfway < 1.0);
        assert_eq!(motion.sample(false, mid, 200), halfway);
        assert_eq!(motion.sample(false, mid, 0), 0.0);
        assert!(!motion.running());
    }
}
