//! Request-key fields shared by the task editor and the automation form.
use super::*;

pub(super) struct DedupeFields {
    pub(super) replay: Entity<InputState>,
    pub(super) coalesce: Entity<InputState>,
    pub(super) seconds: Entity<InputState>,
}

impl DedupeFields {
    pub(super) fn new(saved: &WebhookDedupe, window: &mut Window, cx: &mut App) -> Self {
        let mut input = |value: &str, hint: &'static str| {
            cx.new(|cx| {
                let mut input = InputState::new(window, cx).placeholder(hint);
                input.set_value(value, window, cx);
                input
            })
        };
        Self {
            replay: input(&saved.replay_key, "{{headers.delivery-id}}"),
            coalesce: input(&saved.coalesce_key, "{{body.id}}"),
            seconds: input(
                &saved
                    .coalesce_window_seconds
                    .map(|n| n.to_string())
                    .unwrap_or_default(),
                "120",
            ),
        }
    }

    pub(super) fn build(&self, cx: &App) -> Result<WebhookDedupe, String> {
        let text = value(&self.seconds, cx);
        let seconds = text.trim();
        let fields = WebhookDedupe {
            replay_key: value(&self.replay, cx).trim().into(),
            coalesce_key: value(&self.coalesce, cx).trim().into(),
            coalesce_window_seconds: (!seconds.is_empty())
                .then(|| seconds.parse::<u32>())
                .transpose()
                .map_err(|_| "Coalesce window must be whole seconds from 10 to 3600.")?,
        };
        fields.validate()?;
        Ok(fields)
    }

    pub(super) fn render(&self, disabled: bool) -> Div {
        div().flex().flex_col().gap_2()
            .child(field("Duplicate request key",&self.replay,false,disabled))
            .child(muted("Requests with the same key run once within seven days."))
            .child(field("Coalesce key",&self.coalesce,false,disabled))
            .child(field("Coalesce window (seconds)",&self.seconds,false,disabled))
            .child(muted("Run the latest matching request when the first request’s window closes. Choose 10–3600 seconds. Requests with an empty key run normally."))
    }
}
