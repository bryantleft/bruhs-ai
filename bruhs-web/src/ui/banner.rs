use leptos::prelude::*;

use crate::ui::Icon;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[expect(
    dead_code,
    reason = "Semantic banner tones shared across upcoming web call sites"
)]
pub enum BannerTone {
    #[default]
    Warning,
    Error,
    Info,
    Success,
    Neutral,
}

impl BannerTone {
    fn surface_classes(self) -> &'static str {
        match self {
            Self::Warning => "bg-durian-500/10 inset-ring inset-ring-durian-400/30 text-durian-100",
            Self::Error => "bg-lychee-500/10 inset-ring inset-ring-lychee-400/35 text-lychee-100",
            Self::Info => {
                "bg-blueberry-500/10 inset-ring inset-ring-blueberry-400/30 text-blueberry-200"
            }
            Self::Success => "bg-pandan-500/10 inset-ring inset-ring-pandan-400/30 text-pandan-100",
            Self::Neutral => "bg-longan-800/75 inset-ring inset-ring-white/10 text-rambutan-100",
        }
    }

    fn icon_color(self) -> &'static str {
        match self {
            Self::Warning => "text-durian-300",
            Self::Error => "text-lychee-300",
            Self::Info => "text-blueberry-300",
            Self::Success => "text-pandan-300",
            Self::Neutral => "text-rambutan-400",
        }
    }

    fn default_icon(self) -> &'static str {
        match self {
            Self::Warning => "alert-triangle",
            Self::Error => "circle-alert",
            Self::Info => "help-circle",
            Self::Success => "check",
            Self::Neutral => "help-circle",
        }
    }

    fn role(self) -> &'static str {
        match self {
            Self::Warning | Self::Error => "alert",
            Self::Info | Self::Success | Self::Neutral => "status",
        }
    }
}

/// Compact semantic inline banner with leading icon, title, optional detail,
/// and right-aligned actions.
#[component]
pub fn Banner(
    #[prop(optional)] tone: BannerTone,
    #[prop(optional)] icon: Option<&'static str>,
    #[prop(optional, into)] class: String,
    #[prop(optional)] actions: Option<Children>,
    children: Children,
) -> impl IntoView {
    let icon_name = icon.unwrap_or_else(|| tone.default_icon());
    let container_class = format!(
        "flex flex-wrap items-center justify-between gap-x-3 gap-y-2 rounded-grape px-3 py-2 text-label-md {} {}",
        tone.surface_classes(),
        class
    );
    let icon_class = format!("shrink-0 {}", tone.icon_color());

    view! {
        <div role=tone.role() class=container_class>
            <div class="flex min-w-0 flex-1 items-center gap-2.5">
                <span class=icon_class>
                    <Icon name=icon_name size=16 />
                </span>
                <div class="min-w-0 flex-1">
                    {children()}
                </div>
            </div>
            {actions.map(|render_actions| view! {
                <div class="flex shrink-0 items-center gap-1.5">
                    {render_actions()}
                </div>
            })}
        </div>
    }
}
