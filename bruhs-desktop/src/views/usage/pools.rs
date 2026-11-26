use bruhs_models::{
    agent_extensions::{QuotaWindow, UsageLimitAccount},
    num::ToFloatLossy as _,
};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum Kind {
    Session,
    Weekly,
    Monthly,
    Other,
}

fn window_key(window: &QuotaWindow, provider: &str) -> (Kind, String, String) {
    let name = window.name.to_lowercase();
    let duration = window.duration_minutes;
    let kind = match provider {
        "Codex" => match (name.as_str(), duration) {
            ("individual", _) | (_, Some(43200..)) => Kind::Monthly,
            (_, Some(10080..)) | ("secondary", None) => Kind::Weekly,
            _ => Kind::Session,
        },
        "Cursor" => Kind::Monthly,
        "Grok" => match name.as_str() {
            "weekly" => Kind::Weekly,
            "monthly" => Kind::Monthly,
            _ => Kind::Other,
        },
        _ if duration == Some(300)
            || matches!(name.as_str(), "session" | "5-hour" | "five_hour") =>
        {
            Kind::Session
        }
        _ if duration == Some(10080)
            || name.starts_with("weekly")
            || name.starts_with("seven_day") =>
        {
            Kind::Weekly
        }
        _ if duration == Some(43200) || name.starts_with("monthly") || name == "go · monthly" => {
            Kind::Monthly
        }
        _ => Kind::Other,
    };
    let standard = matches!(
        name.as_str(),
        "primary"
            | "secondary"
            | "session"
            | "5-hour"
            | "five_hour"
            | "weekly"
            | "seven_day"
            | "monthly"
    );
    let label = if provider == "Codex" || standard {
        match (kind, name.as_str()) {
            (Kind::Session, _) => "Session".into(),
            (Kind::Weekly, _) => "Weekly".into(),
            (Kind::Monthly, "individual") => "Monthly credit limit".into(),
            (Kind::Monthly, _) => "Monthly".into(),
            (Kind::Other, _) => window.name.clone(),
        }
    } else if let Some(model) = window.name.strip_prefix("seven_day_") {
        let mut model = model.replace('_', " ");
        if let Some(first) = model.get_mut(..1) {
            first.make_ascii_uppercase();
        }
        format!("Weekly · {model}")
    } else {
        let mut label = window.name.replace('_', " ");
        if let Some(first) = label.get_mut(..1) {
            first.make_ascii_uppercase();
        }
        label
    };
    let key = if provider == "Codex" {
        label.clone()
    } else {
        window.window_id.clone().unwrap_or_else(|| label.clone())
    };
    (kind, key, label)
}

fn plan_weight(account: &UsageLimitAccount, kind: Kind) -> Option<f64> {
    let plan = account.plan.as_deref()?.to_ascii_lowercase();
    match (account.provider.as_str(), plan.as_str(), kind) {
        ("Codex", "plus", Kind::Session | Kind::Weekly) | ("Claude", "pro", Kind::Session) => {
            Some(1.)
        }
        ("Codex", "prolite", Kind::Session | Kind::Weekly) | ("Claude", "max", Kind::Session) => {
            Some(5.)
        }
        ("Codex", "pro", Kind::Session | Kind::Weekly) | ("Claude", "ultra", Kind::Session) => {
            Some(20.)
        }
        _ => None,
    }
}

#[derive(Clone, PartialEq)]
pub(super) struct Segment {
    pub(super) left: f64,
    pub(super) reset: Option<i64>,
    pub(super) time_left: Option<f64>,
    refill: f64,
}

impl Segment {
    fn new(account: &UsageLimitAccount, window: &QuotaWindow, kind: Kind, now: i64) -> Self {
        let used = window.used_percent.clamp(0., 100.);
        if kind == Kind::Session {
            if let Some(week) = account.windows.iter().find(|week| {
                week.used_percent >= 100.
                    && match account.provider.as_str() {
                        "Codex" => window_key(week, "Codex").0 == Kind::Weekly,
                        "Claude" => {
                            matches!(week.name.to_lowercase().as_str(), "weekly" | "seven_day")
                        }
                        _ => false,
                    }
            }) {
                let current = now.max(window.observed_at).max(week.observed_at);
                let (reset, refill) = match window.resets_at.zip(week.resets_at) {
                    Some((session, weekly)) if session > current && weekly > current => {
                        if session <= weekly {
                            (Some(weekly), 100.)
                        } else if used >= 100. {
                            (Some(session), 100.)
                        } else {
                            (Some(weekly), 100. - used)
                        }
                    }
                    _ => (None, 0.),
                };
                return Self {
                    left: 0.,
                    reset,
                    time_left: None,
                    refill,
                };
            }
        }
        let time_left = window
            .resets_at
            .zip(window.duration_minutes.filter(|duration| *duration > 0))
            .map(|(reset, duration)| {
                (reset.saturating_sub(now).to_f64_lossy() / (f64::from(duration) * 60.))
                    .clamp(0., 1.)
            });
        Self {
            left: 100. - used,
            reset: window.resets_at,
            time_left,
            refill: used,
        }
    }

    pub(super) fn refill(&self) -> f64 {
        if self.reset.is_some() {
            self.refill.min(100. - self.left)
        } else {
            0.
        }
    }
}

#[derive(Default)]
pub(super) struct Pool {
    pub(super) label: String,
    pub(super) description: Option<&'static str>,
    pub(super) segments: BTreeMap<usize, Segment>,
    pub(super) left: f64,
    pub(super) pace: Option<&'static str>,
    pub(super) refill: Option<(i64, f64)>,
}

impl Pool {
    fn measure(&mut self, accounts: &[&UsageLimitAccount], kind: Kind) {
        let weighted = self
            .segments
            .keys()
            .all(|&index| plan_weight(accounts[index], kind).is_some());
        let weight = |index| {
            if weighted {
                plan_weight(accounts[index], kind).unwrap_or(1.)
            } else {
                1.
            }
        };
        let total: f64 = self.segments.keys().copied().map(weight).sum();
        self.left = self
            .segments
            .iter()
            .map(|(&index, segment)| segment.left * weight(index))
            .sum::<f64>()
            / total;
        let (pace, timed) =
            self.segments
                .iter()
                .fold((0., 0.), |(pace, timed), (&index, segment)| {
                    segment.time_left.map_or((pace, timed), |fraction| {
                        (
                            pace + (segment.left - fraction * 100.) * weight(index),
                            timed + weight(index),
                        )
                    })
                });
        self.pace = (timed > 0.).then(|| match pace / timed {
            delta if delta < -5. => "Ahead of pace",
            delta if delta > 5. => "Under pace",
            _ => "On pace",
        });
        self.refill = self
            .segments
            .iter()
            .filter_map(|(&index, segment)| {
                let gain = (segment.refill * weight(index) / total).round();
                segment
                    .reset
                    .filter(|_| gain > 0.)
                    .map(|reset| (reset, gain))
            })
            .min_by_key(|(reset, _)| *reset);
    }
}

pub(super) fn collect(
    provider: &str,
    accounts: &[&UsageLimitAccount],
    now: i64,
) -> (Vec<usize>, Vec<(Kind, Pool)>) {
    let mut rows = BTreeMap::<(Kind, String), Pool>::new();
    for (index, account) in accounts.iter().enumerate() {
        for window in &account.windows {
            let (kind, mut key, label) = window_key(window, provider);
            if rows
                .get(&(kind, key.clone()))
                .is_some_and(|row| row.segments.contains_key(&index))
            {
                key.push('/');
                key.push_str(&window.name);
            }
            let row = rows.entry((kind, key)).or_insert_with(|| Pool {
                label,
                ..Pool::default()
            });
            row.segments
                .insert(index, Segment::new(account, window, kind, now));
        }
    }
    for ((kind, _), row) in &mut rows {
        row.measure(accounts, *kind);
    }
    let first = rows.first_key_value().map(|(_, row)| row);
    let mut order: Vec<_> = (0..accounts.len()).collect();
    order.sort_by(|&a, &b| {
        let reset = |index| {
            first
                .and_then(|row| row.segments.get(&index))
                .and_then(|segment| segment.reset)
                .unwrap_or(i64::MAX)
        };
        reset(a).cmp(&reset(b)).then_with(|| {
            accounts[a]
                .label
                .to_ascii_lowercase()
                .cmp(&accounts[b].label.to_ascii_lowercase())
        })
    });

    if provider == "Cursor" {
        let total = (Kind::Monthly, "totalPercentUsed".to_owned());
        let models = rows.get(&(Kind::Monthly, "autoPercentUsed".to_owned()));
        let other = rows.get(&(Kind::Monthly, "apiPercentUsed".to_owned()));
        let shared = rows.len() == 2
            && rows
                .get(&total)
                .zip(models)
                .is_some_and(|(a, b)| a.segments == b.segments);
        if models.is_some() && (other.is_some() || shared) {
            rows.remove(&total);
        }
        let total_only = rows.len() == 1 && rows.contains_key(&total);
        for ((_, key), row) in &mut rows {
            row.description = match key.as_str() {
                "totalPercentUsed" if !total_only => {
                    Some("Total usage for the current billing cycle.")
                }
                "autoPercentUsed" if !shared => {
                    Some("Grok, Composer, and Auto draw from this allowance.")
                }
                "apiPercentUsed" => Some("Third-party models draw from this allowance."),
                _ => None,
            };
            if shared || total_only {
                row.label.clear();
            }
        }
    }
    let mut rows: Vec<_> = rows.into_iter().collect();
    if provider == "Cursor" {
        rows.sort_by_key(|((_, key), _)| match key.as_str() {
            "totalPercentUsed" => 0,
            "autoPercentUsed" => 1,
            "apiPercentUsed" => 2,
            _ => 3,
        });
    }
    (
        order,
        rows.into_iter()
            .map(|((kind, _), row)| (kind, row))
            .collect(),
    )
}
