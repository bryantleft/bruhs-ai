//! Open VSX community color themes. Search verifies each advertised manifest;
//! installation verifies the downloaded package again before changing the library.
mod package;
mod panel;

use super::library::InstalledTheme;
use futures_util::{StreamExt, stream};
pub(super) use panel::Browser;
use reqwest::{Client, Response, Url};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    sync::{Arc, LazyLock},
    time::Duration,
};

const FILE_LIMIT: usize = 256 * 1024;
const PACKAGE_LIMIT: usize = 20 * 1024 * 1024;
const LICENSES: &[&str] = &[
    "0BSD",
    "Apache-2.0",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "CC0-1.0",
    "ISC",
    "MIT",
    "MPL-2.0",
    "Unlicense",
];
static HTTP: LazyLock<Client> = LazyLock::new(|| {
    Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("Open VSX HTTP client")
});

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sort {
    Downloads,
    Rating,
    Newest,
    Relevance,
}
impl Sort {
    const ALL: [Self; 4] = [Self::Downloads, Self::Rating, Self::Newest, Self::Relevance];
    fn key(self) -> &'static str {
        match self {
            Self::Downloads => "downloadCount",
            Self::Rating => "rating",
            Self::Newest => "timestamp",
            Self::Relevance => "relevance",
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Downloads => "Most downloaded",
            Self::Rating => "Best rated",
            Self::Newest => "Newest",
            Self::Relevance => "Most relevant",
        }
    }
}

#[derive(Clone, Debug)]
struct Registry {
    origin: Url,
}
impl Default for Registry {
    fn default() -> Self {
        Self {
            origin: Url::parse("https://open-vsx.org/")
                .expect("hardcoded Open VSX origin is a valid URL"),
        }
    }
}

#[derive(Clone, Debug)]
struct Extension {
    id: String,
    name: String,
    publisher: String,
    description: String,
    downloads: u64,
    version: String,
    license: String,
    collection: String,
    source: Option<Url>,
    image: Option<Arc<gpui::Image>>,
    icon: Option<Url>,
    manifest: Url,
    download: Url,
    checksum: Url,
}

fn short_hash(s: &str) -> String {
    format!("{:x}", Sha256::digest(s.as_bytes()))[..12].to_owned()
}
fn theme_id(extension: &str, path: &str) -> String {
    format!("ovx-theme-{}", short_hash(&format!("{extension}:{path}")))
}

fn manifest_themes(manifest: &Value) -> Result<Vec<&Value>, String> {
    let themes = manifest["contributes"]["themes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|t| t.is_object())
        .collect::<Vec<_>>();
    if themes.is_empty() {
        return Err("That extension does not contain color themes.".into());
    }
    Ok(themes)
}
fn check_license(manifest: &Value, license: &str) -> bool {
    manifest["license"]
        .as_str()
        .is_some_and(|s| s.trim().eq_ignore_ascii_case(license))
}
fn jsonc(bytes: &[u8]) -> Result<Value, String> {
    serde_json_lenient::from_slice(bytes).map_err(|e| format!("Invalid extension JSON. {e}"))
}
async fn bytes(mut response: Response, limit: usize) -> Result<Vec<u8>, String> {
    if !response.status().is_success() {
        return Err(format!("Open VSX returned HTTP {}.", response.status()));
    }
    if response.content_length().is_some_and(|n| n > limit as u64) {
        return Err("The Open VSX response exceeds the size limit.".into());
    }
    let mut out = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        if chunk.len() > limit - out.len() {
            return Err("The Open VSX response exceeds the size limit.".into());
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}
async fn get(url: &Url, limit: usize) -> Result<Vec<u8>, String> {
    let res = HTTP
        .get(url.clone())
        .send()
        .await
        .map_err(|e| format!("Could not reach Open VSX. {e}"))?;
    bytes(res, limit).await
}

impl Registry {
    fn parse_extension(&self, v: Value) -> Result<Option<Extension>, String> {
        let need = |key: &str| {
            v[key]
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| format!("The registry entry has no {key}."))
        };
        let publisher = need("namespace")?.to_owned();
        let id = format!("{}.{}", publisher, need("name")?);
        let license = v["license"].as_str().unwrap_or_default().trim();
        if !LICENSES.contains(&license) {
            return Ok(None);
        }
        let file = |key: &str| {
            v["files"][key]
                .as_str()
                .and_then(|s| Url::parse(s).ok())
                .filter(|u| {
                    u.origin() == self.origin.origin()
                        && u.username().is_empty()
                        && u.password().is_none()
                })
        };
        let required = |key: &str| file(key).ok_or_else(|| format!("Missing registry {key} URL."));
        let source = ["repository", "homepage", "url"].iter().find_map(|k| {
            v[*k]
                .as_str()
                .or_else(|| v[*k]["url"].as_str())
                .and_then(|s| Url::parse(s).ok())
                .filter(|u| {
                    u.scheme() == "https" && u.username().is_empty() && u.password().is_none()
                })
        });
        let mut collection = format!("open-vsx:{}", id.to_ascii_lowercase());
        if collection.len() > 128
            || !collection
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b".:-".contains(&c))
        {
            collection = format!("open-vsx:{}", short_hash(&id));
        }
        Ok(Some(Extension {
            id,
            name: v["displayName"]
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(need("name")?)
                .into(),
            publisher,
            description: v["description"].as_str().unwrap_or_default().into(),
            downloads: v["downloadCount"].as_u64().unwrap_or(0),
            version: need("version")?.into(),
            license: license.into(),
            collection,
            source,
            image: None,
            icon: file("icon"),
            manifest: required("manifest")?,
            download: required("download")?,
            checksum: required("sha256")?,
        }))
    }
    async fn details(&self, publisher: &str, name: &str) -> Result<Option<Extension>, String> {
        let mut url = self.origin.clone();
        url.path_segments_mut()
            .map_err(|()| "Open VSX origin cannot be a base URL.".to_owned())?
            .clear()
            .extend(["api", publisher, name]);
        let v = serde_json::from_slice(&get(&url, FILE_LIMIT).await?).map_err(|e| e.to_string())?;
        let Some(ext) = self.parse_extension(v)? else {
            return Ok(None);
        };
        let (manifest, head) = tokio::join!(
            get(&ext.manifest, FILE_LIMIT),
            HTTP.head(ext.download.clone()).send()
        );
        let head = head.map_err(|e| e.to_string())?;
        if !head.status().is_success()
            || head
                .headers()
                .get(reqwest::header::CONTENT_LENGTH)
                .and_then(|h| h.to_str().ok()?.parse::<u64>().ok())
                .is_some_and(|n| n > PACKAGE_LIMIT as u64)
        {
            return Ok(None);
        }
        let manifest = jsonc(&manifest?)?;
        Ok(
            (manifest_themes(&manifest).is_ok() && check_license(&manifest, &ext.license))
                .then_some(ext),
        )
    }
    async fn search(&self, query: &str, sort: Sort) -> Result<Vec<Extension>, String> {
        let mut url = self
            .origin
            .join("api/-/search")
            .map_err(|e| e.to_string())?;
        url.query_pairs_mut().extend_pairs([
            ("query", query),
            ("category", "Themes"),
            ("sortBy", sort.key()),
            ("sortOrder", "desc"),
            ("size", "16"),
        ]);
        let result: Value =
            serde_json::from_slice(&get(&url, FILE_LIMIT * 2).await?).map_err(|e| e.to_string())?;
        let items = result["extensions"]
            .as_array()
            .ok_or("Open VSX returned unreadable search results.")?;
        let mut requests = Vec::new();
        for item in items {
            if let (Some(publisher), Some(name)) =
                (item["namespace"].as_str(), item["name"].as_str())
            {
                requests.push(self.details(publisher, name));
                if requests.len() == 16 {
                    break;
                }
            }
        }
        let results = futures_util::future::join_all(requests).await;
        if !results.is_empty() && results.iter().all(Result::is_err) {
            return Err("Open VSX theme details are unavailable right now.".into());
        }
        let supported = results.into_iter().filter_map(Result::ok).flatten().take(8);
        Ok(stream::iter(supported.map(|mut ext| async move {
            if let Some(url) = &ext.icon {
                if let Ok(Ok(data)) =
                    tokio::time::timeout(Duration::from_secs(1), get(url, 512 * 1024)).await
                {
                    let reader = image::ImageReader::new(std::io::Cursor::new(&data))
                        .with_guessed_format()
                        .ok();
                    if let Some(reader) = reader {
                        let format = match reader.format() {
                            Some(image::ImageFormat::Png) => Some(gpui::ImageFormat::Png),
                            Some(image::ImageFormat::Jpeg) => Some(gpui::ImageFormat::Jpeg),
                            _ => None,
                        };
                        if let Some(format) = format {
                            if reader
                                .into_dimensions()
                                .is_ok_and(|(w, h)| w <= 2048 && h <= 2048)
                            {
                                ext.image = Some(Arc::new(gpui::Image::from_bytes(format, data)));
                            }
                        }
                    }
                }
            }
            ext
        }))
        .buffered(8)
        .collect()
        .await)
    }
}
impl Extension {
    async fn install(&self) -> Result<Vec<Arc<InstalledTheme>>, String> {
        let manifest = jsonc(&get(&self.manifest, FILE_LIMIT).await?)?;
        if manifest_themes(&manifest)?.len() > 40 {
            return Err("The extension contains too many themes.".into());
        }
        let (archive, checksum) =
            tokio::try_join!(get(&self.download, PACKAGE_LIMIT), get(&self.checksum, 256))?;
        let expected = std::str::from_utf8(&checksum)
            .ok()
            .and_then(|v| v.split_whitespace().next())
            .unwrap_or("");
        if !format!("{:x}", Sha256::digest(&archive)).eq_ignore_ascii_case(expected) {
            return Err("That Open VSX theme failed its integrity check.".into());
        }
        package::convert(self, &archive)
    }
}
