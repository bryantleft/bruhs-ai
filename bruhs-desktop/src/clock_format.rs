//! Shared device preference for human-readable times. Input and wire formats stay fixed.
use serde::{Deserialize, Serialize};
use std::cell::Cell;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ClockFormat {
    #[default]
    System,
    Twelve,
    TwentyFour,
}

thread_local! {
    static TWENTY_FOUR: Cell<bool> = Cell::new(system_uses_24_hours());
}

impl ClockFormat {
    pub fn key(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Twelve => "12h",
            Self::TwentyFour => "24h",
        }
    }

    pub fn apply(self) {
        TWENTY_FOUR.set(match self {
            Self::System => system_uses_24_hours(),
            Self::Twelve => false,
            Self::TwentyFour => true,
        });
    }
}

pub(crate) fn uses_24_hours() -> bool {
    TWENTY_FOUR.get()
}

pub(crate) fn pattern() -> &'static str {
    if uses_24_hours() {
        "%H:%M"
    } else {
        "%-I:%M %p"
    }
}

#[cfg(target_os = "macos")]
fn regional_pattern(template: &str) -> String {
    use objc2_foundation::{NSDateFormatter, NSLocale, NSString};
    NSDateFormatter::dateFormatFromTemplate_options_locale(
        &NSString::from_str(template),
        0,
        Some(&NSLocale::currentLocale()),
    )
    .map(|format| format.to_string())
    .unwrap_or_default()
}

#[cfg(target_os = "macos")]
fn system_uses_24_hours() -> bool {
    regional_pattern("j").contains(['H', 'k'])
}

#[cfg(all(unix, not(target_os = "macos")))]
fn system_uses_24_hours() -> bool {
    // Read the user's LC_TIME independently of the process-global C locale.
    unsafe {
        let locale = libc::newlocale(libc::LC_TIME_MASK, c"".as_ptr(), std::ptr::null_mut());
        if locale.is_null() {
            return true;
        }
        let format = libc::nl_langinfo_l(libc::T_FMT, locale);
        let uses_24_hours = !format.is_null() && {
            let format = std::ffi::CStr::from_ptr(format).to_string_lossy();
            !format.contains("%I")
                && !format.contains("%l")
                && !format.contains("%r")
                && !format.contains("%p")
        };
        libc::freelocale(locale);
        uses_24_hours
    }
}

#[cfg(not(unix))]
fn system_uses_24_hours() -> bool {
    // Windows LOCALE_ITIME follows the user's regional settings.
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        extern "system" {
            fn GetLocaleInfoEx(locale: *const u16, kind: u32, data: *mut u16, length: i32) -> i32;
        }
        let mut digits = [0u16; 4];
        return unsafe {
            GetLocaleInfoEx(std::ptr::null(), 0x23, digits.as_mut_ptr(), 4) > 0
                && digits[0] == u16::from(b'1')
        };
    }
    #[cfg(not(windows))]
    true
}

/// Numeric date pattern, including regional punctuation before the clock.
pub(crate) fn date_time_prefix_pattern() -> String {
    #[cfg(target_os = "macos")]
    {
        let mut date = regional_pattern("yMd");
        let combined = regional_pattern("yMdjm");
        if let Some(clock) = combined.strip_prefix(&date) {
            let punctuation = clock
                .split(|c: char| c.is_alphanumeric() || c == '\'')
                .next()
                .unwrap_or_default();
            date.push_str(punctuation.trim_end());
        }
        date
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    unsafe {
        let locale = libc::newlocale(libc::LC_TIME_MASK, c"".as_ptr(), std::ptr::null_mut());
        if locale.is_null() {
            return String::new();
        }
        let pattern = libc::nl_langinfo_l(libc::D_FMT, locale);
        let result = if pattern.is_null() {
            String::new()
        } else {
            std::ffi::CStr::from_ptr(pattern)
                .to_string_lossy()
                .into_owned()
        };
        libc::freelocale(locale);
        result
    }
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        extern "system" {
            fn GetLocaleInfoEx(locale: *const u16, kind: u32, data: *mut u16, length: i32) -> i32;
        }
        let mut pattern = [0u16; 128];
        let count = unsafe { GetLocaleInfoEx(std::ptr::null(), 0x1f, pattern.as_mut_ptr(), 128) };
        String::from_utf16_lossy(&pattern[..count.saturating_sub(1).max(0) as usize])
    }
    #[cfg(not(any(unix, windows)))]
    {
        String::new()
    }
}
