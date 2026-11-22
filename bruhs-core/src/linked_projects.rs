//! The project-list grant exposes the desktop's saved checkouts and their Git identities.
pub mod names;
pub mod connection;
use crate::git::{ProjectIdentity, project_identity};
use futures_util::{StreamExt, stream};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Checkout {
    pub path: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub identity: Option<ProjectIdentity>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Desktop {
    current_model: Option<String>,
    project_defaults: BTreeMap<String, ProjectModel>,
    project_list: ProjectList,
    recent_projects: Vec<Location>,
    active_project: Option<Location>,
    threads: Vec<Thread>,
}

#[derive(Deserialize)]
struct ProjectModel { model: Option<String> }

#[derive(Default, Deserialize)]
#[serde(default)]
struct ProjectList {
    titles: BTreeMap<String, String>,
    entries: Vec<Location>,
}
#[derive(Deserialize)]
struct Location {
    path: String,
    #[serde(default)]
    name: String,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Thread {
    project_name: String,
    project_path: Option<String>,
    project_settings_root: Option<String>,
}

fn saved_desktop(file: PathBuf) -> io::Result<Desktop> {
    let file = match std::fs::File::open(file) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Desktop::default()),
        Err(e) => return Err(e),
    };
    Ok(serde_json::from_reader(io::BufReader::new(file))?)
}

fn saved_projects(file: PathBuf) -> io::Result<Vec<Checkout>> {
    saved_desktop(file).map(checkouts)
}

fn checkouts(state: Desktop) -> Vec<Checkout> {
    let threads = state.threads.into_iter().filter_map(|thread| {
        if thread.project_name.is_empty() {
            return None;
        }
        Some(Location {
            path: thread.project_settings_root.or(thread.project_path)?,
            name: thread.project_name,
        })
    });
    let mut paths = BTreeMap::new();
    for location in state
        .project_list
        .entries
        .into_iter()
        .chain(state.recent_projects)
        .chain(state.active_project)
        .chain(threads)
    {
        let path = Path::new(&location.path);
        if !path.is_absolute() {
            continue;
        }
        let name = if location.name.is_empty() {
            path.file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(&location.path)
                .to_owned()
        } else {
            location.name
        };
        paths.entry(location.path).or_insert(name);
    }
    paths
        .into_iter()
        .map(|(path, name)| {
            let title = state.project_list.titles.get(&path).cloned();
            Checkout {
                path,
                name,
                title,
                identity: None,
            }
        })
        .collect()
}

/// Paths and preferred task models in the host's saved projects. Git discovery is only
/// necessary for checkout grouping, not for opening the task editor or changing its project.
#[derive(Serialize, Deserialize)]
pub struct TaskProjects {
    pub default_model: String,
    pub projects: BTreeMap<String, Option<String>>,
}

pub async fn task_projects(file: PathBuf) -> io::Result<TaskProjects> {
    tokio::task::spawn_blocking(move || {
        let mut desktop = saved_desktop(file)?;
        let model = desktop.current_model.take();
        let preferences = std::mem::take(&mut desktop.project_defaults);
        Ok(TaskProjects {
            default_model: model
                .unwrap_or_else(|| bruhs_models::agent::DEFAULT_AGENT_MODEL.into()),
            projects: checkouts(desktop)
                .into_iter()
                .map(|p| {
                    let model = preferences
                        .get(&p.path)
                        .and_then(|v| v.model.clone());
                    (p.path, model)
                })
                .collect(),
        })
    })
    .await
    .map_err(io::Error::other)?
}

pub async fn inventory(file: PathBuf) -> io::Result<Vec<Checkout>> {
    let paths = tokio::task::spawn_blocking(move || -> io::Result<_> {
        let pending = names::pending(&file)?;
        let mut paths = saved_projects(file)?;
        names::overlay(&pending, &mut paths);
        Ok(paths)
    })
    .await??;
    let mut result = stream::iter(paths)
        .map(|mut checkout| async move {
            let mut identity = project_identity(Path::new(&checkout.path)).await;
            if let Some(repo) = &mut identity {
                // A local Git directory identifies related worktrees on this host.
                // Its physical location is outside the project-list projection.
                if let Some(dir) = repo.repository.strip_prefix("local:") {
                    use sha2::{Digest, Sha256};
                    repo.repository = format!("local:{:x}", Sha256::digest(dir.as_bytes()));
                }
            }
            checkout.identity = identity;
            checkout
        })
        .buffer_unordered(4)
        .collect::<Vec<_>>()
        .await;
    result.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(result)
}
