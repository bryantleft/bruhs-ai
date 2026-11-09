//! Bruhs-themed select: an input-styled trigger showing the current label
//! and a floating list on the menu tone with group headings, check marks,
//! ↑/↓ + Enter + Esc, Home/End and first-letter typeahead. Replaces the
//! native `<select>` popup wherever the OS chrome would break the theme.

use leptos::html::Div;
use leptos::prelude::*;

use crate::ui::Icon;

#[derive(Clone, PartialEq, Debug)]
pub struct DropdownOption {
    pub value: String,
    pub label: String,
    pub group: Option<String>,
    pub description: Option<String>,
}

impl DropdownOption {
    pub fn new(value: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
            group: None,
            description: None,
        }
    }

    pub fn in_group(mut self, group: impl Into<String>) -> Self {
        self.group = Some(group.into());
        self
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
}

/// Index of the first option (searching from `after + 1`, wrapping) whose
/// label starts with `ch`, case-insensitively.
pub fn typeahead_index(options: &[DropdownOption], after: usize, ch: char) -> Option<usize> {
    let n = options.len();
    if n == 0 {
        return None;
    }
    let ch = ch.to_lowercase().next()?;
    (1..=n).map(|k| (after + k) % n).find(|&i| {
        options[i]
            .label
            .chars()
            .next()
            .and_then(|c| c.to_lowercase().next())
            == Some(ch)
    })
}

/// Label to show on the trigger for `value`.
pub fn current_label(options: &[DropdownOption], value: &str) -> String {
    options
        .iter()
        .find(|o| o.value == value)
        .map_or_else(|| value.to_string(), |o| o.label.clone())
}

#[component]
pub fn Dropdown(
    /// Options in display order; groups must be contiguous. Reactive, so
    /// lists that come from signals (projects, people) stay current.
    #[prop(into)]
    options: Signal<Vec<DropdownOption>>,
    #[prop(into)] value: Signal<String>,
    /// Shown on the trigger while `value` matches no option.
    #[prop(optional, into)]
    placeholder: String,
    #[prop(optional)] on_change: Option<Callback<String>>,
    #[prop(optional, into)] disabled: Signal<bool>,
    /// Accessible name for the trigger.
    #[prop(optional, into)]
    label: String,
    /// Extra trigger classes (width etc.).
    #[prop(optional, into)]
    class: String,
    /// Anchor the list to the trigger's right edge.
    #[prop(optional)]
    align_end: bool,
) -> impl IntoView {
    let open = RwSignal::new(false);
    let highlight = RwSignal::new(0usize);
    let list_ref = NodeRef::<Div>::new();
    let trigger_ref = NodeRef::<leptos::html::Button>::new();
    let label_attr = (!label.is_empty()).then_some(label);

    let count = move || options.with_untracked(|o| o.len());
    let open_list = move || {
        if disabled.get_untracked() {
            return;
        }
        let cur = value.get_untracked();
        let idx = options
            .with_untracked(|o| o.iter().position(|x| x.value == cur))
            .unwrap_or(0);
        highlight.set(idx);
        open.set(true);
        gloo_timers::callback::Timeout::new(0, move || {
            if let Some(Some(el)) = list_ref.try_get_untracked() {
                let _ = el.focus();
            }
        })
        .forget();
    };
    let close = move |refocus: bool| {
        open.set(false);
        if refocus {
            if let Some(Some(b)) = trigger_ref.try_get_untracked() {
                let _ = b.focus();
            }
        }
    };
    let choose = move |i: usize| {
        if let Some(v) = options.with_untracked(|o| o.get(i).map(|x| x.value.clone())) {
            if let Some(cb) = on_change {
                cb.run(v);
            }
        }
        close(true);
    };

    // Keep the highlighted row in view while arrowing through a long list.
    Effect::new(move |_| {
        let i = highlight.get();
        if !open.get() {
            return;
        }
        if let Some(Some(list)) = list_ref.try_get_untracked() {
            if let Ok(Some(el)) = list.query_selector(&format!("[data-index=\"{i}\"]")) {
                el.scroll_into_view_with_bool(false);
            }
        }
    });

    let on_trigger_key = move |ev: leptos::ev::KeyboardEvent| match ev.key().as_str() {
        "ArrowDown" | "ArrowUp" | "Enter" | " " => {
            ev.prevent_default();
            open_list();
        }
        _ => {}
    };
    let on_list_key = move |ev: leptos::ev::KeyboardEvent| {
        let n = count();
        let key = ev.key();
        match key.as_str() {
            "ArrowDown" => {
                ev.prevent_default();
                highlight.update(|h| *h = (*h + 1).min(n.saturating_sub(1)));
            }
            "ArrowUp" => {
                ev.prevent_default();
                highlight.update(|h| *h = h.saturating_sub(1));
            }
            "Home" => {
                ev.prevent_default();
                highlight.set(0);
            }
            "End" => {
                ev.prevent_default();
                highlight.set(n.saturating_sub(1));
            }
            "Enter" | " " => {
                ev.prevent_default();
                choose(highlight.get_untracked());
            }
            "Escape" | "Tab" => {
                ev.prevent_default();
                close(true);
            }
            _ => {
                let mut chars = key.chars();
                if let (Some(c), None) = (chars.next(), chars.next()) {
                    if c.is_alphanumeric() {
                        if let Some(i) = options
                            .with_untracked(|o| typeahead_index(o, highlight.get_untracked(), c))
                        {
                            highlight.set(i);
                        }
                    }
                }
            }
        }
    };

    let popup_side = if align_end { "right-0" } else { "left-0" };
    let popup_class = format!(
        "absolute top-full z-50 mt-1 flex max-h-72 min-w-full w-max max-w-[22rem] flex-col overflow-y-auto rounded-grape bg-longan-700 p-1 inset-ring inset-ring-white/15 focus:outline-none {popup_side}"
    );

    view! {
        <div class=format!("relative inline-flex {class}")>
            <button
                node_ref=trigger_ref
                type="button"
                aria-haspopup="listbox"
                aria-expanded=move || open.get().to_string()
                aria-label=label_attr
                disabled=move || disabled.get()
                class="inline-flex h-8 w-full items-center justify-between gap-2 rounded-grape bg-white/5 px-3 text-left text-body-sm text-rambutan-100 inset-ring inset-ring-white/10 transition-colors hover:bg-white/[0.07] focus-visible:outline-2 focus-visible:-outline-offset-1 focus-visible:outline-persimmon-400 disabled:cursor-not-allowed disabled:opacity-60"
                on:click=move |_| if open.get_untracked() { close(false) } else { open_list() }
                on:keydown=on_trigger_key
            >
                {move || {
                    let v = value.get();
                    let known = options.with(|o| o.iter().any(|x| x.value == v));
                    if known || placeholder.is_empty() {
                        view! {
                            <span class="min-w-0 truncate">{options.with(|o| current_label(o, &v))}</span>
                        }.into_any()
                    } else {
                        view! {
                            <span class="min-w-0 truncate text-rambutan-500">{placeholder.clone()}</span>
                        }.into_any()
                    }
                }}
                <Icon name="chevron-down" size=12 class="shrink-0 text-rambutan-500" />
            </button>
            {move || open.get().then(|| view! {
                <div class="fixed inset-0 z-40 cursor-default" aria-hidden="true" on:click=move |_| close(false)></div>
                <div
                    node_ref=list_ref
                    role="listbox"
                    tabindex="-1"
                    class=popup_class.clone()
                    on:keydown=on_list_key
                >
                    {options.with(|opts| {
                        let mut last_group: Option<String> = None;
                        opts.iter().enumerate().map(|(i, o)| {
                            let header = if o.group != last_group {
                                last_group = o.group.clone();
                                o.group.clone()
                            } else {
                                None
                            };
                            let val = o.value.clone();
                            let is_selected = Signal::derive(move || value.get() == val);
                            let is_hot = Signal::derive(move || highlight.get() == i);
                            let text = o.label.clone();
                            let desc = o.description.clone();
                            view! {
                                {header.map(|g| view! {
                                    <div class="px-2.5 pb-1 pt-2 text-label-sm uppercase tracking-widest text-rambutan-400">{g}</div>
                                })}
                                <button
                                    type="button"
                                    role="option"
                                    data-index=i.to_string()
                                    aria-selected=move || is_selected.get().to_string()
                                    class=move || format!(
                                        "flex w-full items-center gap-2 rounded-seed px-2.5 py-1.5 text-left text-label-sm transition-colors {}",
                                        if is_hot.get() { "bg-white/[0.06] text-rambutan-100" } else { "text-rambutan-200" }
                                    )
                                    on:mouseenter=move |_| highlight.set(i)
                                    on:click=move |_| choose(i)
                                >
                                    <span class="flex min-w-0 flex-1 flex-col">
                                        <span class="truncate">{text}</span>
                                        {desc.map(|d| view! { <span class="text-label-sm text-rambutan-400">{d}</span> })}
                                    </span>
                                    {move || is_selected.get().then(|| view! {
                                        <Icon name="check" size=14 class="shrink-0 text-rambutan-100" />
                                    })}
                                </button>
                            }
                        }).collect_view()
                    })}
                </div>
            })}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> Vec<DropdownOption> {
        vec![
            DropdownOption::new("a", "Alpha").in_group("One"),
            DropdownOption::new("b", "Beta").in_group("One"),
            DropdownOption::new("c", "Gamma").in_group("Two"),
        ]
    }

    #[test]
    fn typeahead_wraps_and_matches_case_insensitively() {
        let o = opts();
        assert_eq!(typeahead_index(&o, 0, 'g'), Some(2));
        assert_eq!(typeahead_index(&o, 2, 'A'), Some(0));
        assert_eq!(typeahead_index(&o, 1, 'z'), None);
        assert_eq!(typeahead_index(&[], 0, 'a'), None);
    }

    #[test]
    fn current_label_falls_back_to_value() {
        let o = opts();
        assert_eq!(current_label(&o, "b"), "Beta");
        assert_eq!(current_label(&o, "zzz"), "zzz");
    }
}
