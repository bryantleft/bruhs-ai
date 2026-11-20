//! The BRUHSBAK container: a 64-byte header, then AES-256-GCM STREAM chunks
//! (BE32 counter, 7-byte nonce prefix) whose associated data is the whole
//! header. The key comes from Argon2id over the password, or is a raw random
//! key for sealed sets.
//!
//! Header layout (all integers big-endian):
//!
//! | bytes  | field                                   |
//! |--------|-----------------------------------------|
//! | 0..8   | magic `BRUHSBAK`                        |
//! | 8..10  | format version (`u16`)                  |
//! | 10     | kind (1 full, 2 web part, 3 sealed)     |
//! | 11     | KDF (0 raw key, 1 Argon2id v0x13)       |
//! | 12..16 | Argon2 memory KiB (`u32`, 0 for raw)    |
//! | 16..20 | Argon2 passes (`u32`, 0 for raw)        |
//! | 20..24 | Argon2 lanes (`u32`, 0 for raw)         |
//! | 24..40 | salt                                    |
//! | 40..47 | STREAM nonce prefix                     |
//! | 47     | zero                                    |
//! | 48..52 | plaintext bytes per chunk (`u32`)       |
//! | 52..64 | zero                                    |
//!
//! This crate is wasm-safe: it does I/O only through `Read`/`Write` and takes
//! its random bytes from the caller.

use std::io;

mod stream;

pub use stream::{seal, unseal, Decryptor, Encryptor, Key, Secret};

pub const MAGIC: [u8; 8] = *b"BRUHSBAK";
pub const FORMAT_VERSION: u16 = 1;
pub const HEADER_LEN: usize = 64;
pub const CHUNK_BYTES: u32 = 1 << 20;

/// Bounds a header may ask for, checked before any derivation or allocation.
const MEMORY_KIB: (u32, u32) = (19_456, 1_048_576);
const PASSES: (u32, u32) = (2, 10);
const LANES: (u32, u32) = (1, 16);
const CHUNK_RANGE: (u32, u32) = (65_536, 8_388_608);
/// The most Argon2 memory a web part may ask of a browser tab (256 MiB).
const WEB_MEMORY_MAX_KIB: u32 = 262_144;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Full = 1,
    WebPart = 2,
    Sealed = 3,
}

impl Kind {
    const fn byte(self) -> u8 {
        match self {
            Self::Full => 1,
            Self::WebPart => 2,
            Self::Sealed => 3,
        }
    }

    const fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            1 => Some(Self::Full),
            2 => Some(Self::WebPart),
            3 => Some(Self::Sealed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KdfParams {
    pub memory_kib: u32,
    pub passes: u32,
    pub lanes: u32,
}

impl KdfParams {
    /// Full backups made by the server: 256 MiB, 3 passes.
    pub const NATIVE: Self = Self {
        memory_kib: 262_144,
        passes: 3,
        lanes: 1,
    };
    /// The browser's web part: 64 MiB, 3 passes (single-threaded wasm).
    pub const WEB: Self = Self {
        memory_kib: 65_536,
        passes: 3,
        lanes: 1,
    };
    /// The least a header may ask for. Tests use it to stay fast.
    pub const FLOOR: Self = Self {
        memory_kib: 19_456,
        passes: 2,
        lanes: 1,
    };

    pub fn check(&self) -> Result<(), FormatError> {
        let within = |value: u32, (low, high): (u32, u32)| (low..=high).contains(&value);
        if within(self.memory_kib, MEMORY_KIB)
            && within(self.passes, PASSES)
            && within(self.lanes, LANES)
        {
            Ok(())
        } else {
            Err(FormatError::BadParams)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kdf {
    RawKey,
    Argon2id(KdfParams),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub kind: Kind,
    pub kdf: Kdf,
    pub salt: [u8; 16],
    pub nonce_prefix: [u8; 7],
    pub chunk_bytes: u32,
}

impl Header {
    /// A header with `CHUNK_BYTES` chunks; `random` gives the salt (first 16
    /// bytes) then the nonce prefix (last 7).
    pub fn new(kind: Kind, kdf: Kdf, random: [u8; 23]) -> Self {
        let mut salt = [0; 16];
        let mut nonce_prefix = [0; 7];
        salt.copy_from_slice(&random[..16]);
        nonce_prefix.copy_from_slice(&random[16..]);
        Self {
            kind,
            kdf,
            salt,
            nonce_prefix,
            chunk_bytes: CHUNK_BYTES,
        }
    }

    pub fn to_bytes(&self) -> [u8; HEADER_LEN] {
        let mut bytes = [0; HEADER_LEN];
        bytes[..8].copy_from_slice(&MAGIC);
        bytes[8..10].copy_from_slice(&FORMAT_VERSION.to_be_bytes());
        bytes[10] = self.kind.byte();
        if let Kdf::Argon2id(params) = self.kdf {
            bytes[11] = 1;
            bytes[12..16].copy_from_slice(&params.memory_kib.to_be_bytes());
            bytes[16..20].copy_from_slice(&params.passes.to_be_bytes());
            bytes[20..24].copy_from_slice(&params.lanes.to_be_bytes());
        }
        bytes[24..40].copy_from_slice(&self.salt);
        bytes[40..47].copy_from_slice(&self.nonce_prefix);
        bytes[48..52].copy_from_slice(&self.chunk_bytes.to_be_bytes());
        bytes
    }

    /// Reads and checks a header without deriving or allocating anything.
    pub fn parse(bytes: &[u8; HEADER_LEN]) -> Result<Self, FormatError> {
        if bytes[..8] != MAGIC {
            return Err(FormatError::NotABackup);
        }
        match u16::from_be_bytes([bytes[8], bytes[9]]) {
            0 => return Err(FormatError::BadHeader),
            version if version > FORMAT_VERSION => return Err(FormatError::NewerVersion),
            _ => {}
        }
        if bytes[47] != 0 || bytes[52..].iter().any(|b| *b != 0) {
            return Err(FormatError::BadHeader);
        }
        let kind = Kind::from_byte(bytes[10]).ok_or(FormatError::BadHeader)?;
        let word = |at: usize| {
            u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
        };
        let kdf = match bytes[11] {
            0 if bytes[12..24].iter().all(|b| *b == 0) => Kdf::RawKey,
            1 => Kdf::Argon2id(KdfParams {
                memory_kib: word(12),
                passes: word(16),
                lanes: word(20),
            }),
            _ => return Err(FormatError::BadHeader),
        };
        let mut salt = [0; 16];
        let mut nonce_prefix = [0; 7];
        salt.copy_from_slice(&bytes[24..40]);
        nonce_prefix.copy_from_slice(&bytes[40..47]);
        let header = Self {
            kind,
            kdf,
            salt,
            nonce_prefix,
            chunk_bytes: word(48),
        };
        header.validate()?;
        Ok(header)
    }

    /// The checks a header must pass to be read or written: the kind pairs
    /// with its key method, and every parameter is in range.
    pub(crate) fn validate(&self) -> Result<(), FormatError> {
        match (self.kind, self.kdf) {
            (Kind::Full | Kind::WebPart, Kdf::Argon2id(params)) => {
                params.check()?;
                if self.kind == Kind::WebPart && params.memory_kib > WEB_MEMORY_MAX_KIB {
                    return Err(FormatError::BadParams);
                }
            }
            (Kind::Sealed, Kdf::RawKey) => {}
            _ => return Err(FormatError::BadHeader),
        }
        if (CHUNK_RANGE.0..=CHUNK_RANGE.1).contains(&self.chunk_bytes) {
            Ok(())
        } else {
            Err(FormatError::BadParams)
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FormatError {
    #[error("this file is not a Bruhs backup")]
    NotABackup,
    #[error("this backup comes from a newer Bruhs")]
    NewerVersion,
    #[error("the backup header is damaged")]
    BadHeader,
    #[error("the backup asks for settings out of range")]
    BadParams,
    #[error("wrong password, or the backup is damaged")]
    WrongPasswordOrDamaged,
    #[error("the backup ends early")]
    Truncated,
    #[error("the backup has data after its end")]
    TrailingData,
    #[error("the backup could not be read or written")]
    Io(#[from] io::Error),
}

impl FormatError {
    /// The same error again, for a reader that keeps failing after its first
    /// error (an io error keeps only its kind).
    fn again(&self) -> Self {
        match self {
            Self::NotABackup => Self::NotABackup,
            Self::NewerVersion => Self::NewerVersion,
            Self::BadHeader => Self::BadHeader,
            Self::BadParams => Self::BadParams,
            Self::WrongPasswordOrDamaged => Self::WrongPasswordOrDamaged,
            Self::Truncated => Self::Truncated,
            Self::TrailingData => Self::TrailingData,
            Self::Io(error) => Self::Io(error.kind().into()),
        }
    }
}

#[cfg(test)]
mod tests;
