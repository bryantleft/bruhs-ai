//! Authenticated access to a native desktop broker, independent of the companion AppState.
pub mod authority;
mod broker;
pub mod diff;
pub mod question_link;
mod server;
pub use broker::{MobileBroker, MobilePending, MobileQuestionFile, MobileResponder};
pub use server::{MobileHost, MobileHostConfig, MobileTransport};
use std::time::Duration;

/// Same deadline and queue contract as the native workspace bridge.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
pub const BROKER_CAPACITY: usize = 16;
/// Same one-use interaction window as service OAuth login.
pub const PAIRING_LIFETIME: Duration = Duration::from_secs(5 * 60);
/// Explicit device interaction ceiling, matching private credential interactions.
pub const MAX_DEVICE_LIFETIME: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug, thiserror::Error)]
pub enum MobileError {
    #[error("This mobile session is no longer valid. Pair again on the laptop.")]
    Unauthorized,
    #[error("This device does not have permission for that action.")]
    Forbidden,
    #[error("Mobile access is off on the laptop.")]
    Disabled,
    #[error("Mobile storage failed. Access is blocked for this desktop launch. Fix storage, restart Bruhs, and pair again.")]
    Storage,
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Conflict(String),
    #[error("The desktop is busy or offline. Read its current state before retrying a command.")]
    Unavailable,
}
pub type MobileResult<T> = Result<T, MobileError>;

/// Cached native QR geometry; the secret is never written to an image file.
pub struct MobileQr {
    pub size: usize,
    pub modules: Vec<bool>,
}
impl MobileQr {
    pub fn encode(offer: &authority::MobilePairingOffer) -> MobileResult<Self> {
        let code = qrcodegen::QrCode::encode_text(&offer.url, qrcodegen::QrCodeEcc::Medium)
            .map_err(|_| {
                MobileError::Invalid(
                    "The pairing URL is too long for a QR code. Use a shorter HTTPS origin.".into(),
                )
            })?;
        let size = usize::try_from(code.size()).map_err(|_| MobileError::Unavailable)?;
        let modules = (0..code.size())
            .flat_map(|y| (0..code.size()).map(move |x| (x, y)))
            .map(|(x, y)| code.get_module(x, y))
            .collect();
        Ok(Self { size, modules })
    }
}
