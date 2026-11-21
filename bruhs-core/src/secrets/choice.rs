//! Where keychain items live, from `BRUHS_SECRET_STORE`: the OS keychain,
//! or, in a debug build, memory (D25) or a file outside the data roots
//! (D26). The server, its restore applier and the desktop's own restore
//! all read it here, so a test run never reaches the owner's keychain.

use std::path::{Path, PathBuf};

use super::SecretStore;

/// The variable the acceptance scripts set.
pub const SECRET_STORE_ENV: &str = "BRUHS_SECRET_STORE";

/// Where keychain items live.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretChoice {
    /// The OS keychain.
    Os,
    /// `memory`: in memory, gone on exit (D25).
    Memory,
    /// `file:<dir>`: `<dir>/items.json`, which outlives the process (D26).
    File(PathBuf),
}

impl SecretChoice {
    /// This process's choice, from [`SECRET_STORE_ENV`].
    pub fn from_env() -> Self {
        secret_store_choice(std::env::var(SECRET_STORE_ENV).ok().as_deref())
    }

    /// A store for this choice.
    pub fn store(&self) -> SecretStore {
        match self {
            Self::Os => SecretStore::os(),
            Self::Memory => SecretStore::in_memory(),
            #[cfg(any(test, debug_assertions))]
            Self::File(dir) => SecretStore::test_file(dir.clone()),
            // `secret_store_choice` never answers it in a release build.
            #[cfg(not(any(test, debug_assertions)))]
            Self::File(_) => SecretStore::os(),
        }
    }
}

/// What `value` asks for: `memory` or `file:<absolute dir>`. Only a debug
/// build honors it; a release build warns and keeps the OS keychain.
pub fn secret_store_choice(value: Option<&str>) -> SecretChoice {
    let Some(value) = value else {
        return SecretChoice::Os;
    };
    let choice = match value.strip_prefix("file:") {
        Some(dir) if Path::new(dir).is_absolute() => SecretChoice::File(dir.into()),
        Some(_) => {
            tracing::warn!("BRUHS_SECRET_STORE=file: needs an absolute folder; ignoring it");
            return SecretChoice::Os;
        }
        None if value == "memory" => SecretChoice::Memory,
        None => {
            tracing::warn!("BRUHS_SECRET_STORE accepts only memory or file:<dir>; ignoring it");
            return SecretChoice::Os;
        }
    };
    if !cfg!(debug_assertions) {
        tracing::warn!("BRUHS_SECRET_STORE is ignored in a release build");
        return SecretChoice::Os;
    }
    match &choice {
        SecretChoice::Memory => {
            tracing::warn!("BRUHS_SECRET_STORE=memory: secrets live in memory and go on exit");
        }
        SecretChoice::File(_) => {
            tracing::warn!("BRUHS_SECRET_STORE=file: keychain items live in a test file");
        }
        SecretChoice::Os => {}
    }
    choice
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run in release too (`cargo test --release -p bruhs-core --lib --
    /// secret_store_choice`): a release build refuses every choice.
    #[test]
    fn secret_store_choices() {
        let debug = |choice: SecretChoice| {
            if cfg!(debug_assertions) {
                choice
            } else {
                SecretChoice::Os
            }
        };
        assert_eq!(secret_store_choice(None), SecretChoice::Os);
        assert_eq!(
            secret_store_choice(Some("memory")),
            debug(SecretChoice::Memory)
        );
        assert_eq!(
            secret_store_choice(Some("file:/x")),
            debug(SecretChoice::File("/x".into()))
        );
        for ignored in ["os", "Memory", "file:", "file:relative/keys", "File:/x"] {
            assert_eq!(
                secret_store_choice(Some(ignored)),
                SecretChoice::Os,
                "{ignored}"
            );
        }
    }
}
