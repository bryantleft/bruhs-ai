//! Semantic inline and full-width notification banners.
//!
//! Uses tone-and-ring styling (no drop shadows) with semantic fruit tokens
//! (`durian` warning, `lychee` error, `blueberry` info, `pandan` success,
//! `longan`/`rambutan` neutral) matching `docs/ui-standards.md`.

use crate::icons;
use crate::theme;
use crate::zoom::px;
use gpui::prelude::*;
use gpui::{div, svg, AnyElement, ElementId, FontWeight, Hsla, Role, SharedString};

/// Corner radius for an inline banner card.
pub const BANNER_RADIUS_PX: f32 = 10.0;
/// Leading icon size inside a banner.
pub const BANNER_ICON_SIZE_PX: f32 = theme::ICON_PX;
/// Primary message font size.
pub const BANNER_TITLE_TEXT_SIZE_PX: f32 = theme::APP_UI_TEXT_SIZE_PX;
/// Optional secondary detail font size.
pub const BANNER_DETAIL_TEXT_SIZE_PX: f32 = 12.0;
/// Minimum height of a full-width strip banner.
pub const BANNER_STRIP_MIN_HEIGHT_PX: f32 = 40.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BannerTone {
    Warning,
    Error,
    Info,
    Success,
    Neutral,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BannerStyle {
    /// Rounded card with a full 1px ring for placement above composers or inside views.
    #[default]
    Inline,
    /// Edge-to-edge strip with a bottom hairline for window-top alerts.
    Strip,
}

impl BannerTone {
    fn palette(self, style: BannerStyle) -> BannerPalette {
        let strip = style == BannerStyle::Strip;
        match self {
            Self::Warning => BannerPalette {
                bg: theme::durian_500().opacity(if strip { 0.16 } else { 0.10 }),
                border: theme::durian_400().opacity(if strip { 0.55 } else { 0.28 }),
                icon: theme::durian_300(),
                title: theme::durian_100(),
                detail: theme::durian_200().opacity(0.82),
                default_icon: icons::ALERT_TRIANGLE,
                default_role: Role::Alert,
            },
            Self::Error => BannerPalette {
                bg: theme::lychee_500().opacity(if strip { 0.16 } else { 0.11 }),
                border: theme::lychee_400().opacity(if strip { 0.55 } else { 0.32 }),
                icon: theme::lychee_300(),
                title: theme::lychee_100(),
                detail: theme::lychee_200().opacity(0.85),
                default_icon: icons::CIRCLE_ALERT,
                default_role: Role::Alert,
            },
            Self::Info => BannerPalette {
                bg: theme::blueberry_500().opacity(if strip { 0.16 } else { 0.10 }),
                border: theme::blueberry_400().opacity(if strip { 0.50 } else { 0.30 }),
                icon: theme::blueberry_300(),
                title: theme::blueberry_200(),
                detail: theme::blueberry_300().opacity(0.85),
                default_icon: icons::HELP_CIRCLE,
                default_role: Role::Status,
            },
            Self::Success => BannerPalette {
                bg: theme::pandan_500().opacity(if strip { 0.16 } else { 0.10 }),
                border: theme::pandan_400().opacity(if strip { 0.50 } else { 0.30 }),
                icon: theme::pandan_300(),
                title: theme::pandan_100(),
                detail: theme::pandan_200().opacity(0.85),
                default_icon: icons::CHECK,
                default_role: Role::Status,
            },
            Self::Neutral => BannerPalette {
                bg: theme::longan_800().opacity(if strip { 0.90 } else { 0.72 }),
                border: if strip {
                    theme::ring_floating()
                } else {
                    theme::ring_card()
                },
                icon: theme::rambutan_400(),
                title: theme::rambutan_100(),
                detail: theme::rambutan_400(),
                default_icon: icons::HELP_CIRCLE,
                default_role: Role::Status,
            },
        }
    }
}

struct BannerPalette {
    bg: Hsla,
    border: Hsla,
    icon: Hsla,
    title: Hsla,
    detail: Hsla,
    default_icon: &'static str,
    default_role: Role,
}

/// Compact semantic banner builder with leading icon, title, optional detail
/// line, and trailing actions.
pub struct Banner {
    id: ElementId,
    tone: BannerTone,
    style: BannerStyle,
    role: Option<Role>,
    icon: Option<&'static str>,
    title: SharedString,
    title_element: Option<AnyElement>,
    detail: Option<SharedString>,
    actions: Vec<AnyElement>,
    debug_selector: Option<String>,
    aria_label: Option<SharedString>,
}

/// Create a new inline [`Banner`] with a semantic tone and primary message.
pub fn banner(
    id: impl Into<ElementId>,
    tone: BannerTone,
    title: impl Into<SharedString>,
) -> Banner {
    Banner {
        id: id.into(),
        tone,
        style: BannerStyle::Inline,
        role: None,
        icon: None,
        title: title.into(),
        title_element: None,
        detail: None,
        actions: Vec::new(),
        debug_selector: None,
        aria_label: None,
    }
}

impl Banner {
    pub fn style(mut self, style: BannerStyle) -> Self {
        self.style = style;
        self
    }

    pub fn role(mut self, role: Role) -> Self {
        self.role = Some(role);
        self
    }

    pub fn icon(mut self, icon_path: &'static str) -> Self {
        self.icon = Some(icon_path);
        self
    }

    /// Replace the default title element (for example, to attach a tooltip to
    /// a truncated error string).
    pub fn title_element(mut self, element: impl IntoElement) -> Self {
        self.title_element = Some(element.into_any_element());
        self
    }

    pub fn detail(mut self, detail: impl Into<SharedString>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn detail_opt(mut self, detail: Option<impl Into<SharedString>>) -> Self {
        self.detail = detail.map(Into::into);
        self
    }

    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.actions.push(action.into_any_element());
        self
    }

    pub fn debug_selector(mut self, selector: impl Into<String>) -> Self {
        self.debug_selector = Some(selector.into());
        self
    }

    pub fn aria_label(mut self, label: impl Into<SharedString>) -> Self {
        self.aria_label = Some(label.into());
        self
    }
}

impl IntoElement for Banner {
    type Element = AnyElement;

    fn into_element(self) -> Self::Element {
        let palette = self.tone.palette(self.style);
        let role = self.role.unwrap_or(palette.default_role);
        let icon_path = self.icon.unwrap_or(palette.default_icon);
        let has_detail = self.detail.is_some();

        let title_node = self.title_element.unwrap_or_else(|| {
            div()
                .min_w_0()
                .font_weight(FontWeight::MEDIUM)
                .text_size(px(BANNER_TITLE_TEXT_SIZE_PX))
                .text_color(palette.title)
                .child(self.title)
                .into_any_element()
        });

        let text_column = if let Some(detail) = self.detail {
            div()
                .min_w_0()
                .flex_1()
                .flex()
                .flex_col()
                .child(title_node)
                .child(
                    div()
                        .mt(px(2.0))
                        .text_size(px(BANNER_DETAIL_TEXT_SIZE_PX))
                        .text_color(palette.detail)
                        .child(detail),
                )
                .into_any_element()
        } else {
            div()
                .min_w_0()
                .flex_1()
                .flex()
                .items_center()
                .child(title_node)
                .into_any_element()
        };

        let icon_node = svg()
            .path(icon_path)
            .size(px(BANNER_ICON_SIZE_PX))
            .text_color(palette.icon)
            .flex_shrink_0()
            .when(has_detail, |el| el.mt(px(1.5)));

        let content = div()
            .min_w_0()
            .flex_1()
            .flex()
            .when(has_detail, |el| el.items_start())
            .when(!has_detail, |el| el.items_center())
            .gap_2p5()
            .child(icon_node)
            .child(text_column);

        let container = div()
            .id(self.id)
            .role(role)
            .w_full()
            .flex_shrink_0()
            .font_family(theme::FontFace::Interface.with_default(theme::UI_FONT_FAMILY))
            .text_color(palette.title)
            .bg(palette.bg)
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .when(self.style == BannerStyle::Inline, |el| {
                el.rounded(px(BANNER_RADIUS_PX))
                    .border_1()
                    .border_color(palette.border)
                    .px_3()
                    .py_2()
            })
            .when(self.style == BannerStyle::Strip, |el| {
                el.min_h(px(BANNER_STRIP_MIN_HEIGHT_PX))
                    .border_b_1()
                    .border_color(palette.border)
                    .px_3()
                    .py_1p5()
            })
            .when_some(self.debug_selector, |el, selector| {
                el.debug_selector(move || selector)
            })
            .when_some(self.aria_label, |el, label| el.aria_label(label))
            .child(content)
            .when(!self.actions.is_empty(), |el| {
                el.child(
                    div()
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .children(self.actions),
                )
            });

        container.into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{banner, BannerStyle, BannerTone, BANNER_STRIP_MIN_HEIGHT_PX};
    use crate::button::{web_button, ButtonState, WebButtonSize, WebButtonVariant};
    use ::core::prelude::v1::test;
    use gpui::{div, Context, IntoElement, ParentElement, Render, Styled, TestAppContext, Window};

    struct Sample;

    impl Render for Sample {
        fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
            div()
                .size_full()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    banner(
                        "inline-warn",
                        BannerTone::Warning,
                        "Claude is not authenticated.",
                    )
                    .debug_selector("inline-warn-banner")
                    .action(
                        web_button(
                            "sample-login",
                            WebButtonVariant::Primary,
                            WebButtonSize::Small,
                            ButtonState::Enabled,
                        )
                        .child("Log in"),
                    ),
                )
                .child(
                    banner("strip-err", BannerTone::Error, "Could not save settings.")
                        .style(BannerStyle::Strip)
                        .detail("Disk is full.")
                        .debug_selector("strip-err-banner"),
                )
        }
    }

    #[gpui::test]
    fn inline_and_strip_banners_render_with_expected_geometry(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let (_, cx) = cx.add_window_view(|_, _| Sample);
        cx.run_until_parked();
        let inline = cx
            .debug_bounds("inline-warn-banner")
            .expect("inline-warn-banner");
        assert!(inline.size.height > gpui::px(0.0));
        let strip = cx
            .debug_bounds("strip-err-banner")
            .expect("strip-err-banner");
        assert!(strip.size.height >= gpui::px(BANNER_STRIP_MIN_HEIGHT_PX));
    }
}
