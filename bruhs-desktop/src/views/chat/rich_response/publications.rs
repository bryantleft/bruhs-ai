use super::*;
use bruhs_models::logic::rich_response::{response_blocks, ArtifactBlock};

fn artifact_blocks(text: &str) -> Vec<ArtifactBlock> {
    let mut blocks = response_blocks(text, false)
        .into_iter()
        .filter_map(|block| match block {
            ResponseBlock::Artifact(artifact) => Some(artifact),
            _ => None,
        })
        .collect::<Vec<_>>();
    if blocks.is_empty() {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(text) {
            if let Some(content) = value.get("content").and_then(serde_json::Value::as_array) {
                for content in content {
                    if let Some(text) = content.get("text").and_then(serde_json::Value::as_str) {
                        blocks.extend(artifact_blocks(text));
                    }
                }
            }
        }
    }
    blocks
}

fn publications(message: &Message) -> Vec<ArtifactBlock> {
    let mut seen = std::collections::BTreeSet::new();
    message
        .tool_calls
        .iter()
        .filter(|call| !call.is_error && call.tool.ends_with("publish_artifact"))
        .filter_map(|call| call.output.as_deref())
        .flat_map(artifact_blocks)
        .filter(|artifact| seen.insert(artifact.id.clone()))
        .collect()
}

pub(super) fn export_markdown(message: &Message) -> String {
    let mut markdown = message.content.clone();
    let echoed = artifact_blocks(&markdown)
        .into_iter()
        .map(|artifact| artifact.id)
        .collect::<std::collections::BTreeSet<_>>();
    for artifact in publications(message) {
        if !echoed.contains(&artifact.id) {
            if let Ok(json) = serde_json::to_string(&artifact) {
                markdown.push_str(&format!("\n\n```bruhs-artifact\n{json}\n```"));
            }
        }
    }
    markdown
}

impl ChatView {
    pub(in crate::views::chat) fn has_published_artifacts(&self, reply: &Message) -> bool {
        !publications(reply).is_empty()
    }
    pub(in crate::views::chat) fn render_published_artifacts(
        &self,
        message: usize,
        reply: &Message,
        cx: &Context<'_, Self>,
    ) -> Vec<AnyElement> {
        let echoed = artifact_blocks(&reply.content)
            .into_iter()
            .map(|artifact| artifact.id)
            .collect::<std::collections::BTreeSet<_>>();
        publications(reply)
            .into_iter()
            .filter(|artifact| !echoed.contains(&artifact.id))
            .enumerate()
            .map(|(index, artifact)| {
                let identity: SharedString = artifact.id.clone().into();
                self.render_rich_response(
                    message,
                    usize::MAX.saturating_sub(index),
                    &Rc::new(ResponseBlock::Artifact(artifact)),
                    &identity,
                    cx,
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{artifact_blocks, export_markdown};
    use crate::views::chat::{DesktopToolCall, Message, TurnState};

    #[test]
    fn publisher_results_are_previewed_before_final_text_and_exported_once() {
        let output = "```bruhs-artifact\n{\"id\":\"00000000-0000-4000-8000-000000000000\",\"title\":\"Counter\"}\n```";
        let wrapped = serde_json::json!({"content":[{"type":"text","text":output}]}).to_string();
        assert_eq!(artifact_blocks(&wrapped).len(), 1);
        let mut message = Message::assistant_streaming(1, None);
        message.state = TurnState::Finished;
        message.tool_calls.push(DesktopToolCall {
            text_offset: None,
            id: "tool".into(),
            tool: "mcp__bruhs_artifacts__publish_artifact".into(),
            input: serde_json::Value::Null,
            output: Some(wrapped),
            is_error: false,
        });
        assert_eq!(artifact_blocks(&export_markdown(&message)).len(), 1);
        message.content = output.into();
        assert_eq!(artifact_blocks(&export_markdown(&message)).len(), 1);
    }
}
