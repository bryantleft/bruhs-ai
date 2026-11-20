//! Keys, the chunked STREAM writer and reader, and sealed sets.

use std::io::{self, Read, Write};

use aead::stream::{NewStream, StreamBE32, StreamPrimitive};
use aes_gcm::{Aes256Gcm, KeyInit};
use argon2::{Algorithm, Argon2, Params, Version};
use zeroize::Zeroizing;

use crate::{FormatError, Header, Kdf, Kind, HEADER_LEN};

/// AES-GCM tag bytes at the end of every chunk.
const TAG_LEN: usize = 16;

type Stream = StreamBE32<Aes256Gcm>;

/// A 32-byte AES-256 key. It is wiped on drop and never printed or copied.
pub struct Key(Zeroizing<[u8; 32]>);

impl Key {
    /// Runs Argon2id over `password` with the header's salt and parameters.
    /// A raw-key header has no password, so it is `NotABackup`.
    pub fn derive(password: &str, header: &Header) -> Result<Self, FormatError> {
        let Kdf::Argon2id(params) = header.kdf else {
            return Err(FormatError::NotABackup);
        };
        header.validate()?;
        let params = Params::new(params.memory_kib, params.passes, params.lanes, Some(32))
            .map_err(|_| FormatError::BadParams)?;
        let mut key = Zeroizing::new([0; 32]);
        Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
            .hash_password_into(password.as_bytes(), &header.salt, &mut key[..])
            .map_err(|_| FormatError::BadParams)?;
        Ok(Self(key))
    }

    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(Zeroizing::new(bytes))
    }

    /// The raw key, only for storing a sealed set's key.
    pub fn expose(&self) -> &[u8; 32] {
        &self.0
    }

    fn stream(&self, header: &Header) -> Stream {
        let aead = Aes256Gcm::new(self.expose().into());
        Stream::from_aead(aead, (&header.nonce_prefix).into())
    }
}

fn chunk_len(header: &Header) -> Result<usize, FormatError> {
    usize::try_from(header.chunk_bytes).map_err(|_| FormatError::BadParams)
}

/// Writes the header, then the plaintext in sealed chunks. Only `finish` seals
/// the last chunk, so a stream that was never finished reads as truncated.
pub struct Encryptor<W: Write> {
    writer: W,
    stream: Stream,
    aad: [u8; HEADER_LEN],
    chunk: usize,
    /// The chunk being filled; sealed in place, so it holds `chunk + TAG_LEN`
    /// without growing.
    buf: Zeroizing<Vec<u8>>,
    position: u32,
    broken: bool,
}

impl<W: Write> Encryptor<W> {
    pub fn new(mut writer: W, header: &Header, key: &Key) -> io::Result<Self> {
        header.validate().map_err(io::Error::other)?;
        let chunk = chunk_len(header).map_err(io::Error::other)?;
        let aad = header.to_bytes();
        writer.write_all(&aad)?;
        Ok(Self {
            writer,
            stream: key.stream(header),
            aad,
            chunk,
            buf: Zeroizing::new(Vec::with_capacity(chunk + TAG_LEN)),
            position: 0,
            broken: false,
        })
    }

    fn check(&self) -> io::Result<()> {
        if self.broken {
            Err(io::Error::other("backup writer stopped after an error"))
        } else {
            Ok(())
        }
    }

    fn seal_chunk(&mut self, last: bool) -> io::Result<()> {
        self.check()?;
        self.broken = true;
        let next = if last {
            Some(self.position)
        } else {
            self.position.checked_add(1)
        };
        let next = next.ok_or_else(|| io::Error::other("backup is too long"))?;
        self.stream
            .encrypt_in_place(self.position, last, &self.aad, &mut *self.buf)
            .map_err(|_| io::Error::other("backup chunk could not be sealed"))?;
        self.writer.write_all(&self.buf)?;
        self.buf.clear();
        self.position = next;
        self.broken = false;
        Ok(())
    }

    /// Seals the last chunk (empty when the plaintext ended on a chunk
    /// boundary) and hands back the writer.
    pub fn finish(mut self) -> io::Result<W> {
        self.seal_chunk(true)?;
        self.writer.flush()?;
        let Self { writer, .. } = self;
        Ok(writer)
    }
}

impl<W: Write> Write for Encryptor<W> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.check()?;
        if data.is_empty() {
            return Ok(0);
        }
        // A full chunk is sealed as not last only once more bytes arrive.
        if self.buf.len() == self.chunk {
            self.seal_chunk(false)?;
        }
        let take = data.len().min(self.chunk - self.buf.len());
        self.buf.extend_from_slice(&data[..take]);
        Ok(take)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.check()?;
        self.writer.flush()
    }
}

/// What opens a stream: a password for Argon2id headers, a key for raw-key
/// (sealed) headers.
pub enum Secret<'a> {
    Password(&'a str),
    Key(&'a Key),
}

/// Reads a stream back. `open` checks the header and decrypts the first
/// chunk; `Read` yields the plaintext; `finish` checks the stream's end.
pub struct Decryptor<R: Read> {
    reader: R,
    header: Header,
    aad: [u8; HEADER_LEN],
    stream: Stream,
    chunk: usize,
    /// Read-ahead: one sealed chunk plus one byte, to tell a last chunk from
    /// one that has more after it.
    ahead: Vec<u8>,
    ahead_len: usize,
    plain: Zeroizing<Vec<u8>>,
    at: usize,
    position: u32,
    done: bool,
    failed: Option<FormatError>,
}

impl<R: Read> Decryptor<R> {
    pub fn open(mut reader: R, secret: Secret<'_>) -> Result<Self, FormatError> {
        let mut bytes = [0; HEADER_LEN];
        let got = read_up_to(&mut reader, &mut bytes)?;
        if got < HEADER_LEN {
            let seen = got.min(crate::MAGIC.len());
            return Err(if got == 0 || bytes[..seen] != crate::MAGIC[..seen] {
                FormatError::NotABackup
            } else {
                FormatError::Truncated
            });
        }
        let header = Header::parse(&bytes)?;
        let derived;
        let key = match (secret, header.kdf) {
            (Secret::Password(password), Kdf::Argon2id(_)) => {
                derived = Key::derive(password, &header)?;
                &derived
            }
            (Secret::Key(key), Kdf::RawKey) => key,
            _ => return Err(FormatError::NotABackup),
        };
        let chunk = chunk_len(&header)?;
        let mut decryptor = Self {
            reader,
            stream: key.stream(&header),
            aad: bytes,
            header,
            chunk,
            ahead: vec![0; chunk + TAG_LEN + 1],
            ahead_len: 0,
            plain: Zeroizing::new(Vec::with_capacity(chunk + TAG_LEN)),
            at: 0,
            position: 0,
            done: false,
            failed: None,
        };
        decryptor.advance()?;
        Ok(decryptor)
    }

    pub fn header(&self) -> &Header {
        &self.header
    }

    /// Checks that the last chunk was reached and no plaintext was left
    /// unread, reading through any chunks the caller didn't.
    pub fn finish(mut self) -> Result<R, FormatError> {
        if let Some(error) = self.failed.take() {
            return Err(error);
        }
        let mut unread = self.at < self.plain.len();
        while !self.done {
            self.advance()?;
            unread |= !self.plain.is_empty();
        }
        if unread {
            return Err(FormatError::TrailingData);
        }
        let Self { reader, .. } = self;
        Ok(reader)
    }

    /// Decrypts the next chunk into `plain`, remembering the first failure.
    fn advance(&mut self) -> Result<(), FormatError> {
        if let Some(error) = &self.failed {
            return Err(error.again());
        }
        let result = self.next_chunk();
        if let Err(error) = &result {
            self.plain.clear();
            self.failed = Some(error.again());
        }
        result
    }

    fn next_chunk(&mut self) -> Result<(), FormatError> {
        let sealed = self.chunk + TAG_LEN;
        self.fill()?;
        let len = self.ahead_len;
        if len > sealed {
            // A full chunk with more after it is not the last.
            if self.decrypt(sealed, false).is_err() {
                return Err(if self.decrypt(sealed, true).is_ok() {
                    FormatError::TrailingData
                } else {
                    FormatError::WrongPasswordOrDamaged
                });
            }
            self.position = self
                .position
                .checked_add(1)
                .ok_or(FormatError::WrongPasswordOrDamaged)?;
            self.ahead.copy_within(sealed..len, 0);
            self.ahead_len = len - sealed;
        } else if len == sealed {
            // A full chunk with nothing after it must be the last.
            self.decrypt(sealed, true)
                .map_err(|_| FormatError::Truncated)?;
            self.finish_stream();
        } else if len == 0 {
            // The stream ended on a chunk boundary without a last chunk.
            return Err(FormatError::Truncated);
        } else {
            self.decrypt(len, true)
                .map_err(|_| FormatError::WrongPasswordOrDamaged)?;
            self.finish_stream();
        }
        Ok(())
    }

    fn finish_stream(&mut self) {
        self.done = true;
        self.ahead_len = 0;
    }

    /// Fills the read-ahead buffer, stopping early only at the end of input.
    fn fill(&mut self) -> Result<(), FormatError> {
        self.ahead_len += read_up_to(&mut self.reader, &mut self.ahead[self.ahead_len..])?;
        Ok(())
    }

    /// Decrypts the first `len` read-ahead bytes into `plain`. The ciphertext
    /// stays in place, so a failed try can be retried with the other flag.
    fn decrypt(&mut self, len: usize, last: bool) -> Result<(), aead::Error> {
        self.plain.clear();
        self.plain.extend_from_slice(&self.ahead[..len]);
        self.at = 0;
        let result = self
            .stream
            .decrypt_in_place(self.position, last, &self.aad, &mut *self.plain);
        if result.is_err() {
            self.plain.clear();
        }
        result
    }
}

impl<R: Read> Read for Decryptor<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if let Some(error) = &self.failed {
            return Err(io::Error::other(error.again()));
        }
        if out.is_empty() {
            return Ok(0);
        }
        while self.at == self.plain.len() {
            if self.done {
                return Ok(0);
            }
            self.advance().map_err(io::Error::other)?;
        }
        let take = out.len().min(self.plain.len() - self.at);
        out[..take].copy_from_slice(&self.plain[self.at..self.at + take]);
        self.at += take;
        Ok(take)
    }
}

/// Reads until `buf` is full or the input ends; returns the bytes read.
fn read_up_to(reader: &mut impl Read, buf: &mut [u8]) -> Result<usize, FormatError> {
    let mut got = 0;
    while got < buf.len() {
        match reader.read(&mut buf[got..]) {
            Ok(0) => break,
            Ok(n) => got += n,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(FormatError::Io(error)),
        }
    }
    Ok(got)
}

/// Encrypts `plain` under a raw key as a sealed set (kind 3). `random` gives
/// the header's salt and nonce prefix.
pub fn seal(plain: &[u8], key: &Key, random: [u8; 23]) -> Vec<u8> {
    let header = Header::new(Kind::Sealed, Kdf::RawKey, random);
    let chunks = plain.len() / crate::CHUNK_BYTES.try_into().unwrap_or(usize::MAX) + 1;
    let out = Vec::with_capacity(HEADER_LEN + plain.len() + chunks * TAG_LEN);
    // A sealed header always validates and writing to a Vec cannot fail; the
    // only other error is a stream of 2^32 chunks.
    let mut encryptor = Encryptor::new(out, &header, key).expect("a sealed header is valid");
    encryptor
        .write_all(plain)
        .expect("a sealed set fits in memory");
    encryptor.finish().expect("a sealed set fits in memory")
}

/// Opens a sealed set made by `seal`. Any other kind is `NotABackup`.
pub fn unseal(blob: &[u8], key: &Key) -> Result<Zeroizing<Vec<u8>>, FormatError> {
    let mut decryptor = Decryptor::open(blob, Secret::Key(key))?;
    if decryptor.header().kind != Kind::Sealed {
        return Err(FormatError::NotABackup);
    }
    let mut out = Zeroizing::new(Vec::with_capacity(blob.len()));
    loop {
        out.extend_from_slice(&decryptor.plain[decryptor.at..]);
        decryptor.at = decryptor.plain.len();
        if decryptor.done {
            break;
        }
        decryptor.advance()?;
    }
    decryptor.finish()?;
    Ok(out)
}
