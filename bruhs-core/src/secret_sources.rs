//! Owner-configured variables for provider launches and account metadata requests.
use bruhs_models::agent::AgentProvider;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::process::Command;

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct SecretSources {
    pub sources: BTreeMap<String, Source>,
    pub bindings: Vec<Binding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pending_generator_reset: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Source {
    pub name: String,
    pub account: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Binding {
    pub target: String,
    pub variable: String,
    pub source: String,
    pub reference: String,
}
pub enum Change {
    SaveSource(String, Source),
    RemoveSource(String),
    Bind(Binding),
    Unbind { target: String, variable: String },
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
fn file() -> io::Result<PathBuf> {
    directories::ProjectDirs::from("dev", "bruhs", "bruhs-ai")
        .map(|d| d.config_dir().join("secret-sources.json"))
        .ok_or_else(|| io::Error::other("The host settings directory is unavailable."))
}
fn read(path: &Path) -> io::Result<SecretSources> {
    match fs::read(path) {
        Ok(data) => serde_json::from_slice(&data)
            .map_err(|_| invalid("The secret provider settings could not be read.")),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(SecretSources::default()),
        Err(e) => Err(e),
    }
}
fn unset_consumers(source: &str) -> io::Result<()> {
    crate::text_generators::unset_secret_source(source)?;
    crate::usage_hubs::unset_source(source)?;
    crate::agent_extensions::environment::unset_source(source)
}

pub fn load() -> io::Result<SecretSources> {
    let path = file()?;
    let data = read(&path)?;
    if data.pending_generator_reset.is_some() {
        // The source is already absent. A locked generator database must not
        // prevent other accounts from resolving their configured fields.
        if let Ok(repaired) = update_with(&path, None, unset_consumers) {
            return Ok(repaired);
        }
    }
    Ok(data)
}
pub(crate) fn configured() -> io::Result<SecretSources> {
    read(&file()?)
}
pub(super) fn source_exists(id: &str) -> io::Result<bool> {
    Ok(configured()?.sources.contains_key(id))
}
pub fn change(edit: Change) -> io::Result<SecretSources> {
    update_with(&file()?, Some(edit), unset_consumers)
}
fn update_with(
    path: &Path,
    edit: Option<Change>,
    unset: impl Fn(&str) -> io::Result<()>,
) -> io::Result<SecretSources> {
    let parent = path
        .parent()
        .ok_or_else(|| invalid("Invalid settings folder."))?;
    fs::create_dir_all(parent)?;
    let lock = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(path.with_extension("lock"))?;
    lock.lock()?;
    let mut data = read(path)?;
    // Complete a previously committed removal before allowing an ID to be reused.
    finish_removal(path, &mut data, &unset)?;
    if let Some(edit) = edit {
        if let Change::RemoveSource(id) = &edit {
            data.pending_generator_reset = Some(id.clone());
        }
        data.apply(edit)?;
        persist(path, &data)?;
        finish_removal(path, &mut data, &unset)?;
    }
    Ok(data)
}
fn finish_removal(
    path: &Path,
    data: &mut SecretSources,
    unset: &impl Fn(&str) -> io::Result<()>,
) -> io::Result<()> {
    if let Some(id) = &data.pending_generator_reset {
        unset(id)?;
        data.pending_generator_reset = None;
        persist(path, data)?;
    }
    Ok(())
}
fn persist(path: &Path, data: &SecretSources) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| invalid("Invalid settings folder."))?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer(&mut temp, data)?;
    temp.flush()?;
    temp.as_file().sync_all()?;
    temp.persist(path)?;
    Ok(())
}
fn safe_label(value: &str) -> bool {
    !value.trim().is_empty()
        && value.len() <= 512
        && bruhs_models::secrets::review_text(value) == value
}
impl SecretSources {
    fn apply(&mut self, edit: Change) -> io::Result<()> {
        match edit {
            Change::SaveSource(id, s) => {
                if !safe_label(&id)
                    || !safe_label(&s.name)
                    || (!s.account.is_empty() && !safe_label(&s.account))
                {
                    return Err(invalid("Enter a provider name and a valid account."));
                }
                if self
                    .sources
                    .iter()
                    .any(|(key, other)| key != &id && other.name.eq_ignore_ascii_case(&s.name))
                {
                    return Err(invalid("A secret provider already uses that name."));
                }
                self.sources.insert(id, s);
            }
            Change::RemoveSource(id) => {
                self.sources.remove(&id);
                self.bindings.retain(|b| b.source != id);
            }
            Change::Unbind { target, variable } => self
                .bindings
                .retain(|b| b.target != target || b.variable != variable),
            Change::Bind(b) => {
                self.validate(&b)?;
                self.bindings
                    .retain(|old| old.target != b.target || old.variable != b.variable);
                self.bindings.push(b);
            }
        }
        Ok(())
    }
    fn validate(&self, b: &Binding) -> io::Result<()> {
        if !self.sources.contains_key(&b.source) {
            return Err(invalid("Select an existing secret provider."));
        }
        let target = b
            .target
            .parse::<AgentProvider>()
            .is_ok_and(|p| p != AgentProvider::Acp)
            || b.target
                .strip_prefix("acp/")
                .is_some_and(|id| safe_label(id) && !id.contains('/'));
        if !target {
            return Err(invalid(
                "Select an agent or enter acp/ followed by an instance ID.",
            ));
        }
        bruhs_models::secrets::validate_credential_variable(&b.variable).map_err(invalid)?;
        if crate::connections::PRIVATE_ENV.contains(&b.variable.as_str())
            || b.variable.starts_with("OP_")
        {
            return Err(invalid("This variable is reserved for the host."));
        }
        validate_binding_reference(&b.reference)
    }

    /// Existing environment bindings may use field paths accepted by older versions.
    pub fn can_check_reference(&self, field: &SecretReference) -> io::Result<()> {
        if self
            .bindings
            .iter()
            .any(|b| b.source == field.source && b.reference == field.reference)
        {
            validate_binding_reference(&field.reference)
        } else {
            validate_reference(&field.reference)
        }
    }
    async fn read_binding(&self, binding: &Binding, op: &Path) -> io::Result<String> {
        self.validate(binding)?;
        read_field(op, &self.sources[&binding.source], &binding.reference).await
    }

    /// Probe one provider variable without opening unrelated credential fields.
    pub(crate) async fn provider_variable(
        &self,
        target: &str,
        name: &str,
    ) -> io::Result<Option<String>> {
        match self.bindings.iter().find(|b| b.target == target && b.variable == name) {
            Some(binding) => self.read_binding(binding, Path::new("op")).await.map(Some),
            None => Ok(std::env::var(name).ok()),
        }
    }

    async fn resolve(&self, target: &str, op: &Path) -> io::Result<Vec<(String, String)>> {
        let mut vars = Vec::new();
        for b in self.bindings.iter().filter(|b| b.target == target) {
            let value = self.read_binding(b, op).await?;
            vars.push((b.variable.clone(), value));
        }
        Ok(vars)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretReference {
    pub source: String,
    pub reference: String,
}

fn validate_binding_reference(reference: &str) -> io::Result<()> {
    if safe_label(reference)
        && reference
            .strip_prefix("op://")
            .is_some_and(|r| r.split('/').count() >= 3 && r.split('/').all(|part| !part.is_empty()))
    {
        Ok(())
    } else {
        Err(invalid(
            "Enter a 1Password reference such as op://vault/item/field.",
        ))
    }
}

pub fn validate_reference(reference: &str) -> io::Result<()> {
    let invalid_ref = || invalid("Use a 1Password reference like op://vault/item/field.");
    let path = reference
        .strip_prefix("op://")
        .filter(|_| reference.len() <= 1024)
        .ok_or_else(invalid_ref)?;
    let part_char = |c: u8| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-');
    let path = if let Some((path, query)) = path.split_once('?') {
        let (key, value) = query.split_once('=').ok_or_else(invalid_ref)?;
        if key.is_empty()
            || !key.bytes().all(|c| c.is_ascii_lowercase() || c == b'-')
            || value.is_empty()
            || !value
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
        {
            return Err(invalid_ref());
        }
        path
    } else {
        path
    };
    let mut parts = path.split('/');
    if !(3..=4).contains(&parts.clone().count())
        || !parts
            .all(|p| !p.is_empty() && p.trim() == p && p.bytes().all(|c| c == b' ' || part_char(c)))
    {
        return Err(invalid_ref());
    }
    Ok(())
}
async fn read_field(op: &Path, source: &Source, reference: &str) -> io::Result<String> {
    let mut command = Command::new(op);
    command.args(["read", "--no-newline"]);
    if !source.account.is_empty() {
        command.arg("--account").arg(&source.account);
    }
    command
        .arg(reference)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(60), command.output())
        .await
        .map_err(|_| {
            io::Error::other("1Password timed out. Unlock the configured account and retry.")
        })?
        .map_err(|_| io::Error::other("Could not start the 1Password CLI on this host."))?;
    if !output.status.success() {
        return Err(io::Error::other(
                    "1Password could not read a configured field. Check the account and reference in Secrets.",
                ));
    }
    let value = String::from_utf8(output.stdout)
        .map_err(|_| invalid("A referenced field is not a text value."))?;
    if value.contains('\0') {
        return Err(invalid(
            "The referenced field contains an invalid character.",
        ));
    }
    Ok(value)
}
impl SecretReference {
    /// Check a reference on this host. Settings receives only the resolution status.
    pub async fn check(&self) -> io::Result<()> {
        let data = tokio::task::spawn_blocking(load)
            .await
            .map_err(io::Error::other)??;
        data.can_check_reference(self)?;
        self.resolve_value(&data).await.map(drop)
    }

    pub(crate) async fn read(&self) -> io::Result<zeroize::Zeroizing<String>> {
        validate_reference(&self.reference)?;
        let data = tokio::task::spawn_blocking(load)
            .await
            .map_err(io::Error::other)??;
        self.resolve_value(&data).await
    }
    async fn resolve_value(&self, data: &SecretSources) -> io::Result<zeroize::Zeroizing<String>> {
        let source = data.sources.get(&self.source).ok_or_else(|| {
            invalid("This 1Password source is unavailable. Choose another source in Settings.")
        })?;
        let value =
            zeroize::Zeroizing::new(read_field(Path::new("op"), source, &self.reference).await?);
        if value.trim().is_empty() {
            return Err(invalid("The referenced field is empty."));
        }
        Ok(value)
    }
}
pub(crate) async fn for_turn(
    provider: AgentProvider,
    model: &str,
) -> io::Result<Vec<(String, String)>> {
    let target = if provider == AgentProvider::Acp {
        model
            .strip_prefix("acp/")
            .and_then(|m| m.split_once('/'))
            .map(|(id, _)| format!("acp/{id}"))
            .unwrap_or_default()
    } else {
        provider.as_str().into()
    };
    for_target(&target).await
}

pub(crate) async fn for_target(target: &str) -> io::Result<Vec<(String, String)>> {
    let data = tokio::task::spawn_blocking(load)
        .await
        .map_err(io::Error::other)??;
    data.resolve(target, Path::new("op")).await
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    fn update(path: &Path, edit: Change) -> io::Result<SecretSources> {
        update_with(path, Some(edit), |_| Ok(()))
    }
    #[test]
    fn removal_replays_a_failed_generator_reset_before_reusing_the_source_id() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("sources.json");
        let save = || {
            Change::SaveSource(
                "same-id".into(),
                Source {
                    name: "vault".into(),
                    account: "".into(),
                },
            )
        };
        update(&file, save()).unwrap();
        let failed = update_with(&file, Some(Change::RemoveSource("same-id".into())), |_| {
            Err(io::Error::other("unavailable"))
        });
        assert!(failed.is_err());
        let data = read(&file).unwrap();
        assert!(data.sources.is_empty());
        assert_eq!(data.pending_generator_reset.as_deref(), Some("same-id"));
        let count = std::cell::Cell::new(0);
        let data = update_with(&file, Some(save()), |id| {
            assert_eq!(id, "same-id");
            count.set(count.get() + 1);
            Ok(())
        })
        .unwrap();
        assert_eq!(count.get(), 1);
        assert!(data.pending_generator_reset.is_none() && data.sources.contains_key("same-id"));
    }

    #[test]
    fn generator_reference_accepts_vault_item_section_field_and_optional_attribute() {
        for valid in [
            "op://Team Vault/Build key/password",
            "op://my-vault/Project/Private/api.key?attribute=otp",
        ] {
            assert!(validate_reference(valid).is_ok());
        }
        for invalid in [
            "op://vault/item",
            "op://v/i/s/f/extra",
            "op://v//key",
            "op://v/ item/f",
            "op://v/i/f?k=v&other=x",
            "token",
        ] {
            assert!(validate_reference(invalid).is_err());
        }
    }
    fn binding(target: &str, variable: &str) -> Binding {
        Binding {
            target: target.into(),
            variable: variable.into(),
            source: "source1".into(),
            reference: "op://Vault with spaces/item/field".into(),
        }
    }
    #[test]
    fn an_existing_variable_can_check_its_legacy_reference_on_its_saved_source() {
        let mut data = SecretSources::default();
        let mut saved = binding("codex", "MODEL_TOKEN");
        saved.reference = "op://Team/Démo/key".into();
        let mut field = SecretReference {
            source: saved.source.clone(),
            reference: saved.reference.clone(),
        };
        assert!(data.can_check_reference(&field).is_err());
        data.bindings.push(saved);
        assert!(data.can_check_reference(&field).is_ok());
        field.source = "another-source".into();
        assert!(data.can_check_reference(&field).is_err());
    }

    #[tokio::test]
    async fn owner_bindings_resolve_only_for_the_selected_runtime_and_remove_together() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("sources.json");
        let op = temp.path().join("op");
        let source = Source {
            name: "Work".into(),
            account: "work.example".into(),
        };
        update(&file, Change::SaveSource("source1".into(), source.clone())).unwrap();
        assert!(update(&file, Change::SaveSource("source2".into(), source)).is_err());
        assert!(update(&file, Change::Bind(binding("codex", "PATH"))).is_err());
        assert!(update(&file, Change::Bind(binding("acp", "OWNER_KEY"))).is_err());
        let data = update(&file, Change::Bind(binding("acp/owned", "OWNER_KEY"))).unwrap();
        assert!(data.resolve("claude", &op).await.unwrap().is_empty());
        assert!(data.resolve("acp/other", &op).await.unwrap().is_empty());
        fs::write(&op,"#!/bin/sh\n[ \"$#\" -eq 5 ] && [ \"$1\" = read ] && [ \"$2\" = --no-newline ] && [ \"$3\" = --account ] && [ \"$4\" = work.example ] && [ \"$5\" = 'op://Vault with spaces/item/field' ] || exit 1\nprintf fixture-only-output").unwrap();
        fs::set_permissions(&op, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            data.resolve("acp/owned", &op).await.unwrap(),
            [("OWNER_KEY".into(), "fixture-only-output".into())]
        );
        assert!(!fs::read_to_string(&file)
            .unwrap()
            .contains("fixture-only-output"));
        fs::write(&op, "#!/bin/sh\necho fixture-only-output >&2\nexit 1").unwrap();
        assert!(!data
            .resolve("acp/owned", &op)
            .await
            .unwrap_err()
            .to_string()
            .contains("fixture-only-output"));
        update(&file, Change::RemoveSource("source1".into())).unwrap();
        assert!(read(&file).unwrap().bindings.is_empty());
    }
}
