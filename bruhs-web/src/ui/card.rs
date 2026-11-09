use leptos::prelude::*;

#[derive(Clone, Copy, PartialEq)]
pub enum CardVariant {
    /// Solid muted background with subtle ring (default).
    Solid,
    /// Slightly elevated — used for surfaces sitting above the page background.
    Elevated,
    /// Subtle ring only — for very lightweight grouping.
    Subtle,
}

#[component]
pub fn Card(
    #[prop(optional)] variant: Option<CardVariant>,
    #[prop(optional, into)] class: String,
    children: Children,
) -> impl IntoView {
    let v = variant.unwrap_or(CardVariant::Solid);
    let variant_class = match v {
        CardVariant::Solid =>
            "bg-longan-800 inset-ring inset-ring-white/10",
        CardVariant::Elevated =>
            "bg-longan-700 inset-ring inset-ring-white/15",
        CardVariant::Subtle =>
            "bg-longan-900 inset-ring inset-ring-white/5",
    };

    let final_class = format!("rounded-lychee p-4 {variant_class} {class}");

    view! {
        <div class=final_class>
            {children()}
        </div>
    }
}

#[component]
pub fn CardHeader(#[prop(optional, into)] class: String, children: Children) -> impl IntoView {
    let final_class = format!("flex items-center justify-between mb-3 {class}");
    view! { <div class=final_class>{children()}</div> }
}

#[component]
pub fn CardTitle(#[prop(optional, into)] class: String, children: Children) -> impl IntoView {
    let final_class = format!("text-h4 text-rambutan-100 {class}");
    view! { <div class=final_class>{children()}</div> }
}

#[component]
pub fn CardContent(#[prop(optional, into)] class: String, children: Children) -> impl IntoView {
    let final_class = format!("text-body text-rambutan-200 {class}");
    view! { <div class=final_class>{children()}</div> }
}
