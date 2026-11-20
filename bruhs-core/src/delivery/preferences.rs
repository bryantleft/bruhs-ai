use super::{DeliveryError, DeliveryResult, DeliveryService};
use bruhs_models::delivery::{
    DeliveryOverrides, DeliveryPreferenceChange, DeliveryPreferences, MergeMethod,
};
use std::path::Path;

fn effective(
    preferences: &std::collections::BTreeMap<String, DeliveryOverrides>,
    path: Option<&Path>,
) -> DeliveryPreferences {
    let env = preferences.get("").cloned().unwrap_or_default();
    let overrides = path.map_or_else(
        || env.clone(),
        |p| {
            preferences
                .get(p.to_string_lossy().as_ref())
                .cloned()
                .unwrap_or_default()
        },
    );
    DeliveryPreferences {
        review_guides_enabled: overrides
            .review_guides_enabled
            .or(env.review_guides_enabled)
            .unwrap_or(false),
        review_guide_model: overrides
            .review_guide_model
            .clone()
            .or(env.review_guide_model)
            .unwrap_or_default(),
        review_guide_instructions: overrides
            .review_guide_instructions
            .clone()
            .or(env.review_guide_instructions)
            .unwrap_or_default(),
        writing_style: overrides
            .writing_style
            .or(env.writing_style)
            .unwrap_or_default(),
        writer_instructions: overrides
            .writer_instructions
            .clone()
            .or(env.writer_instructions)
            .unwrap_or_default(),
        writer_model: overrides
            .writer_model
            .clone()
            .or(env.writer_model)
            .unwrap_or_default(),
        merge_method: overrides
            .merge_method
            .or(env.merge_method)
            .unwrap_or(MergeMethod::Squash),
        follow_templates: overrides
            .follow_templates
            .or(env.follow_templates)
            .unwrap_or(true),
        remove_agent_credits: overrides
            .remove_agent_credits
            .or(env.remove_agent_credits)
            .unwrap_or(false),
        overrides,
    }
}
pub(super) fn effective_writer(
    preferences: &std::collections::BTreeMap<String, DeliveryOverrides>,
    path: &Path,
    main: Option<&Path>,
) -> DeliveryPreferences {
    let mut result = effective(preferences, Some(path));
    if let Some(main) = main {
        let parent = effective(preferences, Some(main));
        if result.overrides.review_guides_enabled.is_none() {
            result.review_guides_enabled = parent.review_guides_enabled;
        }
        if result.overrides.review_guide_model.is_none() {
            result.review_guide_model = parent.review_guide_model;
        }
        if result.overrides.review_guide_instructions.is_none() {
            result.review_guide_instructions = parent.review_guide_instructions;
        }
        if result.overrides.writer_model.is_none() {
            result.writer_model = parent.writer_model;
        }
        if result.overrides.writing_style.is_none() {
            result.writing_style = parent.writing_style;
        }
        if result.overrides.writer_instructions.is_none() {
            result.writer_instructions = parent.writer_instructions;
        }
        if result.overrides.follow_templates.is_none() {
            result.follow_templates = parent.follow_templates;
        }
    }
    result
}
impl DeliveryService {
    /// Branch creation also works when the companion service is offline. The host
    /// atomically replaces this journal, so a reader observes one completed save.
    pub async fn saved_writer_model(
        path: &Path,
    ) -> DeliveryResult<bruhs_models::delivery::WriterModel> {
        #[derive(serde::Deserialize, Default)]
        struct Saved {
            #[serde(default)]
            preferences: std::collections::BTreeMap<String, DeliveryOverrides>,
        }
        let file = Self::default_path()?.join("journal.json");
        let main = super::writer::main_checkout(path).await?;
        let path = path.to_owned();
        tokio::task::spawn_blocking(move || {
            let file = match std::fs::File::open(file) {
                Ok(file) => file,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(Default::default());
                }
                Err(err) => return Err(DeliveryError::Storage(err.to_string())),
            };
            let saved: Saved = serde_json::from_reader(std::io::BufReader::new(file))
                .map_err(|err| DeliveryError::Storage(err.to_string()))?;
            let project = path.canonicalize().unwrap_or(path);
            Ok(effective_writer(&saved.preferences, &project, main.as_deref()).writer_model)
        })
        .await
        .map_err(|err| DeliveryError::Storage(err.to_string()))?
    }

    pub async fn preferences(&self, path: Option<&Path>) -> DeliveryPreferences {
        let main = match path {
            Some(path) => super::writer::main_checkout(path).await.ok().flatten(),
            None => None,
        };
        let journal = self.state.lock().await;
        match path {
            Some(path) => effective_writer(&journal.preferences, path, main.as_deref()),
            None => effective(&journal.preferences, None),
        }
    }
    pub async fn set_preference(
        &self,
        path: Option<&Path>,
        change: DeliveryPreferenceChange,
    ) -> DeliveryResult<DeliveryPreferences> {
        let endpoint_writer = match &change {
            DeliveryPreferenceChange::WriterModel(Some(bruhs_models::delivery::WriterModel::Separate { model, .. }))
                if crate::text_generators::split_selection(model).is_some() =>
            {
                crate::text_generators::active(model).await.map_err(DeliveryError::Storage)?
            }
            _ => false,
        };
        let main = match path {
            Some(path) => super::writer::main_checkout(path).await?,
            None => None,
        };
        self.mutate(|journal| {
            let key = path
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            let mut prefs = journal.preferences.get(&key).cloned().unwrap_or_default();
            match change {
                DeliveryPreferenceChange::GenerationOption {
                    target,
                    model,
                    kind,
                    value,
                } => {
                    use bruhs_models::delivery::{
                        generation::{model_options, GenerationTarget},
                        GuideModel, WriterModel,
                    };
                    let current = match path {
                        Some(path) => effective_writer(&journal.preferences, path, main.as_deref()),
                        None => effective(&journal.preferences, None),
                    };
                    let (_, previous) = target
                        .selection(&current)
                        .filter(|(active, _)| *active == model)
                        .ok_or_else(|| {
                            DeliveryError::Invalid(
                                "The selected model changed. Choose its options again.".into(),
                            )
                        })?;
                    let mut options = previous.clone();
                    if !model_options(&model)
                        .iter()
                        .any(|o| o.kind == kind && options.set(o, &value))
                    {
                        return Err(DeliveryError::Invalid(
                            "Choose an option supported by this model.".into(),
                        ));
                    }
                    match target {
                        GenerationTarget::Writer => {
                            prefs.writer_model = Some(WriterModel::Separate { model, options })
                        }
                        GenerationTarget::ReviewGuide => {
                            prefs.review_guide_model = Some(GuideModel::Separate { model, options })
                        }
                    }
                }
                DeliveryPreferenceChange::ReviewGuidesEnabled(value) => {
                    prefs.review_guides_enabled = value
                }
                DeliveryPreferenceChange::ReviewGuideInstructions(value) => {
                    prefs.review_guide_instructions = value.map(|text| text.trim().to_owned())
                }
                DeliveryPreferenceChange::ReviewGuideModel(value) => {
                    if let Some(bruhs_models::delivery::GuideModel::Separate { model, options }) =
                        &value
                    {
                        if !options.valid_for(model) {
                            return Err(DeliveryError::Invalid(
                                "Choose options supported by this model.".into(),
                            ));
                        }
                        if !bruhs_models::agent::model_info(model)
                            .is_some_and(|m| !m.provider.is_external())
                        {
                            return Err(DeliveryError::Invalid(
                                "Choose a coding agent for review guides.".into(),
                            ));
                        }
                    }
                    prefs.review_guide_model = value;
                }
                DeliveryPreferenceChange::WritingStyle(mode) => prefs.writing_style = mode,
                DeliveryPreferenceChange::WriterInstructions(instructions) => {
                    prefs.writer_instructions = instructions.map(|text| text.trim().to_owned());
                }
                DeliveryPreferenceChange::ResetWritingStyle => {
                    prefs.writer_instructions = None;
                    prefs.writing_style = None;
                }
                DeliveryPreferenceChange::WriterModel(value) => {
                    if let Some(bruhs_models::delivery::WriterModel::Separate { model, options }) =
                        &value
                    {
                        if !options.valid_for(model) {
                            return Err(DeliveryError::Invalid(
                                "Choose options supported by this model.".into(),
                            ));
                        }
                        if !endpoint_writer && !bruhs_models::agent::model_info(model)
                            .is_some_and(|m| !m.provider.is_external())
                        {
                            return Err(DeliveryError::Invalid(
                                "Choose an available text generation model.".into(),
                            ));
                        }
                    }
                    prefs.writer_model = value;
                }
                DeliveryPreferenceChange::MergeMethod(v) => prefs.merge_method = v,
                DeliveryPreferenceChange::FollowTemplates(v) => prefs.follow_templates = v,
                DeliveryPreferenceChange::RemoveAgentCredits(v) => prefs.remove_agent_credits = v,
            }
            if prefs == DeliveryOverrides::default() {
                journal.preferences.remove(&key);
            } else {
                journal.preferences.insert(key, prefs);
            }
            Ok(match path {
                Some(path) => effective_writer(&journal.preferences, path, main.as_deref()),
                None => effective(&journal.preferences, None),
            })
        })
        .await
    }

    /// Read a versioned regular file from HEAD. A committed symlink is never followed.
    pub async fn pull_template(&self, path: &Path) -> DeliveryResult<Option<String>> {
        if !self.preferences(Some(path)).await.follow_templates {
            return Ok(None);
        }
        self.tracked_pull_template(path).await
    }

    pub(super) async fn tracked_pull_template(
        &self,
        path: &Path,
    ) -> DeliveryResult<Option<String>> {
        const CANDIDATES: &[&str] = &[
            ".github/pull_request_template.md",
            ".github/PULL_REQUEST_TEMPLATE.md",
            "pull_request_template.md",
            "PULL_REQUEST_TEMPLATE.md",
            "docs/pull_request_template.md",
            "docs/PULL_REQUEST_TEMPLATE.md",
            ".gitlab/merge_request_templates/Default.md",
        ];
        let mut args = vec!["ls-tree", "-r", "-z", "HEAD", "--"];
        args.extend(CANDIDATES);
        let tree = self.read(path, "git", &args).await?;
        for candidate in CANDIDATES {
            let Some(oid) = tree
                .split('\0')
                .filter_map(|row| row.split_once('\t'))
                .find_map(|(metadata, name)| {
                    if name != *candidate {
                        return None;
                    }
                    let mut fields = metadata.split_ascii_whitespace();
                    (matches!(fields.next(), Some("100644" | "100755"))
                        && fields.next() == Some("blob"))
                    .then(|| fields.next())
                    .flatten()
                })
            else {
                continue;
            };
            if !(oid.len() == 40 || oid.len() == 64) || !oid.bytes().all(|c| c.is_ascii_hexdigit())
            {
                return Err(DeliveryError::Command(
                    "Git returned an invalid template object.".into(),
                ));
            }
            return self
                .read(path, "git", &["cat-file", "blob", oid])
                .await
                .map(Some);
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn concurrent_writer_options_snapshot_the_inherited_model_and_reject_stale_edits() {
        use bruhs_models::delivery::{
            generation::{GenerationTarget, OptionKind},
            WriterModel,
        };
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        let host = DeliveryService::open(root.path()).await.unwrap();
        let legacy =
            serde_json::from_value(serde_json::json!({"kind":"separate","model":"gpt-6.1-sol"}))
                .unwrap();
        host.set_preference(None, DeliveryPreferenceChange::WriterModel(Some(legacy)))
            .await
            .unwrap();
        let edit = |model: &str, kind, value: &str| DeliveryPreferenceChange::GenerationOption {
            target: GenerationTarget::Writer,
            model: model.to_owned(),
            kind,
            value: value.to_owned(),
        };
        let (effort, tier) = tokio::join!(
            host.set_preference(
                Some(&project),
                edit("gpt-6.1-sol", OptionKind::Effort, "ultra")
            ),
            host.set_preference(
                Some(&project),
                edit("gpt-6.1-sol", OptionKind::ServiceTier, "priority")
            ),
        );
        effort.unwrap();
        tier.unwrap();
        for invalid in [
            edit("gpt-6-luna", OptionKind::Effort, "max"),
            edit("gpt-6.1-sol", OptionKind::ContextWindow, "1m"),
        ] {
            assert!(host.set_preference(Some(&project), invalid).await.is_err());
        }
        drop(host);
        let host = DeliveryService::open(root.path()).await.unwrap();
        let env = host.preferences(None).await;
        let repo = host.preferences(Some(&project)).await;
        assert!(
            matches!(env.writer_model, WriterModel::Separate { options,.. } if options.is_empty())
        );
        let (model, options) = GenerationTarget::Writer.selection(&repo).unwrap();
        assert_eq!(options.value(model, OptionKind::Effort), Some("ultra"));
        assert_eq!(
            options.value(model, OptionKind::ServiceTier),
            Some("priority")
        );
    }

    #[tokio::test]
    async fn writer_settings_inherit_field_by_field_in_a_linked_checkout() {
        use bruhs_models::delivery::{WriterModel, WritingStyle};
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        let linked = root.path().join("linked");
        let host = DeliveryService::open(root.path()).await.unwrap();
        let separate = WriterModel::Separate {
            options: Default::default(),
            model: "claude-haiku-4-5".into(),
        };
        for (path, change) in [
            (
                None,
                DeliveryPreferenceChange::WriterModel(Some(separate.clone())),
            ),
            (
                Some(project.as_path()),
                DeliveryPreferenceChange::WritingStyle(Some(WritingStyle::Custom)),
            ),
            (
                Some(project.as_path()),
                DeliveryPreferenceChange::WriterInstructions(Some("  Keep issue IDs.  ".into())),
            ),
            (
                Some(linked.as_path()),
                DeliveryPreferenceChange::WriterModel(Some(WriterModel::TextGeneration)),
            ),
        ] {
            host.set_preference(path, change).await.unwrap();
        }
        drop(host);
        let host = DeliveryService::open(root.path()).await.unwrap();
        let read = || async {
            effective_writer(
                &host.state.lock().await.preferences,
                &linked,
                Some(&project),
            )
        };
        let p = read().await;
        assert_eq!(p.writer_model, WriterModel::TextGeneration);
        assert_eq!(p.writing_style, WritingStyle::Custom);
        assert_eq!(p.writer_instructions, "Keep issue IDs.");
        assert!(host
            .set_preference(
                Some(&linked),
                DeliveryPreferenceChange::WriterModel(Some(WriterModel::Separate {
                    options: Default::default(),
                    model: "missing-model".into()
                }))
            )
            .await
            .is_err());
        host.set_preference(Some(&linked), DeliveryPreferenceChange::WriterModel(None))
            .await
            .unwrap();
        assert_eq!(read().await.writer_model, separate);
        host.set_preference(Some(&project), DeliveryPreferenceChange::ResetWritingStyle)
            .await
            .unwrap();
        assert_eq!(read().await.writing_style, WritingStyle::Repository);
        assert!(read().await.writer_instructions.is_empty());
    }
    #[tokio::test]
    async fn repository_preferences_inherit_and_survive_reopening_the_host() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("checkout");
        let svc = DeliveryService::open(tmp.path()).await.unwrap();
        svc.set_preference(
            None,
            DeliveryPreferenceChange::MergeMethod(Some(MergeMethod::Rebase)),
        )
        .await
        .unwrap();
        svc.set_preference(
            Some(&repo),
            DeliveryPreferenceChange::FollowTemplates(Some(false)),
        )
        .await
        .unwrap();
        drop(svc);
        let svc = DeliveryService::open(tmp.path()).await.unwrap();
        assert_eq!(
            svc.preferences(Some(&repo)).await.merge_method,
            MergeMethod::Rebase
        );
        assert!(!svc.preferences(Some(&repo)).await.follow_templates);
        assert!(svc.preferences(None).await.follow_templates);
        let reset = svc
            .set_preference(Some(&repo), DeliveryPreferenceChange::FollowTemplates(None))
            .await
            .unwrap();
        assert!(reset.follow_templates);
    }
    #[tokio::test]
    async fn template_comes_from_a_tracked_blob_in_the_selected_checkout() {
        let repo = tempfile::tempdir().unwrap();
        let store = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            assert!(std::process::Command::new("git")
                .args(args)
                .current_dir(repo.path())
                .output()
                .unwrap()
                .status
                .success())
        };
        git(&["init", "-q"]);
        std::fs::create_dir(repo.path().join(".github")).unwrap();
        std::fs::write(
            repo.path().join(".github/PULL_REQUEST_TEMPLATE.md"),
            "## Summary\n\n## Tests\n",
        )
        .unwrap();
        git(&["add", ".github"]);
        git(&[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-qm",
            "template",
        ]);
        let service = DeliveryService::open(store.path()).await.unwrap();
        assert!(service
            .pull_template(repo.path())
            .await
            .unwrap()
            .unwrap()
            .contains("## Tests"));
        service
            .set_preference(None, DeliveryPreferenceChange::FollowTemplates(Some(false)))
            .await
            .unwrap();
        assert_eq!(service.pull_template(repo.path()).await.unwrap(), None);
    }
}
