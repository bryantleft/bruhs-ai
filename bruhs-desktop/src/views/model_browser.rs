//! Shared provider navigation and device-local favorites for desktop model pickers.
use super::workspace::{accent_dot, provider_label};
use crate::{assets, theme};
use bruhs_models::agent::AgentProvider;
use bruhs_ui::press::{Interactivity, focusable_control, on_press};
use bruhs_ui::zoom::px;
use gpui::prelude::*;
use gpui::*;
use std::collections::BTreeSet;

pub(super) const SEARCH_HINT: &str = "Search or enter model ID…";
pub(super) const FAVORITES: &str = "favorites";
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
                let (label, icon) = if tab == FAVORITES {
                    (
                        "Favorites".to_owned(),
                        div().text_size(px(19.)).child("★").into_any_element(),
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
                    let icon = match (provider, accent) {
                        (_, Some(color)) => accent_dot(color).into_any_element(),
                        (AgentProvider::Claude | AgentProvider::Codex, _) => svg()
                            .path(assets::provider_logo_path(provider))
                            .size(px(18.))
                            .text_color(if self.active == tab {
                                theme::persimmon_400()
                            } else {
                                theme::rambutan_300()
                            })
                            .into_any_element(),
                        _ => div()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(label.chars().take(2).collect::<String>())
                            .into_any_element(),
                    };
                    (label, icon)
                };
                let selected = self.active == tab;
                let id = format!("{prefix}-tab-{tab}");
                let tab = tab.to_owned();
                let select = select.clone();
                let previous = self.tabs[(index + self.tabs.len() - 1) % self.tabs.len()].clone();
                let next = self.tabs[(index + 1) % self.tabs.len()].clone();
                let navigate = select.clone();
                let focus = rail_focus.clone();
                let search = search_focus.clone();
                let tip = label.clone();
                let el = div()
                    .id(SharedString::from(id.clone()))
                    .debug_selector(move || id)
                    .size(px(30.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.))
                    .text_color(theme::rambutan_300())
                    .when(selected, |el| {
                        el.bg(theme::longan_700())
                            .text_color(theme::persimmon_400())
                    })
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
                            key @ ("up" | "down") => {
                                navigate(
                                    if key == "down" {
                                        next.clone()
                                    } else {
                                        previous.clone()
                                    },
                                    window,
                                    cx,
                                );
                                focus.focus(window, cx);
                            }
                            _ => return,
                        }
                        window.prevent_default();
                        cx.stop_propagation();
                    })
                    .child(icon);
                on_press(
                    focusable_control(el, Role::Button, label, Interactivity::Enabled)
                        .aria_toggled(if selected {
                            Toggled::True
                        } else {
                            Toggled::False
                        }),
                    move |window, cx| select(tab.clone(), window, cx),
                )
            });
        div()
            .id(SharedString::from(format!("{prefix}-rail")))
            .w(px(40.))
            .flex_none()
            .max_h(px(240.))
            .overflow_y_scroll()
            .py_1()
            .flex()
            .flex_col()
            .gap_1()
            .items_center()
            .border_r_1()
            .border_color(theme::longan_700())
            .children(buttons)
            .into_any_element()
    }
}

pub(super) fn star(
    prefix: &'static str,
    id: &str,
    selected: bool,
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
        .size(px(24.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .text_size(px(15.))
        .when(!selected, |el| {
            el.opacity(0.45)
                .group_hover(format!("{prefix}-row-{id}"), |el| el.opacity(1.))
                .focus_visible(|el| el.opacity(1.))
        })
        .hover(|s| s.bg(theme::longan_700()))
        .text_color(if selected {
            theme::persimmon_400()
        } else {
            theme::rambutan_400()
        })
        .tooltip(move |window, cx| bruhs_ui::tooltip::Tooltip::new(label).build(window, cx))
        .child(if selected { "★" } else { "☆" });
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
    use super::{Browser, FAVORITES, Favorites, favorite_key};
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
