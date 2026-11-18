//! User-triggered downloads use a resolver that passes only public addresses
//! to the connector, including every redirect. No check/connect DNS race.
use super::{private_directory, RichError, RichStore};
use bruhs_models::logic::attachments::{file_extension, safe_upload_filename, MAX_FILE_BYTES};
use futures_core::Stream;
use std::{
    net::{IpAddr, SocketAddr},
    sync::{Arc, OnceLock},
};
use tokio::io::AsyncWriteExt;

struct PublicResolver;
impl reqwest::dns::Resolve for PublicResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let name = name.as_str().to_owned();
        Box::pin(async move {
            let addresses: Vec<SocketAddr> =
                tokio::net::lookup_host((name.as_str(), 0)).await?.collect();
            if addresses.is_empty() || addresses.iter().any(|address| !public_ip(address.ip())) {
                return Err(std::io::Error::other(
                    "Downloads cannot reach private, local, or reserved addresses.",
                )
                .into());
            }
            Ok(Box::new(addresses.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, _, _] = ip.octets();
            !ip.is_private()
                && !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_unspecified()
                && !ip.is_multicast()
                && !ip.is_broadcast()
                && !ip.is_documentation()
                && a != 0
                && a < 224
                && !(a == 100 && (64..=127).contains(&b))
                && !(a == 198 && (b == 18 || b == 19))
                && !(a == 192 && (b == 0 || b == 88))
        }
        IpAddr::V6(ip) => {
            if let Some(mapped) = ip.to_ipv4_mapped() {
                return public_ip(IpAddr::V4(mapped));
            }
            let parts = ip.segments();
            parts[0] & 0xe000 == 0x2000
                && parts[0] != 0x2002
                && !(parts[0] == 0x2001 && (parts[1] < 0x200 || parts[1] == 0xdb8))
        }
    }
}

pub(crate) fn validate_url(url: &url::Url) -> Result<(), RichError> {
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.host().is_none()
    {
        return Err(RichError::Invalid(
            "Downloads require an HTTP(S) URL without embedded credentials.".into(),
        ));
    }
    match url.host() {
        Some(url::Host::Ipv4(ip)) if !public_ip(IpAddr::V4(ip)) => {
            return Err(RichError::Invalid(
                "Use the scoped media route for local files, not the external download route."
                    .into(),
            ))
        }
        Some(url::Host::Ipv6(ip)) if !public_ip(IpAddr::V6(ip)) => {
            return Err(RichError::Invalid(
                "External downloads cannot target local addresses.".into(),
            ))
        }
        _ => {}
    }
    Ok(())
}

pub(crate) fn client() -> Result<&'static reqwest::Client, RichError> {
    static CLIENT: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .no_proxy()
                // Match the existing native API request budget for connection
                // and stalled reads, without timing out an active transfer.
                .connect_timeout(std::time::Duration::from_secs(20))
                .read_timeout(std::time::Duration::from_secs(20))
                .dns_resolver(Arc::new(PublicResolver))
                .redirect(reqwest::redirect::Policy::custom(|attempt| {
                    if validate_url(attempt.url()).is_err() {
                        attempt.error("Redirect targeted a local or unsupported address.")
                    } else {
                        reqwest::redirect::Policy::default().redirect(attempt)
                    }
                }))
                .build()
                .map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(|message| RichError::Invalid(format!("Could not initialize downloads: {message}")))
}

pub async fn download(store: &RichStore, raw: &str) -> Result<super::CapturedMedia, RichError> {
    let url = url::Url::parse(raw)
        .map_err(|error| RichError::Invalid(format!("Invalid download URL: {error}")))?;
    validate_url(&url)?;
    let response = client()?
        .get(url.clone())
        .send()
        .await
        .map_err(|error| RichError::Invalid(format!("Media download failed: {error}")))?
        .error_for_status()
        .map_err(|error| RichError::Invalid(format!("Media download failed: {error}")))?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_FILE_BYTES)
    {
        return Err(RichError::Invalid(
            "External media exceeds the 50 MiB attachment budget.".into(),
        ));
    }
    let mime = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("");
    let source_ext = file_extension(url.path());
    let suffix = match mime {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        "image/avif" => "avif",
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        "audio/mpeg" => "mp3",
        "audio/wav" | "audio/x-wav" => "wav",
        "audio/ogg" => "ogg",
        "audio/mp4" => "m4a",
        "application/pdf" => "pdf",
        "text/vtt" => "vtt",
        _ => source_ext
            .as_deref()
            .filter(|ext| {
                matches!(
                    *ext,
                    "png"
                        | "jpg"
                        | "jpeg"
                        | "gif"
                        | "webp"
                        | "svg"
                        | "mp4"
                        | "webm"
                        | "mp3"
                        | "wav"
                        | "m4a"
                        | "pdf"
                        | "vtt"
                )
            })
            .unwrap_or(""),
    }
    .to_owned();
    if suffix.is_empty() {
        return Err(RichError::Invalid(
            "The remote response is not a supported media type.".into(),
        ));
    }
    let store = store.clone();
    let media = store.media.clone();
    let temp = tokio::task::spawn_blocking(move || {
        private_directory(&media)?;
        tempfile::Builder::new()
            .prefix("download-")
            .suffix(&format!(".{suffix}"))
            .tempfile_in(media)
    })
    .await
    .map_err(|e| RichError::Invalid(e.to_string()))??;
    let mut file = tokio::fs::File::from_std(temp.reopen()?);
    let mut bytes = 0_u64;
    let stream = response.bytes_stream();
    tokio::pin!(stream);
    while let Some(chunk) = std::future::poll_fn(|cx| Stream::poll_next(stream.as_mut(), cx)).await
    {
        let chunk = chunk
            .map_err(|e| RichError::Invalid(format!("Media download was interrupted: {e}")))?;
        bytes = bytes.saturating_add(
            u64::try_from(chunk.len()).map_err(|e| RichError::Invalid(e.to_string()))?,
        );
        if bytes > MAX_FILE_BYTES {
            return Err(RichError::Invalid(
                "External media exceeds the 50 MiB attachment budget.".into(),
            ));
        }
        file.write_all(&chunk).await?;
    }
    file.sync_all().await?;
    drop(file);
    tokio::task::spawn_blocking(move || store.capture(&temp.path().to_string_lossy(), None))
        .await
        .map_err(|e| RichError::Invalid(e.to_string()))?
}

pub fn download_name(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|url| {
            url.path_segments()
                .and_then(Iterator::last)
                .map(safe_upload_filename)
        })
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "media".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remote_downloads_reject_local_reserved_and_mapped_targets() {
        for ip in [
            "127.0.0.1",
            "10.0.0.1",
            "169.254.169.254",
            "100.64.0.1",
            "192.0.0.1",
            "198.18.0.1",
            "0.0.0.0",
            "::1",
            "::ffff:127.0.0.1",
            "fc00::1",
            "2001:db8::1",
            "2002:7f00:1::1",
        ] {
            assert!(!public_ip(ip.parse().unwrap()), "{ip}");
        }
        assert!(public_ip("1.1.1.1".parse().unwrap()));
        assert!(public_ip("2606:4700:4700::1111".parse().unwrap()));
    }
    #[test]
    fn redirects_and_embedded_credentials_share_the_same_validation() {
        for url in [
            "file:///etc/passwd",
            "http://127.0.0.1/x",
            "http://[::1]/x",
            "https://name:password@example.com/x",
        ] {
            assert!(validate_url(&url::Url::parse(url).unwrap()).is_err());
        }
    }
}
