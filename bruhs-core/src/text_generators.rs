//! Named short-text endpoints shared by the local desktop and the delivery host.
mod http;
mod storage;
pub use storage::secret_source_consumers;
pub(super) use storage::unset_secret_source;

use crate::{secret_sources::SecretReference, thread_title::NamingImage};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeroize::Zeroizing;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    #[default]
    OpenAi,
    OpenRouter,
    Anthropic,
    Custom,
}

impl Preset {
    pub const ALL: [Self; 4] = [
        Self::OpenAi,
        Self::OpenRouter,
        Self::Anthropic,
        Self::Custom,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::OpenAi => "OpenAI",
            Self::OpenRouter => "OpenRouter",
            Self::Anthropic => "Anthropic",
            Self::Custom => "Custom",
        }
    }
    pub fn url(self) -> &'static str {
        match self {
            Self::OpenAi => "https://api.openai.com/v1",
            Self::OpenRouter => "https://openrouter.ai/api/v1",
            Self::Anthropic => "https://api.anthropic.com/v1",
            Self::Custom => "",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Model {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Generator {
    pub id: Uuid,
    pub revision: i64,
    pub preset: Preset,
    pub label: String,
    pub url: String,
    pub enabled: bool,
    pub models: Vec<Model>,
    key: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<SecretReference>,
}
impl Generator {
    pub fn has_key(&self) -> bool {
        self.key.is_some() || self.reference.is_some()
    }
    async fn credential(&self) -> Result<Option<Zeroizing<String>>, String> {
        match &self.reference {
            Some(reference) => reference.read().await.map(Some).map_err(|e| e.to_string()),
            None => storage::read_key(self.key).await,
        }
    }
    pub fn offers(&self, model: &str) -> bool {
        self.enabled && self.models.iter().any(|m| m.id == model)
    }
    pub fn selection(&self, model: &str) -> String {
        format!("generator:{}/{model}", self.id)
    }
}
pub fn split_selection(selection: &str) -> Option<(Uuid, &str)> {
    let (id, model) = selection.strip_prefix("generator:")?.split_once('/')?;
    (!model.is_empty()).then_some((Uuid::parse_str(id).ok()?, model))
}

pub enum CredentialEdit {
    Keep,
    Value(Zeroizing<String>),
    Reference(SecretReference),
}
pub struct Draft {
    pub preset: Preset,
    pub label: String,
    pub url: String,
    pub credential: CredentialEdit,
}
pub async fn list() -> Result<Vec<Generator>, String> {
    storage::blocking(storage::list).await
}
pub async fn connect(previous: Option<Generator>, draft: Draft) -> Result<Generator, String> {
    let url = http::base_url(&draft.url)?;
    let mut row = previous.clone().unwrap_or_else(|| Generator {
        id: Uuid::new_v4(),
        revision: 0,
        preset: draft.preset,
        label: String::new(),
        url: String::new(),
        enabled: true,
        models: Vec::new(),
        key: None,
        reference: None,
    });
    if row.has_key() && row.url != url && matches!(&draft.credential, CredentialEdit::Keep) {
        return Err("Enter the API key or reference for the new endpoint.".into());
    }
    let (key, replacement) = match draft.credential {
        CredentialEdit::Keep => (row.credential().await?, None),
        CredentialEdit::Reference(reference) => {
            let key = reference.read().await.map_err(|e| e.to_string())?;
            row.reference = Some(reference);
            (Some(key), None)
        }
        CredentialEdit::Value(value) => {
            row.reference = None;
            ((!value.is_empty()).then(|| value.clone()), Some(value))
        }
    };
    if draft.preset != Preset::Custom && key.as_ref().is_none_or(|v| v.trim().is_empty()) {
        return Err("Enter an API key.".into());
    }
    row.preset = draft.preset;
    row.url = url;
    row.label = draft.label.trim().to_owned();
    if row.label.is_empty() {
        row.label = if row.preset == Preset::Custom {
            reqwest::Url::parse(&row.url)
                .ok()
                .and_then(|u| u.host_str().map(str::to_owned))
                .unwrap_or_else(|| "Custom".into())
        } else {
            row.preset.label().into()
        };
    }
    row.models = http::models(&row, key.as_ref().map(|s| s.as_str())).await?;
    storage::blocking(move || storage::save(previous.as_ref(), row, replacement)).await
}
pub async fn refresh(mut row: Generator) -> Result<Generator, String> {
    let previous = row.clone();
    let key = row.credential().await?;
    row.models = http::models(&row, key.as_ref().map(|s| s.as_str())).await?;
    storage::blocking(move || storage::save(Some(&previous), row, None)).await
}
pub async fn enable(mut row: Generator, enabled: bool) -> Result<Generator, String> {
    let previous = row.clone();
    row.enabled = enabled;
    storage::blocking(move || storage::save(Some(&previous), row, None)).await
}
pub async fn remove(row: Generator) -> Result<(), String> {
    storage::blocking(move || storage::remove(&row)).await
}
pub async fn test(row: Generator, model: String) -> Result<(), String> {
    generate(
        &row.selection(&model),
        "Return the word OK.",
        "Connection test",
        &[],
    )
    .await
    .map(|_| ())
}
pub(crate) fn active_blocking(selection: &str) -> Result<bool, String> {
    let Some((id, slug)) = split_selection(selection) else {
        return Ok(false);
    };
    Ok(storage::get(id)?.is_some_and(|row| row.offers(slug)))
}
pub async fn active(selection: &str) -> Result<bool, String> {
    let selected = selection.to_owned();
    storage::blocking(move || active_blocking(&selected)).await
}
pub(crate) async fn generate(
    selection: &str,
    instructions: &str,
    prompt: &str,
    images: &[NamingImage],
) -> Result<String, String> {
    let (id, model) = split_selection(selection).ok_or("Choose a text generator model.")?;
    let row = storage::blocking(move || storage::get(id))
        .await?
        .filter(|g| g.offers(model))
        .ok_or("This text generator is unavailable. Choose another model in Settings.")?;
    let key = row.credential().await?;
    http::generate(
        &row,
        key.as_ref().map(|s| s.as_str()),
        model,
        instructions,
        prompt,
        images,
    )
    .await
}
