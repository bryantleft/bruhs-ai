//! Synthetic in-memory credentials for isolated broker/HTTP tests only.
use super::*;

impl Connections {
    #[doc(hidden)]
    pub async fn test_connected(provider: ServiceProvider, value: &str) -> Self {
        let mut config = oauth::Config::for_client(
            provider,
            "synthetic-client".into(),
            Some("synthetic-client-secret".into()),
            None,
        )
        .expect("synthetic provider configuration");
        config.auth = "http://127.0.0.1/authorize".into();
        config.token = "http://127.0.0.1/token".into();
        config.identity = "http://127.0.0.1/identity".into();
        let manager = Self(Arc::new(Inner {
            configs: [(provider, config)].into(),
            store: storage::Store::memory(),
            attempts: Mutex::new(HashMap::new()),
        }));
        manager
            .0
            .store
            .lease(provider)
            .await
            .expect("lease the synthetic in-memory connection")
            .write(&Tokens {
                id: "synthetic-connection".into(),
                access_token: value.into(),
                refresh_token: Some("synthetic-refresh".into()),
                expires_at_ms: None,
                account_id: "stable-subject".into(),
                account: "you@example.test".into(),
                scopes: manager.0.configs[&provider].scopes.clone(),
                registration: manager.0.configs[&provider].registration(),
            })
            .await
            .expect("save synthetic in-memory credentials");
        manager
    }
}
