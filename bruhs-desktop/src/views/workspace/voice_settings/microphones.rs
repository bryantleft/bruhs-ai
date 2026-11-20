//! The first connected entry wins at recording time, including the system default.
use super::*;

#[derive(Clone)]
struct MicrophoneDrag(Option<String>);
impl Render for MicrophoneDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .rounded_md()
            .bg(theme::longan_900())
            .child(self.0.clone().unwrap_or_else(|| "System default".into()))
    }
}

impl Workspace {
    fn move_microphone(
        &mut self,
        source: &Option<String>,
        target: usize,
        cx: &mut Context<'_, Self>,
    ) {
        let mut order = self.dictation_prefs.ordered_microphones();
        let Some(index) = order.iter().position(|entry| entry == source) else {
            return;
        };
        if target >= order.len() || target == index {
            return;
        }
        let item = order.remove(index);
        order.insert(target, item);
        self.save_microphones(order, cx);
    }

    fn save_microphones(&mut self, order: Vec<Option<String>>, cx: &mut Context<'_, Self>) {
        self.dictation_prefs.set_microphone_order(order);
        self.stamp_run_prefs_on_threads(cx);
        self.persist_and_notify(cx);
    }

    pub(super) fn microphone_rows(&self, cx: &Context<'_, Self>) -> Vec<AnyElement> {
        let refresh = web_button(
            "settings-voice-refresh-mics",
            WebButtonVariant::Outline,
            WebButtonSize::Small,
            ButtonState::Enabled,
        )
        .aria_label("Refresh microphones")
        .child("Refresh");
        let header = self.settings_row(
            "desktop-settings-row-voice-mic",
            "Microphone",
            "Voice input uses the first connected microphone in this list.",
            on_press_view(refresh, cx, |this, _, cx| this.refresh_voice_devices(cx))
                .into_any_element(),
        );
        let order = self.dictation_prefs.ordered_microphones();
        let devices = self
            .voice_devices
            .as_ref()
            .and_then(|result| result.as_ref().ok());
        let active = devices.and_then(|names| {
            crate::voice_capture::preferred_input(
                &order,
                |name| names.iter().any(|connected| connected == name),
                !names.is_empty(),
            )
        });
        let mut list = div()
            .id("voice-microphone-priority")
            .aria_label("Microphones, preferred first")
            .mx_3()
            .mb_2()
            .rounded_md()
            .border_1()
            .border_color(theme::longan_700())
            .overflow_hidden();
        for (index, entry) in order.iter().enumerate() {
            let label = entry.as_deref().unwrap_or("System default");
            let missing = entry
                .as_ref()
                .zip(devices)
                .is_some_and(|(name, names)| !names.contains(name));
            let drag = MicrophoneDrag(entry.clone());
            let mut row = div()
                .id(("voice-input", index))
                .flex()
                .items_center()
                .gap_2()
                .px_2()
                .py_1()
                .when(index > 0, |row| {
                    row.border_t_1().border_color(theme::longan_700())
                })
                .drag_over::<MicrophoneDrag>(|row, _, _, _| row.bg(theme::longan_800()))
                .on_drop(cx.listener(move |this, drag: &MicrophoneDrag, _, cx| {
                    this.move_microphone(&drag.0, index, cx)
                }))
                .child(
                    div()
                        .id(("voice-input-drag", index))
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .on_drag(drag, |drag, _, _, cx| cx.new(|_| drag.clone()))
                        .child(label.to_owned()),
                );
            if active == Some(index) || missing {
                row = row.child(
                    div()
                        .id(("microphone-status", index))
                        .role(Role::Status)
                        .aria_label(format!(
                            "{label}, {}",
                            if missing { "not connected" } else { "in use" }
                        ))
                        .text_xs()
                        .text_color(theme::rambutan_400())
                        .child(if missing { "Not connected" } else { "In use" }),
                );
            }
            for (direction, caption, target) in [
                ("up", "↑", index.checked_sub(1)),
                ("down", "↓", (index + 1 < order.len()).then_some(index + 1)),
            ] {
                let source = entry.clone();
                let button = web_button(
                    SharedString::from(format!("voice-mic-{index}-{direction}")),
                    WebButtonVariant::Ghost,
                    WebButtonSize::Small,
                    ButtonState::disabled_if(target.is_none()),
                )
                .aria_label(format!("Move {label} {direction}"))
                .child(caption);
                row = row.child(on_press_view(button, cx, move |this, _, cx| {
                    if let Some(target) = target {
                        this.move_microphone(&source, target, cx);
                    }
                }));
            }
            if entry.is_some() {
                let source = entry.clone();
                let forget = web_button(
                    ("voice-forget-mic", index),
                    WebButtonVariant::Ghost,
                    WebButtonSize::Small,
                    ButtonState::Enabled,
                )
                .aria_label(format!("Forget {label}"))
                .child("Forget");
                row = row.child(on_press_view(forget, cx, move |this, _, cx| {
                    let mut order = this.dictation_prefs.ordered_microphones();
                    order.retain(|item| item != &source);
                    this.save_microphones(order, cx);
                }));
            }
            list = list.child(row);
        }
        let mut rows = vec![header, list.into_any_element()];
        if let Some(Err(error)) = &self.voice_devices {
            rows.push(
                div()
                    .mx_3()
                    .text_xs()
                    .child(error.clone())
                    .into_any_element(),
            );
        }
        rows
    }
}
