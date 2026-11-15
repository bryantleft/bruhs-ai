use bruhs_models::mail::MailHeader;
use serde::Deserialize;

#[derive(Deserialize)]
pub(super) struct Page<T> {
    pub value: Vec<T>,
    #[serde(rename = "@odata.nextLink")]
    pub next: Option<String>,
    #[serde(rename = "@odata.deltaLink")]
    pub delta: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Profile {
    pub id: String,
    pub mail: Option<String>,
    pub user_principal_name: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct Id {
    pub id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Folder {
    pub id: String,
    pub child_folder_count: u64,
    #[serde(rename = "@odata.type")]
    pub kind: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Change {
    pub id: String,
    pub conversation_id: Option<String>,
    #[serde(rename = "@removed")]
    pub removed: Option<serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Recipient {
    pub email_address: Address,
}

#[derive(Deserialize)]
pub(super) struct Address {
    pub name: Option<String>,
    pub address: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Body {
    pub content_type: String,
    pub content: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Message {
    pub id: String,
    pub conversation_id: String,
    pub parent_folder_id: String,
    pub subject: String,
    pub body: Body,
    pub from: Option<Recipient>,
    pub sender: Option<Recipient>,
    pub to_recipients: Vec<Recipient>,
    pub cc_recipients: Vec<Recipient>,
    pub bcc_recipients: Vec<Recipient>,
    pub reply_to: Vec<Recipient>,
    pub internet_message_id: Option<String>,
    pub internet_message_headers: Vec<MailHeader>,
    pub received_date_time: Option<String>,
    pub sent_date_time: Option<String>,
    pub created_date_time: String,
    pub is_read: bool,
    pub is_draft: bool,
    pub importance: String,
    pub categories: Vec<String>,
    // Always expanded with metadata-only $select, including inline-only mail.
    pub attachments: Vec<Attachment>,
    #[serde(rename = "attachments@odata.nextLink")]
    pub attachments_next: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Attachment {
    pub id: String,
    #[serde(rename = "@odata.type")]
    pub kind: String,
    pub name: String,
    pub content_type: Option<String>,
    pub size: u64,
    pub is_inline: bool,
    pub content_id: Option<String>,
}
