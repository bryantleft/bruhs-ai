use super::*;
use axum::{
    extract::{RawQuery, State},
    http::HeaderMap,
    response::{Html, IntoResponse},
    routing::get,
    Router,
};
use tokio::{net::TcpListener, sync::oneshot};

struct Login {
    manager: Connections,
    config: oauth::Config,
    id: String,
    state: String,
    redirect: String,
    verifier: Zeroizing<String>,
    expected: Option<String>,
    expected_account: Option<String>,
    used: Mutex<bool>,
    cancel: watch::Receiver<bool>,
    done: Mutex<Option<oneshot::Sender<()>>>,
}

impl Connections {
    pub async fn start(
        &self,
        provider: ServiceProvider,
    ) -> Result<ConnectionStart, ConnectionError> {
        self.start_scoped(provider, None).await
    }

    pub(crate) async fn start_scoped(
        &self,
        provider: ServiceProvider,
        turn: Option<watch::Receiver<bool>>,
    ) -> Result<ConnectionStart, ConnectionError> {
        let config = self
            .0
            .configs
            .get(&provider)
            .ok_or(ConnectionError::NotConfigured)?
            .clone();
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(|_| ConnectionError::Listener)?;
        let redirect = format!(
            "http://127.0.0.1:{}/",
            listener
                .local_addr()
                .map_err(|_| ConnectionError::Listener)?
                .port()
        );
        let id = uuid::Uuid::new_v4().to_string();
        let state = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let (verifier, _) = crate::google_calendar::pkce::generate_verifier_and_challenge();
        let saved = self.0.store.metadata(provider).await?;
        if saved
            .as_ref()
            .is_some_and(|m| m.state != ConnectionState::Disconnected && m.account_id.is_none())
        {
            return Err(ConnectionError::IdentityUnavailable);
        }
        let expected_account = saved
            .as_ref()
            .filter(|m| m.state != ConnectionState::Disconnected)
            .and_then(|m| m.account_id.clone());
        if saved.as_ref().is_some_and(|m| {
            m.state != ConnectionState::Disconnected && m.registration != config.registration()
        }) {
            return Err(ConnectionError::RegistrationChanged);
        }
        let mut auth_url = config.auth_url(&redirect, &state, &verifier)?;
        // Google only returns a refresh token on renewed consent after a grant was used before.
        // The first connection reuses SSO; explicit reconnect after a lost/expired grant renews consent.
        if provider == ServiceProvider::Google
            && saved.as_ref().is_some_and(|m| {
                matches!(
                    m.state,
                    ConnectionState::Disconnected
                        | ConnectionState::NeedsSignIn
                        | ConnectionState::Unavailable
                ) || (!m.refresh_available
                    && m.expires_at_ms
                        .is_some_and(|expiry| expiry <= chrono::Utc::now().timestamp_millis()))
            })
        {
            let mut url =
                url::Url::parse(&auth_url).map_err(|_| ConnectionError::InvalidResponse)?;
            url.query_pairs_mut().append_pair("prompt", "consent");
            auth_url = url.into();
        }
        if let Some(account) = saved
            .as_ref()
            .map(|m| m.account.as_str())
            .filter(|account| !account.is_empty())
        {
            let mut url =
                url::Url::parse(&auth_url).map_err(|_| ConnectionError::InvalidResponse)?;
            url.query_pairs_mut().append_pair(
                if provider == ServiceProvider::Github {
                    "login"
                } else {
                    "login_hint"
                },
                account,
            );
            auth_url = url.into();
        }
        let expected = saved.map(|m| m.id);
        let (cancel, cancelled) = watch::channel(false);
        {
            let mut attempts = self
                .0
                .attempts
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            // One attempt per provider; completed attempts are reclaimed on the next Connect.
            attempts.retain(|_, attempt| {
                if attempt.provider == provider {
                    attempt.cancel.send_replace(true);
                    false
                } else {
                    true
                }
            });
            attempts.insert(
                id.clone(),
                Attempt {
                    provider,
                    status: ConnectionAttempt {
                        id: id.clone(),
                        state: AttemptState::Pending,
                        message: None,
                    },
                    cancel,
                },
            );
        }
        let (done, finished) = oneshot::channel();
        let login = Arc::new(Login {
            manager: self.clone(),
            config,
            id: id.clone(),
            state,
            redirect,
            verifier: Zeroizing::new(verifier),
            expected,
            expected_account,
            used: Mutex::new(false),
            cancel: cancelled.clone(),
            done: Mutex::new(Some(done)),
        });
        let router = Router::new()
            .route("/", get(callback))
            .layer(axum::middleware::map_response(
                |mut response: axum::response::Response| async move {
                    let headers = response.headers_mut();
                    headers.insert(
                        axum::http::header::CACHE_CONTROL,
                        axum::http::HeaderValue::from_static("no-store"),
                    );
                    headers.insert(
                        axum::http::header::REFERRER_POLICY,
                        axum::http::HeaderValue::from_static("no-referrer"),
                    );
                    headers.insert(
                        axum::http::header::X_FRAME_OPTIONS,
                        axum::http::HeaderValue::from_static("DENY"),
                    );
                    response
                },
            ))
            .with_state(login.clone());
        tokio::spawn(async move {
            let shutdown_login = login.clone();
            let shutdown = async move {
                let mut cancelled = cancelled;
                let turn_cancelled = async move {
                    if let Some(mut turn) = turn {
                        if !*turn.borrow() {
                            let _ = turn.changed().await;
                        }
                    } else {
                        std::future::pending::<()>().await;
                    }
                };
                // Matches the existing Calendar pending-login interaction window.
                tokio::select! {
                    _ = finished => return,
                    _ = cancelled.changed() => {},
                    _ = turn_cancelled => {},
                    _ = tokio::time::sleep(std::time::Duration::from_secs(5 * 60)) => {},
                }
                shutdown_login.manager.cancel(&shutdown_login.id);
            };
            if axum::serve(listener, router)
                .with_graceful_shutdown(shutdown)
                .await
                .is_err()
            {
                login.complete(Err(ConnectionError::Listener));
            }
        });
        Ok(ConnectionStart { id, auth_url })
    }
}

impl Login {
    fn complete(&self, result: Result<(), ConnectionError>) {
        if let Some(attempt) = self
            .manager
            .0
            .attempts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_mut(&self.id)
        {
            if attempt.status.state == AttemptState::Pending {
                match result {
                    Ok(()) => attempt.status.state = AttemptState::Connected,
                    Err(error) => {
                        attempt.status.state = AttemptState::Failed;
                        attempt.status.message = Some(error.to_string());
                    }
                }
            }
        }
        if let Some(done) = self
            .done
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            let _ = done.send(());
        }
    }

    async fn exchange(&self, code: &str) -> Result<(), ConnectionError> {
        let lease = self.manager.0.store.lease(self.config.provider).await?;
        if *self.cancel.borrow() {
            return Err(ConnectionError::Cancelled);
        }
        if lease.metadata().await?.as_ref().map(|p| &p.id) != self.expected.as_ref() {
            return Err(ConnectionError::Cancelled);
        }
        let previous = lease.read().await?;
        let tokens = oauth::exchange(
            &self.config,
            code,
            &self.redirect,
            &self.verifier,
            previous.as_ref(),
        )
        .await?;
        if self
            .expected_account
            .as_ref()
            .is_some_and(|expected| expected != &tokens.account_id)
        {
            return Err(ConnectionError::AccountMismatch);
        }
        if *self.cancel.borrow() {
            return Err(ConnectionError::Cancelled);
        }
        lease.write(&tokens).await?;
        if *self.cancel.borrow() {
            if let Some(previous) = previous {
                lease.write(&previous).await?;
            } else {
                lease.clear().await?;
            }
            return Err(ConnectionError::Cancelled);
        }
        drop(lease);
        Ok(())
    }
}

async fn callback(
    State(login): State<Arc<Login>>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
) -> impl IntoResponse {
    let expected_host = login
        .redirect
        .strip_prefix("http://")
        .and_then(|s| s.split('/').next());
    let valid_host = headers.get("host").and_then(|h| h.to_str().ok()) == expected_host;
    let pairs: Vec<_> =
        url::form_urlencoded::parse(query.as_deref().unwrap_or_default().as_bytes()).collect();
    let values = |name: &str| {
        pairs
            .iter()
            .filter(|(key, _)| key == name)
            .map(|(_, value)| value.as_ref())
            .collect::<Vec<_>>()
    };
    let state = values("state");
    if !valid_host || state.len() != 1 || state[0] != login.state || *login.cancel.borrow() {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Html(page(
                "This sign-in request is invalid or expired. Return to Bruhs and connect again.",
            )),
        );
    }
    {
        let mut used = login.used.lock().unwrap_or_else(PoisonError::into_inner);
        if *used {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                Html(page("This sign-in request has already been used.")),
            );
        }
        *used = true;
    }
    let codes = values("code");
    let succeeded = if codes.len() == 1 && values("error").is_empty() {
        let login = login.clone();
        let code = Zeroizing::new(codes[0].to_owned());
        // Losing the callback socket must not interrupt keychain commit or cancellation cleanup.
        tokio::spawn(async move {
            let result = login.exchange(&code).await;
            let succeeded = result.is_ok();
            login.complete(result);
            succeeded
        })
        .await
        .unwrap_or(false)
    } else {
        login.complete(Err(ConnectionError::SignIn));
        false
    };
    let message = if succeeded {
        "Account connected. You can close this tab and return to Bruhs. Agent operations still need approval."
    } else {
        "The account could not be connected. Return to Bruhs for details and try again."
    };
    (axum::http::StatusCode::OK, Html(page(message)))
}

fn page(message: &str) -> String {
    format!("<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"referrer\" content=\"no-referrer\"><title>Connect account - Bruhs</title></head><body><h1>Bruhs</h1><p>{message}</p></body></html>")
}
