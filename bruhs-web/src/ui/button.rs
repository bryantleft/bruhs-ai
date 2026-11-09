use leptos::prelude::*;

#[derive(Clone, Copy, PartialEq)]
pub enum ButtonVariant {
    Primary,
    Ghost,
    Outline,
    Destructive,
}

#[derive(Clone, Copy, PartialEq)]
pub enum ButtonSize {
    Default,
    Small,
    IconSmall,
    IconDefault,
}

pub fn button_variant_class(v: ButtonVariant) -> &'static str {
    match v {
        ButtonVariant::Primary =>
            "bg-persimmon-500 text-longan-950 hover:bg-persimmon-400 active:bg-persimmon-600",
        ButtonVariant::Ghost =>
            "text-rambutan-200 hover:bg-longan-800 hover:text-rambutan-100",
        ButtonVariant::Outline =>
            "bg-transparent text-rambutan-100 inset-ring inset-ring-rambutan-100/15 hover:bg-longan-800",
        ButtonVariant::Destructive =>
            "bg-lychee-600 text-lychee-50 hover:bg-lychee-500",
    }
}

pub fn button_size_class(s: ButtonSize) -> &'static str {
    // `min-w-6`/`min-w-10` on icon sizes enforces a minimum touch target of
    // 24×24 (small) / 40×40 (default), matching WCAG 2.5.8 AA requirements.
    match s {
        ButtonSize::Default => "px-3 py-2 text-button rounded-grape min-h-10",
        ButtonSize::Small => "px-2.5 py-1.5 text-button-sm rounded-grape min-h-8",
        ButtonSize::IconDefault => "p-2 rounded-grape min-w-10 min-h-10",
        ButtonSize::IconSmall => "p-1.5 rounded-grape min-w-6 min-h-6",
    }
}

/// Build the complete button class string for a given variant/size and any extra classes.
/// Use this when you need to compose a `<button>` directly (e.g. when you need access to
/// the underlying click event with state captures that can't be expressed via `Callback`).
pub fn button_class(variant: ButtonVariant, size: ButtonSize, extra: &str) -> String {
    format!(
        "inline-flex items-center justify-center gap-1.5 transition focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-persimmon-400 disabled:opacity-60 disabled:cursor-not-allowed {} {} {}",
        button_variant_class(variant),
        button_size_class(size),
        extra,
    )
}

#[component]
pub fn Button(
    #[prop(optional)] variant: Option<ButtonVariant>,
    #[prop(optional)] size: Option<ButtonSize>,
    #[prop(optional, into)] class: String,
    #[prop(optional, into)] title: String,
    #[prop(optional, into)] disabled: Signal<bool>,
    #[prop(optional)] on_click: Option<Callback<web_sys::MouseEvent>>,
    children: Children,
) -> impl IntoView {
    let v = variant.unwrap_or(ButtonVariant::Primary);
    let s = size.unwrap_or(ButtonSize::Default);
    let final_class = button_class(v, s, &class);

    view! {
        // type="button" is critical: HTML's default is "submit", which causes
        // any Button inside a form (e.g. the chat header rendered next to the
        // composer) to fire the parent form's onsubmit when clicked. Setting
        // type="button" makes click a pure action, never a form submission.
        <button
            type="button"
            class=final_class
            disabled=move || disabled.get()
            title=title
            on:click=move |ev| {
                if let Some(cb) = on_click {
                    cb.run(ev);
                }
            }
        >
            {children()}
        </button>
    }
}
