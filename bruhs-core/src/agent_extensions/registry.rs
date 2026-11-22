//! Discover and prepare versioned agents from the official ACP Registry.
mod archive;
mod install;
#[cfg(test)]
mod tests;
pub mod updates;
use bruhs_models::agent_extensions::valid_environment_name;
pub use install::{prepare, Prepared};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io,
    path::{Component, Path, PathBuf},
    time::Duration,
};

const INDEX: &str = "https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json";
const INDEX_LIMIT: usize = 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Launch {
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
}
#[derive(Clone, Debug, Serialize)]
enum Distribution {
    Binary {
        archive: String,
        cmd: PathBuf,
        sha256: Option<String>,
        launch: Launch,
    },
    Package {
        manager: Manager,
        name: String,
        version: String,
        launch: Launch,
    },
}
#[derive(Clone, Copy, Debug, Serialize)]
enum Manager {
    Npm,
    Uv,
}

#[derive(Clone, Debug)]
pub struct Agent {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub about: Option<String>,
    authors: String,
    distribution: Distribution,
}

impl Agent {
    pub fn origin(&self) -> bruhs_models::agent_extensions::RegistryOrigin {
        use bruhs_models::agent_extensions::{RegistryDefaults, RegistryOrigin};
        let launch = match &self.distribution {
            Distribution::Binary { launch, .. } | Distribution::Package { launch, .. } => launch,
        };
        RegistryOrigin {
            agent_id: self.id.clone(),
            version: self.version.clone(),
            distribution: self.method().into(),
            executable_override: None,
            auth_method_id: None,
            defaults: Some(RegistryDefaults {
                args: launch.args.clone(),
                environment: launch.env.clone(),
            }),
        }
    }

    pub fn method(&self) -> &'static str {
        match &self.distribution {
            Distribution::Binary { .. } => "Binary",
            Distribution::Package {
                manager: Manager::Npm,
                ..
            } => "npm",
            Distribution::Package {
                manager: Manager::Uv,
                ..
            } => "uv",
        }
    }
    pub fn search_rank(&self, query: &str) -> Option<u8> {
        let q = query.trim().to_lowercase();
        let id = self.id.to_lowercase();
        let name = self.name.to_lowercase();
        if q.is_empty() || name == q || id == q {
            return Some(0);
        }
        if name.starts_with(&q) || id.starts_with(&q) {
            return Some(1);
        }
        let identity = format!("{id} {name}");
        let words: Vec<_> = identity.split(|c: char| !c.is_alphanumeric()).collect();
        let terms: Vec<_> = q.split_whitespace().collect();
        if terms.iter().all(|t| words.iter().any(|w| w.starts_with(t))) {
            return Some(2);
        }
        if terms.iter().all(|t| identity.contains(t)) {
            return Some(3);
        }
        if terms.iter().all(|t| self.authors.contains(t)) {
            return Some(4);
        }
        let all = format!(
            "{identity} {} {}",
            self.authors,
            self.description.to_lowercase()
        );
        terms.iter().all(|t| all.contains(t)).then_some(5)
    }
}

fn identifier(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.as_bytes()[0].is_ascii_alphanumeric()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

fn relative(s: &str) -> Option<PathBuf> {
    if s.contains(['\\', ':', '\0']) {
        return None;
    }
    let mut path = PathBuf::new();
    for c in Path::new(s).components() {
        match c {
            Component::CurDir => (),
            Component::Normal(part) => path.push(part),
            _ => return None,
        }
    }
    (!path.as_os_str().is_empty()).then_some(path)
}

fn https(value: &str) -> Option<url::Url> {
    let url = url::Url::parse(value).ok()?;
    (url.scheme() == "https"
        && url.has_host()
        && url.username().is_empty()
        && url.password().is_none())
    .then_some(url)
}

impl Manager {
    fn tool(self) -> &'static str {
        match self {
            Self::Npm => "npm",
            Self::Uv => "uv",
        }
    }
    fn executable(self) -> Option<PathBuf> {
        let tool = if cfg!(windows) {
            match self {
                Self::Npm => "npm.cmd",
                Self::Uv => "uv.exe",
            }
        } else {
            self.tool()
        };
        crate::updates::find_on_path(tool, std::env::var_os("PATH").as_ref())
    }
}

fn package(spec: &str, m: Manager) -> Option<(String, String)> {
    let (name, version) = match m {
        Manager::Uv => spec.rsplit_once("==").or_else(|| spec.rsplit_once('@'))?,
        Manager::Npm => spec.rsplit_once('@')?,
    };
    if let Some((scope, n)) = name.split_once('/') {
        if !matches!(m, Manager::Npm) || !identifier(scope.strip_prefix('@')?) || !identifier(n) {
            return None;
        }
    } else if !identifier(name) {
        return None;
    }
    let v = semver::Version::parse(version.strip_prefix('v').unwrap_or(version)).ok()?;
    Some((name.to_owned(), v.to_string()))
}

fn launch(value: &serde_json::Value) -> Option<Launch> {
    let l: Launch = serde_json::from_value(value.clone()).ok()?;
    (l.args.len() <= 64
        && l.env.len() <= 128
        && l.args
            .iter()
            .chain(l.env.values())
            .all(|s| s.len() <= 4096 && !s.contains('\0'))
        && l.env.keys().all(|s| valid_environment_name(s)))
    .then_some(l)
}

fn native_target() -> String {
    format!(
        "{}-{}",
        if cfg!(target_os = "macos") {
            "darwin"
        } else {
            std::env::consts::OS
        },
        std::env::consts::ARCH
    )
}

fn distribution(v: &serde_json::Value, target: &str, npm: bool, uv: bool) -> Option<Distribution> {
    let b = v["binary"].get(target).or_else(|| {
        (target == "windows-aarch64")
            .then(|| v["binary"].get("windows-x86_64"))
            .flatten()
    });
    if let Some(b) = b {
        let archive = b["archive"].as_str()?;
        https(archive)?;
        let cmd = relative(b["cmd"].as_str()?)?;
        let sha256 = match b.get("sha256") {
            Some(s) => Some(
                s.as_str()
                    .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))?
                    .to_owned(),
            ),
            None => None,
        };
        return Some(Distribution::Binary {
            archive: archive.into(),
            cmd,
            sha256,
            launch: launch(b)?,
        });
    }
    for (key, manager, available) in [("npx", Manager::Npm, npm), ("uvx", Manager::Uv, uv)] {
        if !available {
            continue;
        }
        if let Some(row) = v.get(key) {
            let (name, version) = package(row["package"].as_str()?, manager)?;
            return Some(Distribution::Package {
                manager,
                name,
                version,
                launch: launch(row)?,
            });
        }
    }
    None
}

fn parse(bytes: &[u8], target: &str, npm: bool, uv: bool) -> io::Result<Vec<Agent>> {
    let index: serde_json::Value = serde_json::from_slice(bytes)?;
    let rows = index["agents"]
        .as_array()
        .ok_or_else(|| io::Error::other("Invalid ACP registry index."))?;
    let mut agents: Vec<_> = rows
        .iter()
        .filter_map(|r| {
            let id = r["id"].as_str().filter(|s| identifier(s))?;
            let name = r["name"]
                .as_str()
                .filter(|s| !s.trim().is_empty() && s.len() <= 256)?;
            let version = r["version"]
                .as_str()
                .filter(|s| !s.is_empty() && s.len() <= 128)?;
            Some(Agent {
                id: id.into(),
                name: name.into(),
                version: version.into(),
                about: ["website", "repository"]
                    .iter()
                    .find_map(|key| https(r[key].as_str()?))
                    .map(|url| url.to_string()),
                authors: r["authors"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|a| a.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
                    .to_lowercase(),
                description: r["description"]
                    .as_str()
                    .unwrap_or("")
                    .chars()
                    .take(1024)
                    .collect(),
                distribution: distribution(&r["distribution"], target, npm, uv)?,
            })
        })
        .collect();
    agents.sort_by_cached_key(|a| (a.name.to_lowercase(), a.id.clone()));
    Ok(agents)
}

fn client(timeout: Duration) -> io::Result<reqwest::Client> {
    reqwest::Client::builder()
        .https_only(true)
        .timeout(timeout)
        .user_agent("bruhs-acp-registry")
        .build()
        .map_err(io::Error::other)
}

pub struct Catalog {
    pub agents: Vec<Agent>,
    pub cached: bool,
}
fn parse_for_device(bytes: &[u8]) -> io::Result<Vec<Agent>> {
    parse(
        bytes,
        &native_target(),
        Manager::Npm.executable().is_some(),
        Manager::Uv.executable().is_some(),
    )
}

fn cached_agents(path: &Path) -> io::Result<Vec<Agent>> {
    if std::fs::metadata(path)?.len() > INDEX_LIMIT as u64 {
        return Err(io::Error::other("Invalid cached ACP index."));
    }
    parse_for_device(&std::fs::read(path)?)
}

pub(crate) fn cached_provider_website(id: &str) -> Option<String> {
    let path = super::directory().ok()?.join("registry/index.json");
    cached_agents(&path)
        .ok()?
        .into_iter()
        .find(|agent| agent.id == id)?
        .about
}

pub async fn catalog(refresh: bool) -> io::Result<Catalog> {
    let dir = super::directory()?.join("registry");
    let path = dir.join("index.json");
    if !refresh
        && std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age < Duration::from_secs(300))
    {
        if let Ok(agents) = cached_agents(&path) {
            return Ok(Catalog {
                agents,
                cached: false,
            });
        }
    }
    let fetch = async {
        let mut response = client(Duration::from_secs(30))?
            .get(INDEX)
            .send()
            .await
            .map_err(io::Error::other)?
            .error_for_status()
            .map_err(io::Error::other)?;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(io::Error::other)? {
            if bytes.len() + chunk.len() > INDEX_LIMIT {
                return Err(io::Error::other("ACP registry index is too large."));
            }
            bytes.extend_from_slice(&chunk);
        }
        let agents = parse_for_device(&bytes)?;
        let save_index = || -> io::Result<()> {
            super::private_directory(&dir)?;
            let mut file = tempfile::NamedTempFile::new_in(&dir)?;
            std::io::Write::write_all(&mut file, &bytes)?;
            file.persist(&path)?;
            Ok(())
        };
        if let Err(err) = save_index() {
            tracing::debug!(%err,"Could not cache the ACP registry index");
        }
        Ok(Catalog {
            agents,
            cached: false,
        })
    }
    .await;
    fetch.or_else(|error| {
        cached_agents(&path)
            .map(|agents| Catalog {
                agents,
                cached: true,
            })
            .map_err(|_| error)
    })
}
