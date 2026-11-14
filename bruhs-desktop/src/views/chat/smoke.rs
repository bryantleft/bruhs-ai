//! Hooks for `BRUHS_DESKTOP_COMMANDS_SMOKE` and
//! `BRUHS_DESKTOP_THREADS_SMOKE` (see `src/smoke.rs`). Debug
//! builds only: a release build has no way to auto-submit prompts.

use super::*;

impl ChatView {
    pub(crate) fn smoke_seed_rich_response(&mut self, cx: &mut Context<'_, Self>) {
        self.thread_title = "Inline interactive response".into();
        let mut message = Message::assistant_streaming(1, None);
        message.content = r#"```bruhs-html Interactive counter
<style>body{background:#141210;color:#dfcfc3}button{position:fixed;left:50%;top:50%;transform:translate(-50%,-50%);font:inherit;padding:12px 24px;border:1px solid #b4befe;border-radius:8px;background:#2f2922;color:inherit}</style>
<p>Inline interaction</p><button id="counter">0</button>
<script>bruhs.ready.then(saved=>{const button=document.getElementById('counter');button.textContent=String(saved?.count || 0);button.onclick=()=>{button.textContent=String(Number(button.textContent)+1);bruhs.setState({count:Number(button.textContent)})};button.click();let parentBlocked=false;try{parent.document.body}catch{parentBlocked=true}parent.postMessage({tag:'bruhs-rich-response-smoke',count:button.textContent,parentBlocked},'*')})</script>
```"#.into();
        message.state = TurnState::Finished;
        self.messages = vec![message];
        self.is_streaming = false;
        self.scroll_messages_to_bottom();
        cx.emit(ChatViewEvent::MessagesChanged);
        cx.notify();
    }

    /// Put `text` in the composer and press Enter, through the same path a
    /// key press takes.
    pub(crate) fn smoke_enter(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.write_composer(text, window, cx);
        self.send_follow_up(false, window, cx);
    }

    /// The composer's text.
    pub(crate) fn smoke_composer(&self, cx: &App) -> String {
        self.input_state.read(cx).value().to_string()
    }

    /// The model this thread runs on, and the name of its role if it has one.
    pub(crate) fn smoke_model_and_role(&self) -> (String, Option<String>) {
        (
            self.model_id.clone(),
            self.role_profile.as_ref().map(|role| role.name.clone()),
        )
    }

    /// The latest prompt's text.
    pub(crate) fn smoke_last_prompt(&self) -> String {
        self.messages
            .iter()
            .rev()
            .find(|message| matches!(message.role, MessageRole::User))
            .map(|message| message.content.clone())
            .unwrap_or_default()
    }

    /// The last assistant reply's text.
    pub(crate) fn smoke_last_reply(&self) -> String {
        self.messages
            .iter()
            .rev()
            .find(|message| matches!(message.role, MessageRole::Assistant))
            .map(|message| message.content.clone())
            .unwrap_or_default()
    }
}
