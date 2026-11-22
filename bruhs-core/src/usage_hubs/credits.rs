//! Codex reset credits reported by a particular account on a saved hub.
use super::{
    normalize_url,
    report::{account_call, accounts, client, codex_headers, management_response},
    Hub,
};
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{collections::BTreeMap, io};
use url::Url;
use uuid::Uuid;

const CREDITS: &str = "https://chatgpt.com/backend-api/wham/rate-limit-reset-credits";
const CONSUME: &str = "https://chatgpt.com/backend-api/wham/rate-limit-reset-credits/consume";
// CLIProxyAPI clients use the same UUIDv5 for retries from any device.
const NAMESPACE: Uuid = Uuid::from_u128(0x6f1c2a9e_2d4b_4c1e_9a7f_3b8d5e0c1a42);

pub struct ResetCredits {
    pub count: usize,
    pub next: Option<ResetCredit>,
}
#[derive(Clone)]
pub struct ResetCredit {
    pub expires_at: DateTime<Utc>,
    id: String,
    account: String,
    identity: String,
    endpoint: Url,
}
impl ResetCredit {
    /// Stable within one saved hub revision, provider account and redeemable credit.
    pub fn selection_id(&self, hub: &Hub) -> Uuid {
        let account = Uuid::new_v5(&hub.revision, self.account.as_bytes());
        let identity = Uuid::new_v5(&account, self.identity.as_bytes());
        Uuid::new_v5(&identity, self.id.as_bytes())
    }
}
#[derive(Deserialize)]
struct ListedCredit {
    id: String,
    status: String,
    reset_type: String,
    expires_at: String,
}
fn identity(file: &Value) -> Option<&str> {
    file["id_token"]["chatgpt_account_id"]
        .as_str()
        .filter(|id| !id.trim().is_empty())
        .or_else(|| file["id"].as_str())
}
pub(super) async fn available(
    client: &Client,
    endpoint: &Url,
    key: &str,
    file: &Value,
) -> io::Result<ResetCredits> {
    let data = account_call(
        client,
        endpoint,
        key,
        file,
        CREDITS,
        codex_headers(file),
        None,
    )
    .await?;
    #[derive(Deserialize)]
    struct Bank {
        credits: Vec<ListedCredit>,
    }
    let bank: Bank = serde_json::from_value(data)
        .map_err(|_| io::Error::other("The provider returned an invalid credit list."))?;
    let now = Utc::now();
    let mut eligible = BTreeMap::new();
    for entry in bank.credits {
        if entry.status != "available"
            || entry.reset_type != "codex_rate_limits"
            || entry.id.is_empty()
        {
            continue;
        }
        if let Ok(expiry) = entry.expires_at.parse::<DateTime<Utc>>() {
            if expiry > now {
                eligible
                    .entry(entry.id)
                    .and_modify(|old: &mut DateTime<Utc>| *old = (*old).min(expiry))
                    .or_insert(expiry);
            }
        }
    }
    let count = eligible.len();
    let next = eligible
        .into_iter()
        .min_by_key(|(_, expiry)| *expiry)
        .and_then(|(id, expires_at)| {
            Some(ResetCredit {
                id,
                expires_at,
                account: file["id"].as_str()?.into(),
                identity: identity(file)?.into(),
                endpoint: endpoint.clone(),
            })
        });
    Ok(ResetCredits { count, next })
}

pub struct Redemption {
    pub message: &'static str,
    pub cooldown_warning: bool,
}
pub async fn redeem(hub: &Hub, credit: &ResetCredit) -> io::Result<Redemption> {
    let key = hub.checked_key().await?;
    let management = normalize_url(&hub.url)?
        .join("v0/management/")
        .map_err(io::Error::other)?;
    redeem_at(&client()?, &management, &key, credit).await
}
async fn redeem_at(
    client: &Client,
    management: &Url,
    key: &str,
    credit: &ResetCredit,
) -> io::Result<Redemption> {
    let endpoint = management.join("api-call").map_err(io::Error::other)?;
    if endpoint != credit.endpoint {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "This credit belongs to another hub.",
        ));
    }
    let files = accounts(client, management, key).await?;
    let file = files
        .iter()
        .find(|f| {
            f["id"] == credit.account
                && f["provider"] == "codex"
                && f["disabled"] != true
                && identity(f) == Some(credit.identity.as_str())
        })
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "The selected Codex account changed or is disabled. Refresh its usage.",
            )
        })?;
    let id = Uuid::new_v5(
        &NAMESPACE,
        format!("{}:{}", credit.identity, credit.id).as_bytes(),
    );
    let result = account_call(
        client,
        &endpoint,
        key,
        file,
        CONSUME,
        codex_headers(file),
        Some(json!({"credit_id":credit.id,"redeem_request_id":id})),
    )
    .await
    .map_err(|_| {
        io::Error::other("Could not confirm the result. Retry to check the same credit.")
    })?;
    let (message, clear) =
        match result["code"].as_str() {
            Some("reset") => ("Reset applied. Your windows have cleared.", true),
            Some("already_redeemed") => ("That credit was already redeemed.", true),
            Some("nothing_to_reset") => ("Nothing to reset right now.", false),
            Some("no_credit") => ("No reset credit left.", false),
            _ => return Err(io::Error::other(
                "The provider returned an unknown reset result. Retry to check the same credit.",
            )),
        };
    let cooldown_warning = if clear {
        let url = management.join("reset-quota").map_err(io::Error::other)?;
        management_response(
            client
                .post(url)
                .bearer_auth(key)
                .json(&json!({"auth_index":file["auth_index"]})),
        )
        .await
        .is_err()
    } else {
        false
    };
    Ok(Redemption {
        message,
        cooldown_warning,
    })
}

#[cfg(test)]
mod tests;
