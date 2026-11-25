//! Sample the desktop and its descendants while Diagnostics is visible.
use super::*;
use bruhs_models::num::{ToFloatLossy, ToIntSaturating};
use std::{
    collections::{HashMap, VecDeque},
    fmt,
    time::{Duration, Instant},
};

const MINIMUM_CPU_UPDATE_INTERVAL: Duration = Duration::from_millis(200);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct Pid(u32);

impl Pid {
    const fn from_u32(value: u32) -> Self {
        Self(value)
    }

    #[cfg(test)]
    const fn as_u32(self) -> u32 {
        self.0
    }
}

impl fmt::Display for Pid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Clone)]
struct ProcessSample {
    pid: Pid,
    start_time: u64,
    name: String,
    cpu_ns: u64,
    memory: u64,
    read_bytes: u64,
    write_bytes: u64,
}

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
struct PrevCounters {
    cpu_ns: u64,
    read_bytes: u64,
    write_bytes: u64,
}
struct Sampler {
    previous: HashMap<(Pid, u64), PrevCounters>,
    last: Option<Instant>,
}
impl Sampler {
    fn new() -> Self {
        Self {
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
        let all = os::list_process_parents();
        let mut children = HashMap::<Pid, Vec<Pid>>::new();
        for (pid, parent) in all {
            if let Some(parent) = parent {
                children.entry(parent).or_default().push(pid);
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
        let mut total = Usage::default();
        let mut counters = HashMap::new();
        let mut rows = Vec::new();
        for (pid, depth) in tree {
            let Some(p) = os::inspect_process(pid) else {
                continue;
            };
            let key = (p.pid, p.start_time);
            let (cpu, read, write) = self.previous.get(&key).zip(seconds).map_or(
                (0.0_f32, 0.0_f64, 0.0_f64),
                |(prev, s)| {
                    let delta_cpu_ns = p.cpu_ns.saturating_sub(prev.cpu_ns).to_f64_lossy();
                    let cpu_pct = (delta_cpu_ns / (s * 1_000_000_000.0) * 100.0).to_f32_lossy();
                    let read_rate = p.read_bytes.saturating_sub(prev.read_bytes).to_f64_lossy() / s;
                    let write_rate = p
                        .write_bytes
                        .saturating_sub(prev.write_bytes)
                        .to_f64_lossy()
                        / s;
                    (cpu_pct, read_rate, write_rate)
                },
            );
            counters.insert(
                key,
                PrevCounters {
                    cpu_ns: p.cpu_ns,
                    read_bytes: p.read_bytes,
                    write_bytes: p.write_bytes,
                },
            );
            let usage = Usage {
                cpu,
                memory: p.memory,
                read,
                write,
            };
            total.cpu += usage.cpu;
            total.memory = total.memory.saturating_add(usage.memory);
            total.read += read;
            total.write += write;
            rows.push(Row {
                pid: p.pid,
                depth,
                name: p.name,
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
    let started = os::inspect_process(root)?.start_time;
    std::thread::sleep(MINIMUM_CPU_UPDATE_INTERVAL);
    let snapshot = sampler.sample(root);
    if os::inspect_process(root)?.start_time != started {
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

#[cfg(target_os = "macos")]
mod os {
    use super::{Pid, ProcessSample};
    use std::ffi::CStr;
    use std::mem::{size_of, MaybeUninit};
    use std::sync::OnceLock;

    #[repr(C)]
    struct MachTimebaseInfo {
        numer: u32,
        denom: u32,
    }

    extern "C" {
        fn mach_timebase_info(info: *mut MachTimebaseInfo) -> libc::c_int;
    }

    fn timebase() -> (u64, u64) {
        static TB: OnceLock<(u64, u64)> = OnceLock::new();
        *TB.get_or_init(|| {
            let mut info = MachTimebaseInfo { numer: 1, denom: 1 };
            // SAFETY: `info` is a valid aligned mutable pointer to `mach_timebase_info_data_t`.
            unsafe {
                mach_timebase_info(&raw mut info);
            }
            (u64::from(info.numer.max(1)), u64::from(info.denom.max(1)))
        })
    }

    fn mach_ticks_to_ns(ticks: u64) -> u64 {
        let (numer, denom) = timebase();
        let ns = u128::from(ticks).saturating_mul(u128::from(numer)) / u128::from(denom);
        u64::try_from(ns).unwrap_or(u64::MAX)
    }

    fn read_bsdinfo(pid: libc::c_int) -> Option<libc::proc_bsdinfo> {
        let mut info = MaybeUninit::<libc::proc_bsdinfo>::zeroed();
        let size = libc::c_int::try_from(size_of::<libc::proc_bsdinfo>()).ok()?;
        // SAFETY: `info` has size `size_of::<proc_bsdinfo>()` matching `PROC_PIDTBSDINFO`.
        let ret = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTBSDINFO,
                0,
                info.as_mut_ptr().cast::<libc::c_void>(),
                size,
            )
        };
        if ret < size {
            return None;
        }
        // SAFETY: kernel wrote the full `proc_bsdinfo` struct (`ret >= size`).
        Some(unsafe { info.assume_init() })
    }

    fn read_taskinfo(pid: libc::c_int) -> Option<libc::proc_taskinfo> {
        let mut info = MaybeUninit::<libc::proc_taskinfo>::zeroed();
        let size = libc::c_int::try_from(size_of::<libc::proc_taskinfo>()).ok()?;
        // SAFETY: `info` has size `size_of::<proc_taskinfo>()` matching `PROC_PIDTASKINFO`.
        let ret = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTASKINFO,
                0,
                info.as_mut_ptr().cast::<libc::c_void>(),
                size,
            )
        };
        if ret < size {
            return None;
        }
        // SAFETY: kernel wrote the full `proc_taskinfo` struct (`ret >= size`).
        Some(unsafe { info.assume_init() })
    }

    fn read_rusage(pid: libc::c_int) -> Option<libc::rusage_info_v2> {
        let mut info = MaybeUninit::<libc::rusage_info_v2>::zeroed();
        // SAFETY: `info` is a valid `rusage_info_v2` buffer for `RUSAGE_INFO_V2`.
        let ret = unsafe {
            libc::proc_pid_rusage(
                pid,
                libc::RUSAGE_INFO_V2,
                info.as_mut_ptr().cast::<libc::rusage_info_t>(),
            )
        };
        if ret != 0 {
            return None;
        }
        // SAFETY: `proc_pid_rusage` returned 0 and populated `rusage_info_v2`.
        Some(unsafe { info.assume_init() })
    }

    fn cstr_from_array(buf: &[libc::c_char]) -> String {
        let bytes: Vec<u8> = buf
            .iter()
            .take_while(|&&b| b != 0)
            .map(|&b| b.to_ne_bytes()[0])
            .collect();
        if let Ok(cstr) = CStr::from_bytes_with_nul(&[bytes.as_slice(), &[0]].concat()) {
            cstr.to_string_lossy().into_owned()
        } else {
            String::from_utf8_lossy(&bytes).into_owned()
        }
    }

    pub(super) fn list_process_parents() -> Vec<(Pid, Option<Pid>)> {
        // SAFETY: Passing null/0 queries the current process count.
        let count = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
        if count <= 0 {
            return Vec::new();
        }
        let capacity = usize::try_from(count).unwrap_or(0).saturating_add(64);
        let mut pids = vec![0 as libc::pid_t; capacity];
        let buf_bytes = libc::c_int::try_from(capacity.saturating_mul(size_of::<libc::pid_t>()))
            .unwrap_or(libc::c_int::MAX);
        // SAFETY: `pids` holds `buf_bytes` writable bytes.
        let filled =
            unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast::<libc::c_void>(), buf_bytes) };
        if filled <= 0 {
            return Vec::new();
        }
        let n = usize::try_from(filled).unwrap_or(0).min(pids.len());
        let mut out = Vec::with_capacity(n);
        for &raw in &pids[..n] {
            if raw <= 0 {
                continue;
            }
            if let Some(bsd) = read_bsdinfo(raw) {
                let pid = Pid::from_u32(bsd.pbi_pid);
                let parent = (bsd.pbi_ppid != 0).then(|| Pid::from_u32(bsd.pbi_ppid));
                out.push((pid, parent));
            }
        }
        out
    }

    pub(super) fn inspect_process(pid: Pid) -> Option<ProcessSample> {
        let raw = libc::c_int::try_from(pid.0).ok()?;
        let bsd = read_bsdinfo(raw)?;
        let name = {
            let long = cstr_from_array(&bsd.pbi_name);
            if long.is_empty() {
                cstr_from_array(&bsd.pbi_comm)
            } else {
                long
            }
        };
        let (memory, cpu_ns) = match read_taskinfo(raw) {
            Some(task) => (
                task.pti_resident_size,
                mach_ticks_to_ns(task.pti_total_user.saturating_add(task.pti_total_system)),
            ),
            None => (0, 0),
        };
        let (read_bytes, write_bytes, rusage_mem) = match read_rusage(raw) {
            Some(ru) => (
                ru.ri_diskio_bytesread,
                ru.ri_diskio_byteswritten,
                ru.ri_resident_size,
            ),
            None => (0, 0, 0),
        };
        Some(ProcessSample {
            pid,
            start_time: bsd
                .pbi_start_tvsec
                .saturating_mul(1_000_000)
                .saturating_add(bsd.pbi_start_tvusec),
            name,
            cpu_ns,
            memory: memory.max(rusage_mem),
            read_bytes,
            write_bytes,
        })
    }
}

#[cfg(target_os = "linux")]
mod os {
    use super::{Pid, ProcessSample};

    fn page_size() -> u64 {
        // SAFETY: `sysconf(_SC_PAGESIZE)` is a read-only POSIX query.
        let size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        u64::try_from(size).ok().filter(|&v| v > 0).unwrap_or(4096)
    }

    fn clock_ticks_per_sec() -> u64 {
        // SAFETY: `sysconf(_SC_CLK_TCK)` is a read-only POSIX query.
        let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        u64::try_from(ticks).ok().filter(|&v| v > 0).unwrap_or(100)
    }

    fn parse_stat(pid: Pid) -> Option<(String, Option<Pid>, u64, u64, u64)> {
        let stat = std::fs::read_to_string(format!("/proc/{}/stat", pid.0)).ok()?;
        let open = stat.find('(')?;
        let close = stat.rfind(')')?;
        let name = stat[open + 1..close].to_string();
        let fields: Vec<&str> = stat[close + 2..].split_whitespace().collect();
        let ppid = fields.get(1)?.parse::<u32>().ok()?;
        let utime = fields.get(11)?.parse::<u64>().ok()?;
        let stime = fields.get(12)?.parse::<u64>().ok()?;
        let start_time = fields.get(19)?.parse::<u64>().ok()?;
        let rss_pages = fields.get(21)?.parse::<u64>().ok().unwrap_or(0);
        let tps = clock_ticks_per_sec();
        let cpu_ns = utime.saturating_add(stime).saturating_mul(1_000_000_000) / tps;
        let memory = rss_pages.saturating_mul(page_size());
        Some((
            name,
            (ppid != 0).then(|| Pid::from_u32(ppid)),
            start_time,
            cpu_ns,
            memory,
        ))
    }

    fn parse_io(pid: Pid) -> (u64, u64) {
        let Ok(text) = std::fs::read_to_string(format!("/proc/{}/io", pid.0)) else {
            return (0, 0);
        };
        let mut read_bytes = 0u64;
        let mut write_bytes = 0u64;
        for line in text.lines() {
            if let Some(v) = line.strip_prefix("read_bytes:") {
                read_bytes = v.trim().parse().unwrap_or(0);
            } else if let Some(v) = line.strip_prefix("write_bytes:") {
                write_bytes = v.trim().parse().unwrap_or(0);
            }
        }
        (read_bytes, write_bytes)
    }

    pub(super) fn list_process_parents() -> Vec<(Pid, Option<Pid>)> {
        let Ok(entries) = std::fs::read_dir("/proc") else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for entry in entries.flatten() {
            let Some(pid_num) = entry
                .file_name()
                .to_str()
                .and_then(|s| s.parse::<u32>().ok())
            else {
                continue;
            };
            let pid = Pid::from_u32(pid_num);
            if let Some((_, parent, _, _, _)) = parse_stat(pid) {
                out.push((pid, parent));
            }
        }
        out
    }

    pub(super) fn inspect_process(pid: Pid) -> Option<ProcessSample> {
        let (name, _parent, start_time, cpu_ns, memory) = parse_stat(pid)?;
        let (read_bytes, write_bytes) = parse_io(pid);
        Some(ProcessSample {
            pid,
            start_time,
            name,
            cpu_ns,
            memory,
            read_bytes,
            write_bytes,
        })
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod os {
    use super::{Pid, ProcessSample};

    pub(super) fn list_process_parents() -> Vec<(Pid, Option<Pid>)> {
        vec![(Pid::from_u32(std::process::id()), None)]
    }

    pub(super) fn inspect_process(pid: Pid) -> Option<ProcessSample> {
        (pid.0 == std::process::id()).then(|| ProcessSample {
            pid,
            start_time: 1,
            name: std::env::current_exe()
                .ok()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                .unwrap_or_else(|| "bruhs-desktop".to_string()),
            cpu_ns: 0,
            memory: 1,
            read_bytes: 0,
            write_bytes: 0,
        })
    }
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
            Self::Cpu => f64::from(u.cpu),
            Self::Memory => u.memory.to_f64_lossy(),
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
            "Samples desktop and child processes while open.",
            controls.into_any_element(),
        )];
        if let Some(s) = &monitor.latest {
            let window = f64::from(monitor.range.minutes()) * 60.;
            let mut buckets = [None::<f64>; 42];
            for (time, u) in &monitor.history {
                let age = s.at.saturating_duration_since(*time).as_secs_f64();
                if age < window {
                    let i = buckets.len()
                        - 1
                        - (age * buckets.len().to_f64_lossy() / window).to_usize_saturating();
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
                    let height = v.map_or(0., |n| ((n / peak).to_f32_lossy() * 74.).max(2.));
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
                    "{} CPU, {} read, {} write",
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
                    "{} processes, {}, sampled in {} ms",
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
                                .pl(px((p.depth.min(8) * 10).to_f32_lossy()))
                                .child(format!("{}, PID {}", p.name, p.pid)),
                        )
                        .child(format!("{cpu}, {}", Metric::Memory.display(p.usage)))
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
