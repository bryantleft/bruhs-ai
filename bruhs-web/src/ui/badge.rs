//! Semantic status badges and unread count pills matching `bruhs_ui::badge`.

use leptos::prelude::*;

use crate::ui::Icon;

/// Semantic tone for a [`Badge`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[expect(
    dead_code,
    reason = "Full semantic badge palette mirrors bruhs_ui::badge for upcoming web call sites"
)]
pub enum BadgeTone {
    #[default]
    Neutral,
    Info,
    Success,
    Warning,
    Error,
    Accent,
}

impl BadgeTone {
    fn pill_classes(self) -> &'static str {
        match self {
            Self::Neutral => "bg-longan-800 text-rambutan-300",
            Self::Info => "bg-blueberry-500/15 text-blueberry-200",
            Self::Success => "bg-pandan-500/15 text-pandan-400",
            Self::Warning => "bg-durian-500/15 text-durian-300",
            Self::Error => "bg-lychee-500/15 text-lychee-300",
            Self::Accent => "bg-persimmon-500/15 text-persimmon-300",
        }
    }

    fn inline_classes(self) -> &'static str {
        match self {
            Self::Neutral => "text-rambutan-300",
            Self::Info => "text-blueberry-200",
            Self::Success => "text-pandan-400",
            Self::Warning => "text-durian-300",
            Self::Error => "text-lychee-300",
            Self::Accent => "text-persimmon-300",
        }
    }
}

/// Compact semantic status badge, optionally prefixed by a 12 px icon.
#[component]
pub fn Badge(
    #[prop(optional)] tone: BadgeTone,
    #[prop(optional)] icon: Option<&'static str>,
    /// Render as borderless inline status text + icon (e.g., "Up to date").
    #[prop(optional)]
    inline: bool,
    #[prop(optional, into)] class: String,
    children: Children,
) -> impl IntoView {
    let classes = if inline {
        format!(
            "inline-flex shrink-0 items-center gap-1 text-label-sm {} {class}",
            tone.inline_classes()
        )
    } else {
        format!(
            "inline-flex shrink-0 items-center gap-1 rounded-seed px-1.5 text-label-sm {} {class}",
            tone.pill_classes()
        )
    };
    view! {
        <span class=classes>
            {icon.map(|name| view! { <Icon name=name size=12 class="shrink-0" /> })}
            {children()}
        </span>
    }
}

/// Format an unread count for [`CountBadge`], capping values above 99 at `"99+"`.
pub fn format_count(count: usize) -> String {
    if count > 99 {
        "99+".into()
    } else {
        count.to_string()
    }
}

/// Unread count pill with an accompanying screen-reader label.
#[component]
pub fn CountBadge(
    #[prop(into)] count: Signal<usize>,
    #[prop(optional, into)] sr_suffix: Option<String>,
) -> impl IntoView {
    let suffix = StoredValue::new(sr_suffix.unwrap_or_else(|| "unread".to_string()));
    view! {
        <Show when=move || { count.get() > 0 }>
            <span
                aria-hidden="true"
                class="shrink-0 rounded-orb bg-blueberry-400 px-1.5 text-label-sm text-longan-950"
            >
                {move || format_count(count.get())}
            </span>
            <span class="sr-only">
                {move || suffix.with_value(|s| format!("{} {s}", count.get()))}
            </span>
        </Show>
    }
}

#[cfg(test)]
mod tests {
    use super::format_count;

    #[test]
    fn count_badge_caps_over_ninety_nine() {
        assert_eq!(format_count(0), "0");
        assert_eq!(format_count(3), "3");
        assert_eq!(format_count(99), "99");
        assert_eq!(format_count(100), "99+");
    }
}
