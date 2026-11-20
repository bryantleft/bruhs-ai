use std::io::{Read, Write};

use super::*;

const R: [u8; 23] = [7; 23];
const PW: &str = "correct horse battery";
/// The first 16 bytes after the header of `sealed(b"bruhs", &full())`.
const KNOWN_ANSWER: &str = "cee082da472d93c9642077bdd70eb634";

fn chunk() -> usize {
    usize::try_from(CHUNK_BYTES).unwrap()
}

fn full() -> Header {
    Header::new(Kind::Full, Kdf::Argon2id(KdfParams::FLOOR), R)
}

fn sealed(plain: &[u8], h: &Header) -> Vec<u8> {
    let key = Key::derive(PW, h).unwrap();
    let mut e = Encryptor::new(Vec::new(), h, &key).unwrap();
    e.write_all(plain).unwrap();
    e.finish().unwrap()
}

fn format_error(e: std::io::Error) -> FormatError {
    *e.into_inner().unwrap().downcast::<FormatError>().unwrap()
}

fn read_all(blob: &[u8], pw: &str) -> Result<Vec<u8>, FormatError> {
    let mut d = Decryptor::open(blob, Secret::Password(pw))?;
    let mut out = Vec::new();
    d.read_to_end(&mut out).map_err(format_error)?;
    d.finish()?;
    Ok(out)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| u8::try_from(i % 251).unwrap()).collect()
}

#[test]
fn round_trips_at_every_chunk_boundary() {
    let c = chunk();
    for len in [0, 1, c - 1, c, c + 1, 3 * c + 7] {
        let plain = pattern(len);
        assert_eq!(
            read_all(&sealed(&plain, &full()), PW).unwrap(),
            plain,
            "{len}"
        );
    }
}

#[test]
fn small_writes_round_trip_like_one_large_write() {
    let plain = pattern(2 * chunk() + 3);
    let key = Key::derive(PW, &full()).unwrap();
    let mut e = Encryptor::new(Vec::new(), &full(), &key).unwrap();
    for piece in plain.chunks(4093) {
        e.write_all(piece).unwrap();
    }
    let blob = e.finish().unwrap();
    assert_eq!(blob, sealed(&plain, &full()));
    let mut out = Vec::new();
    let mut r = Decryptor::open(&blob[..], Secret::Password(PW)).unwrap();
    let mut small = [0u8; 777];
    loop {
        let n = r.read(&mut small).unwrap();
        if n == 0 {
            break;
        }
        out.extend_from_slice(&small[..n]);
    }
    r.finish().unwrap();
    assert_eq!(out, plain);
}

#[test]
fn header_layout_is_pinned() {
    let bytes = full().to_bytes();
    assert_eq!(&bytes[..8], b"BRUHSBAK");
    assert_eq!(&bytes[8..12], &[0, 1, 1, 1]);
    assert_eq!(
        u32::from_be_bytes(bytes[12..16].try_into().unwrap()),
        19_456
    );
    assert_eq!(u32::from_be_bytes(bytes[16..20].try_into().unwrap()), 2);
    assert_eq!(u32::from_be_bytes(bytes[20..24].try_into().unwrap()), 1);
    assert_eq!(&bytes[24..47], &R);
    assert_eq!(
        u32::from_be_bytes(bytes[48..52].try_into().unwrap()),
        CHUNK_BYTES
    );
    assert!(bytes[47] == 0 && bytes[52..].iter().all(|b| *b == 0));
    assert_eq!(Header::parse(&bytes).unwrap(), full());
    assert_eq!(
        (KdfParams::NATIVE.memory_kib, KdfParams::WEB.memory_kib),
        (262_144, 65_536)
    );
    for p in [KdfParams::NATIVE, KdfParams::WEB, KdfParams::FLOOR] {
        p.check().unwrap();
    }
    let raw = Header::new(Kind::Sealed, Kdf::RawKey, R).to_bytes();
    assert_eq!(&raw[10..24], &[3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
}

#[test]
fn wrong_passwords_and_changes_fail() {
    let c = chunk();
    let blob = sealed(&vec![1; 3 * c], &full());
    assert!(matches!(
        read_all(&blob, "wrong password!"),
        Err(FormatError::WrongPasswordOrDamaged)
    ));
    let mut header_flip = blob.clone();
    header_flip[30] ^= 1; // salt
    assert!(read_all(&header_flip, PW).is_err());
    let mut prefix_flip = blob.clone();
    prefix_flip[44] ^= 1; // nonce prefix
    assert!(matches!(
        read_all(&prefix_flip, PW),
        Err(FormatError::WrongPasswordOrDamaged)
    ));
    let mut body_flip = blob.clone();
    body_flip[HEADER_LEN + c + 40] ^= 1;
    assert!(matches!(
        read_all(&body_flip, PW),
        Err(FormatError::WrongPasswordOrDamaged)
    ));
    let cut = &blob[..blob.len() - (c + 16)];
    assert!(matches!(read_all(cut, PW), Err(FormatError::Truncated)));
    let mut extra = blob.clone();
    extra.push(0);
    assert!(matches!(
        read_all(&extra, PW),
        Err(FormatError::TrailingData)
    ));
}

#[test]
fn short_and_cut_streams_fail() {
    let c = chunk();
    let blob = sealed(&pattern(c + 10), &full());
    // Cut inside the last (short) chunk.
    assert!(matches!(
        read_all(&blob[..blob.len() - 1], PW),
        Err(FormatError::WrongPasswordOrDamaged)
    ));
    // Cut right after the first, non-last chunk.
    assert!(matches!(
        read_all(&blob[..HEADER_LEN + c + 16], PW),
        Err(FormatError::Truncated)
    ));
    // Nothing after the header.
    assert!(matches!(
        read_all(&blob[..HEADER_LEN], PW),
        Err(FormatError::Truncated)
    ));
    // Part of a header.
    assert!(matches!(
        read_all(&blob[..20], PW),
        Err(FormatError::Truncated)
    ));
    assert!(matches!(read_all(b"", PW), Err(FormatError::NotABackup)));
    assert!(matches!(
        read_all(b"PK\x03\x04", PW),
        Err(FormatError::NotABackup)
    ));
    // A short last chunk with a byte after it still fails.
    let mut extra = blob;
    extra.push(0);
    assert!(read_all(&extra, PW).is_err());
}

#[test]
fn errors_stick_and_finish_needs_the_whole_stream() {
    let c = chunk();
    let blob = sealed(&pattern(3 * c), &full());
    let mut body_flip = blob.clone();
    body_flip[HEADER_LEN + c + 40] ^= 1;
    let mut d = Decryptor::open(&body_flip[..], Secret::Password(PW)).unwrap();
    let mut out = Vec::new();
    assert!(d.read_to_end(&mut out).is_err());
    let mut buf = [0u8; 8];
    assert!(matches!(
        format_error(d.read(&mut buf).unwrap_err()),
        FormatError::WrongPasswordOrDamaged
    ));
    assert!(matches!(
        d.finish(),
        Err(FormatError::WrongPasswordOrDamaged)
    ));

    // Stopping early: plaintext still follows.
    let mut d = Decryptor::open(&blob[..], Secret::Password(PW)).unwrap();
    let mut first = vec![0u8; 10];
    d.read_exact(&mut first).unwrap();
    assert!(matches!(d.finish(), Err(FormatError::TrailingData)));

    // Stopping early on a cut stream reports the cut.
    let cut = &blob[..blob.len() - (c + 16)];
    let d = Decryptor::open(cut, Secret::Password(PW)).unwrap();
    assert!(matches!(d.finish(), Err(FormatError::Truncated)));

    // An empty backup has nothing to read and finishes.
    let empty = sealed(b"", &full());
    let d = Decryptor::open(&empty[..], Secret::Password(PW)).unwrap();
    assert_eq!(d.header(), &full());
    assert_eq!(d.finish().unwrap(), &b""[..]);
}

#[test]
fn crafted_headers_are_refused_before_any_work() {
    let mut b = full().to_bytes();
    b[12..16].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(matches!(Header::parse(&b), Err(FormatError::BadParams)));
    let mut v = full().to_bytes();
    v[9] = 2;
    assert!(matches!(Header::parse(&v), Err(FormatError::NewerVersion)));
    let mut z = full().to_bytes();
    z[9] = 0;
    assert!(matches!(Header::parse(&z), Err(FormatError::BadHeader)));
    let mut m = full().to_bytes();
    m[0] = b'X';
    assert!(matches!(Header::parse(&m), Err(FormatError::NotABackup)));
    let mut r = full().to_bytes();
    r[60] = 1;
    assert!(matches!(Header::parse(&r), Err(FormatError::BadHeader)));
    let mut r47 = full().to_bytes();
    r47[47] = 1;
    assert!(matches!(Header::parse(&r47), Err(FormatError::BadHeader)));
    let mut c = full().to_bytes();
    c[48..52].copy_from_slice(&1u32.to_be_bytes());
    assert!(matches!(Header::parse(&c), Err(FormatError::BadParams)));
    let mut big = full().to_bytes();
    big[48..52].copy_from_slice(&(8_388_608u32 + 1).to_be_bytes());
    assert!(matches!(Header::parse(&big), Err(FormatError::BadParams)));
    let mut kind = full().to_bytes();
    kind[10] = 4;
    assert!(matches!(Header::parse(&kind), Err(FormatError::BadHeader)));
    let mut kdf = full().to_bytes();
    kdf[11] = 2;
    assert!(matches!(Header::parse(&kdf), Err(FormatError::BadHeader)));
    for (at, value) in [(16, 1u32), (16, 11), (20, 0), (20, 17), (12, 19_455)] {
        let mut p = full().to_bytes();
        p[at..at + 4].copy_from_slice(&value.to_be_bytes());
        assert!(
            matches!(Header::parse(&p), Err(FormatError::BadParams)),
            "{at} {value}"
        );
    }
    let mut most = full().to_bytes();
    most[12..16].copy_from_slice(&1_048_576u32.to_be_bytes());
    assert!(Header::parse(&most).is_ok());
}

#[test]
fn kinds_pair_with_their_key_method() {
    let argon = Kdf::Argon2id(KdfParams::FLOOR);
    for (kind, kdf) in [
        (Kind::Full, Kdf::RawKey),
        (Kind::WebPart, Kdf::RawKey),
        (Kind::Sealed, argon),
    ] {
        let bytes = Header::new(kind, kdf, R).to_bytes();
        assert!(
            matches!(Header::parse(&bytes), Err(FormatError::BadHeader)),
            "{kind:?}"
        );
    }
    // Raw-key headers carry no KDF parameters.
    let mut raw = Header::new(Kind::Sealed, Kdf::RawKey, R).to_bytes();
    raw[15] = 1;
    assert!(matches!(Header::parse(&raw), Err(FormatError::BadHeader)));
}

#[test]
fn a_web_part_cannot_ask_a_browser_for_more_memory() {
    let at = |memory_kib| KdfParams {
        memory_kib,
        ..KdfParams::WEB
    };
    let web = |memory_kib| Header::new(Kind::WebPart, Kdf::Argon2id(at(memory_kib)), R);
    assert!(Header::parse(&web(262_144).to_bytes()).is_ok());
    assert!(matches!(
        Header::parse(&web(262_145).to_bytes()),
        Err(FormatError::BadParams)
    ));
    let full_at = Header::new(Kind::Full, Kdf::Argon2id(at(262_145)), R);
    assert!(Header::parse(&full_at.to_bytes()).is_ok());
    // Deriving checks too, for headers built in code.
    assert!(matches!(
        Key::derive(PW, &web(262_145)),
        Err(FormatError::BadParams)
    ));
}

#[test]
fn a_secret_must_match_the_key_method() {
    // A password on a sealed set, and a raw key on a password backup.
    let key = Key::from_bytes([9; 32]);
    let set = seal(b"items", &key, R);
    assert!(matches!(
        Decryptor::open(&set[..], Secret::Password(PW)),
        Err(FormatError::NotABackup)
    ));
    let sealed_header = Header::parse(set[..HEADER_LEN].try_into().unwrap()).unwrap();
    assert!(matches!(
        Key::derive(PW, &sealed_header),
        Err(FormatError::NotABackup)
    ));
    let backup = sealed(b"bruhs", &full());
    assert!(matches!(
        Decryptor::open(&backup[..], Secret::Key(&key)),
        Err(FormatError::NotABackup)
    ));
    assert!(matches!(
        unseal(&backup, &key),
        Err(FormatError::NotABackup)
    ));
}

#[test]
fn sealed_sets_need_their_key() {
    let key = Key::from_bytes([9; 32]);
    let blob = seal(b"{\"items\":[]}", &key, R);
    assert_eq!(&*unseal(&blob, &key).unwrap(), b"{\"items\":[]}");
    assert!(unseal(&blob, &Key::from_bytes([8; 32])).is_err());
    let header = Header::parse(blob[..64].try_into().unwrap()).unwrap();
    assert_eq!((header.kind, header.kdf), (Kind::Sealed, Kdf::RawKey));
    assert_eq!(key.expose(), &[9; 32]);
    let mut extra = blob;
    extra.push(0);
    assert!(matches!(
        unseal(&extra, &key),
        Err(FormatError::WrongPasswordOrDamaged)
    ));
    let big = pattern(chunk() + 5);
    assert_eq!(&*unseal(&seal(&big, &key, R), &key).unwrap(), &big[..]);
}

#[test]
fn a_known_answer_pins_the_format() {
    // KNOWN_ANSWER was recorded once from this exact input; any later change to
    // the header, KDF, nonce layout or AAD changes it.
    let blob = sealed(b"bruhs", &full());
    assert_eq!(hex(&blob[HEADER_LEN..HEADER_LEN + 16]), KNOWN_ANSWER);
    assert_eq!(blob.len(), HEADER_LEN + 5 + 16);
}
