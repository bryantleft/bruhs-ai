//! Scheduling policy for recurring host work. Manual user actions always run immediately.
use super::*;
use std::time::Duration;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(in crate::views::workspace) enum Profile {
    Performance,
    Balanced,
    BatterySaver,
    #[default]
    Custom,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Policy {
    pub profile: Profile,
    pub locked: bool,
    pub low: bool,
    pub battery: bool,
    pub active_seconds: u32,
    pub idle_seconds: u32,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            profile: Profile::Custom,
            locked: false,
            low: false,
            battery: false,
            active_seconds: 30,
            idle_seconds: 300,
        }
    }
}
impl Profile {
    fn key(self) -> &'static str {
        match self {
            Self::Balanced => "balanced",
            Self::Performance => "performance",
            Self::BatterySaver => "battery-saver",
            Self::Custom => "custom",
        }
    }
    fn preset(self) -> (u32, u32, Policy) {
        let (git, health, active, idle) = match self {
            Self::Balanced => (30, 300, 30, 300),
            Self::Performance => (15, 60, 30, 120),
            Self::BatterySaver => (0, 900, 60, 600),
            Self::Custom => (0, 0, 30, 300),
        };
        (
            git,
            health,
            Policy {
                profile: self,
                locked: true,
                low: self != Self::Performance,
                battery: self == Self::BatterySaver,
                active_seconds: active,
                idle_seconds: idle,
            },
        )
    }
}
#[derive(Debug, Default, Clone, Copy)]
pub(super) struct Power {
    pub locked: bool,
    pub low: bool,
    pub battery: bool,
}
impl Policy {
    fn monitors(self) -> bool {
        self.locked || self.low || self.battery
    }
    fn pauses(self, power: Option<Power>) -> bool {
        match power {
            Some(p) => {
                (self.locked && p.locked) || (self.low && p.low) || (self.battery && p.battery)
            }
            None => self.monitors(),
        }
    }
}
impl MaintenancePreferences {
    pub(in crate::views::workspace) fn for_new_install() -> Self {
        let (fetch_seconds, _, activity) = Profile::Balanced.preset();
        Self {
            fetch_seconds,
            activity,
            ..Self::default()
        }
    }
}
impl Workspace {
    pub(in crate::views::workspace) fn background_paused(&self) -> bool {
        self.maintenance
            .prefs
            .activity
            .pauses(self.maintenance.power)
    }
    pub(in crate::views::workspace) fn custom_background(&mut self) {
        self.maintenance.prefs.activity.profile = Profile::Custom;
    }
    pub(in crate::views::workspace) fn monitor_power(&mut self, cx: &mut Context<'_, Self>) {
        self.maintenance.power_timer = None;
        self.maintenance.power = None;
        if !self.maintenance.prefs.activity.monitors() {
            return;
        }
        self.maintenance.power_timer = Some(cx.spawn(async move |this, cx| {
            loop {
                let sample = cx
                    .background_executor()
                    .spawn(async { power_sample() })
                    .await;
                let Ok(interval) = this.update(cx, |this, cx| {
                    let was_paused = this.background_paused();
                    this.maintenance.power = Some(sample);
                    if was_paused && !this.background_paused() && this.health_interval > 0 {
                        this.check_provider_health(cx);
                    }
                    let policy = this.maintenance.prefs.activity;
                    let active = cx.active_window().is_some()
                        || this.threads.iter().any(|t| t.chat.read(cx).is_streaming);
                    cx.notify();
                    if active {
                        policy.active_seconds
                    } else {
                        policy.idle_seconds
                    }
                }) else {
                    return;
                };
                cx.background_executor()
                    .timer(Duration::from_secs(u64::from(interval.clamp(5, 3600))))
                    .await;
            }
        }));
    }
    pub(in crate::views::workspace) fn set_background_option(
        &mut self,
        id: &str,
        value: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if id == "background-profile" {
            let profile = match value {
                "balanced" => Profile::Balanced,
                "performance" => Profile::Performance,
                "battery-saver" => Profile::BatterySaver,
                "custom" => Profile::Custom,
                _ => return,
            };
            if profile == Profile::Custom {
                self.custom_background();
            } else {
                let (git, health, policy) = profile.preset();
                self.maintenance.prefs.activity = policy;
                self.maintenance.prefs.fetch_seconds = git;
                self.health_interval = health;
                self.maintenance
                    .seconds
                    .update(cx, |f, cx| f.set_value(git.to_string(), window, cx));
                self.health_interval_input
                    .update(cx, |f, cx| f.set_value(health.to_string(), window, cx));
            }
        } else {
            let Ok(seconds) = value.parse::<u32>() else {
                return;
            };
            self.custom_background();
            match id {
                "background-active" => self.maintenance.prefs.activity.active_seconds = seconds,
                "background-idle" => self.maintenance.prefs.activity.idle_seconds = seconds,
                _ => return,
            }
        }
        self.monitor_power(cx);
        self.schedule_project_refresh(cx);
        self.schedule_health_checks(cx);
        self.persist_and_notify(cx);
    }
    pub(super) fn background_rows(&self, cx: &Context<'_, Self>) -> Vec<AnyElement> {
        let policy = self.maintenance.prefs.activity;
        let status = if self.maintenance.power.is_some() && self.background_paused() {
            "Automatic checks are paused by the current power policy."
        } else {
            "Set the frequency of Git fetches, provider health checks and host power checks."
        };
        let mut rows = vec![self.settings_row(
            "settings-row-background-profile",
            "Background activity profile",
            status,
            self.choice_control(
                "background-profile",
                "Background activity profile",
                policy.profile.key(),
                &[
                    ("balanced", "Balanced"),
                    ("performance", "Performance"),
                    ("battery-saver", "Battery saver"),
                    ("custom", "Custom"),
                ],
                cx,
            ),
        )];
        if policy.profile != Profile::Custom {
            return rows;
        }
        for (id, label, enabled) in [
            ("background-pause-lock", "Pause while locked", policy.locked),
            (
                "background-pause-low",
                "Pause in low power mode",
                policy.low,
            ),
            (
                "background-pause-battery",
                "Pause on battery",
                policy.battery,
            ),
        ] {
            rows.push(self.settings_row(
                id,
                label,
                "Pause scheduled fetches and provider health checks.",
                settings_ui::switch(id, enabled, false, label, cx, move |this, _, cx| {
                    let policy = &mut this.maintenance.prefs.activity;
                    match id {
                        "background-pause-lock" => policy.locked = !policy.locked,
                        "background-pause-low" => policy.low = !policy.low,
                        _ => policy.battery = !policy.battery,
                    }
                    this.monitor_power(cx);
                    this.persist_and_notify(cx);
                }),
            ));
        }
        const CADENCE: &[(&str, &str)] = &[
            ("15", "15 seconds"),
            ("30", "30 seconds"),
            ("60", "1 minute"),
            ("120", "2 minutes"),
            ("300", "5 minutes"),
            ("600", "10 minutes"),
        ];
        for (id, label, seconds) in [
            (
                "background-active",
                "Active power check interval",
                policy.active_seconds,
            ),
            (
                "background-idle",
                "Idle power check interval",
                policy.idle_seconds,
            ),
        ] {
            rows.push(self.settings_row(
                id,
                label,
                "How often to check the local computer’s power state.",
                self.choice_control(id, label, &seconds.to_string(), CADENCE, cx),
            ));
        }
        rows
    }
}
#[cfg(target_os = "macos")]
fn power_sample() -> Power {
    use core_foundation::{
        base::{CFType, CFTypeRef, TCFType},
        boolean::CFBoolean,
        dictionary::{CFDictionary, CFDictionaryRef},
        string::{CFString, CFStringRef},
    };
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGSessionCopyCurrentDictionary() -> CFDictionaryRef;
    }
    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        fn IOPSCopyPowerSourcesInfo() -> CFTypeRef;
        fn IOPSGetProvidingPowerSourceType(info: CFTypeRef) -> CFStringRef;
    }
    let session = unsafe { CGSessionCopyCurrentDictionary() };
    let locked = if session.is_null() {
        false
    } else {
        let session: CFDictionary<CFString, CFType> =
            unsafe { TCFType::wrap_under_create_rule(session) };
        session
            .find(&CFString::new("CGSSessionScreenIsLocked"))
            .and_then(|v| v.downcast::<CFBoolean>())
            .is_some_and(bool::from)
    };
    let info = unsafe { IOPSCopyPowerSourcesInfo() };
    let battery = if info.is_null() {
        false
    } else {
        let info: CFType = unsafe { TCFType::wrap_under_create_rule(info) };
        let raw = unsafe { IOPSGetProvidingPowerSourceType(info.as_CFTypeRef()) };
        !raw.is_null()
            && unsafe { CFString::wrap_under_get_rule(raw) } == CFString::new("Battery Power")
    };
    let low = objc2_foundation::NSProcessInfo::processInfo().isLowPowerModeEnabled();
    Power {
        locked,
        low,
        battery,
    }
}
#[cfg(target_os = "linux")]
fn power_sample() -> Power {
    use std::{fs, path::Path, process::Command};
    let root = Path::new("/sys/class/power_supply");
    let battery = fs::read_dir(root).into_iter().flatten().flatten().any(|e| {
        fs::read_to_string(e.path().join("status")).is_ok_and(|s| s.trim() == "Discharging")
    });
    let locked = Command::new("loginctl")
        .args(["show-session", "self", "-p", "LockedHint", "--value"])
        .output()
        .is_ok_and(|o| o.status.success() && o.stdout.trim_ascii() == b"yes");
    let low = fs::read_to_string("/sys/firmware/acpi/platform_profile")
        .is_ok_and(|s| s.trim() == "low-power");
    Power {
        locked,
        low,
        battery,
    }
}
#[cfg(target_os = "windows")]
fn power_sample() -> Power {
    #[repr(C)]
    struct Status {
        ac: u8,
        flag: u8,
        percent: u8,
        saver: u8,
        life: u32,
        full: u32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetSystemPowerStatus(status: *mut Status) -> i32;
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        fn OpenInputDesktop(flags: u32, inherit: i32, access: u32) -> *mut std::ffi::c_void;
        fn CloseDesktop(desktop: *mut std::ffi::c_void) -> i32;
    }
    let mut status = Status {
        ac: 255,
        flag: 0,
        percent: 0,
        saver: 0,
        life: 0,
        full: 0,
    };
    let valid = unsafe { GetSystemPowerStatus(&mut status) } != 0;
    let desktop = unsafe { OpenInputDesktop(0, 0, 0x100) };
    let locked = desktop.is_null();
    if !locked {
        unsafe {
            CloseDesktop(desktop);
        }
    }
    Power {
        locked,
        battery: valid && status.ac == 0,
        low: valid && status.saver == 1,
    }
}
#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn power_sample() -> Power {
    Power::default()
}

#[cfg(test)]
mod tests {
    use super::{Policy, Power, Profile};
    #[test]
    fn scheduled_work_obeys_power_conditions_and_manual_interval_overrides() {
        let (git, health, performance) = Profile::Performance.preset();
        assert_eq!((git, health), (15, 60));
        let battery = Power {
            battery: true,
            ..Default::default()
        };
        let low = Power {
            low: true,
            ..Default::default()
        };
        let locked = Power {
            locked: true,
            ..Default::default()
        };
        assert!(!performance.pauses(Some(battery)) && !performance.pauses(Some(low)));
        assert!(performance.pauses(Some(locked)));
        let (git, health, balanced) = Profile::Balanced.preset();
        assert_eq!((git, health), (30, 300));
        assert!(balanced.pauses(Some(low)));
        assert!(!balanced.pauses(Some(battery)));
        let (git, health, saver) = Profile::BatterySaver.preset();
        assert_eq!((git, health), (0, 900));
        assert!(saver.pauses(Some(battery)));
        assert!(saver.pauses(None));
        assert!(!saver.pauses(Some(Power::default())));
        assert!(!Policy::default().pauses(None));
    }
}
