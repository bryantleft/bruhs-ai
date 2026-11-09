//! Settings sections backed by the meetings hub — the web's Calendar,
//! Recording, Transcription, Speakers and People panels
//! (`google_calendar_settings.rs`, `calendar_picker.rs`,
//! `auto_record_settings.rs`, `whisper_model_settings.rs`,
//! `diarize_settings.rs`, `personality_settings.rs`) — plus the web
//! `Switch`, `Stepper` and `Slider` widgets they are built from.

use bruhs_models::num::{ToFloatLossy, ToIntSaturating};
use bruhs_models::{Calendar, GoogleCalendarSyncState, WhisperModel};
use gpui::prelude::*;
use gpui::*;
use gpui_component::checkbox::Checkbox;
use gpui_component::input::Input;

use super::{is_web_button_activation_key, Workspace, SETTINGS_ROW_CONTROL_WIDTH_PX};
use crate::views::dropdown::{dropdown, DropdownGroup, DropdownOption};
use crate::views::meetings_hub::MeetingPrefs;
use crate::views::web_button::{web_button, ButtonState, WebButtonSize, WebButtonVariant};
use crate::{assets, theme};
use bruhs_models::time::{format_relative_ms, RelativeTimeStyle};
use bruhs_models::EpochMs;
use gpui_component::input::{InputEvent, InputState};

pub(super) const SETTINGS_WHISPER_DROPDOWN_ID: &str = "desktop-whisper-model";
pub(super) const SETTINGS_TRANSCRIPTION_PROVIDER_DROPDOWN_ID: &str =
    "desktop-transcription-provider";
pub(super) const SETTINGS_LIVE_WHISPER_DROPDOWN_ID: &str = "desktop-live-whisper-model";
/// Web whisper dropdowns: `sm:w-80`.
const WHISPER_DROPDOWN_WIDTH_PX: f32 = 320.0;
/// bruhs type scale: `text-label` 14 / `text-label-sm` 12 (Argon 500),
/// `text-body-sm` 14 / `text-code-sm` 12 (Neon 400).
const LABEL_PX: f32 = 14.0;
const LABEL_SM_PX: f32 = 12.0;
const BODY_SM_PX: f32 = 14.0;
const CODE_SM_PX: f32 = 12.0;
/// Web `Switch`: `h-6 w-11` (24×44, the 24px target floor), `size-5` thumb
/// at `translate-x-0.5` / `1.375rem`.
const SWITCH_WIDTH_PX: f32 = 44.0;
const SWITCH_HEIGHT_PX: f32 = 24.0;
const SWITCH_THUMB_PX: f32 = 20.0;
const SWITCH_THUMB_OFF_X_PX: f32 = 2.0;
const SWITCH_THUMB_ON_X_PX: f32 = 22.0;
const SWITCH_DISABLED_OPACITY: f32 = 0.5;
/// Web `Stepper`: `h-8`, `w-8` side buttons, `w-12` value, `rounded-grape`.
const STEPPER_HEIGHT_PX: f32 = 32.0;
const STEPPER_SIDE_WIDTH_PX: f32 = 32.0;
const STEPPER_VALUE_WIDTH_PX: f32 = 48.0;
const STEPPER_ICON_PX: f32 = 12.0;
const STEPPER_DISABLED_OPACITY: f32 = 0.6;
const STEPPER_SIDE_DISABLED_OPACITY: f32 = 0.4;
/// Web `Slider`: `h-5` row, `h-1.5` track, `1rem` thumb.
const SLIDER_HEIGHT_PX: f32 = 20.0;
const SLIDER_TRACK_PX: f32 = 6.0;
const SLIDER_THUMB_PX: f32 = 16.0;
const GRAPE_RADIUS_PX: f32 = 8.0;
const SEED_RADIUS_PX: f32 = 4.0;

/// Web `stepper::snap`: snap to the step grid (from `min`) and clamp.
pub fn snap(v: f64, min: Option<f64>, max: Option<f64>, step: f64) -> f64 {
    let step = if step > 0.0 { step } else { 1.0 };
    let base = min.unwrap_or(0.0);
    let mut out = base + ((v - base) / step).round() * step;
    if let Some(lo) = min {
        out = out.max(lo);
    }
    if let Some(hi) = max {
        out = out.min(hi);
    }
    if step.fract() == 0.0 {
        out = out.round();
    }
    if out == 0.0 {
        0.0
    } else {
        out
    }
}

/// Web `stepper::format_value`: integer grids print as integers.
pub fn format_value(v: f64, step: f64) -> String {
    if step.fract() == 0.0 {
        format!("{}", v.round().to_i64_saturating())
    } else {
        format!("{v:.2}")
    }
}

/// Web `slider::fill_width` fraction: where the thumb centre sits.
pub fn slider_fraction(value: f64, min: f64, max: f64) -> f64 {
    let span = max - min;
    if span > 0.0 {
        ((value - min) / span).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Value under a pointer `x` px into a track `width` px wide: the thumb
/// travels the inner `width - thumb` like a native range input.
pub fn slider_value_at(x: f32, width: f32, min: f64, max: f64, step: f64) -> f64 {
    let inner = (width - SLIDER_THUMB_PX).max(1.0);
    let fraction = f64::from(((x - SLIDER_THUMB_PX / 2.0) / inner).clamp(0.0, 1.0));
    snap(min + fraction * (max - min), Some(min), Some(max), step)
}

/// Web `GoogleCalendarSettings::status_text`.
pub fn google_status_text(state: &GoogleCalendarSyncState, now: EpochMs) -> String {
    if !state.is_connected() {
        return "Not connected. Connect to import events and enable auto-record.".to_string();
    }
    let who = match state.email() {
        Some(email) => format!("Connected as {email}"),
        None => "Connected".to_string(),
    };
    if let Some(err) = &state.last_error {
        return format!("{who}. {err}");
    }
    // A sync time no clock could produce reads as never synced.
    match state.last_synced_at.and_then(|ms| EpochMs::new(ms).ok()) {
        Some(synced) => format!(
            "{who}. Last synced {}.",
            format_relative_ms(synced, now, RelativeTimeStyle::Ago)
        ),
        None => who,
    }
}

pub(super) fn whisper_model_for_key(key: &str) -> Option<WhisperModel> {
    WhisperModel::all().iter().find(|m| m.key() == key).copied()
}

/// Web `whisper_model_settings::options`: "{label} ({size} MB)".
fn whisper_options() -> Vec<DropdownGroup> {
    vec![DropdownGroup {
        label: None,
        mark: None,
        options: WhisperModel::all()
            .iter()
            .map(|m| {
                let d = m.descriptor();
                DropdownOption {
                    value: m.key(),
                    label: format!("{} ({} MB)", d.label, d.size_mb),
                    description: None,
                }
            })
            .collect(),
    }]
}

/// The two range sliders in the Speakers section.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SettingsSlider {
    MaxSpeakers,
    LinkThreshold,
}

impl SettingsSlider {
    pub(super) const COUNT: usize = 2;

    fn index(self) -> usize {
        match self {
            Self::MaxSpeakers => 0,
            Self::LinkThreshold => 1,
        }
    }

    /// `(min, max, step)` — web `DiarizeSettingsPanel` sliders.
    fn spec(self) -> (f64, f64, f64) {
        match self {
            Self::MaxSpeakers => (2.0, 20.0, 1.0),
            Self::LinkThreshold => (0.5, 0.95, 0.05),
        }
    }

    fn selector(self) -> &'static str {
        match self {
            Self::MaxSpeakers => "desktop-settings-slider-max-speakers",
            Self::LinkThreshold => "desktop-settings-slider-link-threshold",
        }
    }

    fn aria_label(self) -> &'static str {
        match self {
            Self::MaxSpeakers => "Max speakers",
            Self::LinkThreshold => "Linking threshold",
        }
    }
}

fn focus_ring(style: StyleRefinement) -> StyleRefinement {
    style.border_2().border_color(theme::persimmon_400())
}

fn label_text(text: impl Into<SharedString>, size: f32, color: Hsla) -> Div {
    div()
        // Web `text-label-sm` is tracked .025em; the wider Argon instance matches it.
        .when(size <= 12.0, |el| {
            el.font_family(theme::UI_SMALL_FONT_FAMILY)
        })
        .text_size(px(size))
        .font_weight(FontWeight::MEDIUM)
        .line_height(relative(1.5))
        .text_color(color)
        .child(text.into())
}

fn code_text(text: impl Into<SharedString>, size: f32, color: Hsla) -> Div {
    div()
        .font_family(theme::CODE_FONT_FAMILY)
        .text_size(px(size))
        .line_height(relative(1.5))
        .text_color(color)
        .child(text.into())
}

/// Click + Enter/Space activation, like the web's `<button>`s.
fn activate<F>(el: Stateful<Div>, cx: &Context<'_, Workspace>, action: F) -> Stateful<Div>
where
    F: Fn(&mut Workspace, &mut Window, &mut Context<'_, Workspace>) + Clone + 'static,
{
    let key_action = action.clone();
    el.on_click(cx.listener(move |this, _ev: &ClickEvent, window, cx| action(this, window, cx)))
        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
            if is_web_button_activation_key(event.keystroke.key.as_str()) {
                window.prevent_default();
                cx.stop_propagation();
                key_action(this, window, cx);
            }
        }))
}

/// Web `Switch`: persimmon track when on, longan tone-step with a
/// white/10 ring when off, no shadows.
pub(super) fn switch<F>(
    id: &'static str,
    checked: bool,
    disabled: bool,
    aria_label: &'static str,
    cx: &Context<'_, Workspace>,
    action: F,
) -> AnyElement
where
    F: Fn(&mut Workspace, &mut Window, &mut Context<'_, Workspace>) + Clone + 'static,
{
    // The off-state ring is a border in gpui, which the thumb sits inside.
    let border = if checked { 0.0 } else { 1.0 };
    let thumb_x = if checked {
        SWITCH_THUMB_ON_X_PX
    } else {
        SWITCH_THUMB_OFF_X_PX
    } - border;
    let thumb_y = (SWITCH_HEIGHT_PX - SWITCH_THUMB_PX) / 2.0 - border;
    div()
        .id(id)
        .debug_selector(move || id.to_string())
        .relative()
        .flex_shrink_0()
        .w(px(SWITCH_WIDTH_PX))
        .h(px(SWITCH_HEIGHT_PX))
        .rounded_full()
        .map(|el| {
            if checked {
                el.bg(theme::persimmon_500())
            } else {
                el.bg(theme::longan_700())
                    .border_1()
                    .border_color(gpui::white().opacity(0.10))
            }
        })
        .role(Role::Switch)
        .aria_label(aria_label)
        .aria_toggled(checked.into())
        .when(disabled, |el| {
            el.opacity(SWITCH_DISABLED_OPACITY).cursor_not_allowed()
        })
        .when(!disabled, |el| {
            activate(
                el.cursor_pointer().tab_index(0).focus_visible(focus_ring),
                cx,
                action,
            )
        })
        .child(
            div()
                .absolute()
                .left(px(thumb_x))
                .top(px(thumb_y))
                .size(px(SWITCH_THUMB_PX))
                .rounded_full()
                .bg(theme::rambutan_100()),
        )
        .into_any_element()
}

/// The two numeric steppers in the Recording section.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SettingsStepper {
    MinAttendees,
    StopDelay,
}

impl SettingsStepper {
    pub(super) const ALL: [SettingsStepper; 2] = [Self::MinAttendees, Self::StopDelay];

    pub(super) fn index(self) -> usize {
        match self {
            Self::MinAttendees => 0,
            Self::StopDelay => 1,
        }
    }

    /// `(min, max, step)` — web `AutoRecordSettingsPanel` steppers.
    fn spec(self) -> (f64, f64, f64) {
        match self {
            Self::MinAttendees => (0.0, 50.0, 1.0),
            Self::StopDelay => (0.0, 900.0, 5.0),
        }
    }

    fn suffix(self) -> &'static str {
        match self {
            Self::MinAttendees => "",
            Self::StopDelay => "s",
        }
    }

    fn selector(self) -> &'static str {
        match self {
            Self::MinAttendees => "desktop-settings-min-attendees",
            Self::StopDelay => "desktop-settings-stop-delay",
        }
    }

    fn aria_label(self) -> &'static str {
        match self {
            Self::MinAttendees => "Minimum attendees",
            Self::StopDelay => "Seconds after the event ends",
        }
    }

    /// Text shown in the field for the stored pref.
    pub(super) fn text(self, prefs: &MeetingPrefs) -> String {
        let (_, _, step) = self.spec();
        format_value(stepper_pref_value(self, prefs), step)
    }
}

fn stepper_pref_value(kind: SettingsStepper, prefs: &MeetingPrefs) -> f64 {
    match kind {
        SettingsStepper::MinAttendees => f64::from(prefs.auto_record.min_attendees),
        SettingsStepper::StopDelay => prefs.auto_record.stop_delay_secs.to_f64_lossy(),
    }
}

fn stepper_side<F>(
    id: SharedString,
    icon_path: &'static str,
    aria_label: &'static str,
    disabled: bool,
    cx: &Context<'_, Workspace>,
    action: F,
) -> AnyElement
where
    F: Fn(&mut Workspace, &mut Window, &mut Context<'_, Workspace>) + Clone + 'static,
{
    let selector = id.clone();
    let group = id.clone();
    div()
        .id(id)
        .debug_selector(move || selector.to_string())
        .group(group.clone())
        .w(px(STEPPER_SIDE_WIDTH_PX))
        .h_full()
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .role(Role::Button)
        .aria_label(aria_label)
        .when(disabled, |el| {
            el.opacity(STEPPER_SIDE_DISABLED_OPACITY)
                .cursor_not_allowed()
        })
        .when(!disabled, |el| {
            activate(
                el.cursor_pointer()
                    .tab_index(0)
                    .focus_visible(focus_ring)
                    .hover(|el| el.bg(gpui::white().opacity(0.05))),
                cx,
                action,
            )
        })
        .child(
            svg()
                .path(icon_path)
                .size(px(STEPPER_ICON_PX))
                .text_color(theme::rambutan_400())
                .when(!disabled, |el| {
                    el.group_hover(group, |el| el.text_color(theme::rambutan_100()))
                }),
        )
        .into_any_element()
}

impl Workspace {
    pub(super) fn update_meeting_prefs(
        &mut self,
        update: impl FnOnce(&mut MeetingPrefs),
        cx: &mut Context<'_, Self>,
    ) {
        self.meetings_hub
            .update(cx, |hub, cx| hub.update_prefs(update, cx));
    }

    fn settings_stepper_value(&self, kind: SettingsStepper, cx: &Context<'_, Self>) -> f64 {
        stepper_pref_value(kind, &self.meetings_hub.read(cx).prefs)
    }

    /// Snap, store and echo the value back into the field (web `emit`).
    pub(super) fn set_settings_stepper_value(
        &mut self,
        kind: SettingsStepper,
        value: f64,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let (min, max, step) = kind.spec();
        let value = snap(value, Some(min), Some(max), step);
        if value != self.settings_stepper_value(kind, cx) {
            self.update_meeting_prefs(
                |prefs| match kind {
                    SettingsStepper::MinAttendees => {
                        prefs.auto_record.min_attendees = value.to_u32_saturating();
                    }
                    SettingsStepper::StopDelay => {
                        prefs.auto_record.stop_delay_secs = value.to_i64_saturating();
                    }
                },
                cx,
            );
        }
        let text = format_value(value, step);
        self.settings_stepper_inputs[kind.index()].update(cx, |input, cx| {
            if input.value().as_ref() != text {
                input.set_value(text, window, cx);
            }
        });
    }

    /// Enter / blur in the field: web `on_commit` — garbage puts the
    /// current value back.
    pub(super) fn commit_settings_stepper_text(
        &mut self,
        kind: SettingsStepper,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let text = self.settings_stepper_inputs[kind.index()]
            .read(cx)
            .value()
            .trim()
            .to_string();
        let value = text
            .parse::<f64>()
            .unwrap_or_else(|_| self.settings_stepper_value(kind, cx));
        self.set_settings_stepper_value(kind, value, window, cx);
    }

    /// Web `Stepper`: minus / value / plus on one input-toned pill; the
    /// value is a text field, ↑/↓ nudge it.
    fn render_settings_stepper(
        &self,
        kind: SettingsStepper,
        disabled: bool,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let (min, max, step) = kind.spec();
        let value = self.settings_stepper_value(kind, cx);
        let at_min = value <= min;
        let at_max = value >= max;
        let selector = kind.selector();
        let suffix = kind.suffix();
        let input = self.settings_stepper_inputs[kind.index()].clone();
        div()
            .id(selector)
            .debug_selector(move || selector.to_string())
            .h(px(STEPPER_HEIGHT_PX))
            .flex()
            .items_stretch()
            .overflow_hidden()
            .rounded(px(GRAPE_RADIUS_PX))
            .bg(gpui::white().opacity(0.05))
            .border_1()
            .border_color(gpui::white().opacity(0.10))
            .when(disabled, |el| el.opacity(STEPPER_DISABLED_OPACITY))
            .child(stepper_side(
                format!("{selector}-decrease").into(),
                assets::MINUS_ICON_PATH,
                "Decrease",
                disabled || at_min,
                cx,
                move |this, window, cx| {
                    let current = this.settings_stepper_value(kind, cx);
                    this.set_settings_stepper_value(kind, current - step, window, cx)
                },
            ))
            .child(
                div()
                    .id(SharedString::from(format!("{selector}-value")))
                    .w(px(STEPPER_VALUE_WIDTH_PX))
                    .min_w_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .font_family(theme::CODE_FONT_FAMILY)
                    .text_size(px(BODY_SM_PX))
                    .text_color(theme::rambutan_100())
                    .when(disabled, |el| el.cursor_not_allowed())
                    .when(!disabled, |el| {
                        el.on_key_down(cx.listener(
                            move |this, event: &KeyDownEvent, window, cx| {
                                let dir = match event.keystroke.key.as_str() {
                                    "up" => 1.0,
                                    "down" => -1.0,
                                    _ => return,
                                };
                                window.prevent_default();
                                cx.stop_propagation();
                                let current = this.settings_stepper_value(kind, cx);
                                this.set_settings_stepper_value(
                                    kind,
                                    current + dir * step,
                                    window,
                                    cx,
                                );
                            },
                        ))
                    })
                    .child(
                        Input::new(&input)
                            .appearance(false)
                            .disabled(disabled)
                            .text_center()
                            .w_full()
                            .aria_label(kind.aria_label()),
                    ),
            )
            .when(!suffix.is_empty(), |el| {
                el.child(div().flex().items_center().pr_1().child(label_text(
                    suffix,
                    LABEL_SM_PX,
                    theme::rambutan_500(),
                )))
            })
            .child(stepper_side(
                format!("{selector}-increase").into(),
                assets::PLUS_ICON_PATH,
                "Increase",
                disabled || at_max,
                cx,
                move |this, window, cx| {
                    let current = this.settings_stepper_value(kind, cx);
                    this.set_settings_stepper_value(kind, current + step, window, cx)
                },
            ))
            .into_any_element()
    }

    fn settings_slider_value(&self, kind: SettingsSlider, cx: &Context<'_, Self>) -> f64 {
        let prefs = &self.meetings_hub.read(cx).prefs;
        match kind {
            SettingsSlider::MaxSpeakers => f64::from(prefs.diarize.max_speakers),
            SettingsSlider::LinkThreshold => {
                f64::from(prefs.diarize.cross_meeting_linking_threshold)
            }
        }
    }

    pub(super) fn set_settings_slider_value(
        &mut self,
        kind: SettingsSlider,
        value: f64,
        cx: &mut Context<'_, Self>,
    ) {
        let (min, max, step) = kind.spec();
        let value = snap(value, Some(min), Some(max), step);
        if value == self.settings_slider_value(kind, cx) {
            return;
        }
        self.update_meeting_prefs(
            |prefs| match kind {
                SettingsSlider::MaxSpeakers => {
                    prefs.diarize.max_speakers = value.to_u32_saturating().clamp(2, 20);
                }
                SettingsSlider::LinkThreshold => {
                    prefs.diarize.cross_meeting_linking_threshold = value.to_f32_lossy();
                }
            },
            cx,
        );
    }

    fn set_settings_slider_from_x(
        &mut self,
        kind: SettingsSlider,
        x: Pixels,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(bounds) = self.settings_slider_bounds[kind.index()].get() else {
            return;
        };
        let (min, max, step) = kind.spec();
        let value = slider_value_at(
            f32::from(x - bounds.left()),
            f32::from(bounds.size.width),
            min,
            max,
            step,
        );
        self.set_settings_slider_value(kind, value, cx);
    }

    fn begin_settings_slider_drag(
        &mut self,
        kind: SettingsSlider,
        x: Pixels,
        cx: &mut Context<'_, Self>,
    ) {
        self.settings_slider_drag = Some(kind);
        self.set_settings_slider_from_x(kind, x, cx);
    }

    /// Mouse moves anywhere over the settings page while a slider drag is
    /// on (the native range input keeps tracking outside its box too).
    pub(super) fn continue_settings_slider_drag(
        &mut self,
        event: &MouseMoveEvent,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(kind) = self.settings_slider_drag else {
            return;
        };
        if event.pressed_button == Some(MouseButton::Left) {
            self.set_settings_slider_from_x(kind, event.position.x, cx);
        } else {
            self.settings_slider_drag = None;
        }
    }

    pub(super) fn end_settings_slider_drag(&mut self) {
        self.settings_slider_drag = None;
    }

    /// Web `Slider`: longan track, persimmon fill up to the thumb centre,
    /// rambutan thumb; click or drag sets, ←/→ step.
    fn render_settings_slider(&self, kind: SettingsSlider, cx: &Context<'_, Self>) -> AnyElement {
        let (min, max, step) = kind.spec();
        let value = self.settings_slider_value(kind, cx);
        let fraction = slider_fraction(value, min, max).to_f32_lossy();
        let bounds_cell = self.settings_slider_bounds[kind.index()].clone();
        let selector = kind.selector();
        let track_top = (SLIDER_HEIGHT_PX - SLIDER_TRACK_PX) / 2.0;
        let thumb_top = (SLIDER_HEIGHT_PX - SLIDER_THUMB_PX) / 2.0;
        let half_thumb = SLIDER_THUMB_PX / 2.0;
        div()
            .id(selector)
            .debug_selector(move || selector.to_string())
            .relative()
            .w_full()
            .h(px(SLIDER_HEIGHT_PX))
            .cursor_pointer()
            .role(Role::Slider)
            .aria_label(kind.aria_label())
            .aria_numeric_value(value)
            .aria_min_numeric_value(min)
            .aria_max_numeric_value(max)
            .aria_numeric_value_step(step)
            .tab_index(0)
            .focus_visible(focus_ring)
            .child(
                canvas(
                    move |bounds, _window, _cx| bounds_cell.set(Some(bounds)),
                    |_bounds, _state, _window, _cx| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(px(track_top))
                    .h(px(SLIDER_TRACK_PX))
                    .rounded_full()
                    .bg(theme::longan_700())
                    .border_1()
                    .border_color(gpui::white().opacity(0.05)),
            )
            .child(
                // The fill runs from the track's left edge to the thumb
                // centre: a fixed half-thumb cap plus a share of the inner
                // travel (web `calc(0.5rem + (100% - 1rem) * pct)`).
                div()
                    .absolute()
                    .left(px(half_thumb))
                    .right(px(half_thumb))
                    .top(px(track_top))
                    .h(px(SLIDER_TRACK_PX))
                    .child(
                        div()
                            .absolute()
                            .left(px(-half_thumb))
                            .top_0()
                            .w(px(half_thumb))
                            .h_full()
                            .rounded_tl(px(SLIDER_TRACK_PX))
                            .rounded_bl(px(SLIDER_TRACK_PX))
                            .bg(theme::persimmon_500()),
                    )
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .top_0()
                            .w(relative(fraction))
                            .h_full()
                            .bg(theme::persimmon_500()),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .left(px(half_thumb))
                    .right(px(half_thumb))
                    .top(px(thumb_top))
                    .h(px(SLIDER_THUMB_PX))
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .left(relative(fraction))
                            .ml(px(-half_thumb))
                            .size(px(SLIDER_THUMB_PX))
                            .rounded_full()
                            .bg(theme::rambutan_100())
                            .border_1()
                            .border_color(rgba(0x14121099)),
                    ),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _window, cx| {
                    cx.stop_propagation();
                    this.begin_settings_slider_drag(kind, event.position.x, cx);
                }),
            )
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                let current = this.settings_slider_value(kind, cx);
                let next = match event.keystroke.key.as_str() {
                    "left" | "down" => current - step,
                    "right" | "up" => current + step,
                    "home" => min,
                    "end" => max,
                    _ => return,
                };
                window.prevent_default();
                cx.stop_propagation();
                this.set_settings_slider_value(kind, next, cx);
            }))
            .into_any_element()
    }

    /// Web `SettingsRowBody` with a slider and its two end labels.
    fn render_settings_slider_body(
        &self,
        kind: SettingsSlider,
        low_label: &'static str,
        high_label: &'static str,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        div()
            .px_4()
            .pb_4()
            .flex()
            .flex_col()
            .child(self.render_settings_slider(kind, cx))
            .child(
                div()
                    .mt_1()
                    .flex()
                    .justify_between()
                    .child(label_text(low_label, LABEL_SM_PX, theme::rambutan_500()))
                    .child(label_text(high_label, LABEL_SM_PX, theme::rambutan_500())),
            )
            .into_any_element()
    }

    // ── Calendar ────────────────────────────────────────────────────────

    /// Web `GoogleCalendarSettings`.
    pub(super) fn render_settings_calendar_rows(&self, cx: &Context<'_, Self>) -> Vec<AnyElement> {
        let hub = self.meetings_hub.read(cx);
        let sync = hub.google_sync.clone();
        let busy = hub.google_busy;
        let error = hub.google_error.clone();
        let calendars = hub.calendars.clone();
        let has_google_events = hub.has_google_events();
        let status = google_status_text(&sync, EpochMs::now());

        let controls: AnyElement = if sync.is_connected() {
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(activate(
                    web_button(
                        "desktop-settings-google-refresh",
                        WebButtonVariant::Ghost,
                        WebButtonSize::Small,
                        ButtonState::disabled_if(busy),
                    )
                    .debug_selector(|| "desktop-settings-google-refresh".to_string())
                    .child("Refresh"),
                    cx,
                    |this, _window, cx| {
                        this.meetings_hub
                            .update(cx, |hub, cx| hub.refresh_google_sync(cx))
                    },
                ))
                .child(activate(
                    web_button(
                        "desktop-settings-google-disconnect",
                        WebButtonVariant::Outline,
                        WebButtonSize::Small,
                        ButtonState::disabled_if(busy),
                    )
                    .debug_selector(|| "desktop-settings-google-disconnect".to_string())
                    .child("Disconnect"),
                    cx,
                    |this, _window, cx| {
                        this.meetings_hub
                            .update(cx, |hub, cx| hub.disconnect_google(cx))
                    },
                ))
                .into_any_element()
        } else {
            activate(
                web_button(
                    "desktop-settings-google-connect",
                    WebButtonVariant::Primary,
                    WebButtonSize::Small,
                    ButtonState::disabled_if(busy),
                )
                .debug_selector(|| "desktop-settings-google-connect".to_string())
                .child(if busy { "Connecting…" } else { "Connect" }),
                cx,
                |this, _window, cx| {
                    this.meetings_hub
                        .update(cx, |hub, cx| hub.connect_google(cx))
                },
            )
            .into_any_element()
        };

        let mut rows = vec![self.settings_row_opts(
            "desktop-settings-row-google",
            "Google Calendar",
            "",
            Some(status.into()),
            false,
            controls,
        )];
        if let Some(error) = error {
            rows.push(
                div()
                    .debug_selector(|| "desktop-settings-google-error".to_string())
                    .px_4()
                    .py_2()
                    .child(label_text(error, LABEL_SM_PX, theme::lychee_300()))
                    .into_any_element(),
            );
        }
        if sync.is_connected() {
            rows.push(self.render_calendar_picker(&calendars));
        }
        if !sync.is_connected() && has_google_events {
            rows.push(
                self.settings_row(
                    "desktop-settings-row-imported-events",
                    "Imported events",
                    "Events that came from Google Calendar before it was disconnected.",
                    activate(
                        web_button(
                            "desktop-settings-google-clear",
                            WebButtonVariant::Ghost,
                            WebButtonSize::Small,
                            ButtonState::Enabled,
                        )
                        .debug_selector(|| "desktop-settings-google-clear".to_string())
                        .child("Remove all"),
                        cx,
                        |this, _window, cx| {
                            this.meetings_hub
                                .update(cx, |hub, cx| hub.clear_google_events(cx))
                        },
                    )
                    .into_any_element(),
                ),
            );
        }
        rows
    }

    /// Web `CalendarPicker`: per-calendar sync checkboxes.
    fn render_calendar_picker(&self, calendars: &[Calendar]) -> AnyElement {
        let mut body = div()
            .debug_selector(|| "desktop-settings-calendar-picker".to_string())
            .px_4()
            .pb_4()
            .flex()
            .flex_col()
            .gap_1()
            .child(label_text("Calendars", LABEL_PX, theme::rambutan_500()));
        if calendars.is_empty() {
            body = body.child(
                label_text(
                    "No calendars yet — wait for the next sync.",
                    LABEL_PX,
                    theme::rambutan_500(),
                )
                .italic(),
            );
            return body.into_any_element();
        }
        let mut list = div().flex().flex_col().gap(px(2.0));
        for (ix, calendar) in calendars.iter().enumerate() {
            let id = calendar.id.clone();
            let enabled = calendar.sync_enabled;
            let hub = self.meetings_hub.clone();
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .when(calendar.primary, |el| {
                                el.child(label_text("PRIMARY", LABEL_SM_PX, theme::persimmon_400()))
                            })
                            .child(label_text(
                                calendar.name.clone(),
                                LABEL_PX,
                                theme::rambutan_300(),
                            )),
                    )
                    .child(
                        Checkbox::new(("desktop-settings-calendar-sync", ix))
                            .checked(enabled)
                            .on_click(move |_checked, _window, cx| {
                                let id = id.clone();
                                hub.update(cx, |hub, cx| hub.set_calendar_sync(id, !enabled, cx));
                            }),
                    ),
            );
        }
        body.child(list).into_any_element()
    }

    // ── Recording ───────────────────────────────────────────────────────

    /// Web `AutoRecordSettingsPanel`.
    pub(super) fn render_settings_recording_rows(&self, cx: &Context<'_, Self>) -> Vec<AnyElement> {
        let prefs = self.meetings_hub.read(cx).prefs.clone();
        let settings = prefs.auto_record;
        let disabled = !settings.enabled;
        let mut skipped = prefs.suppressed_events;
        skipped.sort();

        let mut rows = vec![
            self.settings_row(
                "desktop-settings-row-auto-record",
                "Auto-record calendar meetings",
                "Start recording when a Google Calendar event begins. Needs the bruhs-capture daemon running and a calendar connected.",
                switch(
                    "desktop-settings-auto-record",
                    settings.enabled,
                    false,
                    "Auto-record calendar meetings",
                    cx,
                    move |this, _window, cx| {
                        this.update_meeting_prefs(
                            |p| p.auto_record.enabled = !settings.enabled,
                            cx,
                        )
                    },
                ),
            ),
            self.settings_row_opts(
                "desktop-settings-row-min-attendees",
                "Minimum attendees",
                "Only events with at least this many attendees are recorded.",
                None,
                disabled,
                self.render_settings_stepper(SettingsStepper::MinAttendees, disabled, cx),
            ),
            self.settings_row_opts(
                "desktop-settings-row-stop-delay",
                "Stop after the event ends",
                "Grace period before the recording stops.",
                None,
                disabled,
                self.render_settings_stepper(SettingsStepper::StopDelay, disabled, cx),
            ),
            self.settings_row_opts(
                "desktop-settings-row-prompt-before-starting",
                "Prompt before starting",
                "Off means recordings start silently.",
                None,
                disabled,
                switch(
                    "desktop-settings-prompt-before-starting",
                    settings.prompt_before_starting,
                    disabled,
                    "Prompt before starting",
                    cx,
                    move |this, _window, cx| {
                        this.update_meeting_prefs(
                            |p| {
                                p.auto_record.prompt_before_starting =
                                    !settings.prompt_before_starting
                            },
                            cx,
                        )
                    },
                ),
            ),
            self.settings_row_opts(
                "desktop-settings-row-audio-driven",
                "Also detect from audio activity",
                bruhs_models::ACTIVITY_MONITOR_HELP,
                Some(self.meetings_hub.read(cx).monitor_feedback.clone().into()),
                disabled,
                switch(
                    "desktop-settings-audio-driven",
                    settings.audio_driven_enabled,
                    disabled,
                    "Detect meetings from audio activity",
                    cx,
                    move |this, _window, cx| {
                        this.update_meeting_prefs(
                            |p| {
                                p.auto_record.audio_driven_enabled =
                                    !settings.audio_driven_enabled
                            },
                            cx,
                        )
                    },
                ),
            ),
            self.settings_row(
                "desktop-settings-row-always-skip",
                "Always-skip events",
                "Events you told the prompt to never record.",
                label_text(
                    if skipped.is_empty() {
                        "None".to_string()
                    } else {
                        skipped.len().to_string()
                    },
                    LABEL_SM_PX,
                    theme::rambutan_500(),
                )
                .into_any_element(),
            ),
        ];

        if !skipped.is_empty() {
            let mut list = div().flex().flex_col().gap(px(1.0));
            for (ix, key) in skipped.into_iter().enumerate() {
                let remove_key = key.clone();
                list = list.child(
                    div()
                        .id(("desktop-settings-skip-event", ix))
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .rounded(px(GRAPE_RADIUS_PX))
                        .px_2()
                        .py_1()
                        .hover(|el| el.bg(theme::longan_800().opacity(0.4)))
                        .child(
                            code_text(key, CODE_SM_PX, theme::rambutan_300())
                                .min_w_0()
                                .flex_1()
                                .truncate(),
                        )
                        .child(activate(
                            div()
                                .id(("desktop-settings-skip-event-remove", ix))
                                .flex_shrink_0()
                                .rounded(px(SEED_RADIUS_PX))
                                .px(px(6.0))
                                .role(Role::Button)
                                .aria_label("Remove")
                                .tab_index(0)
                                .focus_visible(focus_ring)
                                .cursor_pointer()
                                .text_color(theme::rambutan_400())
                                .hover(|el| el.text_color(theme::lychee_300()))
                                .child(label_text("Remove", LABEL_SM_PX, theme::rambutan_400())),
                            cx,
                            move |this, _window, cx| {
                                let key = remove_key.clone();
                                this.meetings_hub
                                    .update(cx, |hub, cx| hub.remove_suppressed_event(&key, cx));
                            },
                        )),
                );
            }
            rows.push(
                div()
                    .debug_selector(|| "desktop-settings-skip-events".to_string())
                    .px_4()
                    .pb_4()
                    .child(list)
                    .into_any_element(),
            );
        }
        rows
    }

    // ── Transcription ───────────────────────────────────────────────────

    /// Web `WhisperModelSettings`.
    pub(super) fn render_settings_transcription_rows(
        &self,
        cx: &Context<'_, Self>,
    ) -> Vec<AnyElement> {
        let prefs = self.meetings_hub.read(cx).prefs.clone();
        let (provider_toggle, provider_select) =
            self.settings_dropdown_handlers(SETTINGS_TRANSCRIPTION_PROVIDER_DROPDOWN_ID, cx);
        let provider_control = div()
            .w(px(WHISPER_DROPDOWN_WIDTH_PX))
            .child(dropdown(
                SETTINGS_TRANSCRIPTION_PROVIDER_DROPDOWN_ID,
                "Transcription provider",
                prefs.transcription.provider.key(),
                vec![DropdownGroup {
                    label: None,
                    mark: None,
                    options: bruhs_models::TranscriptionProvider::ALL
                        .into_iter()
                        .map(|provider| DropdownOption {
                            value: provider.key(),
                            label: provider.label().into(),
                            description: None,
                        })
                        .collect(),
                }],
                self.settings_listbox(SETTINGS_TRANSCRIPTION_PROVIDER_DROPDOWN_ID),
                provider_toggle,
                provider_select,
            ))
            .into_any_element();
        let mut rows = vec![self.settings_row(
            "desktop-settings-row-transcription-provider",
            "Transcription provider",
            "Applies to the final transcript on your next recording. On-device Whisper keeps audio here. A compatible server receives recorded audio after Stop and may charge for usage. Live previews always stay on-device.",
            provider_control,
        )];
        if prefs.transcription.provider == bruhs_models::TranscriptionProvider::OpenAiCompatible {
            rows.push(self.settings_row(
                "desktop-settings-row-transcription-server-model",
                "Server transcription model",
                "Enter a model ID supported by your configured server. It must return timestamped segments in verbose_json format. Configure the server URL, key, and request deadline in the capture daemon environment.",
                div().w(px(WHISPER_DROPDOWN_WIDTH_PX))
                    .child(Input::new(&self.settings_transcription_model_input)
                        .aria_label("Server transcription model"))
                    .into_any_element(),
            ));
            if let Err(error) = prefs.transcription.validate() {
                rows.push(
                    div()
                        .px_4()
                        .pb_3()
                        .child(label_text(
                            error.to_string(),
                            LABEL_SM_PX,
                            theme::lychee_300(),
                        ))
                        .into_any_element(),
                );
            }
        }
        let (final_toggle, final_select) =
            self.settings_dropdown_handlers(SETTINGS_WHISPER_DROPDOWN_ID, cx);
        let final_control = div()
            .w(px(WHISPER_DROPDOWN_WIDTH_PX))
            .child(dropdown(
                SETTINGS_WHISPER_DROPDOWN_ID,
                "Transcription model",
                prefs.whisper_model.key(),
                whisper_options(),
                self.settings_listbox(SETTINGS_WHISPER_DROPDOWN_ID),
                final_toggle,
                final_select,
            ))
            .into_any_element();
        let (live_toggle, live_select) =
            self.settings_dropdown_handlers(SETTINGS_LIVE_WHISPER_DROPDOWN_ID, cx);
        let live_control = div()
            .w(px(WHISPER_DROPDOWN_WIDTH_PX))
            .child(dropdown(
                SETTINGS_LIVE_WHISPER_DROPDOWN_ID,
                "Live preview model",
                prefs.live_whisper_model.key(),
                whisper_options(),
                self.settings_listbox(SETTINGS_LIVE_WHISPER_DROPDOWN_ID),
                live_toggle,
                live_select,
            ))
            .into_any_element();
        if prefs.transcription.provider == bruhs_models::TranscriptionProvider::LocalWhisper {
            rows.push(self.settings_row(
                "desktop-settings-row-whisper-model",
                "Transcription model",
                "Downloaded on first use to ~/Library/Application Support/dev.bruhs.bruhs-ai/models. Takes effect on the next recording.",
                final_control,
            ));
        }
        rows.push(self.settings_row(
                "desktop-settings-row-live-whisper-model",
                "Live preview model",
            "Used for the live transcript while recording. Smaller is faster; the final pass uses your selected provider and model.",
                live_control,
            ));
        rows
    }

    // ── Speakers ────────────────────────────────────────────────────────

    /// Web `DiarizeSettingsPanel`.
    pub(super) fn render_settings_speakers_rows(&self, cx: &Context<'_, Self>) -> Vec<AnyElement> {
        let diarize = self.meetings_hub.read(cx).prefs.diarize;
        let mut rows = vec![self.settings_row(
            "desktop-settings-row-subspeaker",
            "Distinguish multiple speakers",
            "Clusters voice embeddings on the system-audio channel so remote speakers become Speaker 1, Speaker 2, and so on. ~30 MB model, downloaded on first use.",
            switch(
                "desktop-settings-subspeaker",
                diarize.subspeaker_enabled,
                false,
                "Distinguish multiple speakers",
                cx,
                {
                    let enabled = diarize.subspeaker_enabled;
                    move |this, _window, cx| {
                        this.update_meeting_prefs(|p| p.diarize.subspeaker_enabled = !enabled, cx)
                    }
                },
            ),
        )];
        if diarize.subspeaker_enabled {
            rows.push(self.settings_row(
                "desktop-settings-row-max-speakers",
                "Max speakers",
                "Higher resolves more people on large calls; lower is faster with fewer false splits.",
                code_text(
                    diarize.max_speakers.to_string(),
                    CODE_SM_PX,
                    theme::rambutan_200(),
                )
                .into_any_element(),
            ));
            rows.push(self.render_settings_slider_body(
                SettingsSlider::MaxSpeakers,
                "2 · faster",
                "20 · more resolution",
                cx,
            ));
        }
        rows.push(self.settings_row(
            "desktop-settings-row-video-ocr",
            "Read participant names from video",
            "Reads on-screen name hints locally at 2 fps. Visible names do not prove who is speaking and will not rename transcript speakers. Frames stay in memory; name observations are saved on this device. Requires Screen Recording permission.",
            switch(
                "desktop-settings-video-ocr",
                diarize.video_ocr_enabled,
                false,
                "Read participant names from video",
                cx,
                {
                    let enabled = diarize.video_ocr_enabled;
                    move |this, _window, cx| {
                        this.update_meeting_prefs(|p| p.diarize.video_ocr_enabled = !enabled, cx)
                    }
                },
            ),
        ));
        rows.push(self.settings_row(
            "desktop-settings-row-auto-link",
            "Link voices across meetings",
            "Match new clusters against known speaker profiles by voice similarity. Off makes every cluster a fresh profile you can link by hand.",
            switch(
                "desktop-settings-auto-link",
                diarize.cross_meeting_linking_auto,
                false,
                "Auto-link voices across meetings",
                cx,
                {
                    let enabled = diarize.cross_meeting_linking_auto;
                    move |this, _window, cx| {
                        this.update_meeting_prefs(
                            |p| p.diarize.cross_meeting_linking_auto = !enabled,
                            cx,
                        )
                    }
                },
            ),
        ));
        if diarize.cross_meeting_linking_auto {
            rows.push(
                self.settings_row(
                    "desktop-settings-row-link-threshold",
                    "Linking threshold",
                    "Higher means fewer false matches, lower means better recall. Default 0.75.",
                    code_text(
                        format!("{:.2}", diarize.cross_meeting_linking_threshold),
                        CODE_SM_PX,
                        theme::rambutan_200(),
                    )
                    .into_any_element(),
                ),
            );
            rows.push(self.render_settings_slider_body(
                SettingsSlider::LinkThreshold,
                "0.50 · looser",
                "0.95 · stricter",
                cx,
            ));
        }
        rows
    }

    // ── People ──────────────────────────────────────────────────────────

    /// Web `PersonalitySettings`.
    pub(super) fn render_settings_people_rows(&self, cx: &Context<'_, Self>) -> Vec<AnyElement> {
        let extract = self
            .meetings_hub
            .read(cx)
            .prefs
            .personality
            .extract_observations;
        vec![self.settings_row(
            "desktop-settings-row-observations",
            "Per-person observations",
            "Each meeting summary also captures topics, decisions, action items, tone and notable quotes per speaker. Profiles populate the People page.",
            switch(
                "desktop-settings-observations",
                extract,
                false,
                "Extract per-person observations during summarization",
                cx,
                move |this, _window, cx| {
                    this.update_meeting_prefs(
                        |p| p.personality.extract_observations = !extract,
                        cx,
                    )
                },
            ),
        )]
    }
}

/// Fixed-width column for a row's dropdown (web `ROW_CONTROL` / `sm:w-80`).
pub(super) fn control_column(control: AnyElement) -> AnyElement {
    div()
        .w(px(SETTINGS_ROW_CONTROL_WIDTH_PX))
        .child(control)
        .into_any_element()
}

impl Workspace {
    pub(super) fn new_transcription_model_input(
        prefs: &MeetingPrefs,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
        subscriptions: &mut Vec<Subscription>,
    ) -> Entity<InputState> {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Model ID from your server")
                .default_value(prefs.transcription.remote_model.clone())
        });
        subscriptions.push(cx.subscribe_in(
            &input,
            window,
            |this: &mut Self, input, event: &InputEvent, _window, cx| {
                if matches!(event, InputEvent::Change) {
                    let value = input.read(cx).value().to_string();
                    this.update_meeting_prefs(|prefs| prefs.transcription.remote_model = value, cx);
                }
            },
        ));
        input
    }

    /// The Recording steppers' text fields; Enter or blur commits.
    pub(super) fn new_settings_stepper_inputs(
        stepper_prefs: &MeetingPrefs,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
        subscriptions: &mut Vec<Subscription>,
    ) -> Vec<Entity<InputState>> {
        let settings_stepper_inputs: Vec<Entity<InputState>> = SettingsStepper::ALL
            .iter()
            .map(|kind| {
                let kind = *kind;
                let text = kind.text(stepper_prefs);
                let input = cx.new(|cx| InputState::new(window, cx).default_value(text));
                subscriptions.push(cx.subscribe_in(
                    &input,
                    window,
                    move |this: &mut Self, _input, ev: &InputEvent, window, cx| {
                        if matches!(ev, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                            this.commit_settings_stepper_text(kind, window, cx);
                        }
                    },
                ));
                input
            })
            .collect();
        settings_stepper_inputs
    }
}

#[cfg(test)]
mod tests {
    use super::{
        format_value, google_status_text, slider_fraction, slider_value_at, snap,
        whisper_model_for_key, whisper_options, EpochMs,
    };
    use bruhs_models::{GoogleCalendarSyncState, GoogleConnection, WhisperModel};

    #[test]
    fn snap_rounds_to_grid_and_clamps_like_web() {
        assert_eq!(snap(7.4, Some(0.0), None, 1.0), 7.0);
        assert_eq!(snap(7.6, Some(0.0), None, 1.0), 8.0);
        assert_eq!(snap(-3.0, Some(0.0), None, 1.0), 0.0);
        assert_eq!(snap(99.0, Some(2.0), Some(20.0), 1.0), 20.0);
        assert_eq!(snap(0.73, Some(0.5), Some(0.95), 0.05), 0.75);
        assert_eq!(snap(5.0, None, None, 0.0), 5.0);
    }

    #[test]
    fn format_follows_step_grid_like_web() {
        assert_eq!(format_value(60.0, 1.0), "60");
        assert_eq!(format_value(0.75, 0.05), "0.75");
    }

    #[test]
    fn slider_fraction_tracks_the_thumb_centre() {
        assert_eq!(slider_fraction(2.0, 2.0, 20.0), 0.0);
        assert_eq!(slider_fraction(20.0, 2.0, 20.0), 1.0);
        assert_eq!(slider_fraction(11.0, 2.0, 20.0), 0.5);
        assert_eq!(slider_fraction(5.0, 5.0, 5.0), 0.0);
    }

    #[test]
    fn slider_value_at_maps_pointer_to_snapped_values() {
        // 216 px track: 16 px thumb leaves 200 px of travel.
        assert_eq!(slider_value_at(0.0, 216.0, 2.0, 20.0, 1.0), 2.0);
        assert_eq!(slider_value_at(216.0, 216.0, 2.0, 20.0, 1.0), 20.0);
        assert_eq!(slider_value_at(108.0, 216.0, 2.0, 20.0, 1.0), 11.0);
        assert_eq!(slider_value_at(112.0, 216.0, 0.5, 0.95, 0.05), 0.75);
        assert_eq!(slider_value_at(-40.0, 216.0, 0.5, 0.95, 0.05), 0.5);
    }

    #[test]
    fn google_status_text_matches_web_copy() {
        let mut state = GoogleCalendarSyncState::default();
        assert_eq!(
            google_status_text(&state, EpochMs::UNIX_EPOCH),
            "Not connected. Connect to import events and enable auto-record."
        );
        state.connection = GoogleConnection::Connected { email: None };
        assert_eq!(google_status_text(&state, EpochMs::UNIX_EPOCH), "Connected");
        state.connection = GoogleConnection::Connected {
            email: Some("me@example.com".into()),
        };
        state.last_synced_at = Some(0);
        assert_eq!(
            google_status_text(&state, EpochMs::new(90_000).unwrap()),
            "Connected as me@example.com. Last synced 1m ago."
        );
        state.last_error = Some("token expired".into());
        assert_eq!(
            google_status_text(&state, EpochMs::new(90_000).unwrap()),
            "Connected as me@example.com. token expired"
        );
    }

    #[test]
    fn whisper_options_label_like_web() {
        let groups = whisper_options();
        let base = groups[0]
            .options
            .iter()
            .find(|o| o.value == "base_en")
            .expect("base model");
        assert_eq!(base.label, "Base (English) — balanced, default (150 MB)");
        assert_eq!(
            whisper_model_for_key("large_v3_turbo"),
            Some(WhisperModel::LargeV3Turbo)
        );
        assert_eq!(whisper_model_for_key("nope"), None);
    }
}
