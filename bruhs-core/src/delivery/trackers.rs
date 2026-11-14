//! Read-only issue intake; credentials remain inside the SecretStore request boundary.
use super::{DeliveryError, DeliveryResult, DeliveryService};
use bruhs_models::delivery::*;
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    path::{Path, PathBuf},
};
use zeroize::Zeroizing;

pub(crate) fn credential_name(config: &TrackerConfig) -> String {
    let provider = match config.provider {
        TrackerProvider::Linear => "Linear",
        TrackerProvider::Jira => "Jira",
    };
    // Changing the destination or identity never reuses a credential approved for another account.
    format!(
        "{provider} issue intake {} {} {}",
        config.origin,
        config.email,
        if config.oauth { "OAuth" } else { "API token" }
    )
}

fn validated_config(mut config: TrackerConfig) -> DeliveryResult<TrackerConfig> {
    let url = url::Url::parse(config.origin.trim()).map_err(|_| {
        DeliveryError::Invalid(
            "Enter the tracker's HTTPS origin, such as https://example.atlassian.net.".into(),
        )
    })?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.host_str().is_none()
        || url.path().trim_matches('/').contains("..")
    {
        return Err(DeliveryError::Invalid(
            "The tracker origin must use HTTPS without credentials, query parameters or fragments."
                .into(),
        ));
    }
    config.origin = url.as_str().trim_end_matches('/').to_owned();
    config.email = config.email.trim().to_owned();
    if config.email.contains(['\0', '\r', '\n']) {
        return Err(DeliveryError::Invalid(
            "Enter an account email without control characters.".into(),
        ));
    }
    if config.provider == TrackerProvider::Linear {
        if config.origin != "https://api.linear.app" {
            return Err(DeliveryError::Invalid("Linear issue intake uses https://api.linear.app. Issue links may use linear.app; credentials are sent only to the API origin.".into()));
        }
        config.email.clear();
    } else if !config.oauth && (config.email.is_empty() || !config.email.contains('@')) {
        return Err(DeliveryError::Invalid("Jira API-token sign-in requires the account email. Select OAuth only for a bearer access token.".into()));
    }
    Ok(config)
}

pub(crate) fn issue_reference(config: &TrackerConfig, reference: &str) -> DeliveryResult<String> {
    let reference = reference.trim();
    let key = if reference.contains("://") {
        let url = url::Url::parse(reference)
            .map_err(|_| DeliveryError::Invalid("Enter a valid issue URL or identifier.".into()))?;
        let origin = url::Url::parse(&config.origin)
            .map_err(|_| DeliveryError::Invalid("Save tracker settings again.".into()))?;
        if url.scheme() != "https"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(DeliveryError::Invalid(
                "Use a credential-free HTTPS issue link without a query or fragment.".into(),
            ));
        }
        match config.provider {
            TrackerProvider::Linear if url.host_str() == Some("linear.app") => {
                let pieces = url.path_segments().ok_or_else(|| DeliveryError::Invalid("This Linear issue link has no identifier.".into()))?.collect::<Vec<_>>();
                let position = pieces.iter().position(|part| *part == "issue").and_then(|index| pieces.get(index + 1)).ok_or_else(|| DeliveryError::Invalid("Use a Linear /issue/TEAM-123 link.".into()))?;
                (*position).to_owned()
            }
            TrackerProvider::Jira if url.origin() == origin.origin() => {
                let prefix = format!("{}/browse/", origin.path().trim_end_matches('/'));
                url.path().strip_prefix(&prefix).filter(|key| !key.contains('/')).ok_or_else(|| DeliveryError::Invalid("Use a /browse/PROJECT-123 link from the configured Jira site.".into()))?.to_owned()
            }
            _ => return Err(DeliveryError::Invalid("This issue link does not belong to the selected tracker. Its host will not receive saved credentials.".into())),
        }
    } else {
        reference.to_owned()
    };
    let Some((prefix, number)) = key.rsplit_once('-') else {
        return Err(DeliveryError::Invalid(
            "Enter an issue identifier such as ENG-123.".into(),
        ));
    };
    if prefix.is_empty()
        || !prefix
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
        || number.parse::<u64>().ok().is_none_or(|number| number == 0)
    {
        return Err(DeliveryError::Invalid(
            "Enter an uppercase issue identifier such as ENG-123.".into(),
        ));
    }
    Ok(key)
}

impl DeliveryService {
    fn tracker_file(&self, project: &Path) -> PathBuf {
        let hash = Sha256::digest(project.to_string_lossy().as_bytes());
        self.root.join(format!("trackers-{hash:x}.json"))
    }
    async fn tracker_configs(&self, project: &Path) -> DeliveryResult<Vec<TrackerConfig>> {
        match tokio::fs::read(self.tracker_file(project)).await {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| {
                DeliveryError::Storage(
                    "Tracker settings are unreadable. Restore their file before saving changes."
                        .into(),
                )
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(error) => Err(DeliveryError::Storage(error.to_string())),
        }
    }
    pub async fn tracker_settings(&self, path: &Path) -> DeliveryResult<Vec<TrackerSetting>> {
        let project = tokio::fs::canonicalize(path)
            .await
            .map_err(|error| DeliveryError::Invalid(error.to_string()))?;
        let configs = self.tracker_configs(&project).await?;
        let saved = self.tracker_secrets.list().await.map_err(|_| {
            DeliveryError::Storage(
                "Could not read the credential catalog. Unlock the OS keychain and retry.".into(),
            )
        })?;
        Ok(configs
            .into_iter()
            .map(|config| {
                let name = credential_name(&config);
                let credential_saved = saved.iter().any(|saved| {
                    saved.project_path == project.to_string_lossy() && saved.name == name
                });
                TrackerSetting {
                    config,
                    credential_saved,
                }
            })
            .collect())
    }
    pub async fn configure_tracker(
        &self,
        path: &Path,
        setup: TrackerSetup,
    ) -> DeliveryResult<Vec<TrackerSetting>> {
        let project = tokio::fs::canonicalize(path)
            .await
            .map_err(|error| DeliveryError::Invalid(error.to_string()))?;
        let config = validated_config(setup.config)?;
        if let Some(token) = setup.token.filter(|token| !token.is_empty()) {
            let token = Zeroizing::new(token);
            if token.contains(['\0', '\r', '\n']) {
                return Err(DeliveryError::Invalid(
                    "The credential contains invalid HTTP header characters.".into(),
                ));
            }
            self.tracker_secrets
                .save(&project.to_string_lossy(), &credential_name(&config), token)
                .await
                .map_err(|_| {
                    DeliveryError::Storage(
                        "Could not save the tracker token in the OS keychain. Unlock it and retry."
                            .into(),
                    )
                })?;
        }
        // Serialize settings changes with the delivery writer; no private token enters this file.
        let _writer = self.state.lock().await;
        let mut configs = self.tracker_configs(&project).await?;
        configs.retain(|saved| saved.provider != config.provider);
        configs.push(config);
        let destination = self.tracker_file(&project);
        let directory = self.root.clone();
        tokio::task::spawn_blocking(move || -> DeliveryResult<()> {
            let mut file = tempfile::NamedTempFile::new_in(directory.as_path())
                .map_err(|error| DeliveryError::Storage(error.to_string()))?;
            serde_json::to_writer(&mut file, &configs)
                .map_err(|error| DeliveryError::Storage(error.to_string()))?;
            file.flush()
                .and_then(|()| file.as_file().sync_all())
                .map_err(|error| DeliveryError::Storage(error.to_string()))?;
            file.persist(destination)
                .map_err(|error| DeliveryError::Storage(error.error.to_string()))?;
            #[cfg(unix)]
            std::fs::File::open(directory.as_path())
                .and_then(|file| file.sync_all())
                .map_err(|error| DeliveryError::Storage(error.to_string()))?;
            Ok(())
        })
        .await
        .map_err(|error| DeliveryError::Storage(error.to_string()))??;
        drop(_writer);
        self.tracker_settings(&project).await
    }
    pub async fn tracker_issue(
        &self,
        path: &Path,
        provider: TrackerProvider,
        reference: &str,
    ) -> DeliveryResult<Issue> {
        let project = tokio::fs::canonicalize(path)
            .await
            .map_err(|error| DeliveryError::Invalid(error.to_string()))?;
        let config = self.tracker_configs(&project).await?.into_iter().find(|config| config.provider == provider).ok_or_else(|| DeliveryError::Invalid("Save this project's tracker origin and credential in Delivery → Issue tracker before reading an issue.".into()))?;
        let config = validated_config(config)?;
        let key = issue_reference(&config, reference)?;
        self.tracker_secrets
            .read_tracker_issue(
                &self.tracker_client,
                &project.to_string_lossy(),
                &config,
                &credential_name(&config),
                &key,
            )
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn references_cannot_redirect_a_saved_credential() {
        let config = validated_config(TrackerConfig {
            provider: TrackerProvider::Jira,
            origin: "https://team.atlassian.net".into(),
            email: "owner@example.com".into(),
            oauth: false,
        })
        .unwrap();
        assert_eq!(
            issue_reference(&config, "https://team.atlassian.net/browse/ENG-42").unwrap(),
            "ENG-42"
        );
        for reference in [
            "https://other.atlassian.net/browse/ENG-42",
            "https://owner:secret@team.atlassian.net/browse/ENG-42",
            "ENG-42/../../",
            "ENG-0",
        ] {
            assert!(issue_reference(&config, reference).is_err(), "{reference}");
        }
        let linear = validated_config(TrackerConfig {
            provider: TrackerProvider::Linear,
            origin: "https://api.linear.app".into(),
            email: String::new(),
            oauth: false,
        })
        .unwrap();
        assert_eq!(
            issue_reference(&linear, "https://linear.app/team/issue/ENG-42/hello").unwrap(),
            "ENG-42"
        );
        assert!(issue_reference(&linear, "https://linear.app.evil/issue/ENG-42").is_err());
    }
}
