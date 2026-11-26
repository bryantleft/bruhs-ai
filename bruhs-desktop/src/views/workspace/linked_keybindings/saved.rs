//! Workspace state holds display metadata. The OS credential store holds each grant.
use super::*;
use uuid::Uuid;

pub(super) use bruhs_core::linked_projects::connection::{Destination, SavedHost};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::views::workspace) struct SavedScheduleRun {
    pub(super) host: Uuid,
    pub(super) task_hash: String,
    pub(super) request: Uuid,
}

#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub(in crate::views::workspace) struct SavedLinks {
    hosts: Vec<SavedHost>,
    selected: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(in crate::views::workspace) pending_runs: Vec<SavedScheduleRun>,
}
impl SavedLinks {
    pub(in crate::views::workspace) fn is_empty(&self) -> bool {
        self.hosts.is_empty()
    }
    pub(super) fn restore<T: Default>(&self) -> (Vec<LinkedHost<T>>, Option<usize>) {
        let mut hosts = Vec::new();
        for h in &self.hosts {
            let (origin, tunnel) = match &h.destination {
                Destination::Https(origin) => (origin.as_str(), None),
                Destination::Ssh { profile, .. } => (
                    "",
                    Some(Tunnel {
                        profile: *profile,
                        name: h.name.clone(),
                        destination: h.destination.clone(),
                        socket: None,
                    }),
                ),
            };
            let Ok(endpoint) = LinkedHost::<T>::endpoint(origin, tunnel.is_some()) else {
                continue;
            };
            let host = LinkedHost {
                credential: h.credential,
                share_usage: h.usage,
                share_usage_history: h.usage_history,
                edit_usage_prices: h.edit_usage_prices,
                vault_checked: false,
                endpoint,
                token: String::new(),
                data: T::default(),
                tunnel,
                problem: Some("Waiting for connection.".into()),
            };
            if !hosts
                .iter()
                .any(|existing: &LinkedHost<T>| existing.same_environment(&host))
            {
                hosts.push(host);
            }
        }
        let selected = hosts
            .iter()
            .position(|h| Some(h.credential) == self.selected);
        (hosts, selected)
    }
}
impl LinkedKeybindings {
    pub(in crate::views::workspace) fn saved(&self) -> SavedLinks {
        SavedLinks::capture(&self.hosts, self.selected)
    }
}
impl SavedLinks {
    pub(super) fn capture<T: Default>(hosts: &[LinkedHost<T>], selected: Option<usize>) -> Self {
        Self {
            hosts: hosts
                .iter()
                .map(|h| SavedHost {
                    usage: h.share_usage,
                    usage_history: h.share_usage_history,
                    edit_usage_prices: h.edit_usage_prices,
                    credential: h.credential,
                    destination: h.destination(),
                    name: h.name(),
                })
                .collect(),
            pending_runs: Vec::new(),
            selected: selected.and_then(|i| hosts.get(i)).map(|h| h.credential),
        }
    }
}
impl<T> LinkedHost<T> {
    pub(super) fn destination(&self) -> Destination {
        self.tunnel.as_ref().map_or_else(
            || Destination::Https(self.endpoint.origin().ascii_serialization()),
            |t| t.destination.clone(),
        )
    }
}

#[derive(Default, Clone)]
pub(super) struct Vault {
    #[cfg(not(test))]
    projects: bool,
    #[cfg(test)]
    credentials: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, String>>>,
}
impl Vault {
    pub(super) fn projects() -> Self {
        #[cfg(not(test))]
        {
            Self { projects: true }
        }
        #[cfg(test)]
        {
            Self::default()
        }
    }
    #[cfg(not(test))]
    fn entry(&self, key: &str) -> Result<keyring::Entry, String> {
        keyring::Entry::new(
            if self.projects {
                "bruhs-linked-projects"
            } else {
                "bruhs-linked-keybindings"
            },
            key,
        )
        .map_err(|_| "Could not access the OS credential store.".into())
    }
    pub(super) fn read(&self, id: Uuid, destination: &Destination) -> Result<String, String> {
        #[cfg(not(test))]
        if self.projects {
            return bruhs_core::linked_projects::connection::project_token(id, destination)
                .map(|token| token.to_string());
        }
        let key = id.to_string();
        #[cfg(test)]
        let value = self
            .credentials
            .lock()
            .unwrap()
            .get(&key)
            .cloned()
            .ok_or_else(|| "Link this environment again to restore its access token.".to_string());
        #[cfg(not(test))]
        let value = self.entry(&key)?.get_password().map_err(|e| match e {
            keyring::Error::NoEntry => {
                "Link this environment again to restore its access token.".to_string()
            }
            _ => "Unlock the OS credential store, then refresh the environment.".to_string(),
        });
        let (issued_for, token): (Destination, String) = serde_json::from_str(&value?)
            .map_err(|_| "Link this environment again to restore its access token.")?;
        if issued_for != *destination {
            return Err("The saved destination has changed. Link the environment again.".into());
        }
        Ok(token)
    }
    pub(super) fn write(
        &self,
        id: Uuid,
        destination: &Destination,
        token: &str,
    ) -> Result<(), String> {
        let key = id.to_string();
        let value = serde_json::to_string(&(destination, token))
            .map_err(|_| "Could not encode the access token.")?;
        #[cfg(test)]
        {
            self.credentials.lock().unwrap().insert(key, value);
            Ok(())
        }
        #[cfg(not(test))]
        self.entry(&key)?
            .set_password(&value)
            .map_err(|_| "Could not save the access token to the OS credential store.".into())
    }
    pub(super) fn remove(&self, id: Uuid) -> Result<(), String> {
        let key = id.to_string();
        #[cfg(test)]
        {
            self.credentials.lock().unwrap().remove(&key);
            Ok(())
        }
        #[cfg(not(test))]
        match self.entry(&key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err("Could not remove the access token from the OS credential store.".into()),
        }
    }
}
