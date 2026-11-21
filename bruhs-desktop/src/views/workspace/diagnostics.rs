//! Sample the desktop and its descendants while Diagnostics is visible.
use super::*;
use std::{
    collections::{HashMap, VecDeque},
    time::{Duration, Instant},
};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

#[derive(Clone, Copy, Default)]
struct Usage {
    cpu: f32,
    memory: u64,
    read: f64,
    write: f64,
}
struct Row {
    pid: Pid,
    depth: usize,
    name: String,
    usage: Usage,
}
struct Snapshot {
    at: Instant,
    total: Usage,
    rows: Vec<Row>,
    overhead: Duration,
    rates_ready: bool,
}
struct Sampler {
    system: System,
    previous: HashMap<(Pid, u64), (u64, u64)>,
    last: Option<Instant>,
}
impl Sampler {
    fn new() -> Self {
        Self {
            system: System::new(),
            previous: HashMap::new(),
            last: None,
        }
    }
    fn sample(&mut self, root: Pid) -> Snapshot {
        let at = Instant::now();
        let seconds = self
            .last
            .map(|last| at.duration_since(last).as_secs_f64())
            .filter(|s| *s > 0.);
        self.system
            .refresh_processes_specifics(ProcessesToUpdate::All, ProcessRefreshKind::new());
        let mut children = HashMap::<Pid, Vec<Pid>>::new();
        for (pid, p) in self.system.processes() {
            if let Some(parent) = p.parent() {
                children.entry(parent).or_default().push(*pid);
            }
        }
        let mut tree = vec![(root, 0)];
        let mut next = 0;
        while let Some(&(pid, depth)) = tree.get(next) {
            next += 1;
            if let Some(mut descendants) = children.remove(&pid) {
                descendants.sort();
                tree.extend(
                    descendants
                        .into_iter()
                        .filter(|child| *child != root)
                        .map(|child| (child, depth + 1)),
                );
            }
        }
        let ids: Vec<_> = tree.iter().map(|(pid, _)| *pid).collect();
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&ids),
            ProcessRefreshKind::new()
                .with_cpu()
                .with_memory()
                .with_disk_usage(),
        );
        let mut total = Usage::default();
        let mut counters = HashMap::new();
        let mut rows = Vec::new();
        for (pid, depth) in tree {
            let Some(p) = self.system.process(pid) else {
                continue;
            };
            let io = p.disk_usage();
            let key = (pid, p.start_time());
            let bytes = (io.total_read_bytes, io.total_written_bytes);
            let (read, write) =
                self.previous
                    .get(&key)
                    .zip(seconds)
                    .map_or((0., 0.), |(&(r, w), s)| {
                        (
                            bytes.0.saturating_sub(r) as f64 / s,
                            bytes.1.saturating_sub(w) as f64 / s,
                        )
                    });
            counters.insert(key, bytes);
            let usage = Usage {
                cpu: p.cpu_usage(),
                memory: p.memory(),
                read,
                write,
            };
            total.cpu += usage.cpu;
            total.memory = total.memory.saturating_add(usage.memory);
            total.read += read;
            total.write += write;
            rows.push(Row {
                pid,
                depth,
                name: p.name().to_string_lossy().into_owned(),
                usage,
            });
        }
        self.previous = counters;
        self.last = Some(at);
        Snapshot {
            at,
            total,
            rows,
            overhead: at.elapsed(),
            rates_ready: seconds.is_some(),
        }
    }
}
/// Take two samples so CPU and I/O have a real measurement interval.
pub(super) fn process_snapshot(pid: u32) -> Option<String> {
    let root = Pid::from_u32(pid);
    let mut sampler = Sampler::new();
    sampler.sample(root);
    let started = sampler.system.process(root)?.start_time();
    std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
    let snapshot = sampler.sample(root);
    if sampler.system.process(root)?.start_time() != started {
        return None;
    }
    let usage = snapshot.total;
    Some(format!(
        "CPU {} · RAM {}\nRead {} · Write {}",
        Metric::Cpu.display(usage),
        Metric::Memory.display(usage),
        Metric::Read.display(usage),
        Metric::Write.display(usage)
    ))
}

#[derive(Default)]
pub(super) struct Monitor {
    latest: Option<Snapshot>,
    history: VecDeque<(Instant, Usage)>,
    metric: Metric,
    range: Range,
    pub(super) poll: Option<Task<()>>,
}
#[derive(Clone, Copy, Default)]
enum Metric {
    #[default]
    Cpu,
    Memory,
    Read,
    Write,
}
impl Metric {
    fn key(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Memory => "memory",
            Self::Read => "read",
            Self::Write => "write",
        }
    }
    fn number(self, u: Usage) -> f64 {
        match self {
            Self::Cpu => u.cpu as f64,
            Self::Memory => u.memory as f64,
            Self::Read => u.read,
            Self::Write => u.write,
        }
    }
    fn display(self, u: Usage) -> String {
        let n = self.number(u);
        match self {
            Self::Cpu => format!("{n:.1}%"),
            Self::Memory => format!("{:.1} MiB", n / 1048576.),
            Self::Read | Self::Write => format!("{:.1} KiB/s", n / 1024.),
        }
    }
}
#[derive(Clone, Copy, Default)]
enum Range {
    #[default]
    Five,
    Fifteen,
    Thirty,
    Hour,
}
impl Range {
    fn minutes(self) -> u32 {
        match self {
            Self::Five => 5,
            Self::Fifteen => 15,
            Self::Thirty => 30,
            Self::Hour => 60,
        }
    }
    fn key(self) -> &'static str {
        match self {
            Self::Five => "5",
            Self::Fifteen => "15",
            Self::Thirty => "30",
            Self::Hour => "60",
        }
    }
}
impl Workspace {
    pub(super) fn open_diagnostics(&mut self, cx: &mut Context<'_, Self>) {
        self.read_export_settings(cx);
        self.diagnostics.poll = Some(cx.spawn(async move |this, cx| {
            let mut sampler = Sampler::new();
            loop {
                if this
                    .read_with(cx, |this, _| {
                        this.settings_visible()
                            && this.settings_section == SettingsSection::Diagnostics
                    })
                    .ok()
                    != Some(true)
                {
                    break;
                }
                let first = sampler.last.is_none();
                let (reusable, snapshot) = cx
                    .background_spawn(async move {
                        let sample = sampler.sample(Pid::from_u32(std::process::id()));
                        (sampler, sample)
                    })
                    .await;
                sampler = reusable;
                if this
                    .update(cx, |this, cx| {
                        let monitor = &mut this.diagnostics;
                        while monitor.history.front().is_some_and(|(time, _)| {
                            snapshot.at.duration_since(*time) > Duration::from_secs(3600)
                        }) {
                            monitor.history.pop_front();
                        }
                        if snapshot.rates_ready {
                            monitor.history.push_back((snapshot.at, snapshot.total));
                        }
                        monitor.latest = Some(snapshot);
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
                cx.background_executor()
                    .timer(if first {
                        Duration::from_secs(1)
                    } else {
                        Duration::from_secs(5)
                    })
                    .await;
            }
        }));
    }
    pub(super) fn choose_diagnostic(&mut self, id: &str, value: &str, cx: &mut Context<'_, Self>) {
        if id == "diagnostic-range" {
            self.diagnostics.range = match value {
                "5" => Range::Five,
                "15" => Range::Fifteen,
                "30" => Range::Thirty,
                "60" => Range::Hour,
                _ => return,
            };
        } else {
            self.diagnostics.metric = match value {
                "cpu" => Metric::Cpu,
                "memory" => Metric::Memory,
                "read" => Metric::Read,
                "write" => Metric::Write,
                _ => return,
            };
        }
        cx.notify();
    }

    pub(super) fn diagnostic_rows(&self, cx: &Context<'_, Self>) -> Vec<AnyElement> {
        let monitor = &self.diagnostics;
        let controls = div()
            .flex()
            .flex_wrap()
            .gap_2()
            .child(self.choice_control(
                "diagnostic-metric",
                "Resource",
                monitor.metric.key(),
                &[
                    ("cpu", "CPU"),
                    ("memory", "Memory"),
                    ("read", "Read"),
                    ("write", "Write"),
                ],
                cx,
            ))
            .child(self.choice_control(
                "diagnostic-range",
                "Time range",
                monitor.range.key(),
                &[
                    ("5", "5 min"),
                    ("15", "15 min"),
                    ("30", "30 min"),
                    ("60", "1 hour"),
                ],
                cx,
            ));
        let mut rows = vec![self.settings_row(
            "settings-diagnostic-resources",
            "Resource timeline",
            "Desktop and child processes. Samples are collected while this panel is open.",
            controls.into_any_element(),
        )];
        if let Some(s) = &monitor.latest {
            let window = f64::from(monitor.range.minutes()) * 60.;
            let mut buckets = [None::<f64>; 42];
            for (time, u) in &monitor.history {
                let age = s.at.saturating_duration_since(*time).as_secs_f64();
                if age < window {
                    let i = buckets.len() - 1 - (age * buckets.len() as f64 / window) as usize;
                    let n = monitor.metric.number(*u);
                    buckets[i] = Some(buckets[i].unwrap_or(0.).max(n));
                }
            }
            let peak = buckets
                .iter()
                .flatten()
                .copied()
                .fold(0., f64::max)
                .max(f64::EPSILON);
            let value = if matches!(monitor.metric, Metric::Memory) || s.rates_ready {
                monitor.metric.display(s.total)
            } else {
                "Measuring…".into()
            };
            let graph = div()
                .h(px(78.))
                .w_full()
                .flex()
                .items_end()
                .gap(px(2.))
                .children(buckets.map(|v| {
                    let height = v.map_or(0., |n| ((n / peak) as f32 * 74.).max(2.));
                    div().flex_1().h(px(height)).bg(theme::persimmon_400())
                }));
            rows.push(
                div()
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(value)
                    .child(graph)
                    .into_any_element(),
            );
            let rates = if s.rates_ready {
                format!(
                    "{} CPU · {} read · {} write",
                    Metric::Cpu.display(s.total),
                    Metric::Read.display(s.total),
                    Metric::Write.display(s.total)
                )
            } else {
                "Measuring CPU and I/O…".into()
            };
            rows.push(self.settings_row(
                "settings-diagnostic-processes",
                "Live processes",
                format!(
                    "{} processes · {} · sampled in {} ms",
                    s.rows.len(),
                    Metric::Memory.display(s.total),
                    s.overhead.as_millis()
                ),
                div().child(rates).into_any_element(),
            ));
            for p in &s.rows {
                let cpu = if s.rates_ready {
                    Metric::Cpu.display(p.usage)
                } else {
                    "…".into()
                };
                rows.push(
                    div()
                        .px_3()
                        .py_2()
                        .flex()
                        .gap_2()
                        .justify_between()
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .pl(px((p.depth.min(8) * 10) as f32))
                                .child(format!("{} · {}", p.name, p.pid)),
                        )
                        .child(format!("{cpu} · {}", Metric::Memory.display(p.usage)))
                        .into_any_element(),
                );
            }
        } else {
            rows.push(
                div()
                    .p_3()
                    .child("Collecting the first sample…")
                    .into_any_element(),
            );
        }
        rows.extend(self.render_export_rows(cx));
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::{Pid, Sampler};
    #[cfg(unix)]
    #[test]
    fn thread_sample_contains_only_the_selected_process_tree() {
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let root = Pid::from_u32(child.id());
        let sample = Sampler::new().sample(root);
        let usage = super::process_snapshot(child.id());
        child.kill().unwrap();
        child.wait().unwrap();
        assert_eq!(sample.rows.len(), 1);
        assert_eq!(sample.rows[0].pid, root);
        assert!(usage.is_some_and(|text| text.contains("CPU") && text.contains("RAM")));
    }

    #[test]
    fn sampler_includes_this_process_and_measures_a_second_interval() {
        let mut sampler = Sampler::new();
        let a = sampler.sample(Pid::from_u32(std::process::id()));
        assert!(a
            .rows
            .iter()
            .any(|r| r.pid.as_u32() == std::process::id() && r.depth == 0));
        assert!(a.total.memory > 0 && !a.rates_ready);
        let b = sampler.sample(Pid::from_u32(std::process::id()));
        assert!(b.rates_ready && b.at >= a.at);
        assert_eq!(
            b.total.memory,
            b.rows.iter().map(|r| r.usage.memory).sum::<u64>()
        );
    }
}
