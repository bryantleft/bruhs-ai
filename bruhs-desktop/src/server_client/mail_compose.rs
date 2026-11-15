//! Compose requests use fixed server URLs, static diagnostic labels and no
//! redirect or transport retry. A lost send response is recovered by status.
use super::*;
use bruhs_models::mail::*;

impl ServerClient {
    pub fn mail_compose_origin(&self) -> &str {
        &self.base
    }

    fn compose_url(&self, mailbox: &str, segments: &[&str]) -> Result<reqwest::Url, ServerError> {
        let mut url = reqwest::Url::parse(&self.url("/api/mail/mailboxes/"))
            .map_err(|_| ServerError::Invalid("Invalid mail server URL"))?;
        url.path_segments_mut()
            .map_err(|()| ServerError::Invalid("Invalid mail server URL"))?
            .pop_if_empty()
            .push(mailbox)
            .extend(segments.iter().copied());
        Ok(url)
    }
    async fn compose_http(&self) -> Result<&reqwest::Client, ServerError> {
        self.compose_http
            .get_or_try_init(|| async {
                reqwest::Client::builder()
                    .no_proxy()
                    .redirect(reqwest::redirect::Policy::none())
                    .retry(reqwest::retry::never())
                    .timeout(Duration::from_secs(180))
                    .build()
                    .map_err(|error| ServerError::Client(Arc::new(error)))
            })
            .await
    }
    async fn compose_json<T: DeserializeOwned, B: Serialize + ?Sized>(
        &self,
        method: reqwest::Method,
        url: reqwest::Url,
        body: Option<&B>,
    ) -> Result<T, ServerError> {
        const LABEL: &str = "Email draft request";
        let mut request = self.compose_http().await?.request(method, url);
        if let Some(body) = body {
            request = request.json(body);
        }
        Self::decode(Self::check(request.send().await, LABEL).await?, LABEL).await
    }
    pub async fn mail_draft_create(
        &self,
        mailbox: &str,
        body: &MailDraftCreate,
    ) -> Result<MailComposeDraft, ServerError> {
        self.compose_json(
            reqwest::Method::POST,
            self.compose_url(mailbox, &["drafts"])?,
            Some(body),
        )
        .await
    }
    pub async fn mail_draft(
        &self,
        host: &str,
        mailbox: &str,
        client: &str,
    ) -> Result<MailComposeDraft, ServerError> {
        let mut url = self.compose_url(mailbox, &["drafts", client])?;
        url.query_pairs_mut().append_pair("host_id", host);
        self.compose_json::<_, ()>(reqwest::Method::GET, url, None)
            .await
    }
    pub async fn mail_drafts(
        &self,
        host: &str,
        mailbox: &str,
        page: u64,
        revision: Option<i64>,
    ) -> Result<MailDraftsPage, ServerError> {
        let mut url = self.compose_url(mailbox, &["drafts"])?;
        url.query_pairs_mut()
            .append_pair("host_id", host)
            .append_pair("page", &page.to_string());
        if let Some(revision) = revision {
            url.query_pairs_mut()
                .append_pair("expected_revision", &revision.to_string());
        }
        self.compose_json::<_, ()>(reqwest::Method::GET, url, None)
            .await
    }
    pub async fn mail_draft_save(
        &self,
        mailbox: &str,
        client: &str,
        body: &MailDraftSave,
    ) -> Result<MailComposeDraft, ServerError> {
        self.compose_json(
            reqwest::Method::PUT,
            self.compose_url(mailbox, &["drafts", client])?,
            Some(body),
        )
        .await
    }
    pub async fn mail_draft_review(
        &self,
        mailbox: &str,
        client: &str,
        body: &MailReviewRequest,
    ) -> Result<MailSendReview, ServerError> {
        self.compose_json(
            reqwest::Method::POST,
            self.compose_url(mailbox, &["drafts", client, "review"])?,
            Some(body),
        )
        .await
    }
    pub async fn mail_draft_send(
        &self,
        mailbox: &str,
        client: &str,
        body: &MailSendRequest,
    ) -> Result<MailSendStatus, ServerError> {
        self.compose_json(
            reqwest::Method::POST,
            self.compose_url(mailbox, &["drafts", client, "send"])?,
            Some(body),
        )
        .await
    }
    pub async fn mail_send_status(
        &self,
        host: &str,
        mailbox: &str,
        operation: &str,
    ) -> Result<MailSendStatus, ServerError> {
        let mut url = self.compose_url(mailbox, &["sends", operation])?;
        url.query_pairs_mut().append_pair("host_id", host);
        self.compose_json::<_, ()>(reqwest::Method::GET, url, None)
            .await
    }
    pub async fn mail_send_cancel(
        &self,
        host: &str,
        mailbox: &str,
        operation: &str,
    ) -> Result<MailSendStatus, ServerError> {
        self.compose_json(
            reqwest::Method::POST,
            self.compose_url(mailbox, &["sends", operation, "cancel"])?,
            Some(&serde_json::json!({"host_id":host})),
        )
        .await
    }
    pub async fn mail_draft_remove_upload(
        &self,
        host: &str,
        mailbox: &str,
        client: &str,
        id: &str,
    ) -> Result<(), ServerError> {
        let mut url = self.compose_url(mailbox, &["drafts", client, "files", id])?;
        url.query_pairs_mut().append_pair("host_id", host);
        self.compose_json::<_, ()>(reqwest::Method::DELETE, url, None)
            .await
    }
    pub async fn mail_draft_upload(
        &self,
        host: &str,
        mailbox: &str,
        client: &str,
        upload: &str,
        name: &str,
        bytes: Vec<u8>,
    ) -> Result<MailDraftFile, ServerError> {
        if bytes.len() > MAIL_DRAFT_ATTACHMENT_MAX_BYTES {
            return Err(ServerError::Invalid(
                "Keep draft files within 16 MiB combined.",
            ));
        }
        let mut url = self.compose_url(mailbox, &["drafts", client, "files", upload])?;
        url.query_pairs_mut().append_pair("host_id", host);
        let part = reqwest::multipart::Part::bytes(bytes)
            .file_name(name.to_owned())
            .mime_str("application/octet-stream")
            .map_err(|_| ServerError::Invalid("The selected file could not be uploaded."))?;
        const LABEL: &str = "Email draft file upload";
        let response = Self::check(
            self.compose_http()
                .await?
                .post(url)
                .multipart(reqwest::multipart::Form::new().part("file", part))
                .send()
                .await,
            LABEL,
        )
        .await?;
        Self::decode(response, LABEL).await
    }
}
