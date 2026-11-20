//! Apps (spec §7): a bruh's calls to the services the owner connected with
//! their own tokens. [`AppHttp`] is the one client that reaches them (D13,
//! D14); the rules, the catalog and all copy live in `bruhs_models::apps`.

mod http;
pub use http::AppHttp;

/// Names a debug build's test upstream (D24): `http://127.0.0.1:<port>` or
/// `http://[::1]:<port>`. Release builds ignore it.
pub const APPS_UPSTREAM_ENV: &str = "BRUHS_APPS_UPSTREAM";

/// Why an app call didn't happen or didn't finish. Each variant carries fixed
/// copy from `bruhs_models::apps`, never a transport error's text, so no
/// address, header or token reaches a bruh, a log or a route error.
#[derive(Debug, thiserror::Error)]
pub enum AppsError {
    /// Bruhs won't make this call: an undeclared host or one that resolves
    /// here or to a private network.
    #[error("{0}")]
    Refused(&'static str),
    /// The call or the reply breaks a rule: its path, method or query, or a
    /// reply that isn't text.
    #[error("{0}")]
    Invalid(&'static str),
    /// Bruhs can't make app calls right now.
    #[error("{0}")]
    Unavailable(&'static str),
    #[error("{0}")]
    Unreachable(&'static str),
    /// The reply is over `APP_READ_MAX_BYTES`.
    #[error("{0}")]
    TooLarge(&'static str),
    #[error("{0}")]
    TimedOut(&'static str),
}
