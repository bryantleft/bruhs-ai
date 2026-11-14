use super::*;

#[derive(Clone, Copy, Debug)]
pub(super) enum Case {
    GetPending,
    GetSucceeded,
    GetFailed,
    GetCancelled,
    GetMissing,
    GetForeign,
    PendingSome,
    PendingNone,
    PendingMissing,
    ReplaySucceeded,
    ReplayPending,
    ReplayFailed,
    ReplayCancelled,
    ReplayMismatch,
    ReplayInvalid,
    ReplayMissing,
    CancelPending,
    CancelSucceeded,
    CancelFailed,
    CancelMissing,
    FenceFresh,
    FencePending,
    FenceSucceeded,
    FenceRetry,
    FenceMismatch,
    FenceInvalid,
    FenceMissing,
}

impl Case {
    pub(super) const ALL: [Self; 27] = [
        Self::GetPending,
        Self::GetSucceeded,
        Self::GetFailed,
        Self::GetCancelled,
        Self::GetMissing,
        Self::GetForeign,
        Self::PendingSome,
        Self::PendingNone,
        Self::PendingMissing,
        Self::ReplaySucceeded,
        Self::ReplayPending,
        Self::ReplayFailed,
        Self::ReplayCancelled,
        Self::ReplayMismatch,
        Self::ReplayInvalid,
        Self::ReplayMissing,
        Self::CancelPending,
        Self::CancelSucceeded,
        Self::CancelFailed,
        Self::CancelMissing,
        Self::FenceFresh,
        Self::FencePending,
        Self::FenceSucceeded,
        Self::FenceRetry,
        Self::FenceMismatch,
        Self::FenceInvalid,
        Self::FenceMissing,
    ];

    pub(super) fn name(self) -> String {
        format!("{self:?}")
    }

    pub(super) fn fresh(self) -> bool {
        matches!(
            self,
            Self::CancelPending | Self::FenceFresh | Self::FencePending
        )
    }

    pub(super) fn seed_state(self) -> Option<SummaryGenerationState> {
        use SummaryGenerationState as S;
        match self {
            Self::GetPending | Self::PendingSome | Self::ReplayPending => Some(S::Pending),
            Self::GetSucceeded
            | Self::ReplaySucceeded
            | Self::CancelSucceeded
            | Self::FenceSucceeded => Some(S::Succeeded {
                summary: model::summary(false),
            }),
            Self::GetFailed | Self::ReplayFailed | Self::CancelFailed => Some(S::Failed),
            Self::GetCancelled
            | Self::GetForeign
            | Self::ReplayCancelled
            | Self::ReplayMismatch
            | Self::FenceRetry
            | Self::FenceMismatch => Some(S::Cancelled),
            _ => None,
        }
    }

    pub(super) fn identity(self, index: usize) -> (String, String) {
        let meeting = self.name();
        let request = if self.fresh() {
            format!("{meeting}-{index:06}")
        } else {
            format!("{meeting}-receipt")
        };
        (meeting, request)
    }

    pub(super) fn action(self) -> &'static str {
        match self {
            Self::GetPending
            | Self::GetSucceeded
            | Self::GetFailed
            | Self::GetCancelled
            | Self::GetMissing
            | Self::GetForeign => "GET /api/meetings/{id}/generations/{request_id}",
            Self::PendingSome | Self::PendingNone | Self::PendingMissing => {
                "GET /api/meetings/{id}/generations"
            }
            Self::ReplaySucceeded
            | Self::ReplayPending
            | Self::ReplayFailed
            | Self::ReplayCancelled
            | Self::ReplayMismatch
            | Self::ReplayInvalid
            | Self::ReplayMissing => "POST /api/meetings/{id}/summarize",
            _ => "DELETE /api/meetings/{id}/generations/{request_id}",
        }
    }

    pub(super) fn request(self, index: usize) -> (Method, String, Option<Value>) {
        let (mut meeting, mut request) = self.identity(index);
        if matches!(
            self,
            Self::PendingMissing | Self::ReplayMissing | Self::FenceMissing
        ) {
            meeting = "missing".into();
        }
        if matches!(self, Self::GetForeign) {
            meeting = "background".into();
        }
        if matches!(self, Self::ReplayInvalid) {
            request = " ".into();
        }
        let body = match self {
            Self::ReplaySucceeded
            | Self::ReplayPending
            | Self::ReplayFailed
            | Self::ReplayCancelled
            | Self::ReplayInvalid
            | Self::ReplayMissing => Some(json!({"request_id":request})),
            Self::ReplayMismatch => {
                Some(json!({"request_id":request,"extract_observations":false}))
            }
            Self::FenceFresh
            | Self::FencePending
            | Self::FenceSucceeded
            | Self::FenceRetry
            | Self::FenceMissing => Some(json!({})),
            Self::FenceMismatch => Some(json!({"extract_observations":false})),
            Self::FenceInvalid => {
                Some(json!({"extract_observations":false,"replace_observations":true}))
            }
            _ => None,
        };
        let method = match self.action().split_once(' ').expect("HTTP identity").0 {
            "GET" => Method::GET,
            "POST" => Method::POST,
            "DELETE" => Method::DELETE,
            _ => unreachable!("finite generation methods"),
        };
        let path = if method == Method::POST {
            format!("/api/meetings/{meeting}/summarize")
        } else if matches!(
            self,
            Self::PendingSome | Self::PendingNone | Self::PendingMissing
        ) {
            format!("/api/meetings/{meeting}/generations")
        } else {
            format!("/api/meetings/{meeting}/generations/{request}")
        };
        (method, path, body)
    }

    pub(super) fn error(self) -> Option<(u16, &'static str)> {
        match self {
            Self::GetMissing | Self::GetForeign | Self::CancelMissing => Some((404,"Generation request was not found.")),
            Self::PendingMissing | Self::ReplayMissing => Some((404,"Meeting was not found.")),
            Self::ReplayPending => Some((409,"Generation remains unresolved. Check its receipt; cancel it explicitly before requesting a new generation.")),
            Self::ReplayFailed | Self::ReplayCancelled => Some((409,"This generation ended without a committed result. Start a new explicit request; this request will not run again.")),
            Self::ReplayMismatch => Some((409,"Generation request identity was already used with different options.")),
            Self::ReplayInvalid => Some((400,"Invalid generation request identity.")),
            Self::FenceMismatch => Some((409,"Generation request identity was already used with different options.")),
            Self::FenceInvalid => Some((400,"Invalid generation request identity or options.")),
            Self::FenceMissing => Some((404,"meeting not found: missing")),
            _ => None,
        }
    }
}
