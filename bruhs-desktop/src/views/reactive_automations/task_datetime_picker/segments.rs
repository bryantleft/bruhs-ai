//! Keyboard-editable parts of the one-time and recurring schedule fields.
//! The underlying editor keeps its local ISO date or list of daily times.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    Year,
    Month,
    Day,
    Hour,
    Minute,
    Period,
}
use Part::*;

impl Part {
    fn name(self) -> &'static str {
        match self {
            Year => "Year",
            Month => "Month",
            Day => "Day",
            Hour => "Hour",
            Minute => "Minute",
            Period => "AM/PM",
        }
    }
    fn bounds(self, h24: bool) -> (u32, u32) {
        match self {
            Year => (1, 9999),
            Month => (1, 12),
            Day => (1, 31),
            Hour if h24 => (0, 23),
            Hour => (1, 12),
            Minute => (0, 59),
            Period => (0, 1),
        }
    }
}

struct DateLayout {
    order: [Part; 3],
    literals: [String; 4],
}

impl DateLayout {
    fn from_pattern(pattern: &str) -> Option<Self> {
        let posix = pattern.contains('%');
        let pattern = pattern.replace("%D", "%m/%d/%y").replace("%F", "%Y-%m-%d");
        let mut chars = pattern.chars().peekable();
        let mut order = Vec::new();
        let mut literals = [const { String::new() }; 4];
        let mut quoted = false;
        while let Some(c) = chars.next() {
            let token = if posix && c == '%' {
                while chars
                    .next_if(|c| matches!(c, 'E' | 'O' | '-' | '_' | '0'))
                    .is_some()
                {}
                chars.next()?
            } else if !posix && c == '\'' {
                if chars.next_if_eq(&'\'').is_some() {
                    literals[order.len()].push(c);
                } else {
                    quoted = !quoted;
                }
                continue;
            } else if posix || quoted || !c.is_ascii_alphabetic() {
                literals[order.len()].push(c);
                continue;
            } else {
                while chars.next_if_eq(&c).is_some() {}
                c
            };
            let field = match token {
                'Y' | 'y' => Year,
                'M' | 'm' | 'L' => Month,
                'd' | 'e' => Day,
                _ => return None,
            };
            if order.contains(&field) {
                return None;
            }
            order.push(field);
        }
        Some(Self {
            order: order.try_into().ok()?,
            literals,
        })
    }
}

#[derive(Clone, Copy, Default)]
struct Values([Option<u32>; 6]);
impl Values {
    fn read(editor: &AutomationEditor, cx: &App) -> Self {
        let once = editor.source.kind == "once";
        let raw = if once {
            value(&editor.source.value, cx)
        } else {
            value(&editor.source.times, cx)
        };
        let first = raw.split(',').next().unwrap_or_default().trim();
        let mut fields = Self::default();
        let mut clock = first;
        if once {
            // An explicitly pasted offset represents an instant on this device.
            if let Ok(at) = chrono::DateTime::parse_from_rfc3339(first) {
                let local = at.with_timezone(&Local);
                fields.0[Year as usize] = u32::try_from(local.year()).ok();
                fields.0[Month as usize] = Some(local.month());
                fields.0[Day as usize] = Some(local.day());
                fields.set_clock(local.time());
                return fields;
            }
            let (date, time) = first.split_once(' ').unwrap_or((first, ""));
            for (part, text) in [Year, Month, Day].into_iter().zip(date.split('-')) {
                fields.0[part as usize] = text.parse().ok();
            }
            clock = time;
        }
        if let Some(time) = task_schedule::parse_clock(clock) {
            fields.set_clock(time);
        } else {
            let (hm, period) = clock.split_once(' ').unwrap_or((clock, ""));
            if let Some((h, m)) = hm.split_once(':') {
                fields.0[Hour as usize] = h.parse().ok();
                fields.0[Minute as usize] = m.parse().ok();
            }
            fields.0[Period as usize] = match period {
                "AM" => Some(0),
                "PM" => Some(1),
                _ => None,
            };
        }
        fields
    }

    fn set_clock(&mut self, time: NaiveTime) {
        self.0[Hour as usize] = Some(if crate::clock_format::uses_24_hours() {
            time.hour()
        } else {
            time.hour12().1
        });
        self.0[Minute as usize] = Some(time.minute());
        self.0[Period as usize] = Some(time.hour() / 12);
    }

    fn display(self, part: Part) -> String {
        match (part, self.0[part as usize]) {
            (Period, Some(0)) => "AM".into(),
            (Period, Some(_)) => "PM".into(),
            (Year, Some(n)) => format!("{n:04}"),
            (_, Some(n)) => format!("{n:02}"),
            (Year, None) => "yyyy".into(),
            (Month, None) => "mm".into(),
            (Day, None) => "dd".into(),
            (_, None) => "--".into(),
        }
    }

    fn write(self, editor: &mut AutomationEditor, window: &mut Window, cx: &mut App) {
        let mut time = format!("{}:{}", self.display(Hour), self.display(Minute));
        if !crate::clock_format::uses_24_hours() {
            time.push(' ');
            time.push_str(&self.display(Period));
        }
        if editor.source.kind == "once" {
            let text = format!(
                "{}-{}-{} {time}",
                self.display(Year),
                self.display(Month),
                self.display(Day)
            );
            editor.source.value.set_value(text, window, cx);
        } else {
            let old = value(&editor.source.times, cx);
            if let Some((_, other_times)) = old.split_once(',') {
                time.push(',');
                time.push_str(other_times);
            }
            editor.source.times.set_text_value(time.into(), window, cx);
        }
    }
}

pub(super) struct Segments {
    layout: DateLayout,
    focus: [FocusHandle; 6],
    digits: Option<(Part, u32, usize)>,
    pub(super) text_mode: bool,
}
impl Segments {
    pub(super) fn new(cx: &mut App) -> Self {
        Self {
            layout: DateLayout::from_pattern(&crate::clock_format::date_time_prefix_pattern())
                .unwrap_or_else(|| DateLayout {
                    order: [Year, Month, Day],
                    literals: [String::new(), "-".into(), "-".into(), String::new()],
                }),
            focus: std::array::from_fn(|_| cx.focus_handle().tab_stop(true)),
            digits: None,
            text_mode: false,
        }
    }

    fn order(&self, once: bool) -> impl Iterator<Item = Part> + '_ {
        self.layout
            .order
            .iter()
            .copied()
            .filter(move |_| once)
            .chain([Hour, Minute])
            .chain((!crate::clock_format::uses_24_hours()).then_some(Period))
    }

    pub(super) fn use_text_entry(&mut self, enabled: bool) {
        self.text_mode = enabled;
        self.digits = None;
    }

    pub(super) fn focus_first(&self, once: bool, window: &mut Window, cx: &mut App) {
        let first = if once { self.layout.order[0] } else { Hour };
        self.focus[first as usize].focus(window, cx);
    }

    fn focus_neighbor(
        &mut self,
        once: bool,
        part: Part,
        reverse: bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        let next = if reverse {
            self.order(once).take_while(|&p| p != part).last()
        } else {
            self.order(once).skip_while(|&p| p != part).nth(1)
        };
        self.digits = None;
        if let Some(next) = next {
            self.focus[next as usize].focus(window, cx);
        }
    }
}

impl ReactiveAutomations {
    fn edit_segment(
        &mut self,
        part: Part,
        key: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        if self.busy {
            return false;
        }
        let Some(editor) = self.schedule_editor_mut() else {
            return false;
        };
        let once = editor.source.kind == "once";
        let h24 = crate::clock_format::uses_24_hours();
        let mut value = Values::read(editor, cx);
        let (min, max) = part.bounds(h24);
        let current = value.0[part as usize];
        let segments = &mut editor.date_time_picker.segments;
        let pending = segments.digits.take();
        let mut advance = false;
        let updated = match key {
            "left" | "right" => {
                segments.focus_neighbor(once, part, key == "left", window, cx);
                return true;
            }
            "backspace" | "delete" => None,
            "up" | "down" => {
                let up = key == "up";
                if part == Hour && !h24 && current == Some(if up { 11 } else { 12 }) {
                    value.0[Period as usize] = value.0[Period as usize].map(|p| 1 - p);
                }
                Some(match current {
                    None if part == Year => u32::try_from(Local::now().year()).unwrap_or(min),
                    None if up => min,
                    None => max,
                    Some(n) if up && n >= max => min,
                    Some(n) if !up && n <= min => max,
                    Some(n) if up => n + 1,
                    Some(n) => n - 1,
                })
            }
            "a" | "p" if part == Period => Some(u32::from(key == "p")),
            digit
                if digit.len() == 1
                    && digit.bytes().all(|b| b.is_ascii_digit())
                    && part != Period =>
            {
                let digit = u32::from(digit.as_bytes()[0] - b'0');
                let (mut n, mut len) = (digit, 1);
                if let Some((_, previous, count)) = pending.filter(|entry| entry.0 == part) {
                    let limit = if part == Year { 4 } else { 2 };
                    if count < limit && previous * 10 + digit <= max {
                        n = previous * 10 + digit;
                        len = count + 1;
                    }
                }
                advance = part != Year && (len == 2 || n * 10 > max);
                if !advance {
                    segments.digits = Some((part, n, len));
                }
                (n >= min).then_some(n)
            }
            _ => return false,
        };
        if current != updated {
            value.0[part as usize] = updated;
            value.write(editor, window, cx);
        }
        if advance {
            editor
                .date_time_picker
                .segments
                .focus_neighbor(once, part, false, window, cx);
        }
        cx.notify();
        true
    }

    fn schedule_part(
        &self,
        editor: &AutomationEditor,
        values: Values,
        part: Part,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement {
        let (min, max) = part.bounds(crate::clock_format::uses_24_hours());
        let id = format!("schedule-segment-{}", part.name().to_lowercase());
        let focus = editor.date_time_picker.segments.focus[part as usize].clone();
        let text = values.display(part);
        div()
            .id(SharedString::from(id.clone()))
            .debug_selector(move || id)
            .role(Role::SpinButton)
            .aria_label(part.name())
            .aria_value(text.clone())
            .aria_min_numeric_value(f64::from(min))
            .aria_max_numeric_value(f64::from(max))
            .aria_numeric_value_step(1.)
            .when_some(values.0[part as usize], |el, n| {
                el.aria_numeric_value(f64::from(n))
            })
            .when(self.busy, |el| el.opacity(0.6))
            .flex_shrink_0()
            .rounded(px(2.))
            .when(!self.busy, |el| {
                let target = cx.weak_entity();
                let down = cx.weak_entity();
                el.track_focus(&focus)
                    .focus(|el| {
                        el.bg(theme::persimmon_400())
                            .text_color(theme::longan_950())
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            if let Some(editor) = this.schedule_editor_mut() {
                                editor.date_time_picker.segments.digits = None;
                            }
                            focus.focus(window, cx);
                        }),
                    )
                    .on_a11y_action(gpui::AccessibleAction::Increment, move |_, win, cx| {
                        let _ =
                            target.update(cx, |this, cx| this.edit_segment(part, "up", win, cx));
                    })
                    .on_a11y_action(gpui::AccessibleAction::Decrement, move |_, win, cx| {
                        let _ =
                            down.update(cx, |this, cx| this.edit_segment(part, "down", win, cx));
                    })
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                        let mods = event.keystroke.modifiers;
                        if mods.platform || mods.control || mods.alt {
                            return;
                        }
                        if this.edit_segment(part, &event.keystroke.key, window, cx) {
                            window.prevent_default();
                            cx.stop_propagation();
                        }
                    }))
            })
            .child(text)
    }

    pub(super) fn schedule_segments(
        &self,
        editor: &AutomationEditor,
        cx: &Context<'_, Self>,
    ) -> Div {
        let values = Values::read(editor, cx);
        let once = editor.source.kind == "once";
        let layout = &editor.date_time_picker.segments.layout;
        let mut el = div()
            .id("schedule-segmented-field")
            .debug_selector(|| "schedule-segmented-field".into())
            .role(Role::Group)
            .aria_label(if once { "Run on" } else { "Run at" })
            .h(px(32.))
            .w_full()
            .px(px(8.))
            .pr(px(31.))
            .flex()
            .items_center()
            .overflow_hidden()
            .rounded(px(6.))
            .border_1()
            .border_color(theme::ring_floating())
            .bg(theme::longan_950())
            .font_family(crate::theme::UI_FONT_FAMILY)
            .text_size(px(14.));
        if once {
            for (index, part) in layout.order.into_iter().enumerate() {
                el = el
                    .child(layout.literals[index].clone())
                    .child(self.schedule_part(editor, values, part, cx));
            }
            el = el.child(layout.literals[3].clone()).child(div().w(px(6.)));
        }
        el = el
            .child(self.schedule_part(editor, values, Hour, cx))
            .child(":")
            .child(self.schedule_part(editor, values, Minute, cx));
        if !crate::clock_format::uses_24_hours() {
            el = el
                .child(div().w(px(4.)))
                .child(self.schedule_part(editor, values, Period, cx));
        }
        div().child(el)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::core::prelude::v1::test;

    #[test]
    fn regional_date_layout_keeps_field_order_and_literals() {
        for (pattern, order, literals) in [
            ("M/d/y,", [Month, Day, Year], ["", "/", "/", ","]),
            ("%d.%m.%Y", [Day, Month, Year], ["", ".", ".", ""]),
            ("y年M月d日", [Year, Month, Day], ["", "年", "月", "日"]),
            (
                "dd 'de' MM 'de' yyyy",
                [Day, Month, Year],
                ["", " de ", " de ", ""],
            ),
            ("%F", [Year, Month, Day], ["", "-", "-", ""]),
        ] {
            let layout = DateLayout::from_pattern(pattern).unwrap();
            assert_eq!(layout.order, order);
            assert_eq!(layout.literals, literals);
        }
        assert!(DateLayout::from_pattern("").is_none());
    }
}
