//! Shortcut releases belong to the recording that started on key down.
use super::*;
use crate::voice_capture::ShortcutMode;
use std::time::Instant;

pub(super) struct Press {
    chat: gpui::WeakEntity<ChatView>,
    generation: Option<u64>,
    mode: ShortcutMode,
    started: Instant,
    key: String,
    modifiers: gpui::Modifiers,
}

impl Workspace {
    pub(super) fn press_voice_shortcut(
        &mut self,
        key: &gpui::Keystroke,
        cx: &mut Context<'_, Self>,
    ) {
        if self.voice_press.is_some() {
            return;
        }
        let Some(chat) = self.active_thread().map(|thread| thread.chat.clone()) else {
            return;
        };
        let generation = chat.update(cx, |chat, cx| chat.begin_dictation_shortcut(cx));
        self.voice_press = Some(Press {
            chat: chat.downgrade(),
            generation,
            mode: self.dictation_prefs.shortcut_mode,
            started: Instant::now(),
            key: key.key.clone(),
            modifiers: key.modifiers,
        });
    }
    pub(super) fn release_voice_shortcut(&mut self, cx: &mut Context<'_, Self>) {
        let Some(press) = self.voice_press.take() else {
            return;
        };
        if !press.mode.stops_on_release(press.started.elapsed()) {
            return;
        }
        if let (Some(chat), Some(generation)) = (press.chat.upgrade(), press.generation) {
            chat.update(cx, |chat, cx| {
                chat.finish_dictation_shortcut(generation, cx)
            });
        }
    }
    pub(super) fn voice_shortcut_key_up(
        &mut self,
        event: &gpui::KeyUpEvent,
        cx: &mut Context<'_, Self>,
    ) {
        if self
            .voice_press
            .as_ref()
            .is_some_and(|press| press.key == event.keystroke.key)
        {
            self.release_voice_shortcut(cx);
        }
    }
    pub(super) fn voice_shortcut_modifiers(
        &mut self,
        event: &gpui::ModifiersChangedEvent,
        cx: &mut Context<'_, Self>,
    ) {
        if self.voice_press.as_ref().is_some_and(|press| {
            let was = press.modifiers;
            let now = event.modifiers;
            (was.platform && !now.platform)
                || (was.control && !now.control)
                || (was.alt && !now.alt)
                || (was.shift && !now.shift)
                || (was.function && !now.function)
        }) {
            self.release_voice_shortcut(cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ShortcutMode;
    use std::time::Duration;
    #[test]
    fn dictation_taps_and_holds_match_the_shortcut_mode() {
        for (mode, before, after) in [
            (ShortcutMode::Auto, false, true),
            (ShortcutMode::Hold, true, true),
            (ShortcutMode::Toggle, false, false),
        ] {
            assert_eq!(mode.stops_on_release(Duration::from_millis(299)), before);
            assert_eq!(mode.stops_on_release(Duration::from_millis(300)), after);
        }
    }
}
