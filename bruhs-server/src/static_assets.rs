//! Static assets use release-built gzip sidecars without per-request compression.
use std::path::Path;

use axum::http::{header, HeaderValue};
use tower_http::{services::ServeDir, set_header::SetResponseHeader};

pub(super) fn service(directory: &Path) -> SetResponseHeader<ServeDir, HeaderValue> {
    // ServeDir keeps the original MIME type and falls back to the original file
    // when gzip is unavailable or unacceptable. Caches must key on negotiation.
    SetResponseHeader::appending(
        ServeDir::new(directory).precompressed_gzip(),
        header::VARY,
        HeaderValue::from_static("accept-encoding"),
    )
}

#[cfg(test)]
mod tests;
