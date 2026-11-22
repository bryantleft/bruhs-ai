//! Identify the running build in the sidebar header using the bruhs palette.
use super::*;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum Mark {
    #[default]
    Artwork,
    NightlyArtwork,
    DevArtwork,
    Pill,
    None,
}
impl Mark {
    pub fn key(self) -> &'static str {
        match self {
            Self::Artwork => "artwork",
            Self::NightlyArtwork => "nightly-artwork",
            Self::DevArtwork => "dev-artwork",
            Self::Pill => "pill",
            Self::None => "none",
        }
    }
    pub fn parse(k: &str) -> Option<Self> {
        Some(match k {
            "artwork" => Self::Artwork,
            "nightly-artwork" => Self::NightlyArtwork,
            "dev-artwork" => Self::DevArtwork,
            "pill" => Self::Pill,
            "none" => Self::None,
            _ => return None,
        })
    }
}
fn build_kind() -> &'static str {
    let release = env!("BRUHS_RELEASE_VERSION");
    if release.is_empty() {
        "Dev"
    } else if bruhs_core::updates::release::UpdateTrack::for_installed(release)
        == bruhs_core::updates::release::UpdateTrack::Nightly
    {
        "Nightly"
    } else {
        "Stable"
    }
}
impl Workspace {
    pub(super) fn environment_mark_row(&self, cx: &Context<'_, Self>) -> AnyElement {
        let kind = build_kind();
        let mut choices = vec![(
            "artwork",
            match kind {
                "Dev" => "Dev artwork",
                "Nightly" => "Nightly artwork",
                _ => "None",
            },
        )];
        if kind != "Nightly" {
            choices.push(("nightly-artwork", "Nightly artwork"))
        }
        if kind != "Dev" {
            choices.push(("dev-artwork", "Dev artwork"))
        }
        if kind != "Stable" {
            choices.extend([("pill", "Version pill"), ("none", "None")])
        }
        let saved = self.appearance.environment_mark.key();
        let current = if choices.iter().any(|(key, _)| *key == saved) {
            saved
        } else {
            "artwork"
        };
        self.settings_row(
            "settings-row-environment-identification",
            "Environment identification",
            "Build badge in the sidebar header.",
            settings_ui::control_column(self.choice_control(
                "settings-environment-mark",
                "Environment identification",
                current,
                &choices,
                cx,
            )),
        )
    }

    pub(super) fn environment_mark(&self) -> Option<AnyElement> {
        let build = build_kind();
        let label = match self.appearance.environment_mark {
            Mark::None => return None,
            Mark::DevArtwork => "Dev",
            Mark::NightlyArtwork => "Nightly",
            Mark::Artwork if build == "Stable" => return None,
            Mark::Artwork | Mark::Pill => build,
        };
        let control = div()
            .id("sidebar-environment")
            .role(Role::Image)
            .aria_label(format!("{label} environment indicator"))
            .flex_shrink_0();
        if self.appearance.environment_mark == Mark::Pill {
            return Some(
                control
                    .px_1p5()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(theme::longan_700())
                    .text_xs()
                    .text_color(theme::rambutan_400())
                    .child(format!("{label} {}", crate::launcher::version()))
                    .into_any_element(),
            );
        }
        let artwork = if label == "Nightly" {
            div()
                .relative()
                .size(px(15.))
                .rounded_full()
                .bg(theme::blueberry_400())
                .child(
                    div()
                        .absolute()
                        .top(px(-2.))
                        .right(px(-2.))
                        .size(px(13.))
                        .rounded_full()
                        .bg(theme::longan_950()),
                )
                .into_any_element()
        } else {
            svg()
                .path(assets::TOOL_TERMINAL_ICON_PATH)
                .size(px(17.))
                .text_color(theme::persimmon_400())
                .into_any_element()
        };
        Some(control.child(artwork).into_any_element())
    }
}
