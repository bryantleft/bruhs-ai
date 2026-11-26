//! Shared provider navigation and device-local favorites for desktop model pickers.
use super::workspace::{accent_dot, provider_label};
use crate::{assets, theme};
use bruhs_models::agent::AgentProvider;
use bruhs_ui::press::{focusable_control, on_press, Interactivity};
use bruhs_ui::zoom::px;
use gpui::prelude::*;
use gpui::*;
use std::collections::BTreeSet;

pub(super) const SEARCH_HINT: &str = "Search or enter model ID…";
pub(super) const FAVORITES: &str = "favorites";
pub(super) const HEADER_HEIGHT_PX: f32 = 42.0;
pub(super) type Favorites = BTreeSet<String>;

pub(super) fn favorite_key(id: &str) -> String {
    let provider = AgentProvider::for_model(id);
    format!("{}{}", provider.model_prefix(), provider.model_id(id))
}

fn route(id: &str) -> &str {
    if let Some((instance, _)) = id.strip_prefix("acp/").and_then(|s| s.split_once('/')) {
        &id[.."acp/".len() + instance.len()]
    } else {
        AgentProvider::for_model(id).as_str()
    }
}

pub(super) struct Browser {
    tabs: Vec<String>,
    pub active: String,
}

impl Browser {
    pub fn new<'a>(
        ids: impl Iterator<Item = &'a str>,
        selected: Option<&str>,
        current: &str,
        favorites: &Favorites,
    ) -> Self {
        let mut tabs = vec![FAVORITES.to_owned()];
        let mut has_favorites = false;
        for id in ids {
            let tab = route(id);
            if !tabs.iter().any(|name| name == tab) {
                tabs.push(tab.to_owned());
            }
            has_favorites |= favorites.contains(&favorite_key(id));
        }
        let initial = if has_favorites {
            FAVORITES
        } else {
            route(current)
        };
        let active = selected
            .filter(|s| tabs.iter().any(|name| name == *s))
            .or_else(|| tabs.iter().any(|name| name == initial).then_some(initial))
            .unwrap_or_else(|| tabs.get(1).map_or(FAVORITES, String::as_str))
            .to_owned();
        Self { tabs, active }
    }

    pub fn includes(&self, id: &str, favorites: &Favorites) -> bool {
        if self.active == FAVORITES {
            favorites.contains(&favorite_key(id))
        } else {
            route(id) == self.active
        }
    }

    pub fn adjacent_tab(&self, forward: bool) -> String {
        let index = self
            .tabs
            .iter()
            .position(|tab| tab == &self.active)
            .unwrap_or_default();
        let step = if forward { 1 } else { self.tabs.len() - 1 };
        self.tabs[(index + step) % self.tabs.len()].clone()
    }

    pub fn filter_groups(
        &self,
        groups: &mut Vec<bruhs_ui::dropdown::DropdownGroup>,
        favorites: &Favorites,
    ) {
        for group in groups.iter_mut() {
            group
                .options
                .retain(|option| self.includes(option.value, favorites));
            group
                .options
                .sort_by_key(|option| !favorites.contains(&favorite_key(option.value)));
        }
        groups.retain(|group| !group.options.is_empty());
        if let [group] = groups.as_mut_slice() {
            group.label = None;
            group.mark = None;
        }
    }

    pub fn rail(
        &self,
        prefix: &'static str,
        rail_focus: &FocusHandle,
        search_focus: FocusHandle,
        instance: impl Fn(&str) -> (String, Option<u32>),
        select: impl Fn(String, &mut Window, &mut App) + Clone + 'static,
    ) -> AnyElement {
        let buttons = self
            .tabs
            .iter()
            .map(String::as_str)
            .enumerate()
            .map(|(index, tab)| {
                let selected = self.active == tab;
                let (label, icon) = if tab == FAVORITES {
                    (
                        "Favorites".to_owned(),
                        favorite_mark(selected, 20.).into_any_element(),
                    )
                } else {
                    let (label, provider, accent) = if let Some(id) = tab.strip_prefix("acp/") {
                        let (label, accent) = instance(id);
                        (label, AgentProvider::Acp, accent)
                    } else {
                        let provider = AgentProvider::ALL
                            .into_iter()
                            .find(|p| p.as_str() == tab)
                            .expect("model browser tab matches a known provider");
                        (provider_label(provider).to_owned(), provider, None)
                    };
                    let icon = picker_provider_mark(provider, accent, 20.);
                    (label, icon)
                };
                let id = format!("{prefix}-tab-{tab}");
                let tab = tab.to_owned();
                let select = select.clone();
                let last = index + 1 == self.tabs.len();
                let button_height = if index == 0 && !last {
                    HEADER_HEIGHT_PX - 1.
                } else {
                    38.
                };
                let search = search_focus.clone();
                let tip = label.clone();
                let el = div()
                    .id(SharedString::from(id.clone()))
                    .debug_selector(move || id)
                    .relative()
                    .w(px(38.))
                    .h(px(button_height))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.))
                    .hover(|el| el.bg(theme::longan_800()))
                    .tooltip(move |window, cx| {
                        bruhs_ui::tooltip::Tooltip::new(tip.clone()).build(window, cx)
                    })
                    .when(selected, |el| el.track_focus(rail_focus))
                    .on_key_down(move |event: &KeyDownEvent, window, cx| {
                        if event.keystroke.modifiers.modified() {
                            return;
                        }
                        match event.keystroke.key.as_str() {
                            "right" => search.focus(window, cx),
                            "up" if index > 0 => window.focus_prev(cx),
                            "down" if !last => window.focus_next(cx),
                            "up" | "down" => {}
                            _ => return,
                        }
                        window.prevent_default();
                        cx.stop_propagation();
                    })
                    .child(icon)
                    .when(selected, |el| {
                        el.child(
                            div()
                                .absolute()
                                .right(px(1.))
                                .top(px((button_height - 20.) / 2.))
                                .w(px(3.))
                                .h(px(20.))
                                .rounded_full()
                                .bg(theme::persimmon_400()),
                        )
                    });
                let button = on_press(
                    focusable_control(el, Role::Button, label, Interactivity::Enabled)
                        .aria_toggled(if selected {
                            Toggled::True
                        } else {
                            Toggled::False
                        }),
                    move |window, cx| select(tab.clone(), window, cx),
                );
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .child(button)
                    .when(index == 0 && !last, |el| {
                        el.child(
                            div()
                                .debug_selector(move || format!("{prefix}-favorites-divider"))
                                .w(px(28.))
                                .h(px(1.))
                                .bg(theme::ring_card()),
                        )
                    })
            });
        div()
            .id(SharedString::from(format!("{prefix}-rail")))
            .role(Role::Toolbar)
            .aria_label("Providers")
            .w(px(45.))
            .flex_none()
            .min_h_0()
            .max_h(px(346.))
            .overflow_y_scroll()
            .pb_1()
            .flex()
            .flex_col()
            .gap(px(2.))
            .items_center()
            .border_r_1()
            .border_color(theme::ring_floating())
            .bg(theme::longan_900())
            .children(buttons)
            .into_any_element()
    }
}

pub(super) fn picker_provider_mark(
    provider: AgentProvider,
    accent: Option<u32>,
    size: f32,
) -> AnyElement {
    if let Some(color) = accent {
        return div()
            .size(px(size))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .child(accent_dot(color))
            .into_any_element();
    }
    if provider == AgentProvider::Antigravity {
        return img("logos/antigravity-color.svg")
            .size(px(size))
            .flex_none()
            .into_any_element();
    }
    svg()
        .path(assets::provider_logo_path(provider))
        .size(px(size))
        .flex_none()
        .text_color(if provider == AgentProvider::Claude {
            theme::persimmon_400()
        } else {
            theme::rambutan_300()
        })
        .into_any_element()
}

fn favorite_mark(selected: bool, size: f32) -> Svg {
    let (path, color) = if selected {
        (bruhs_ui::icons::STAR_FILLED, theme::durian_400())
    } else {
        (bruhs_ui::icons::STAR, theme::rambutan_400())
    };
    svg()
        .path(path)
        .size(px(size))
        .flex_none()
        .text_color(color)
}

pub(super) fn star(
    prefix: &'static str,
    id: &str,
    selected: bool,
    reveal_on_hover: Option<SharedString>,
    toggle: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let selector = format!("{prefix}-star-{id}");
    let label = if selected {
        "Remove from favorites"
    } else {
        "Add to favorites"
    };
    let el = div()
        .id(SharedString::from(selector.clone()))
        .debug_selector(move || selector)
        .size(px(20.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .hover(|s| s.bg(theme::longan_700()))
        .when_some(reveal_on_hover, |el, row| {
            el.opacity(0.)
                .group_hover(row, |el| el.opacity(1.))
                .focus_visible(|el| el.opacity(1.))
        })
        .tooltip(move |window, cx| bruhs_ui::tooltip::Tooltip::new(label).build(window, cx))
        .child(favorite_mark(selected, 16.));
    on_press(
        focusable_control(
            el,
            Role::Button,
            format!("{label}, {id}"),
            Interactivity::Enabled,
        )
        .aria_toggled(if selected {
            Toggled::True
        } else {
            Toggled::False
        }),
        toggle,
    )
    .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{favorite_key, Browser, Favorites, FAVORITES};
    #[test]
    fn model_picker_favorites_follow_the_provider_instance_and_its_availability() {
        let saved = Favorites::from([
            favorite_key("acp/team/default"),
            favorite_key("codex/gpt-5.5"),
        ]);
        assert_eq!(favorite_key("gpt-5.5"), favorite_key("codex/gpt-5.5"));
        assert_ne!(favorite_key("cursor/gpt-5.5"), favorite_key("gpt-5.5"));
        let models = ["acp/team/default", "acp/other/default", "gpt-5.4"];
        let browser = Browser::new(models.into_iter(), None, "gpt-5.4", &saved);
        assert_eq!(browser.active, FAVORITES);
        assert!(browser.includes(models[0], &saved));
        assert!(!browser.includes(models[1], &saved));
        let remaining = Browser::new(models[1..].iter().copied(), None, "gpt-5.4", &saved);
        assert_eq!(remaining.active, "codex");
    }
}
