use super::*;

fn git(hooks: &Path, args: &[&str], stdin: Option<String>) -> Command {
    let mut argv = vec![
        "-c".into(),
        format!("core.hooksPath={}", hooks.display()),
        "-c".into(),
        "core.fsmonitor=false".into(),
        "-c".into(),
        "rebase.updateRefs=false".into(),
        "-c".into(),
        "rebase.autoStash=false".into(),
        "-c".into(),
        "commit.gpgSign=false".into(),
        "-c".into(),
        "rerere.enabled=false".into(),
    ];
    argv.extend(args.iter().map(|argument| (*argument).to_owned()));
    Command {
        program: "git".into(),
        args: argv,
        stdin,
    }
}

impl DeliveryService {
    async fn local_ref(&self, path: &Path, branch: &str) -> DeliveryResult<String> {
        let name = format!("refs/heads/{branch}");
        if !self
            .read(path, "git", &["for-each-ref", "--format=%(symref)", &name])
            .await?
            .trim()
            .is_empty()
        {
            return Err(conflict(format!("{branch} is a symbolic branch ref. Resolve it to a direct local branch before a stack rebase.")));
        }
        let reference = format!("refs/heads/{branch}^{{commit}}");
        let oid = self.read(path, "git", &["rev-parse", "--verify", &reference]).await
            .map_err(|_| conflict(format!("The local branch {branch} is unavailable. Fetch/create the reviewed stack branches before rebasing.")))?;
        let oid = oid.trim().to_owned();
        valid_oid(&oid)?;
        Ok(oid)
    }

    async fn ancestor(
        &self,
        path: &Path,
        ancestor: &str,
        descendant: &str,
    ) -> DeliveryResult<bool> {
        // rev-list returns success for unrelated histories too; a missing object or
        // command error remains an error rather than being mistaken for ancestry.
        let range = format!("{descendant}..{ancestor}");
        let count = self
            .read(path, "git", &["rev-list", "--count", &range])
            .await?;
        Ok(count.trim() == "0")
    }

    /// The isolated rebase cannot run external filters or merge drivers, so
    /// a stack whose attributes use one must be rebased in a terminal. Only
    /// drivers that an attributes file references count: Git LFS, for one,
    /// installs a global filter for every repository, which a repository
    /// that never uses it should not be refused over. `revisions` are the
    /// commits being replayed and the base they move onto.
    async fn rebase_configuration_safe(
        &self,
        path: &Path,
        revisions: &[&str],
    ) -> DeliveryResult<()> {
        let config = self
            .read(path, "git", &["config", "--null", "--list"])
            .await?;
        let mut external = std::collections::BTreeSet::new();
        let mut attribute_files = Vec::new();
        for entry in config.split('\0') {
            let (key, value) = entry.split_once('\n').unwrap_or((entry, ""));
            if let Some(name) = key
                .strip_prefix("merge.")
                .and_then(|rest| rest.strip_suffix(".driver"))
            {
                external.insert(format!("merge={name}"));
            } else if let Some(rest) = key.strip_prefix("filter.") {
                for suffix in [".clean", ".smudge", ".process"] {
                    if let Some(name) = rest.strip_suffix(suffix) {
                        external.insert(format!("filter={name}"));
                    }
                }
            } else if key == "core.attributesfile" && !value.is_empty() {
                attribute_files.push(std::path::PathBuf::from(value));
            }
        }
        if external.is_empty() {
            return Ok(());
        }
        let info = self
            .read(path, "git", &["rev-parse", "--git-path", "info/attributes"])
            .await?;
        attribute_files.push(path.join(info.trim()));
        let mut used = std::collections::BTreeSet::new();
        for file in attribute_files {
            let file = match file.strip_prefix("~") {
                Ok(rest) => directories::BaseDirs::new()
                    .map_or(file.clone(), |dirs| dirs.home_dir().join(rest)),
                Err(_) => file,
            };
            if let Ok(text) = tokio::fs::read_to_string(&file).await {
                used.extend(attribute_references(&text));
            }
        }
        let empty = self
            .read(path, "git", &["hash-object", "-t", "tree", "--stdin"])
            .await?;
        for revision in revisions {
            let files = self
                .read(
                    path,
                    "git",
                    &[
                        "diff",
                        "--name-only",
                        "-z",
                        empty.trim(),
                        revision,
                        "--",
                        ":(glob)**/.gitattributes",
                    ],
                )
                .await?;
            for file in files.split('\0').filter(|file| !file.is_empty()) {
                let blob = format!("{revision}:{file}");
                let text = self.read(path, "git", &["show", &blob]).await?;
                used.extend(attribute_references(&text));
            }
        }
        if let Some(driver) = used.iter().find(|reference| external.contains(*reference)) {
            return Err(conflict(format!(
                "This repository's attributes use the external Git {} `{}`. Use a terminal for this rebase so its side effects can be reviewed; the isolated rebase does not execute custom drivers.",
                if driver.starts_with("merge=") { "merge driver" } else { "filter" },
                driver.split_once('=').map_or(driver.as_str(), |(_, name)| name)
            )));
        }
        Ok(())
    }

    pub(super) async fn prepare_rebase(
        &self,
        path: &Path,
        plan: &mut StackPlan,
        id: &str,
    ) -> DeliveryResult<()> {
        let mut revisions = vec![plan.base_oid.as_str()];
        revisions.extend(plan.layers.iter().map(|layer| layer.head_oid.as_str()));
        self.rebase_configuration_safe(path, &revisions).await?;
        let root = self.local_ref(path, &plan.base).await?;
        // A rewritten trunk needs the owner's explicit repair decision. A normal
        // local trunk advance is safe and is included in the reviewed target SHA.
        if !self.ancestor(path, &plan.base_oid, &root).await? {
            return Err(conflict(
                "The local root does not descend from the reviewed GitHub root. Fetch and reconcile the root before an isolated stack rebase.",
            ));
        }
        plan.local_base_oid = Some(root.clone());
        let mut parent_local = root;
        let mut parent_remote = plan.base_oid.clone();
        for (index, layer) in plan.layers.iter_mut().enumerate() {
            let local = self.local_ref(path, &layer.head).await?;
            let cut = if self.ancestor(path, &parent_local, &local).await? {
                parent_local.clone()
            } else if index > 0 && self.ancestor(path, &parent_remote, &local).await? {
                // The remote parent's frozen head proves the old boundary when
                // the local parent was amended but its child has not been restacked.
                parent_remote.clone()
            } else if index == 0 {
                let bases = self
                    .read(path, "git", &["merge-base", "--all", &parent_local, &local])
                    .await?;
                let bases: Vec<_> = bases.split_whitespace().collect();
                if bases.len() != 1 {
                    return Err(conflict(
                        "The root and bottom branch have ambiguous merge bases.",
                    ));
                }
                bases[0].to_owned()
            } else {
                return Err(conflict(format!(
                    "The previous head of {} cannot be proven to be an ancestor of {}. Reconcile the branches in a terminal; replaying a guessed boundary could duplicate or discard commits.",
                    layer.base, layer.head
                )));
            };
            valid_oid(&cut)?;
            let range = format!("{cut}..{local}");
            if !self
                .read(path, "git", &["rev-list", "--merges", &range])
                .await?
                .trim()
                .is_empty()
            {
                return Err(conflict(format!(
                    "{} contains merge commits in its replay range. Resolve its history in a terminal before a linear whole-stack rebase.",
                    layer.head
                )));
            }
            layer.local_oid = Some(local.clone());
            layer.replay_after_oid = Some(cut);
            parent_local = local;
            parent_remote.clone_from(&layer.head_oid);
        }
        let root = tokio::fs::canonicalize(&*self.root)
            .await
            .map_err(|error| DeliveryError::Storage(error.to_string()))?;
        plan.temporary_worktree = Some(
            root.join("stack-runs")
                .join(id)
                .join("worktree")
                .to_string_lossy()
                .into_owned(),
        );
        Ok(())
    }

    async fn rebase_leases(&self, path: &Path, plan: &StackPlan) -> DeliveryResult<()> {
        if Some(self.local_ref(path, &plan.base).await?).as_ref() != plan.local_base_oid.as_ref() {
            return Err(conflict(
                "The local root branch changed after review. No stack branch was published.",
            ));
        }
        for layer in &plan.layers {
            if Some(self.local_ref(path, &layer.head).await?).as_ref() != layer.local_oid.as_ref() {
                return Err(conflict(format!(
                    "Local branch {} changed after review. No stack branch was published.",
                    layer.head
                )));
            }
        }
        Ok(())
    }

    pub(super) async fn rebase_stack(
        &self,
        prepared: &PreparedAction,
        plan: &StackPlan,
        cancel: &mut watch::Receiver<bool>,
        cleanup_owned: &mut bool,
    ) -> DeliveryResult<()> {
        let source = Path::new(&prepared.path);
        self.rebase_leases(source, plan).await?;
        let mut revisions = vec![plan.base_oid.as_str()];
        revisions.extend(plan.layers.iter().map(|layer| layer.head_oid.as_str()));
        self.rebase_configuration_safe(source, &revisions).await?;
        let scratch = plan
            .temporary_worktree
            .as_deref()
            .map(Path::new)
            .ok_or_else(|| conflict("The reviewed isolated worktree is missing."))?;
        let directory = scratch
            .parent()
            .ok_or_else(|| conflict("The reviewed isolated worktree path is invalid."))?;
        let expected_root = tokio::fs::canonicalize(&*self.root)
            .await
            .map_err(|error| DeliveryError::Storage(error.to_string()))?;
        if directory != expected_root.join("stack-runs").join(&prepared.id) || scratch.exists() {
            return Err(conflict(
                "The isolated worktree path already exists or no longer belongs to this reviewed action. Inspect it before preparing another rebase.",
            ));
        }
        let runs = expected_root.join("stack-runs");
        match tokio::fs::create_dir(&runs).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let metadata = tokio::fs::symlink_metadata(&runs)
                    .await
                    .map_err(|error| DeliveryError::Storage(error.to_string()))?;
                if !metadata.is_dir() || metadata.file_type().is_symlink() {
                    return Err(conflict(
                        "The private stack-runs path is not a regular directory.",
                    ));
                }
            }
            Err(error) => return Err(DeliveryError::Storage(error.to_string())),
        }
        tokio::fs::create_dir(directory).await.map_err(|error| conflict(format!("The private action scratch directory could not be exclusively created: {error}. Existing contents were left untouched.")))?;
        *cleanup_owned = true;
        let hooks = directory.join("hooks");
        tokio::fs::create_dir_all(&hooks)
            .await
            .map_err(|error| DeliveryError::Storage(error.to_string()))?;
        let first = plan
            .layers
            .first()
            .and_then(|layer| layer.local_oid.as_deref())
            .ok_or_else(|| conflict("The frozen bottom head is unavailable."))?;
        self.stack_exec(
            prepared,
            source,
            git(
                &hooks,
                &[
                    "worktree",
                    "add",
                    "--detach",
                    &scratch.to_string_lossy(),
                    first,
                ],
                None,
            ),
            cancel,
        )
        .await?;
        let mut onto = plan
            .local_base_oid
            .clone()
            .ok_or_else(|| conflict("The frozen local root is unavailable."))?;
        let mut heads = Vec::with_capacity(plan.layers.len());
        for layer in &plan.layers {
            let local = layer
                .local_oid
                .as_deref()
                .ok_or_else(|| conflict("A frozen local stack head is unavailable."))?;
            let cut = layer
                .replay_after_oid
                .as_deref()
                .ok_or_else(|| conflict("A proven replay boundary is unavailable."))?;
            self.stack_exec(
                prepared,
                scratch,
                git(
                    &hooks,
                    &[
                        "rebase",
                        "--merge",
                        "--no-rebase-merges",
                        "--no-autosquash",
                        "--no-autostash",
                        "--no-update-refs",
                        "--empty=drop",
                        "--onto",
                        &onto,
                        cut,
                        local,
                    ],
                    None,
                ),
                cancel,
            )
            .await?;
            onto = self
                .read(scratch, "git", &["rev-parse", "HEAD"])
                .await?
                .trim()
                .to_owned();
            valid_oid(&onto)?;
            heads.push(onto.clone());
            self.stack_log(prepared, None, &format!("Replayed {} in the detached scratch worktree at {onto}. Source branch refs are still unchanged.", layer.head)).await?;
        }
        self.stack_running(prepared, cancel).await?;
        self.validate_stack_graph(source, plan, prepared.forge)
            .await?;
        self.stack_checkout_safe(source, plan).await?;
        self.rebase_leases(source, plan).await?;
        let root = plan
            .local_base_oid
            .as_deref()
            .ok_or_else(|| conflict("The frozen root lease is unavailable."))?;
        let mut transaction = format!("start\nverify refs/heads/{} {root}\n", plan.base);
        for (layer, new) in plan.layers.iter().zip(&heads) {
            let old = layer
                .local_oid
                .as_deref()
                .ok_or_else(|| conflict("A frozen branch lease is unavailable."))?;
            transaction.push_str(&format!("update refs/heads/{} {new} {old}\n", layer.head));
        }
        transaction.push_str("prepare\ncommit\n");
        self.stack_log(prepared, None, "All isolated layers succeeded. Publishing every local branch in one compare-and-swap ref transaction; no remote push will run.").await?;
        self.stack_running(prepared, cancel).await?;
        let output = self
            .run_command(
                source,
                &git(
                    &hooks,
                    &["update-ref", "--no-deref", "--stdin"],
                    Some(transaction),
                ),
                cancel,
                prepared.repository.as_ref(),
            )
            .await?;
        self.stack_log(prepared, Some(plan.layers.len()), &format!("{output}\nEvery reviewed local branch was published atomically. The source checkout and remote branches were left at their existing commits; push these local branches separately after inspecting them.")).await?;
        Ok(())
    }
}

/// `filter=NAME` and `merge=NAME` references in an attributes file, macro
/// definitions included. Unsetting (`-filter`, `!merge`) is not a use.
fn attribute_references(text: &str) -> impl Iterator<Item = String> + '_ {
    text.lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .flat_map(str::split_whitespace)
        .filter(|token| token.starts_with("filter=") || token.starts_with("merge="))
        .map(str::to_owned)
}
