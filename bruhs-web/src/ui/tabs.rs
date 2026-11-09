use leptos::prelude::*;

/// Stub: a basic tab list. Full implementation pending.
#[component]
pub fn Tabs(
    #[prop(into)] value: Signal<String>,
    #[prop(optional)] on_change: Option<Callback<String>>,
    children: Children,
) -> impl IntoView {
    // Provide context so TabsList/TabsTrigger can read/write
    provide_context(TabsContext { value, on_change });
    view! { <div class="flex flex-col">{children()}</div> }
}

#[derive(Clone, Copy)]
pub struct TabsContext {
    pub value: Signal<String>,
    pub on_change: Option<Callback<String>>,
}

#[component]
pub fn TabsList(#[prop(optional, into)] class: String, children: Children) -> impl IntoView {
    let final_class = format!(
        "inline-flex items-center gap-1 rounded-grape bg-longan-800 inset-ring inset-ring-white/10 p-1 {class}"
    );
    view! { <div class=final_class role="tablist">{children()}</div> }
}

#[component]
pub fn TabsTrigger(
    #[prop(into)] value: String,
    #[prop(optional, into)] class: String,
    children: Children,
) -> impl IntoView {
    let ctx = expect_context::<TabsContext>();
    let value_for_active = value.clone();
    let value_for_click = value.clone();
    let is_active = move || ctx.value.get() == value_for_active;

    let class_dyn = move || {
        let active = is_active();
        let active_class = if active {
            "bg-persimmon-500 text-longan-950"
        } else {
            "text-rambutan-200 hover:text-rambutan-100"
        };
        format!("px-3 py-1.5 rounded-grape text-button-sm transition {active_class} {class}")
    };

    view! {
        <button
            class=class_dyn
            role="tab"
            on:click=move |_| {
                if let Some(cb) = ctx.on_change {
                    cb.run(value_for_click.clone());
                }
            }
        >
            {children()}
        </button>
    }
}

#[component]
pub fn TabsContent(
    #[prop(into)] value: String,
    #[prop(optional, into)] class: String,
    children: ChildrenFn,
) -> impl IntoView {
    let ctx = expect_context::<TabsContext>();
    let value_for_match = value;
    let final_class = format!("mt-2 {class}");
    view! {
        <Show when=move || ctx.value.get() == value_for_match fallback=|| ()>
            <div class=final_class.clone() role="tabpanel">{children()}</div>
        </Show>
    }
}
