use super::*;
use bruhs_models::connections::ServiceProvider;
use bruhs_models::mail::*;

impl ServerClient {
    pub async fn gmail_connection(
        &self,
    ) -> Result<bruhs_models::connections::ServiceConnection, ServerError> {
        self.get_json("/api/connections/gmail").await
    }

    pub async fn mail_connection(
        &self,
        provider: ServiceProvider,
    ) -> Result<bruhs_models::connections::ServiceConnection, ServerError> {
        match provider {
            ServiceProvider::Gmail => self.gmail_connection().await,
            ServiceProvider::Outlook => self.get_json("/api/connections/outlook").await,
            _ => Err(ServerError::Invalid(
                "Choose a Gmail or Outlook Mail connection.",
            )),
        }
    }

    pub async fn mailboxes(&self) -> Result<MailboxesSnapshot, ServerError> {
        self.get_json("/api/mail").await
    }

    fn mailbox_url(
        &self,
        mailbox: &str,
        external: Option<&str>,
    ) -> Result<reqwest::Url, ServerError> {
        let mut url = reqwest::Url::parse(&self.url("/api/mail/mailboxes/"))
            .map_err(|_| ServerError::Invalid("Invalid mail server URL"))?;
        {
            let mut path = url
                .path_segments_mut()
                .map_err(|()| ServerError::Invalid("Invalid mail server URL"))?;
            path.pop_if_empty().push(mailbox).push("threads");
            if let Some(external) = external {
                path.push(external);
            }
        }
        Ok(url)
    }

    pub async fn browse_mailbox(
        &self,
        mailbox: &str,
        query: &MailBrowseQuery,
    ) -> Result<MailThreadsPage, ServerError> {
        // Diagnostics use a static route label, never private search text.
        let label = "GET /api/mail/mailboxes/{mailbox}/threads";
        let response = Self::check(
            self.http()?
                .get(self.mailbox_url(mailbox, None)?)
                .query(query)
                .send()
                .await,
            label,
        )
        .await?;
        Self::decode(response, label).await
    }

    pub async fn mail_thread(
        &self,
        host: &str,
        mailbox: &str,
        external: &str,
    ) -> Result<ConnectedMailThread, ServerError> {
        let label = "GET /api/mail/mailboxes/{mailbox}/threads/{thread}";
        let response = Self::check(
            self.http()?
                .get(self.mailbox_url(mailbox, Some(external))?)
                .query(&[("host_id", host)])
                .send()
                .await,
            label,
        )
        .await?;
        Self::decode(response, label).await
    }

    pub async fn sync_gmail(
        &self,
        request: &GmailSyncRequest,
    ) -> Result<MailSyncResult, ServerError> {
        self.sync_mail_at("/api/mail/gmail/sync", request).await
    }

    pub async fn sync_outlook(
        &self,
        request: &GmailSyncRequest,
    ) -> Result<MailSyncResult, ServerError> {
        self.sync_mail_at("/api/mail/outlook/sync", request).await
    }

    pub async fn sync_mail(
        &self,
        provider: ServiceProvider,
        request: &GmailSyncRequest,
    ) -> Result<MailSyncResult, ServerError> {
        match provider {
            ServiceProvider::Gmail => self.sync_gmail(request).await,
            ServiceProvider::Outlook => self.sync_outlook(request).await,
            _ => Err(ServerError::Invalid(
                "Choose a Gmail or Outlook Mail connection.",
            )),
        }
    }

    async fn sync_mail_at(
        &self,
        route: &str,
        request: &GmailSyncRequest,
    ) -> Result<MailSyncResult, ServerError> {
        self.send_json_with_timeout(
            reqwest::Method::POST,
            route,
            Some(request),
            Duration::from_secs(180),
        )
        .await
    }

    pub async fn save_mail_attachment(
        &self,
        host: &str,
        mailbox: &str,
        external: &str,
        attachment: &bruhs_models::mail::MailAttachment,
        path: &std::path::Path,
    ) -> Result<(), ServerError> {
        self.save_attachment_from_provider(
            ServiceProvider::Gmail,
            host,
            mailbox,
            external,
            attachment,
            path,
        )
        .await
    }

    pub async fn save_provider_mail_attachment(
        &self,
        provider: ServiceProvider,
        host: &str,
        mailbox: &str,
        external: &str,
        attachment: &bruhs_models::mail::MailAttachment,
        path: &std::path::Path,
    ) -> Result<(), ServerError> {
        match provider {
            ServiceProvider::Gmail => {
                self.save_mail_attachment(host, mailbox, external, attachment, path)
                    .await
            }
            ServiceProvider::Outlook => {
                self.save_attachment_from_provider(
                    provider, host, mailbox, external, attachment, path,
                )
                .await
            }
            _ => Err(ServerError::Invalid(
                "Choose a Gmail or Outlook Mail connection.",
            )),
        }
    }

    async fn save_attachment_from_provider(
        &self,
        provider: ServiceProvider,
        host: &str,
        mailbox: &str,
        external: &str,
        attachment: &bruhs_models::mail::MailAttachment,
        path: &std::path::Path,
    ) -> Result<(), ServerError> {
        use bruhs_models::connections::ConnectionState;
        // Same 64 MiB response ceiling as the provider adapter. Never buffer
        // beyond the saved size, including when Content-Length is absent.
        if attachment.size_bytes > MAIL_ATTACHMENT_MAX_BYTES {
            return Err(ServerError::Invalid(
                "This attachment exceeds the mail download size limit.",
            ));
        }
        let connection = self.mail_connection(provider).await?;
        let connection_id = connection
            .id
            .filter(|_| connection.state == ConnectionState::Connected)
            .ok_or(ServerError::Invalid(
                "Connect the mail account before downloading this attachment.",
            ))?;
        let prefix = match provider {
            ServiceProvider::Gmail => "/api/mail/mailboxes/",
            ServiceProvider::Outlook => "/api/mail/outlook/mailboxes/",
            _ => {
                return Err(ServerError::Invalid(
                    "Choose a Gmail or Outlook Mail connection.",
                ))
            }
        };
        let mut url = reqwest::Url::parse(&self.url(prefix))
            .map_err(|_| ServerError::Invalid("Invalid mail server URL"))?;
        url.path_segments_mut()
            .map_err(|()| ServerError::Invalid("Invalid mail server URL"))?
            .pop_if_empty()
            .push(mailbox)
            .push("threads")
            .push(external)
            .push("attachments")
            .push(&attachment.id);
        let mut response = Self::check(
            self.http()?
                .get(url)
                .query(&[("host_id", host), ("connection_id", &connection_id)])
                .timeout(Duration::from_secs(180))
                .send()
                .await,
            "Download mail attachment",
        )
        .await?;
        // Only an explicit item-export receipt replaces the cached byte count.
        // Ordinary Outlook files retain the same truncation checks as Gmail.
        let limit = if provider == ServiceProvider::Outlook {
            response
                .headers()
                .get("x-bruhs-mail-export-size")
                .map(|size| {
                    size.to_str()
                        .ok()
                        .and_then(|size| size.parse::<u64>().ok())
                        .filter(|size| *size <= MAIL_ATTACHMENT_MAX_BYTES)
                        .ok_or(ServerError::Invalid("Invalid attachment export size."))
                })
                .transpose()?
                .unwrap_or(attachment.size_bytes)
        } else {
            attachment.size_bytes
        };
        if response.content_length().is_some_and(|size| size > limit) {
            return Err(ServerError::Invalid(
                "The attachment download exceeded its size limit.",
            ));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|source| ServerError::Transport {
                request: "Download mail attachment".into(),
                source: source.without_url(),
            })?
        {
            let length = bytes
                .len()
                .checked_add(chunk.len())
                .and_then(|length| u64::try_from(length).ok())
                .ok_or(ServerError::Invalid(
                    "The attachment download exceeded its saved size.",
                ))?;
            if length > limit {
                return Err(ServerError::Invalid(
                    "The attachment download exceeded its saved size.",
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        if u64::try_from(bytes.len()).ok() != Some(limit) {
            return Err(ServerError::Invalid(
                "The attachment download was incomplete. Retry without replacing the saved file.",
            ));
        }
        let path = path.to_owned();
        // A started private write finishes atomically even if its UI disappears.
        // The blocking worker owns both the file and its cleanup guard.
        tokio::task::spawn_blocking(move || save_attachment_file(&path, &bytes))
            .await
            .map_err(|_| {
                ServerError::Invalid(
                    "The attachment save worker stopped. Check the selected file before retrying.",
                )
            })?
    }
}

fn save_attachment_file(path: &std::path::Path, bytes: &[u8]) -> Result<(), ServerError> {
    use std::io::Write as _;
    struct Temporary(std::path::PathBuf);
    impl Drop for Temporary {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let result = (|| -> std::io::Result<()> {
        let temporary_path =
            path.with_file_name(format!(".bruhs-mail-{}.part", uuid::Uuid::new_v4()));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary_path)?;
        // Cleanup owns only a file successfully created by this operation.
        let temporary = Temporary(temporary_path);
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary.0, path)?;
        drop(temporary);
        #[cfg(unix)]
        if let Some(parent) = path.parent() {
            std::fs::File::open(parent)?.sync_all()?;
        }
        Ok(())
    })();
    result.map_err(|_| {
        ServerError::Invalid(
            "The attachment could not be saved to the selected location. Check storage and retry.",
        )
    })
}
