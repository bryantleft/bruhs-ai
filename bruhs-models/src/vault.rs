//! The vault: logins, keys and 1Password references the owner keeps for
//! threads and bruhs. Entries, scopes and kinds, their validation, sign-in
//! origins, one-time code setup (`totp`) and the forms and rows both apps
//! show (`form`).
//!
//! Only metadata lives in these types: names, scopes, sign-in pages,
//! usernames and `op://` references. A value (key, password, one-time code
//! setup key) travels only in the write-only request bodies
//! ([`VaultCreate`], [`VaultUpdate`]), which have no `Debug` or `Clone`, and
//! no error here repeats one.

mod form;
mod totp;
pub use form::*;
pub use totp::*;

use crate::secrets::{review_text, SavedCredentialState};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

// Page copy.
pub const VAULT_TITLE: &str = "Vault";
pub const VAULT_DESCRIPTION: &str = "Logins, keys and 1Password references, kept in your OS keychain. A bruh fills a login you grant it on that login's own site without seeing it. Every other use asks first.";
pub const NEW_ENTRY: &str = "New entry";
pub const SAVE_ENTRY: &str = "Save entry";
pub const CANCEL: &str = "Cancel";
pub const EDIT: &str = "Edit";
pub const CHECK: &str = "Check";
pub const DELETE: &str = "Delete";
pub const KEEP: &str = "Keep";

// Kind, scope and source labels.
pub const KIND_LOGIN: &str = "Login";
pub const KIND_KEY: &str = "Key";
pub const KIND_ONE_PASSWORD: &str = "1Password";
pub const SCOPE_GLOBAL: &str = "All projects";
pub const SCOPE_PROJECT: &str = "One project";
pub const SCOPE_BRUH: &str = "One bruh";
pub const SOURCE_SAVED: &str = "Saved here";
pub const SOURCE_ONE_PASSWORD: &str = "1Password";

// Field labels.
pub const FIELD_NAME: &str = "Name";
pub const FIELD_KIND: &str = "Kind";
pub const FIELD_USED_BY: &str = "Used by";
pub const FIELD_PROJECT: &str = "Project folder";
pub const FIELD_BRUH: &str = "Bruh";
pub const FIELD_VALUE: &str = "Value";
pub const FIELD_SIGN_IN: &str = "Sign-in page";
pub const FIELD_STORED_IN: &str = "Stored in";
pub const FIELD_USERNAME: &str = "Username";
pub const FIELD_PASSWORD: &str = "Password";
pub const FIELD_TOTP: &str = "One-time code setup key";
pub const FIELD_REMOVE_TOTP: &str = "Remove one-time code";
pub const FIELD_USERNAME_REF: &str = "Username reference";
pub const FIELD_PASSWORD_REF: &str = "Password reference";
pub const FIELD_TOTP_REF: &str = "One-time code reference";
pub const FIELD_REFERENCE: &str = "Reference";
pub const SEARCH_PLACEHOLDER: &str = "Search by name, site or project";

// Hints.
pub const TOTP_HINT: &str = "Paste the setup key or the otpauth link from the site's QR code.";
pub const KEEP_HINT: &str = "Values are never shown. Leave a value empty to keep it.";
pub const REFERENCE_HINT: &str = "References look like op://Vault/Item/field.";
pub const OP_MISSING_HINT: &str =
    "1Password CLI not found. Install it and turn on its app integration to use references.";
pub const FIXED_HINT: &str = "Delete it and create it again to change its name or where it's used.";

// State and result lines.
pub const STATE_SAVED: &str = "Stored in your OS keychain. Value hidden.";
pub const STATE_IN_ONE_PASSWORD: &str = "Read from 1Password when used.";
pub const STATE_NEEDS_ATTENTION: &str = "Needs attention. Replace or delete it to finish a change.";
pub const EMPTY: &str =
    "The vault is empty. Choose New entry to add a login, a key or a 1Password reference.";
pub const NO_MATCH: &str = "Nothing in the vault matches your search.";
pub const CHECK_OK: &str = "Ready.";
pub const ENTRY_UPDATED: &str = "Saved.";
pub const REFRESH: &str = "Refresh";
pub const LOADING: &str = "Loading the vault.";
/// Follows a listing's error.
pub const LOAD_RETRY: &str = "Refresh to try again.";
/// Why the page's buttons wait while one change runs.
pub const WAIT_FOR_CHANGE: &str = "Wait for the current change to finish.";
/// Why the page's buttons wait while one entry's Check reads it.
pub const WAIT_FOR_CHECK: &str = "Wait for the check to finish.";
/// Why One bruh can't be chosen yet.
pub const NO_BRUHS: &str = "Make a bruh first to give it an entry of its own.";
/// The listing refused to a page on another address (ruling F26 and its
/// Task 10 amendment): the page can neither show nor change the vault.
/// [`open_from_own_address`] names the address when it knows the port.
pub const OPEN_FROM_OWN_ADDRESS: &str =
    "Open Bruhs from its own address to see and change the vault.";

/// [`OPEN_FROM_OWN_ADDRESS`] naming the exact address that may change the
/// vault (rereview I1): the server binds only `127.0.0.1`, so another
/// loopback name of its port can be another program's page. `server` is the
/// client's own server URL; with no port in it the copy names none.
#[must_use]
pub fn open_from_own_address(server: &str) -> String {
    server_port(server).map_or_else(
        || OPEN_FROM_OWN_ADDRESS.to_owned(),
        |port| format!("Open Bruhs at http://127.0.0.1:{port} to see and change the vault."),
    )
}

/// The explicit port of an `http(s)://host:port` URL.
fn server_port(url: &str) -> Option<u16> {
    let (_, rest) = url.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let authority = authority
        .rsplit_once('@')
        .map_or(authority, |(_, authority)| authority);
    let (_, port) = authority.rsplit_once(':')?;
    port.parse().ok()
}

/// A vault call that got no answer: the service isn't reachable.
pub const VAULT_UNREACHABLE: &str =
    "Bruhs couldn't reach its service. Check that it's running, then refresh.";
/// A vault call the service failed in a way that has no fixed copy.
pub const VAULT_FAILED: &str = "The vault couldn't do that. Refresh and try again.";
/// How long either app waits for one vault call. A change or a check can
/// wait on a keychain or 1Password prompt: the server bounds a read by 30
/// seconds and `op` by 25, so the page waits longer than both before it
/// gives up with [`VAULT_UNREACHABLE`] and its buttons come back.
pub const VAULT_CALL_TIMEOUT_MS: u32 = 75_000;
/// The longest service sentence a vault page shows; a longer body isn't
/// one of the routes' messages.
const MAX_VAULT_MESSAGE_LEN: usize = 512;

/// A change (a save or a delete) that got no answer may still have landed
/// on the server (rereview M5): the page reloads its listing and says so.
pub const MAY_HAVE_CHANGED: &str =
    "Bruhs didn't answer, so the change may have gone through. Check the list before you try again.";

/// The line a vault page shows above its list after a change, on both apps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VaultNotice {
    /// The change landed: [`ENTRY_UPDATED`].
    Saved,
    /// The change got no answer and the listing reloads:
    /// [`MAY_HAVE_CHANGED`].
    MayHaveChanged,
}

impl VaultNotice {
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::Saved => ENTRY_UPDATED,
            Self::MayHaveChanged => MAY_HAVE_CHANGED,
        }
    }

    /// Whether the line warns (durian) rather than confirms (pandan).
    #[must_use]
    pub const fn warns(self) -> bool {
        matches!(self, Self::MayHaveChanged)
    }
}

/// Why a vault call failed, as either app saw it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VaultFailure<'a> {
    /// No answer in time: the service is down, unreachable or stuck.
    NoAnswer,
    /// The service answered `code`, with `error` its `{"error": …}`
    /// sentence when it sent one.
    Status { code: u16, error: Option<&'a str> },
    /// An answer the app couldn't read, or a call it couldn't send.
    Other,
}

impl VaultFailure<'_> {
    /// Whether a failed change may still have landed, so the page reloads
    /// its listing and shows [`VaultNotice::MayHaveChanged`] rather than
    /// the failure: only a call that got no answer. An answered failure
    /// changed nothing.
    #[must_use]
    pub const fn may_have_landed(self) -> bool {
        matches!(self, Self::NoAnswer)
    }
}

/// What a vault page shows for a failed call, on both apps. A refusal
/// (4xx) or an unavailable keychain or 1Password (503) shows the vault
/// routes' own sentence, which is fixed copy from this module and core's
/// vault errors and never holds a value (`bruhs-server` `routes/vault.rs`).
/// Anything else, a 500 or a body that isn't a short sentence, shows
/// [`VAULT_FAILED`], and no answer at all [`VAULT_UNREACHABLE`].
#[must_use]
pub fn vault_failure_message(failure: VaultFailure<'_>) -> &str {
    match failure {
        VaultFailure::NoAnswer => VAULT_UNREACHABLE,
        VaultFailure::Other => VAULT_FAILED,
        VaultFailure::Status { code, error } => error
            .map(str::trim)
            .filter(|_| (400..500).contains(&code) || code == 503)
            .filter(|error| !error.is_empty() && error.len() <= MAX_VAULT_MESSAGE_LEN)
            .unwrap_or(VAULT_FAILED),
    }
}

// Route copy (D22).
/// A vault request body that isn't an entry. Never repeats the body.
pub const INVALID_ENTRY: &str = "Invalid vault entry.";
/// An entry another save or another window deleted.
pub const ENTRY_GONE: &str = "That entry is gone. Refresh the vault.";
pub const PROJECT_MISSING: &str = "That project folder doesn't exist.";
pub const BRUH_MISSING: &str = "That bruh doesn't exist.";
/// A vault change, or a bruh's grants, sent from a page other than this
/// server's own or a configured one (ruling F26).
pub const OWNER_ONLY: &str = "Open Bruhs from its own address to change the vault.";
/// An answer to a bruh's ask to use a vault key from such a page.
pub const OWNER_ONLY_CARD: &str = "Open Bruhs from its own address to answer this card.";
/// Deleting a bruh, which deletes its own vault entries, from such a page
/// (ruling BC I2).
pub const OWNER_ONLY_DELETE_BRUH: &str = "Open Bruhs from its own address to delete a bruh.";

// Validation copy.
pub const DUPLICATE_NAME: &str = "Another entry used there has that name.";
/// Two saves of one entry crossed; the later one changed nothing.
pub const ENTRY_CHANGED: &str = "This entry changed while you were saving. Try again.";
pub const ENTER_NAME: &str = "Enter a name.";
pub const NAME_CHARACTERS: &str = "Names can't contain control or formatting characters.";
pub const NAME_TOO_LONG: &str = "Keep names under 80 characters.";
pub const ENTER_VALUE: &str = "Enter a value.";
pub const ENTER_PASSWORD: &str = "Enter a password.";
pub const VALUE_NUL: &str = "Values can't contain NUL characters.";
pub const VALUE_TOO_LONG: &str = "Keep values under 16 KB.";
pub const PASSWORD_TOO_LONG: &str = "Keep the password under 4 KB.";
pub const USERNAME_TOO_LONG: &str = "Keep the username under 256 characters.";
pub const USERNAME_CHARACTERS: &str = "Usernames can't contain control characters.";
pub const ENTER_PROJECT: &str = "Enter the project folder.";
pub const CHOOSE_BRUH: &str = "Choose a bruh.";
pub const REFERENCE_SHAPE: &str = "Use a reference like op://Vault/Item/field.";
pub const KIND_FIXED: &str = "Keep the entry's kind. Delete it and create another to change it.";
pub const ORIGIN_FIXED: &str = "Delete it and create it again to use it on another site.";
pub const SIGN_IN_SCHEME: &str = "Use an http or https sign-in page.";
pub const SIGN_IN_LOCAL: &str =
    "Use a sign-in page on another computer. Bruh browsers can't reach this one.";
pub const SIGN_IN_ENCODED: &str = "Use the site's plain address, not an encoded one.";
pub const SIGN_IN_ADDRESS: &str =
    "Use a sign-in page without a username, password or spaces in its address.";

/// Most bytes a key value may hold (16 KiB).
pub const MAX_VALUE_BYTES: usize = 16 * 1024;
/// Most bytes a login password may hold (4 KiB).
pub const MAX_PASSWORD_BYTES: usize = 4 * 1024;
/// Most characters in a login username.
pub const MAX_USERNAME_CHARS: usize = 256;
/// Most characters in an entry name, after trimming.
pub const MAX_NAME_CHARS: usize = 80;
/// Most bytes in an `op://` reference.
pub const MAX_REFERENCE_BYTES: usize = 512;

/// Who an entry serves (D2). Project entries serve thread requests in that
/// canonical folder, Global ones every thread and the bruhs they are
/// granted to, and Bruh entries only that bruh.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CredentialScope {
    Project { path: String },
    Global,
    Bruh { agent_id: String },
}

impl CredentialScope {
    /// The scope's part of a keychain account (D5): `global`,
    /// `bruh:<agent_id>` or `project:<first 16 hex of sha256(path)>`.
    #[must_use]
    pub fn key(&self) -> String {
        match self {
            Self::Global => "global".into(),
            Self::Bruh { agent_id } => format!("bruh:{agent_id}"),
            Self::Project { path } => {
                let hash = format!("{:x}", Sha256::digest(path.as_bytes()));
                format!("project:{}", &hash[..16])
            }
        }
    }
}

/// One field of a login a bruh can fill.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SecretField {
    Username,
    Password,
    Totp,
}

impl SecretField {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Username => "username",
            Self::Password => "password",
            Self::Totp => "one-time code",
        }
    }
}

/// Where a login's values live. Saved keeps the username as metadata and the
/// password and setup key in the keychain; 1Password keeps `op://`
/// references only.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum LoginFields {
    Saved {
        username: String,
        #[serde(default)]
        totp: bool,
    },
    OnePassword {
        username: String,
        password: String,
        #[serde(default)]
        totp: Option<String>,
    },
}

/// A login's metadata: its sign-in page and where its values live.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct LoginMeta {
    pub url: String,
    pub fields: LoginFields,
}

impl LoginMeta {
    /// The exact origin a fill needs ([`login_origin`]).
    #[must_use]
    pub fn origin(&self) -> Option<String> {
        login_origin(&self.url).ok()
    }

    /// The fields a bruh can fill: the password always, the username unless
    /// a Saved login has none (a fill never types an empty one), and the
    /// one-time code when the login has one.
    #[must_use]
    pub fn fields(&self) -> Vec<SecretField> {
        let (username, totp) = match &self.fields {
            LoginFields::Saved { username, totp } => (!username.is_empty(), *totp),
            LoginFields::OnePassword { totp, .. } => (true, totp.is_some()),
        };
        let mut fields = Vec::with_capacity(3);
        if username {
            fields.push(SecretField::Username);
        }
        fields.push(SecretField::Password);
        if totp {
            fields.push(SecretField::Totp);
        }
        fields
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VaultKind {
    Value,
    Login(LoginMeta),
    OnePassword { reference: String },
}

/// The last failure on an entry, kept in memory and shown on its row.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VaultAttention {
    OnePasswordNeedsYou,
    OnePasswordMissing,
    OnePasswordNotFound,
    KeychainNeedsYou,
    Failed,
}

impl VaultAttention {
    #[must_use]
    pub fn message(self) -> &'static str {
        match self {
            Self::OnePasswordNeedsYou => {
                "1Password needs you. Sign in to the 1Password app, then check again."
            }
            Self::OnePasswordMissing => "1Password CLI isn't installed.",
            Self::OnePasswordNotFound => "1Password has no item at that reference.",
            Self::KeychainNeedsYou => {
                "Your keychain needs you. Allow access on this computer, then check again."
            }
            Self::Failed => "Bruhs couldn't read this entry. Check it again.",
        }
    }
}

/// One vault entry as listed. Never holds a value.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct VaultEntry {
    pub id: String,
    pub name: String,
    pub scope: CredentialScope,
    pub kind: VaultKind,
    pub state: SavedCredentialState,
    #[serde(default)]
    pub attention: Option<VaultAttention>,
}

/// A bruh's name, for scope lines and the picker.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct VaultBruh {
    pub agent_id: String,
    pub name: String,
}

/// `GET /api/vault`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
pub struct VaultListing {
    #[serde(default)]
    pub entries: Vec<VaultEntry>,
    #[serde(default)]
    pub bruhs: Vec<VaultBruh>,
    /// Whether the 1Password CLI was found. Finding it never runs it.
    #[serde(default)]
    pub one_password: bool,
}

/// `POST /api/vault/{id}/check`: whether one read worked, never the value.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct VaultCheck {
    pub ok: bool,
    pub message: String,
}

/// Which entry answered a thread's credential request.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SavedSource {
    Project,
    Global,
    OnePassword,
}

/// `POST /api/vault`. Write-only: deliberately no `Debug` or `Clone`.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VaultCreate {
    pub name: String,
    pub scope: CredentialScope,
    pub secret: VaultSecretInput,
}

/// `PUT /api/vault/{id}`. Write-only: deliberately no `Debug` or `Clone`.
/// Name, scope, kind and a login's origin stay as created.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VaultUpdate {
    pub secret: VaultSecretInput,
}

/// An entry's kind with its values. On an update a missing value keeps the
/// stored one.
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum VaultSecretInput {
    Value {
        #[serde(default)]
        value: Option<String>,
    },
    Login {
        url: String,
        fields: LoginInput,
    },
    OnePassword {
        reference: String,
    },
}

/// A login's values. 1Password fields are `op://` references.
#[derive(Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
pub enum LoginInput {
    Saved {
        username: String,
        #[serde(default)]
        password: Option<String>,
        #[serde(default)]
        totp: TotpInput,
    },
    OnePassword {
        username: String,
        password: String,
        #[serde(default)]
        totp: Option<String>,
    },
}

/// What to do with a Saved login's one-time code setup key. It is read
/// through a private strict wire form, so `{"action": "keep", "setup": …}`
/// is refused rather than dropping the key.
#[derive(Serialize, Deserialize, Default)]
#[serde(
    tag = "action",
    rename_all = "snake_case",
    deny_unknown_fields,
    from = "TotpWire"
)]
pub enum TotpInput {
    #[default]
    Keep,
    Remove,
    Set {
        setup: String,
    },
}

/// `TotpInput` on the wire. Serde ignores extra fields on an internally
/// tagged unit variant even under `deny_unknown_fields`, so the field-free
/// actions are empty structs here. Holds a value: no `Debug` or `Clone`.
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum TotpWire {
    Keep {},
    Remove {},
    Set { setup: String },
}

impl From<TotpWire> for TotpInput {
    fn from(wire: TotpWire) -> Self {
        match wire {
            TotpWire::Keep {} => Self::Keep,
            TotpWire::Remove {} => Self::Remove,
            TotpWire::Set { setup } => Self::Set { setup },
        }
    }
}

impl VaultCreate {
    /// A new entry's checks (D6), apart from uniqueness, the project folder
    /// existing and the bruh existing, which the store and server check.
    /// Trims the name in place first, so a body's `" GitHub "` is stored
    /// (and compared for [`DUPLICATE_NAME`]) as `"GitHub"`.
    pub fn validate(&mut self) -> Result<(), &'static str> {
        let trimmed = self.name.trim();
        if trimmed.len() != self.name.len() {
            self.name = trimmed.to_owned();
        }
        validate_entry_name(&self.name)?;
        match &self.scope {
            CredentialScope::Global => {}
            CredentialScope::Project { path } if path.trim().is_empty() => {
                return Err(ENTER_PROJECT)
            }
            CredentialScope::Bruh { agent_id } if agent_id.trim().is_empty() => {
                return Err(CHOOSE_BRUH)
            }
            CredentialScope::Project { .. } | CredentialScope::Bruh { .. } => {}
        }
        self.secret.validate(None)
    }
}

impl VaultUpdate {
    /// An update's checks against the stored kind: the kind stays, and so
    /// does a login's origin (ruling F3). A missing value keeps the stored
    /// one, except a Saved password replacing a 1Password login's.
    pub fn validate(&self, existing: &VaultKind) -> Result<(), &'static str> {
        let same_kind = matches!(
            (&self.secret, existing),
            (VaultSecretInput::Value { .. }, VaultKind::Value)
                | (VaultSecretInput::Login { .. }, VaultKind::Login(_))
                | (
                    VaultSecretInput::OnePassword { .. },
                    VaultKind::OnePassword { .. }
                )
        );
        if !same_kind {
            return Err(KIND_FIXED);
        }
        if let (VaultSecretInput::Login { url, .. }, VaultKind::Login(meta)) =
            (&self.secret, existing)
        {
            if Some(login_origin(url)?) != meta.origin() {
                return Err(ORIGIN_FIXED);
            }
        }
        self.secret.validate(Some(existing))
    }
}

impl VaultSecretInput {
    /// The metadata this input would store; `None` for a Value. A Saved
    /// login's `totp` is true only for `TotpInput::Set`: on an update the
    /// store keeps a stored setup key for `Keep`.
    #[must_use]
    pub fn kind_meta(&self) -> Option<VaultKind> {
        match self {
            Self::Value { .. } => None,
            Self::OnePassword { reference } => Some(VaultKind::OnePassword {
                reference: reference.clone(),
            }),
            Self::Login { url, fields } => {
                let fields = match fields {
                    LoginInput::Saved { username, totp, .. } => LoginFields::Saved {
                        username: username.clone(),
                        totp: matches!(totp, TotpInput::Set { .. }),
                    },
                    LoginInput::OnePassword {
                        username,
                        password,
                        totp,
                    } => LoginFields::OnePassword {
                        username: username.clone(),
                        password: password.clone(),
                        totp: totp.clone(),
                    },
                };
                Some(VaultKind::Login(LoginMeta {
                    url: url.clone(),
                    fields,
                }))
            }
        }
    }

    /// `existing` is the stored kind on an update, `None` on a create, where
    /// every value is required.
    fn validate(&self, existing: Option<&VaultKind>) -> Result<(), &'static str> {
        match self {
            Self::Value { value } => match value {
                Some(value) => validate_value(value, MAX_VALUE_BYTES, VALUE_TOO_LONG)
                    .map_err(|error| error.unwrap_or(ENTER_VALUE)),
                None if existing.is_some() => Ok(()),
                None => Err(ENTER_VALUE),
            },
            Self::OnePassword { reference } => validate_op_reference(reference),
            Self::Login { url, fields } => {
                login_origin(url)?;
                match fields {
                    LoginInput::Saved {
                        username,
                        password,
                        totp,
                    } => {
                        validate_username(username)?;
                        let stored_password = matches!(
                            existing,
                            Some(VaultKind::Login(LoginMeta {
                                fields: LoginFields::Saved { .. },
                                ..
                            }))
                        );
                        match password {
                            Some(password) => {
                                validate_value(password, MAX_PASSWORD_BYTES, PASSWORD_TOO_LONG)
                                    .map_err(|error| error.unwrap_or(ENTER_PASSWORD))?;
                            }
                            None if stored_password => {}
                            None => return Err(ENTER_PASSWORD),
                        }
                        if let TotpInput::Set { setup } = totp {
                            parse_totp_setup(setup)?;
                        }
                        Ok(())
                    }
                    LoginInput::OnePassword {
                        username,
                        password,
                        totp,
                    } => {
                        validate_op_reference(username)?;
                        validate_op_reference(password)?;
                        totp.as_deref().map_or(Ok(()), validate_op_reference)
                    }
                }
            }
        }
    }
}

/// `Err(None)` for an empty value, so the caller names the field.
fn validate_value(
    value: &str,
    max_bytes: usize,
    too_long: &'static str,
) -> Result<(), Option<&'static str>> {
    if value.is_empty() {
        return Err(None);
    }
    if value.contains('\0') {
        return Err(Some(VALUE_NUL));
    }
    if value.len() > max_bytes {
        return Err(Some(too_long));
    }
    Ok(())
}

fn validate_username(username: &str) -> Result<(), &'static str> {
    if username.chars().nth(MAX_USERNAME_CHARS).is_some() {
        return Err(USERNAME_TOO_LONG);
    }
    if username.chars().any(char::is_control) {
        return Err(USERNAME_CHARACTERS);
    }
    Ok(())
}

/// Whether text holds a control or a Unicode formatting control (bidi
/// overrides, zero-width characters), the characters [`review_text`] makes
/// visible.
fn has_hidden_characters(text: &str) -> bool {
    review_text(text) != text
}

/// An entry's name: 1 to 80 characters after trimming, no control or
/// formatting characters.
pub fn validate_entry_name(name: &str) -> Result<(), &'static str> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ENTER_NAME);
    }
    if has_hidden_characters(name) {
        return Err(NAME_CHARACTERS);
    }
    if name.chars().nth(MAX_NAME_CHARS).is_some() {
        return Err(NAME_TOO_LONG);
    }
    Ok(())
}

/// A 1Password secret reference: `op://<vault>/<item>/<field>` or
/// `op://<vault>/<item>/<section>/<field>`, at most 512 bytes, and an
/// optional `?attribute=<word>`. Each part is 1Password's own syntax:
/// letters, digits, spaces, `-`, `_` and `.`, not blank (other names are
/// written by their ids). So no shell metacharacter or lookalike passes.
pub fn validate_op_reference(reference: &str) -> Result<(), &'static str> {
    if reference.len() > MAX_REFERENCE_BYTES {
        return Err(REFERENCE_SHAPE);
    }
    let rest = reference.strip_prefix("op://").ok_or(REFERENCE_SHAPE)?;
    let (path, query) = match rest.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (rest, None),
    };
    let segment = |s: &&str| {
        !s.trim().is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '_' | '.' | '-'))
    };
    let segments: Vec<&str> = path.split('/').collect();
    if !(3..=4).contains(&segments.len()) || !segments.iter().all(segment) {
        return Err(REFERENCE_SHAPE);
    }
    if let Some(query) = query {
        let word = query.strip_prefix("attribute=").ok_or(REFERENCE_SHAPE)?;
        if word.is_empty() || !word.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return Err(REFERENCE_SHAPE);
        }
    }
    Ok(())
}

/// A sign-in page's exact origin (D7): `scheme://host[:port]` as Chrome's
/// `location.origin` shows it. Scheme and host are lowercased, the default
/// port is dropped and an IP host is written canonically (`3232235780` and
/// `192.168.001.004` read `192.168.1.4`), so the origin a fill compares
/// with is the one the page reports. The page must use http or https, have
/// no username, password or spaces in its address, a plain ASCII host
/// without `%` (Chrome decodes and maps those before it resolves them), and
/// not be this computer, which bruh browsers can't reach.
pub fn login_origin(url: &str) -> Result<String, &'static str> {
    let (scheme, rest) = url.split_once("://").ok_or(SIGN_IN_SCHEME)?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return Err(SIGN_IN_SCHEME);
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() {
        return Err(SIGN_IN_SCHEME);
    }
    crate::local_tools::validate_browser_url(&format!("{scheme}://{rest}"))
        .map_err(|_| SIGN_IN_ADDRESS)?;
    if authority.contains('%') || !authority.is_ascii() {
        return Err(SIGN_IN_ENCODED);
    }
    if crate::crew::is_local_host(authority) {
        return Err(SIGN_IN_LOCAL);
    }
    let (host, port) = split_authority(&authority.to_ascii_lowercase()).ok_or(SIGN_IN_SCHEME)?;
    // Checked again in canonical form, whatever spelling reached it.
    if crate::crew::is_local_host(&host) {
        return Err(SIGN_IN_LOCAL);
    }
    let default_port = if scheme == "http" { 80 } else { 443 };
    Ok(match port {
        Some(port) if port != default_port => format!("{scheme}://{host}:{port}"),
        _ => format!("{scheme}://{host}"),
    })
}

/// A lowercased authority's canonical host and port, or `None` when either
/// is malformed.
fn split_authority(authority: &str) -> Option<(String, Option<u16>)> {
    let (host, port) = if let Some(inner) = authority.strip_prefix('[') {
        let (address, after) = inner.split_once(']')?;
        let address = address.parse::<std::net::Ipv6Addr>().ok()?;
        let port = match after {
            "" => None,
            _ => Some(after.strip_prefix(':')?),
        };
        (format!("[{}]", ipv6_origin_text(address)), port)
    } else {
        let (host, port) = match authority.split_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        };
        let valid = host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_'));
        if host.is_empty() || !valid {
            return None;
        }
        let host = if ends_in_number(host) {
            ipv4_host(host)?.to_string()
        } else {
            host.to_owned()
        };
        (host, port)
    };
    let port = match port {
        None => None,
        Some(port) => {
            if port.is_empty() || port.len() > 5 || !port.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            Some(port.parse::<u16>().ok().filter(|port| *port != 0)?)
        }
    };
    Some((host, port))
}

/// The URL standard's "ends in a number": a host whose last label (a
/// trailing dot aside) is decimal digits or `0x` hex is parsed as IPv4, and
/// fails to load when that parse fails.
fn ends_in_number(host: &str) -> bool {
    let host = host.strip_suffix('.').unwrap_or(host);
    let last = host.rsplit('.').next().unwrap_or_default();
    (!last.is_empty() && last.bytes().all(|b| b.is_ascii_digit()))
        || last
            .strip_prefix("0x")
            .is_some_and(|hex| hex.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// The URL standard's IPv4 parser: one to four parts, each decimal, octal
/// (a leading `0`) or hex (`0x`), the last filling the remaining bytes, and
/// one trailing dot allowed.
fn ipv4_host(host: &str) -> Option<std::net::Ipv4Addr> {
    let host = host.strip_suffix('.').unwrap_or(host);
    let number = |part: &str| -> Option<u64> {
        if let Some(hex) = part.strip_prefix("0x") {
            if hex.is_empty() {
                Some(0)
            } else {
                u64::from_str_radix(hex, 16).ok()
            }
        } else if part.len() > 1 && part.starts_with('0') {
            u64::from_str_radix(&part[1..], 8).ok()
        } else if !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()) {
            part.parse().ok()
        } else {
            None
        }
    };
    let parts = host.split('.').map(number).collect::<Option<Vec<u64>>>()?;
    let (last, head) = parts.split_last()?;
    if head.len() > 3 || head.iter().any(|part| *part > 255) {
        return None;
    }
    let room = 8 * (4 - u32::try_from(head.len()).ok()?);
    if *last >= 1u64 << room {
        return None;
    }
    let value = head
        .iter()
        .zip([24u32, 16, 8])
        .fold(*last, |value, (part, shift)| value | part << shift);
    u32::try_from(value).ok().map(std::net::Ipv4Addr::from)
}

/// An IPv6 address as the URL standard writes it: lowercase hex pieces,
/// the first longest run of two or more zero pieces as `::`, and never a
/// dotted IPv4 tail (unlike `Ipv6Addr`'s `Display`).
fn ipv6_origin_text(address: std::net::Ipv6Addr) -> String {
    let pieces = address.segments();
    let mut best: Option<(usize, usize)> = None;
    let mut start = 0;
    while start < pieces.len() {
        if pieces[start] != 0 {
            start += 1;
            continue;
        }
        let end = (start..pieces.len())
            .find(|&index| pieces[index] != 0)
            .unwrap_or(pieces.len());
        if end - start > 1 && best.is_none_or(|(s, e)| end - start > e - s) {
            best = Some((start, end));
        }
        start = end;
    }
    let hex = |range: &[u16]| {
        range
            .iter()
            .map(|piece| format!("{piece:x}"))
            .collect::<Vec<_>>()
            .join(":")
    };
    match best {
        Some((start, end)) => format!("{}::{}", hex(&pieces[..start]), hex(&pieces[end..])),
        None => hex(&pieces),
    }
}

/// Whether a page is on exactly this origin. An unknown page never is.
#[must_use]
pub fn same_origin(page_url: Option<&str>, origin: &str) -> bool {
    page_url
        .and_then(|url| login_origin(url).ok())
        .is_some_and(|page| page == origin)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rereview I1: the notice names `127.0.0.1` and the server's own port,
    /// whichever loopback name the client calls it by.
    #[test]
    fn the_own_address_notice_names_127_0_0_1_and_the_servers_port() {
        for (server, notice) in [
            (
                "http://127.0.0.1:3001",
                "Open Bruhs at http://127.0.0.1:3001 to see and change the vault.",
            ),
            (
                "http://localhost:4100/",
                "Open Bruhs at http://127.0.0.1:4100 to see and change the vault.",
            ),
            (
                "http://[::1]:3005/api",
                "Open Bruhs at http://127.0.0.1:3005 to see and change the vault.",
            ),
            ("https://api.bruhs.localhost", OPEN_FROM_OWN_ADDRESS),
            ("http://[::1]", OPEN_FROM_OWN_ADDRESS),
            ("not a url", OPEN_FROM_OWN_ADDRESS),
        ] {
            assert_eq!(open_from_own_address(server), notice, "{server}");
        }
    }
    use serde_json::json;

    /// Both apps show a failed vault call this way (V Task 14 minor): only
    /// a refusal's or an unavailable store's own sentence, else fixed copy.
    #[test]
    fn a_failed_vault_call_shows_only_the_services_fixed_sentence() {
        let status = |code, error| vault_failure_message(VaultFailure::Status { code, error });
        assert_eq!(status(409, Some(DUPLICATE_NAME)), DUPLICATE_NAME);
        assert_eq!(status(403, Some(OWNER_ONLY)), OWNER_ONLY);
        let keychain = "Could not access the OS credential store. Unlock it and retry.";
        assert_eq!(status(503, Some(keychain)), keychain);
        assert_eq!(status(400, Some(" padded ")), "padded");
        // A 500 or a 502 can carry any internal text; it never shows.
        assert_eq!(
            status(500, Some("snapshot failed: /private/path")),
            VAULT_FAILED
        );
        assert_eq!(status(502, Some("proxy")), VAULT_FAILED);
        for unreadable in [None, Some(""), Some("   ")] {
            assert_eq!(status(400, unreadable), VAULT_FAILED);
        }
        let long = "x".repeat(513);
        assert_eq!(status(400, Some(&long)), VAULT_FAILED);
        assert_eq!(status(400, Some(&long[..512])), &long[..512]);
        // No answer in time says so the same way on both apps.
        assert_eq!(
            vault_failure_message(VaultFailure::NoAnswer),
            VAULT_UNREACHABLE
        );
        assert_eq!(vault_failure_message(VaultFailure::Other), VAULT_FAILED);
        // Rereview M5: only a change with no answer may have landed; the
        // page reloads and warns rather than confirms.
        assert!(VaultFailure::NoAnswer.may_have_landed());
        assert!(!VaultFailure::Other.may_have_landed());
        for code in [409, 503] {
            assert!(!VaultFailure::Status { code, error: None }.may_have_landed());
        }
        assert_eq!(VaultNotice::MayHaveChanged.message(), MAY_HAVE_CHANGED);
        assert!(VaultNotice::MayHaveChanged.warns());
        assert_eq!(VaultNotice::Saved.message(), ENTRY_UPDATED);
        assert!(!VaultNotice::Saved.warns());
        // The page outwaits the server's own bounds: a 30 s keychain read
        // and a 25 s `op` call.
        assert_eq!(VAULT_CALL_TIMEOUT_MS, 75_000);
    }

    /// Compiles only while `T` implements neither `Debug` nor `Clone`: with
    /// either, the trait call below has two candidate impls and is ambiguous.
    macro_rules! assert_no_debug_or_clone {
        ($($ty:ty),+ $(,)?) => {{
            trait AmbiguousIfDebug<A> {
                fn check() {}
            }
            impl<T: ?Sized> AmbiguousIfDebug<()> for T {}
            struct IsDebug;
            impl<T: ?Sized + std::fmt::Debug> AmbiguousIfDebug<IsDebug> for T {}
            trait AmbiguousIfClone<A> {
                fn check() {}
            }
            impl<T: ?Sized> AmbiguousIfClone<()> for T {}
            struct IsClone;
            impl<T: ?Sized + Clone> AmbiguousIfClone<IsClone> for T {}
            $(
                <$ty as AmbiguousIfDebug<_>>::check();
                <$ty as AmbiguousIfClone<_>>::check();
            )+
        }};
    }
    pub(super) use assert_no_debug_or_clone;

    #[test]
    fn scopes_and_kinds_have_stable_wire_shapes() {
        assert_eq!(
            serde_json::to_value(CredentialScope::Bruh {
                agent_id: "a".into()
            })
            .unwrap(),
            json!({"type": "bruh", "agent_id": "a"})
        );
        assert_eq!(CredentialScope::Global.key(), "global");
        assert_eq!(
            CredentialScope::Bruh {
                agent_id: "a1".into()
            }
            .key(),
            "bruh:a1"
        );
        let project = CredentialScope::Project { path: "/p".into() }.key();
        assert!(project.starts_with("project:") && project.len() == "project:".len() + 16);
        let login: VaultKind =
            serde_json::from_value(json!({"kind": "login", "url": "https://a.test/in",
            "fields": {"source": "saved", "username": "me", "totp": true}}))
            .unwrap();
        let VaultKind::Login(meta) = &login else {
            panic!()
        };
        assert_eq!(
            meta.fields(),
            [
                SecretField::Username,
                SecretField::Password,
                SecretField::Totp
            ]
        );
        let entry: VaultEntry = serde_json::from_value(
            json!({"id": "v1-x", "name": "N", "scope": {"type": "global"},
            "kind": {"kind": "value"}, "state": "saved", "future": 1}),
        )
        .unwrap();
        assert_eq!(entry.attention, None);
    }

    #[test]
    fn project_scope_keys_hash_the_path() {
        let key = CredentialScope::Project { path: "/p".into() }.key();
        let hash = format!("{:x}", Sha256::digest(b"/p"));
        assert_eq!(key, format!("project:{}", &hash[..16]));
        assert_ne!(key, CredentialScope::Project { path: "/q".into() }.key());
        assert_eq!(SecretField::Totp.label(), "one-time code");
        let listing: VaultListing = serde_json::from_value(json!({})).unwrap();
        assert_eq!(listing, VaultListing::default());
    }

    #[test]
    fn login_origins_are_exact() {
        assert_eq!(
            login_origin("https://Accounts.Example.com:443/sign-in?x=1"),
            Ok("https://accounts.example.com".into())
        );
        assert_eq!(
            login_origin("http://192.168.1.4:8080/"),
            Ok("http://192.168.1.4:8080".into())
        );
        assert!(login_origin("http://localhost:3001/").is_err());
        assert!(login_origin("https://ex%61mple.com/").is_err());
        assert!(login_origin("ftp://a.test/").is_err());
        assert!(same_origin(Some("https://a.test/after"), "https://a.test"));
        assert!(!same_origin(Some("http://a.test/"), "https://a.test"));
        assert!(!same_origin(Some("https://www.a.test/"), "https://a.test"));
        assert!(!same_origin(None, "https://a.test"));
    }

    #[test]
    fn login_origins_refuse_odd_pages_with_their_copy() {
        assert_eq!(login_origin("ftp://a.test/"), Err(SIGN_IN_SCHEME));
        assert_eq!(login_origin("about:blank"), Err(SIGN_IN_SCHEME));
        assert_eq!(login_origin("http://127.1/"), Err(SIGN_IN_LOCAL));
        assert_eq!(login_origin("https://[::1]:3001/"), Err(SIGN_IN_LOCAL));
        assert_eq!(login_origin("https://bruhs.localhost/"), Err(SIGN_IN_LOCAL));
        assert_eq!(login_origin("https://exämple.com/"), Err(SIGN_IN_ENCODED));
        assert_eq!(
            login_origin("HTTP://A.test:80/x"),
            Ok("http://a.test".into())
        );
        assert_eq!(
            login_origin("http://a.test:443/"),
            Ok("http://a.test:443".into())
        );
        assert_eq!(
            login_origin("https://[2001:db8::1]:8443/"),
            Ok("https://[2001:db8::1]:8443".into())
        );
        for bad in [
            "https://a.test:0/",
            "https://a.test:99999/",
            "https://a.test:x/",
            "https://:443/",
            "https://a*b.test/",
            "https:///in",
            "https://[2001:db8::g]/",
            "https://[2001:db8::1]x/",
        ] {
            assert_eq!(login_origin(bad), Err(SIGN_IN_SCHEME), "{bad}");
        }
        for bad in [
            "https://me:pw@a.test/",
            "https://me@a.test/",
            "https://a.test/ x",
            "https://a.test\\@b.test/",
        ] {
            assert_eq!(login_origin(bad), Err(SIGN_IN_ADDRESS), "{bad}");
        }
        assert!(!same_origin(Some("https://a.test:8443/"), "https://a.test"));
        assert!(same_origin(Some("HTTPS://A.TEST:443/"), "https://a.test"));
    }

    #[test]
    fn ip_hosts_read_as_chrome_reports_them() {
        for (url, origin) in [
            ("http://192.168.001.004/", "http://192.168.1.4"),
            ("http://3232235780/", "http://192.168.1.4"),
            ("http://0xC0A80104:8080/", "http://192.168.1.4:8080"),
            ("http://192.168.260/", "http://192.168.1.4"),
            ("http://010.0.0.1/", "http://8.0.0.1"),
            ("http://8.8.8.8./", "http://8.8.8.8"),
            ("https://[2001:db8:0:0::1]/", "https://[2001:db8::1]"),
            (
                "https://[2001:DB8:0000:0000:0000:0000:0000:0001]/",
                "https://[2001:db8::1]",
            ),
            (
                "https://[2001:db8:0:1:0:0:0:1]/",
                "https://[2001:db8:0:1::1]",
            ),
            ("https://[2001:0:0:1:0:0:1:1]/", "https://[2001::1:0:0:1:1]"),
            (
                "https://[2001:db8:1:1:1:1:0:1]/",
                "https://[2001:db8:1:1:1:1:0:1]",
            ),
            ("https://[::ffff:8.8.8.8]/", "https://[::ffff:808:808]"),
        ] {
            assert_eq!(login_origin(url).as_deref(), Ok(origin), "{url}");
            assert!(same_origin(Some(url), origin), "{url}");
        }
        assert!(same_origin(
            Some("http://192.168.1.4/x"),
            "http://192.168.1.4"
        ));
        for local in [
            "http://0177.0.0.1/",
            "http://2130706433/",
            "https://[0:0:0:0:0:0:0:1]/",
        ] {
            assert_eq!(login_origin(local), Err(SIGN_IN_LOCAL), "{local}");
        }
        for bad in [
            "http://1.2.3.256/",
            "http://a.123/",
            "http://1.2.3.4.5/",
            "http://09.1.1.1/",
        ] {
            assert_eq!(login_origin(bad), Err(SIGN_IN_SCHEME), "{bad}");
        }
        assert_eq!(login_origin("http://a1.test/"), Ok("http://a1.test".into()));
    }

    #[test]
    fn references_and_names_are_checked() {
        assert!(validate_op_reference("op://Private/GitHub/password").is_ok());
        assert!(validate_op_reference(
            "op://Private/GitHub/Security/one-time password?attribute=otp"
        )
        .is_ok());
        assert_eq!(
            validate_op_reference("op://Private/GitHub"),
            Err("Use a reference like op://Vault/Item/field.")
        );
        assert!(
            validate_op_reference("op://a//b").is_err()
                && validate_op_reference("op://a/b/c\n").is_err()
        );
        assert_eq!(validate_entry_name("  "), Err("Enter a name."));
        assert!(validate_entry_name(&"x".repeat(81)).is_err());
        assert!(validate_entry_name("A\u{202e}B").is_err());
    }

    #[test]
    fn references_and_names_refuse_the_rest() {
        for bad in [
            "op://a/b/c/d/e",
            "https://a/b/c",
            "op://a/b/c?ssh-format=openssh",
            "op://a/b/c?attribute=",
            "op://a/b/c?attribute=o t p",
            "op://a/b/\u{200b}c",
            "op://a/b/$(id)",
            "op://a/b/`id`",
            "op://a/b/c;rm x",
            "op://a/b/c|d",
            "op://a/b/\"c\"",
            "op://a/b/'c'",
            "op://a/b/<c>",
            "op://a/b/c&d",
            "op://Priv\u{0430}te/b/c",
            "op://a/ /c",
        ] {
            assert_eq!(validate_op_reference(bad), Err(REFERENCE_SHAPE), "{bad}");
        }
        let long = format!("op://v/i/{}", "f".repeat(MAX_REFERENCE_BYTES));
        assert_eq!(validate_op_reference(&long), Err(REFERENCE_SHAPE));
        assert_eq!(validate_entry_name(&"x".repeat(81)), Err(NAME_TOO_LONG));
        assert!(validate_entry_name(&format!(" {} ", "x".repeat(80))).is_ok());
        assert_eq!(validate_entry_name("A\u{202e}B"), Err(NAME_CHARACTERS));
        assert_eq!(validate_entry_name("A\tB"), Err(NAME_CHARACTERS));
    }

    fn saved(password: Option<&str>) -> VaultSecretInput {
        VaultSecretInput::Login {
            url: "https://github.com/login".into(),
            fields: LoginInput::Saved {
                username: "me".into(),
                password: password.map(Into::into),
                totp: TotpInput::Keep,
            },
        }
    }

    fn github_login() -> VaultKind {
        VaultKind::Login(LoginMeta {
            url: "https://github.com/login".into(),
            fields: LoginFields::Saved {
                username: "me".into(),
                totp: false,
            },
        })
    }

    #[test]
    fn creates_and_updates_follow_their_kind() {
        let create = |secret| VaultCreate {
            name: "GitHub".into(),
            scope: CredentialScope::Global,
            secret,
        };
        assert!(create(saved(Some("pw"))).validate().is_ok());
        assert_eq!(create(saved(None)).validate(), Err("Enter a password."));
        assert_eq!(
            create(VaultSecretInput::Value { value: None }).validate(),
            Err("Enter a value.")
        );
        let existing = VaultKind::Value;
        assert_eq!(
            VaultUpdate {
                secret: saved(None)
            }
            .validate(&existing),
            Err("Keep the entry's kind. Delete it and create another to change it.")
        );
        assert!(VaultUpdate {
            secret: VaultSecretInput::Value { value: None }
        }
        .validate(&existing)
        .is_ok());

        // Ruling F3: a login's origin stays; its path, username and values can change.
        let login = github_login();
        let moved = VaultUpdate {
            secret: VaultSecretInput::Login {
                url: "https://evil.test/login".into(),
                fields: LoginInput::Saved {
                    username: "me".into(),
                    password: None,
                    totp: TotpInput::Keep,
                },
            },
        };
        assert_eq!(
            moved.validate(&login),
            Err("Delete it and create it again to use it on another site.")
        );
        let new_path = VaultUpdate {
            secret: VaultSecretInput::Login {
                url: "https://GitHub.com:443/session".into(),
                fields: LoginInput::Saved {
                    username: "someone".into(),
                    password: None,
                    totp: TotpInput::Keep,
                },
            },
        };
        assert!(new_path.validate(&login).is_ok());
    }

    #[test]
    fn creates_check_scopes_values_and_logins() {
        let create = |scope, secret| VaultCreate {
            name: "N".into(),
            scope,
            secret,
        };
        let value = |value: &str| VaultSecretInput::Value {
            value: Some(value.into()),
        };
        let global = || CredentialScope::Global;
        assert_eq!(
            create(CredentialScope::Project { path: " ".into() }, value("v")).validate(),
            Err(ENTER_PROJECT)
        );
        assert_eq!(
            create(
                CredentialScope::Bruh {
                    agent_id: String::new()
                },
                value("v")
            )
            .validate(),
            Err(CHOOSE_BRUH)
        );
        assert_eq!(create(global(), value("")).validate(), Err(ENTER_VALUE));
        assert_eq!(create(global(), value("a\0b")).validate(), Err(VALUE_NUL));
        assert!(create(global(), value(&"v".repeat(MAX_VALUE_BYTES)))
            .validate()
            .is_ok());
        assert_eq!(
            create(global(), value(&"v".repeat(MAX_VALUE_BYTES + 1))).validate(),
            Err(VALUE_TOO_LONG)
        );
        let login = |username: &str, password: &str, totp| VaultSecretInput::Login {
            url: "https://a.test/in".into(),
            fields: LoginInput::Saved {
                username: username.into(),
                password: Some(password.into()),
                totp,
            },
        };
        assert_eq!(
            create(
                global(),
                login("me", &"p".repeat(MAX_PASSWORD_BYTES + 1), TotpInput::Keep)
            )
            .validate(),
            Err(PASSWORD_TOO_LONG)
        );
        assert_eq!(
            create(global(), login("me", "", TotpInput::Keep)).validate(),
            Err(ENTER_PASSWORD)
        );
        assert_eq!(
            create(global(), login(&"u".repeat(257), "pw", TotpInput::Keep)).validate(),
            Err(USERNAME_TOO_LONG)
        );
        assert_eq!(
            create(global(), login("m\ne", "pw", TotpInput::Keep)).validate(),
            Err(USERNAME_CHARACTERS)
        );
        assert_eq!(
            create(
                global(),
                login(
                    "me",
                    "pw",
                    TotpInput::Set {
                        setup: "GEZD1".into()
                    }
                )
            )
            .validate(),
            Err("Enter a setup key of at least 16 letters and digits from 2 to 7.")
        );
        assert!(create(
            global(),
            login(
                "me",
                "pw",
                TotpInput::Set {
                    setup: "GEZDGNBVGY3TQOJQ".into()
                }
            )
        )
        .validate()
        .is_ok());
        let local = VaultSecretInput::Login {
            url: "http://localhost:3001/".into(),
            fields: LoginInput::Saved {
                username: "me".into(),
                password: Some("pw".into()),
                totp: TotpInput::Keep,
            },
        };
        assert_eq!(create(global(), local).validate(), Err(SIGN_IN_LOCAL));
        let one_password = |totp: Option<&str>| VaultSecretInput::Login {
            url: "https://a.test/in".into(),
            fields: LoginInput::OnePassword {
                username: "op://P/A/username".into(),
                password: "op://P/A/password".into(),
                totp: totp.map(Into::into),
            },
        };
        assert!(
            create(global(), one_password(Some("op://P/A/otp?attribute=otp")))
                .validate()
                .is_ok()
        );
        assert_eq!(
            create(global(), one_password(Some("op://P/A"))).validate(),
            Err(REFERENCE_SHAPE)
        );
        assert_eq!(
            create(
                global(),
                VaultSecretInput::OnePassword {
                    reference: "nope".into()
                }
            )
            .validate(),
            Err(REFERENCE_SHAPE)
        );
        let mut bad_name = create(global(), value("v"));
        bad_name.name = " ".into();
        assert_eq!(bad_name.validate(), Err(ENTER_NAME));
        // A body's name is stored trimmed.
        let mut padded: VaultCreate = serde_json::from_value(json!({"name": " GitHub ",
            "scope": {"type": "global"}, "secret": {"kind": "value", "value": "v"}}))
        .unwrap();
        assert!(padded.validate().is_ok());
        assert_eq!(padded.name, "GitHub");
    }

    #[test]
    fn updates_keep_missing_values_and_switch_sources_with_one() {
        let value = VaultUpdate {
            secret: VaultSecretInput::Value {
                value: Some(String::new()),
            },
        };
        assert_eq!(value.validate(&VaultKind::Value), Err(ENTER_VALUE));
        assert!(VaultUpdate {
            secret: saved(None)
        }
        .validate(&github_login())
        .is_ok());
        let from_one_password = VaultKind::Login(LoginMeta {
            url: "https://github.com/login".into(),
            fields: LoginFields::OnePassword {
                username: "op://P/G/username".into(),
                password: "op://P/G/password".into(),
                totp: None,
            },
        });
        assert_eq!(
            VaultUpdate {
                secret: saved(None)
            }
            .validate(&from_one_password),
            Err(ENTER_PASSWORD)
        );
        assert!(VaultUpdate {
            secret: saved(Some("pw"))
        }
        .validate(&from_one_password)
        .is_ok());
        let reference = VaultKind::OnePassword {
            reference: "op://P/A/b".into(),
        };
        assert_eq!(
            VaultUpdate {
                secret: VaultSecretInput::Value { value: None }
            }
            .validate(&reference),
            Err(KIND_FIXED)
        );
        assert_eq!(
            VaultUpdate {
                secret: VaultSecretInput::OnePassword {
                    reference: "op://P/A".into()
                }
            }
            .validate(&reference),
            Err(REFERENCE_SHAPE)
        );
    }

    #[test]
    fn kind_meta_keeps_metadata_only() {
        assert!(VaultSecretInput::Value {
            value: Some("v".into())
        }
        .kind_meta()
        .is_none());
        let with_code = VaultSecretInput::Login {
            url: "https://a.test/in".into(),
            fields: LoginInput::Saved {
                username: "me".into(),
                password: Some("pw".into()),
                totp: TotpInput::Set {
                    setup: "GEZDGNBVGY3TQOJQ".into(),
                },
            },
        };
        assert_eq!(
            with_code.kind_meta(),
            Some(VaultKind::Login(LoginMeta {
                url: "https://a.test/in".into(),
                fields: LoginFields::Saved {
                    username: "me".into(),
                    totp: true
                },
            }))
        );
        let VaultKind::Login(mut meta) = saved(Some("pw")).kind_meta().unwrap() else {
            panic!()
        };
        assert_eq!(
            meta.fields(),
            [SecretField::Username, SecretField::Password]
        );
        // A Saved login without a username offers no username to fill.
        meta.fields = LoginFields::Saved {
            username: String::new(),
            totp: true,
        };
        assert_eq!(meta.fields(), [SecretField::Password, SecretField::Totp]);
        assert_eq!(meta.origin().as_deref(), Some("https://github.com"));
    }

    /// A random marker that can't appear by chance. Its `1` keeps it from
    /// ever being valid base32.
    pub(super) fn canary() -> String {
        format!("canary1{}", uuid::Uuid::new_v4().simple())
    }

    #[test]
    fn values_never_reach_metadata_debug_or_errors() {
        assert_no_debug_or_clone!(
            VaultCreate,
            VaultUpdate,
            VaultSecretInput,
            LoginInput,
            TotpInput,
            TotpWire
        );
        let (password, setup_word, value) = (canary(), canary(), canary());
        let login = VaultSecretInput::Login {
            url: "https://a.test/in".into(),
            fields: LoginInput::Saved {
                username: "me".into(),
                password: Some(password.clone()),
                totp: TotpInput::Set {
                    setup: format!("GEZDGNBVGY3TQOJQ{setup_word}"),
                },
            },
        };
        let meta = login.kind_meta().unwrap();
        let entry = VaultEntry {
            id: "v1-a".into(),
            name: "N".into(),
            scope: CredentialScope::Global,
            kind: meta,
            state: SavedCredentialState::Saved,
            attention: None,
        };
        let listing = VaultListing {
            entries: vec![entry.clone()],
            ..VaultListing::default()
        };
        let shown = [
            format!("{entry:?}"),
            format!("{listing:?}"),
            serde_json::to_string(&listing).unwrap(),
        ];
        for text in &shown {
            assert!(
                !text.contains(&password) && !text.contains(&setup_word),
                "{text}"
            );
        }
        // Validation answers with fixed copy, never the input.
        let mut create = VaultCreate {
            name: "N".into(),
            scope: CredentialScope::Global,
            secret: login,
        };
        let error = create.validate().unwrap_err();
        assert!(!error.contains(&setup_word));
        let mut nul = VaultCreate {
            name: "N".into(),
            scope: CredentialScope::Global,
            secret: VaultSecretInput::Value {
                value: Some(format!("{value}\0")),
            },
        };
        assert_eq!(nul.validate(), Err(VALUE_NUL));
    }

    #[test]
    fn write_only_bodies_refuse_unknown_fields() {
        let body = |secret: serde_json::Value| {
            serde_json::from_value::<VaultCreate>(
                json!({"name": "N", "scope": {"type": "global"}, "secret": secret}),
            )
        };
        assert!(body(json!({"kind": "value", "value": "v"})).is_ok());
        assert!(body(json!({"kind": "value", "value": "v", "extra": 1})).is_err());
        assert!(body(json!({"kind": "login", "url": "https://a.test",
            "fields": {"source": "saved", "username": "me", "password": "pw",
                "totp": {"action": "set", "setup": "GEZDGNBVGY3TQOJQ"}}}))
        .is_ok());
        assert!(body(json!({"kind": "login", "url": "https://a.test",
            "fields": {"source": "saved", "username": "me", "secret": "pw"}}))
        .is_err());
        let update: VaultUpdate =
            serde_json::from_value(json!({"secret": {"kind": "login", "url": "https://a.test",
                "fields": {"source": "saved", "username": "me"}}}))
            .unwrap();
        assert!(matches!(
            update.secret,
            VaultSecretInput::Login {
                fields: LoginInput::Saved {
                    password: None,
                    totp: TotpInput::Keep,
                    ..
                },
                ..
            }
        ));
        assert!(serde_json::from_value::<VaultUpdate>(
            json!({"secret": {"kind": "value"}, "name": "renamed"})
        )
        .is_err());
        // Every action reads back what a client sends.
        for input in [
            TotpInput::Keep,
            TotpInput::Remove,
            TotpInput::Set {
                setup: "GEZDGNBVGY3TQOJQ".into(),
            },
        ] {
            let wire = serde_json::to_value(&input).unwrap();
            let back: TotpInput = serde_json::from_value(wire.clone()).unwrap();
            assert_eq!(serde_json::to_value(&back).unwrap(), wire);
        }
        assert_eq!(
            serde_json::to_value(TotpInput::Remove).unwrap(),
            json!({"action": "remove"})
        );
        // A setup key beside Keep is refused, not dropped.
        assert!(body(json!({"kind": "login", "url": "https://a.test",
            "fields": {"source": "saved", "username": "me", "password": "pw",
                "totp": {"action": "keep", "setup": "GEZDGNBVGY3TQOJQ"}}}))
        .is_err());
    }
}
