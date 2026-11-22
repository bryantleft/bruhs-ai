use super::*;
use bruhs_models::agent_extensions::AcpDriver;

pub(super) struct KiroPicker {
    pub(super) active: bool,
    pub(super) expanded: bool,
    pub(super) binary: Entity<InputState>,
    registry_was_active: bool,
    other_name: String,
    other_id: String,
    other_suggestion: String,
    other_accent: Option<u32>,
}

pub(super) fn instance_id(label: &str) -> String {
    let suffix = suggested_id(label);
    if suffix == "kiro" || suffix.is_empty() {
        "kiro".into()
    } else {
        format!("kiro_{suffix}")
    }
}

impl KiroPicker {
    pub(super) fn new(original: Option<&AcpInstance>, w: &mut Window, cx: &mut App) -> Self {
        let saved = original.filter(|a| a.driver == Some(AcpDriver::Kiro));
        let binary = saved
            .filter(|a| a.command != "kiro-cli")
            .map_or("", |a| a.command.as_str());
        Self {
            active: saved.is_some(),
            expanded: saved.is_some(),
            binary: cx.new(|cx| {
                InputState::new(w, cx)
                    .placeholder("kiro-cli")
                    .default_value(binary.to_owned())
            }),
            registry_was_active: false,
            other_name: "Kiro".into(),
            other_id: "kiro".into(),
            other_suggestion: "kiro".into(),
            other_accent: None,
        }
    }
}

impl Workspace {
    pub(super) fn kiro_provider_tile(&self, cx: &Context<'_, Self>) -> AnyElement {
        let e = self.provider_editor.as_ref().unwrap();
        let selected = e.kiro.active;
        let tile = web_button(
            "acp-source-kiro",
            WebButtonVariant::Outline,
            WebButtonSize::Small,
            ButtonState::disabled_if(e.busy),
        )
        .debug_selector(|| "acp-source-kiro".into())
        .role(Role::RadioButton)
        .aria_label("Kiro, Early Access")
        .aria_toggled(if selected {
            gpui::Toggled::True
        } else {
            gpui::Toggled::False
        })
        .w_full()
        .h(px(44.))
        .justify_start()
        .gap_2()
        .when(selected, |tile| {
            tile.bg(theme::persimmon_400().opacity(0.08))
                .border_color(theme::persimmon_400())
        })
        .child(
            svg()
                .path("icons/kiro.svg")
                .size(px(17.))
                .text_color(gpui::rgb(0xa16bff)),
        )
        .child("Kiro")
        .child(div().flex_1())
        .when(selected, |tile| {
            tile.child(
                svg()
                    .path(assets::CHECK_ICON_PATH)
                    .size(px(14.))
                    .text_color(theme::persimmon_400()),
            )
        })
        .child(hint("Early Access"));
        let tile = tile.when(!e.busy, |el| {
            on_press_view(el, cx, |ws, w, cx| ws.select_kiro_provider(true, w, cx))
        });
        div()
            .w(px(344.))
            .max_w_full()
            .child(tile)
            .into_any_element()
    }

    pub(super) fn select_kiro_provider(
        &mut self,
        active: bool,
        w: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(e) = self.provider_editor.as_mut() else {
            return;
        };
        if e.kiro.active == active || e.busy || e.original.is_some() {
            return;
        }
        e.kiro.active = active;
        if active {
            e.kiro.registry_was_active = e.registry.active;
            e.registry.active = false;
        } else {
            e.registry.active = e.kiro.registry_was_active;
        }
        let name = std::mem::replace(&mut e.kiro.other_name, e.name.read(cx).value().to_string());
        let id = std::mem::replace(&mut e.kiro.other_id, e.id.read(cx).value().to_string());
        std::mem::swap(&mut e.kiro.other_suggestion, &mut e.suggested_id);
        let color = std::mem::replace(&mut e.kiro.other_accent, e.accent.selected);
        e.accent =
            accent::AccentPicker::new(color.map(|rgb| format!("#{rgb:06x}")).as_deref(), w, cx);
        e.accent.observe_hex(w, cx);
        e.name.update(cx, |f, cx| f.set_value(name, w, cx));
        e.id.update(cx, |f, cx| f.set_value(id, w, cx));
        e.error = None;
        cx.notify();
    }
}
