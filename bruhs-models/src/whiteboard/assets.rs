//! Portable raster payloads. Dimensions are checked before either UI decodes pixels.
use super::*;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;

pub const MAX_BOARD_BYTES: u64 = crate::logic::attachments::MAX_IMAGE_TOTAL_BYTES;
pub const MAX_PORTABLE_BYTES: u64 =
    MAX_BOARD_BYTES + crate::logic::attachments::MAX_IMAGE_TOTAL_BYTES.div_ceil(3) * 4;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetPayload {
    pub sha256: String,
    pub data_base64: String,
}
impl AssetPayload {
    pub fn from_bytes(bytes: &[u8]) -> BoardResult<(Asset, Self)> {
        if bytes.is_empty()
            || u64::try_from(bytes.len()).unwrap_or(u64::MAX)
                > crate::logic::attachments::MAX_IMAGE_BYTES
        {
            return Err(BoardError::Invalid(
                "Choose a local raster image within the attachment image budget.".into(),
            ));
        }
        let (mime, width, height) = dimensions(bytes).ok_or_else(|| {
            BoardError::Invalid("Use a valid PNG, JPEG, GIF or WebP image.".into())
        })?;
        let sha256 = format!("{:x}", Sha256::digest(bytes));
        let asset = Asset {
            id: sha256.clone(),
            mime: mime.into(),
            bytes: u64::try_from(bytes.len())
                .map_err(|_| BoardError::Invalid("Image length is invalid.".into()))?,
            width,
            height,
            sha256: sha256.clone(),
        };
        if width == 0
            || height == 0
            || u64::from(width)
                .checked_mul(u64::from(height))
                .and_then(|n| n.checked_mul(4))
                .is_none_or(|n| n > crate::logic::attachments::MAX_IMAGE_TOTAL_BYTES)
        {
            return Err(BoardError::Invalid(
                "The decoded image exceeds the existing attachment pixel budget.".into(),
            ));
        }
        Ok((
            asset,
            Self {
                sha256,
                data_base64: STANDARD.encode(bytes),
            },
        ))
    }
    pub fn decode(&self, asset: &Asset) -> BoardResult<Vec<u8>> {
        if self.sha256 != asset.sha256
            || u64::try_from(self.data_base64.len()).unwrap_or(u64::MAX)
                > crate::logic::attachments::MAX_IMAGE_BYTES.div_ceil(3) * 4
        {
            return Err(BoardError::Invalid(
                "Image payload identity or size is invalid.".into(),
            ));
        }
        let bytes = STANDARD
            .decode(&self.data_base64)
            .map_err(|_| BoardError::Invalid("The image payload is not valid base64.".into()))?;
        let (actual, _) = Self::from_bytes(&bytes)?;
        if actual.sha256 != asset.sha256
            || actual.bytes != asset.bytes
            || actual.width != asset.width
            || actual.height != asset.height
            || actual.mime != asset.mime
        {
            return Err(BoardError::Invalid(
                "Image bytes do not match their stored digest, dimensions or format.".into(),
            ));
        }
        Ok(bytes)
    }
    pub fn data_url(&self, asset: &Asset) -> BoardResult<String> {
        self.decode(asset)?;
        Ok(format!("data:{};base64,{}", asset.mime, self.data_base64))
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortableBoard {
    pub board: BoardDocument,
    #[serde(default)]
    pub payloads: BTreeMap<String, AssetPayload>,
}
impl PortableBoard {
    pub fn validate(&self) -> BoardResult<()> {
        self.board.validate()?;
        for asset in self.board.assets.values() {
            self.payloads
                .get(&asset.sha256)
                .ok_or_else(|| {
                    BoardError::Invalid("The portable board is missing local image bytes.".into())
                })?
                .decode(asset)?;
        }
        if self
            .payloads
            .keys()
            .any(|hash| !self.board.assets.values().any(|a| &a.sha256 == hash))
        {
            return Err(BoardError::Invalid(
                "The portable board contains unreferenced image payloads.".into(),
            ));
        }
        Ok(())
    }
    pub fn import_copy(mut self, project_path: String, now_ms: i64) -> BoardResult<Self> {
        self.validate()?;
        if self.board.project_path != project_path {
            self.board.sources.clear();
            for shape in self.board.shapes.values_mut() {
                shape.source_id = None;
                shape
                    .links
                    .retain(|link| matches!(link, BoardLink::PullRequest { .. }));
            }
        }
        self.board.id = uuid::Uuid::new_v4().to_string();
        self.board.project_path = project_path;
        self.board.agent_read = false;
        self.board.revision = 0;
        self.board.created_at_ms = now_ms;
        self.board.updated_at_ms = now_ms;
        Ok(self)
    }
}
fn dimensions(bytes: &[u8]) -> Option<(&'static str, u32, u32)> {
    let be = |slice: &[u8]| Some(u32::from_be_bytes(slice.try_into().ok()?));
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") && bytes.get(12..16) == Some(b"IHDR") {
        return Some((
            "image/png",
            be(bytes.get(16..20)?)?,
            be(bytes.get(20..24)?)?,
        ));
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some((
            "image/gif",
            u32::from(u16::from_le_bytes(bytes.get(6..8)?.try_into().ok()?)),
            u32::from(u16::from_le_bytes(bytes.get(8..10)?.try_into().ok()?)),
        ));
    }
    if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        let le24 = |s: &[u8]| u32::from(s[0]) | u32::from(s[1]) << 8 | u32::from(s[2]) << 16;
        if bytes.get(12..16) == Some(b"VP8X") {
            return Some((
                "image/webp",
                le24(bytes.get(24..27)?) + 1,
                le24(bytes.get(27..30)?) + 1,
            ));
        }
        if bytes.get(12..16) == Some(b"VP8L") && bytes.get(20) == Some(&0x2f) {
            let bits = u32::from_le_bytes(bytes.get(21..25)?.try_into().ok()?);
            return Some((
                "image/webp",
                (bits & 0x3fff) + 1,
                ((bits >> 14) & 0x3fff) + 1,
            ));
        }
        if bytes.get(12..16) == Some(b"VP8 ") && bytes.get(23..26) == Some(b"\x9d\x01\x2a") {
            return Some((
                "image/webp",
                u32::from(u16::from_le_bytes(bytes.get(26..28)?.try_into().ok()?)) & 0x3fff,
                u32::from(u16::from_le_bytes(bytes.get(28..30)?.try_into().ok()?)) & 0x3fff,
            ));
        }
    }
    if bytes.starts_with(&[0xff, 0xd8]) {
        let mut at = 2;
        while at < bytes.len() {
            if *bytes.get(at)? != 0xff {
                return None;
            }
            while bytes.get(at) == Some(&0xff) {
                at += 1;
            }
            let marker = *bytes.get(at)?;
            at += 1;
            if marker == 0xd9 || marker == 0xda {
                return None;
            }
            if marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
                continue;
            }
            let len = usize::from(u16::from_be_bytes(bytes.get(at..at + 2)?.try_into().ok()?));
            if len < 2 {
                return None;
            }
            let segment = bytes.get(at..at.checked_add(len)?)?;
            if (0xc0..=0xcf).contains(&marker) && !matches!(marker, 0xc4 | 0xc8 | 0xcc) {
                return Some((
                    "image/jpeg",
                    u32::from(u16::from_be_bytes(segment.get(5..7)?.try_into().ok()?)),
                    u32::from(u16::from_be_bytes(segment.get(3..5)?.try_into().ok()?)),
                ));
            }
            at = at.checked_add(len)?;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn raster_metadata_is_derived_and_tampering_and_pixel_bombs_are_rejected() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        png.extend(2_u32.to_be_bytes());
        png.extend(3_u32.to_be_bytes());
        let (mut asset, payload) = AssetPayload::from_bytes(&png).unwrap();
        assert_eq!((asset.width, asset.height), (2, 3));
        payload.decode(&asset).unwrap();
        asset.width = 4;
        assert!(payload.decode(&asset).is_err());
        png[16..20].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(AssetPayload::from_bytes(&png).is_err());
    }
}
