use bruhs_models::logic::tools::ToolState;
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Visibility {
    #[default]
    ShowAll,
    HideSuccessful,
    HideAll,
    Zen,
}
impl Visibility {
    pub fn key(self) -> &'static str {
        match self {
            Self::ShowAll => "all",
            Self::HideSuccessful => "failures",
            Self::HideAll => "hide",
            Self::Zen => "zen",
        }
    }
    pub fn parse(key: &str) -> Option<Self> {
        Some(match key {
            "all" => Self::ShowAll,
            "failures" => Self::HideSuccessful,
            "hide" => Self::HideAll,
            "zen" => Self::Zen,
            _ => return None,
        })
    }
    pub(super) fn shows(self, state: ToolState) -> bool {
        match state {
            ToolState::Error => true,
            ToolState::Done => self == Self::ShowAll,
            _ => matches!(self, Self::ShowAll | Self::HideSuccessful),
        }
    }
    pub(super) fn answer(self, streaming: bool, text: &str, final_offset: Option<usize>) -> &str {
        if self != Self::Zen {
            return text;
        }
        if streaming {
            return "";
        }
        final_offset.and_then(|o| text.get(o..)).unwrap_or(text)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn zen_retains_errors_and_the_final_answer_with_older_transcript_fallback() {
        assert!(Visibility::Zen.shows(ToolState::Error));
        assert!(!Visibility::HideSuccessful.shows(ToolState::Done));
        assert!(Visibility::HideSuccessful.shows(ToolState::Running));
        assert_eq!(Visibility::Zen.answer(true, "before", None), "");
        assert_eq!(Visibility::Zen.answer(false, "before後", Some(6)), "後");
        assert_eq!(
            Visibility::Zen.answer(false, "older transcript", None),
            "older transcript"
        );
    }
}
