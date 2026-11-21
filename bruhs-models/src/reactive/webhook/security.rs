use serde::{Deserialize, Serialize};

/// Public verification settings. Only the owner save command carries a signing key.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebhookSecurity {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<WebhookSignature>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_delivery_age_minutes: Option<u16>,
}

impl WebhookSecurity {
    pub fn is_empty(&self) -> bool {
        self.signature.is_none() && self.max_delivery_age_minutes.is_none()
    }

    pub fn validate(&self) -> Result<(), String> {
        if self
            .max_delivery_age_minutes
            .is_some_and(|age| !(1..=1440).contains(&age))
        {
            return Err("Enter whole minutes from 1 to 1440, or leave the age limit blank.".into());
        }
        if let Some(sig) = &self.signature {
            let header = sig.header.to_ascii_lowercase();
            if header.is_empty()
                || header.len() > 256
                || !header
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&c))
                || matches!(
                    header.as_str(),
                    "authorization"
                        | "proxy-authorization"
                        | "cookie"
                        | "x-event-id"
                        | "x-github-delivery"
                )
            {
                return Err("Enter a valid HTTP signature header separate from credential and delivery ID headers.".into());
            }
            if sig.prefix.len() > 256 || !sig.prefix.bytes().all(|c| (32..127).contains(&c)) {
                return Err(
                    "Use a signature prefix of up to 256 printable ASCII characters.".into(),
                );
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebhookSignature {
    pub header: String,
    pub encoding: WebhookSignatureEncoding,
    pub prefix: String,
}

impl Default for WebhookSignature {
    fn default() -> Self {
        Self {
            header: "x-hub-signature-256".into(),
            encoding: WebhookSignatureEncoding::Hex,
            prefix: "sha256=".into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WebhookSignatureEncoding {
    Hex,
    Base64,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WebhookSigningSecret(pub String);

impl WebhookSigningSecret {
    pub fn validate(&self) -> Result<(), String> {
        if self.0.trim().is_empty() || self.0.len() > 8192 {
            return Err("Enter a shared signing secret of up to 8192 bytes.".into());
        }
        Ok(())
    }
}

impl std::fmt::Debug for WebhookSigningSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[redacted]")
    }
}
