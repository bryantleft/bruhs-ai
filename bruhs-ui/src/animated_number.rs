//! A tabular number that eases from its last displayed value to a new target.
use crate::{motion, theme};
use gpui::prelude::*;
use gpui::{div, App, ElementId, Role, SharedString, Window};
use std::time::{Duration, Instant};

const DURATION: Duration = Duration::from_millis(180);

#[derive(Default)]
pub struct AnimatedNumber {
    target: Option<f64>,
    transition: Option<Transition>,
}

struct Transition {
    from: f64,
    started: Instant,
}

impl AnimatedNumber {
    /// Keep one instance per metric and reset it when the metric's scope or units change.
    ///
    /// Supply the exact final text, including placeholders, and a formatter for frames
    /// in between. The accessible label always uses the final text.
    pub fn render(
        &mut self,
        id: impl Into<ElementId>,
        target: Option<f64>,
        label: impl Into<SharedString>,
        format: impl FnOnce(f64) -> String,
        window: &Window,
        cx: &App,
    ) -> impl IntoElement {
        let label = label.into();
        let displayed =
            if let Some(value) = self.update(target, Instant::now(), motion::reduce_motion(cx)) {
                window.request_animation_frame();
                format(value).into()
            } else {
                label.clone()
            };
        div()
            .id(id)
            .role(Role::Label)
            .aria_label(label)
            .font_features(theme::tabular_nums())
            .child(displayed)
    }

    // None means the caller can draw the exact supplied final text.
    fn update(&mut self, target: Option<f64>, now: Instant, reduce_motion: bool) -> Option<f64> {
        let target = target.filter(|target| target.is_finite());
        if reduce_motion || target.is_none() {
            self.target = target;
            self.transition = None;
            return None;
        }
        if self.target != target {
            self.transition = self
                .sample(now)
                .or(self.target)
                .map(|from| Transition { from, started: now });
            self.target = target;
        }
        let sample = self.sample(now);
        if sample.is_none() {
            self.transition = None;
        }
        sample
    }

    fn sample(&self, now: Instant) -> Option<f64> {
        let transition = self.transition.as_ref()?;
        let elapsed = now.saturating_duration_since(transition.started);
        if elapsed >= DURATION {
            return None;
        }
        let progress = elapsed.div_duration_f64(DURATION);
        let remaining = (1. - progress).powi(3);
        Some(transition.from * remaining + self.target? * (1. - remaining))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retargets_from_the_visible_value_and_settles() {
        let mut number = AnimatedNumber::default();
        let start = Instant::now();
        assert!(number.update(Some(10.), start, false).is_none());
        let _ = number.update(Some(30.), start, false);
        let halfway = start + DURATION / 2;
        let visible = number.update(Some(30.), halfway, false).unwrap();
        assert!(visible > 10. && visible < 30.);
        let retargeted = number.update(Some(5.), halfway, false).unwrap();
        assert!((retargeted - visible).abs() < f64::EPSILON);
        let falling = number
            .update(Some(5.), halfway + DURATION / 2, false)
            .unwrap();
        assert!(falling < visible && falling > 5.);
        assert!(number.update(Some(5.), halfway + DURATION, false).is_none());
    }

    #[test]
    fn reduced_motion_and_missing_values_clear_the_transition() {
        let mut number = AnimatedNumber::default();
        let now = Instant::now();
        assert!(number.update(Some(10.), now, false).is_none());
        assert!(number.update(Some(20.), now, false).is_some());
        assert!(number.update(Some(20.), now, true).is_none());
        assert!(number.update(Some(20.), now, false).is_none());
        assert!(number.update(None, now, false).is_none());
        assert!(number.update(Some(40.), now, false).is_none());
    }
}
