use bruhs_models::mail::MailHeader;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Profile {
    pub email_address: String,
    pub history_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ThreadList {
    #[serde(default)]
    pub threads: Vec<ThreadId>,
    pub next_page_token: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct ThreadId {
    pub id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Thread {
    pub id: String,
    #[serde(default)]
    pub messages: Vec<Message>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Message {
    pub id: String,
    pub thread_id: String,
    #[serde(default)]
    pub label_ids: Vec<String>,
    pub internal_date: String,
    pub payload: Part,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(super) struct Part {
    #[serde(default)]
    pub part_id: String,
    #[serde(default)]
    pub mime_type: String,
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub headers: Vec<MailHeader>,
    #[serde(default)]
    pub body: Body,
    #[serde(default)]
    pub parts: Vec<Part>,
}

#[derive(Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(super) struct Body {
    #[serde(default)]
    pub size: u64,
    pub data: Option<String>,
    pub attachment_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct HistoryList {
    #[serde(default)]
    pub history: Vec<History>,
    pub next_page_token: Option<String>,
    pub history_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct History {
    #[serde(default)]
    pub messages: Vec<HistoryMessage>,
    #[serde(default)]
    pub messages_added: Vec<HistoryChange>,
    #[serde(default)]
    pub messages_deleted: Vec<HistoryChange>,
    #[serde(default)]
    pub labels_added: Vec<HistoryChange>,
    #[serde(default)]
    pub labels_removed: Vec<HistoryChange>,
}

#[derive(Deserialize)]
pub(super) struct HistoryChange {
    pub message: HistoryMessage,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct HistoryMessage {
    pub thread_id: String,
}
