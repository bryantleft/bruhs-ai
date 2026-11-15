use super::{DeliveryError, DeliveryResult, DeliveryService};
use bruhs_models::delivery::*;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::Path;
#[cfg(test)]
mod tests;

pub(super) fn segment(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes())
        .collect::<String>()
        .replace('+', "%20")
}
fn endpoint(repo: &ForgeRepository, suffix: &str) -> String {
    format!("projects/{}{suffix}", segment(&repo.project))
}
pub(super) fn api(
    repo: &ForgeRepository,
    method: &str,
    suffix: &str,
    body: Option<Value>,
) -> Command {
    let mut args = vec![
        "api".into(),
        endpoint(repo, suffix),
        "--hostname".into(),
        repo.host.clone(),
        "--method".into(),
        method.into(),
    ];
    let stdin = body.map(|value| value.to_string());
    if stdin.is_some() {
        args.extend(["--input".into(), "-".into()]);
    }
    Command {
        program: "glab".into(),
        args,
        stdin,
    }
}
fn invalid(message: impl Into<String>) -> DeliveryError {
    DeliveryError::Invalid(message.into())
}
fn no_quick_actions(text: &str) -> DeliveryResult<()> {
    if text.lines().any(|line| line.trim_start().starts_with('/')) {
        return Err(invalid("GitLab executes slash-prefixed lines as quick actions. Escape the leading slash to post literal text, or use GitLab's own reviewed action flow."));
    }
    Ok(())
}
#[derive(Deserialize)]
struct MergeRequest {
    iid: u64,
    title: String,
    #[serde(default)]
    description: Option<String>,
    web_url: String,
    state: String,
    source_branch: String,
    target_branch: String,
    sha: String,
    updated_at: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    source_project_id: Option<u64>,
    #[serde(default)]
    target_project_id: Option<u64>,
    #[serde(default)]
    diff_refs: Value,
    #[serde(default)]
    merge_commit_sha: Option<String>,
    #[serde(default)]
    squash_commit_sha: Option<String>,
    #[serde(default)]
    detailed_merge_status: String,
    #[serde(default)]
    head_pipeline: Value,
}
fn note(value: &Value) -> PullComment {
    PullComment {
        id: value["id"]
            .as_u64()
            .map_or_else(String::new, |id| id.to_string()),
        author: value["author"]["username"]
            .as_str()
            .unwrap_or_default()
            .into(),
        body: value["body"].as_str().unwrap_or_default().into(),
        url: value["url"].as_str().map(str::to_owned),
        state: None,
        path: value["position"]["new_path"]
            .as_str()
            .or_else(|| value["position"]["old_path"].as_str())
            .map(str::to_owned),
        line: value["position"]["new_line"]
            .as_u64()
            .or_else(|| value["position"]["old_line"].as_u64())
            .and_then(|line| u32::try_from(line).ok()),
    }
}
fn pull(mr: MergeRequest, notes: Vec<PullComment>, approvals: &Value) -> Pull {
    let mergeable = match mr.detailed_merge_status.as_str() {
        "mergeable" => "MERGEABLE",
        "conflict" => "CONFLICTING",
        _ => "UNKNOWN",
    }
    .into();
    let status = mr.head_pipeline["status"].as_str();
    let checks = status
        .map(|status| {
            let (state, conclusion) = match status {
                "success" => ("COMPLETED", Some("SUCCESS")),
                "failed" => ("COMPLETED", Some("FAILURE")),
                "canceled" => ("COMPLETED", Some("CANCELLED")),
                "skipped" => ("COMPLETED", Some("SKIPPED")),
                "running" => ("IN_PROGRESS", None),
                "manual" => ("WAITING", None),
                _ => ("PENDING", None),
            };
            Check {
                name: "Pipeline".into(),
                state: state.into(),
                conclusion: conclusion.map(str::to_owned),
                url: mr.head_pipeline["web_url"].as_str().map(str::to_owned),
            }
        })
        .into_iter()
        .collect();
    let reviews = approvals["approved_by"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|user| PullComment {
            id: format!("approval-{}", user["user"]["id"]),
            author: user["user"]["username"].as_str().unwrap_or_default().into(),
            body: String::new(),
            url: None,
            state: Some("APPROVED".into()),
            path: None,
            line: None,
        })
        .collect();
    Pull {
        number: mr.iid,
        title: mr.title,
        body: mr.description.unwrap_or_default(),
        url: mr.web_url,
        state: match mr.state.as_str() {
            "merged" => "MERGED",
            "closed" => "CLOSED",
            _ => "OPEN",
        }
        .into(),
        draft: mr.draft,
        base: mr.target_branch,
        head: mr.source_branch,
        head_oid: mr.sha,
        head_in_repository: mr.source_project_id == mr.target_project_id,
        base_oid: mr.diff_refs["start_sha"]
            .as_str()
            .unwrap_or_default()
            .into(),
        merge_commit_oid: mr.merge_commit_sha.or(mr.squash_commit_sha),
        updated_at: mr.updated_at,
        mergeable,
        review_decision: if approvals["approved"] == true {
            Some("APPROVED".into())
        } else if mr.detailed_merge_status == "requested_changes" {
            Some("CHANGES_REQUESTED".into())
        } else {
            None
        },
        checks,
        comments: notes,
        reviews,
        files: Vec::new(),
    }
}

impl DeliveryService {
    pub(super) async fn gitlab_json(
        &self,
        path: &Path,
        repo: &ForgeRepository,
        suffix: &str,
    ) -> DeliveryResult<Value> {
        let command = api(repo, "GET", suffix, None);
        let args = command.args.iter().map(String::as_str).collect::<Vec<_>>();
        let output = self.read(path, "glab", &args).await?;
        serde_json::from_str(&output).map_err(|_| {
            DeliveryError::Command(
                "GitLab returned unreadable JSON. Upgrade/sign in to glab and retry.".into(),
            )
        })
    }
    pub(super) async fn gitlab_pulls(
        &self,
        path: &Path,
        repo: &ForgeRepository,
    ) -> DeliveryResult<Vec<Pull>> {
        let data = self
            .gitlab_json(
                path,
                repo,
                "/merge_requests?state=opened&scope=all&per_page=50",
            )
            .await?;
        let mrs: Vec<MergeRequest> = serde_json::from_value(data).map_err(|error| {
            DeliveryError::Command(format!("GitLab merge requests were invalid: {error}"))
        })?;
        Ok(mrs
            .into_iter()
            .map(|mr| pull(mr, Vec::new(), &Value::Null))
            .collect())
    }
    pub(super) async fn gitlab_pulls_from_branch(
        &self,
        path: &Path,
        repo: &ForgeRepository,
        branch: &str,
    ) -> DeliveryResult<Vec<Pull>> {
        let branch = segment(branch);
        let data = self
            .gitlab_json(
                path,
                repo,
                &format!("/merge_requests?state=all&scope=all&source_branch={branch}&order_by=created_at&sort=desc&per_page=5"),
            )
            .await?;
        let mrs: Vec<MergeRequest> = serde_json::from_value(data).map_err(|error| {
            DeliveryError::Command(format!("GitLab merge requests were invalid: {error}"))
        })?;
        Ok(mrs
            .into_iter()
            .map(|mr| pull(mr, Vec::new(), &Value::Null))
            .collect())
    }
    pub(super) async fn gitlab_pull(
        &self,
        path: &Path,
        repo: &ForgeRepository,
        number: u64,
    ) -> DeliveryResult<Pull> {
        let prefix = format!("/merge_requests/{number}");
        let notes_path = format!("{prefix}/notes?per_page=50");
        let approvals_path = format!("{prefix}/approvals");
        let (data, notes, approvals) = tokio::try_join!(
            self.gitlab_json(path, repo, &prefix),
            self.gitlab_json(path, repo, &notes_path),
            self.gitlab_json(path, repo, &approvals_path)
        )?;
        let mr = serde_json::from_value(data).map_err(|error| {
            DeliveryError::Command(format!("GitLab merge request was invalid: {error}"))
        })?;
        Ok(pull(
            mr,
            notes.as_array().into_iter().flatten().map(note).collect(),
            &approvals,
        ))
    }
    pub(super) async fn gitlab_diff(
        &self,
        path: &Path,
        repo: &ForgeRepository,
        number: u64,
    ) -> DeliveryResult<String> {
        let command = api(
            repo,
            "GET",
            &format!("/merge_requests/{number}/raw_diffs"),
            None,
        );
        self.read(
            path,
            "glab",
            &command.args.iter().map(String::as_str).collect::<Vec<_>>(),
        )
        .await
    }
    pub(super) async fn gitlab_comments(
        &self,
        path: &Path,
        repo: &ForgeRepository,
        number: u64,
    ) -> DeliveryResult<Vec<PullComment>> {
        let data = self
            .gitlab_json(
                path,
                repo,
                &format!("/merge_requests/{number}/discussions?per_page=100"),
            )
            .await?;
        let mut notes = Vec::new();
        for discussion in data.as_array().into_iter().flatten() {
            notes.extend(
                discussion["notes"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(note),
            );
        }
        Ok(notes)
    }
    pub(super) async fn gitlab_issue(
        &self,
        path: &Path,
        repo: &ForgeRepository,
        number: u64,
    ) -> DeliveryResult<Issue> {
        let data = self
            .gitlab_json(path, repo, &format!("/issues/{number}"))
            .await?;
        Ok(Issue {
            number: data["iid"]
                .as_u64()
                .ok_or_else(|| invalid("GitLab returned no issue number."))?,
            title: data["title"]
                .as_str()
                .ok_or_else(|| invalid("GitLab returned no issue title."))?
                .into(),
            body: data["description"].as_str().unwrap_or_default().into(),
            url: data["web_url"]
                .as_str()
                .ok_or_else(|| invalid("GitLab returned no issue URL."))?
                .into(),
        })
    }
    pub(super) async fn gitlab_plan(
        &self,
        path: &Path,
        repo: &ForgeRepository,
        action: &DeliveryAction,
        common: Vec<Command>,
    ) -> DeliveryResult<Vec<Command>> {
        let create = |title: &str,
                      body: &str,
                      base: &str,
                      head: &str,
                      draft: bool|
         -> DeliveryResult<Command> {
            no_quick_actions(body)?;
            Ok(api(
                repo,
                "POST",
                "/merge_requests",
                Some(
                    json!({"title":if draft {format!("Draft: {title}")} else {title.into()},"description":body,"source_branch":head,"target_branch":base}),
                ),
            ))
        };
        match action {
            DeliveryAction::CreatePull {
                title,
                body,
                base,
                head,
                draft,
            } => Ok(vec![create(title, body, base, head, *draft)?]),
            DeliveryAction::CommitPushPull {
                title,
                body,
                base,
                head,
                draft,
                ..
            } => {
                let mut commands = common
                    .into_iter()
                    .take_while(|command| command.program != "gh")
                    .collect::<Vec<_>>();
                commands.push(create(title, body, base, head, *draft)?);
                Ok(commands)
            }
            DeliveryAction::Comment { number, body } => {
                no_quick_actions(body)?;
                Ok(vec![api(
                    repo,
                    "POST",
                    &format!("/merge_requests/{number}/notes"),
                    Some(json!({"body":body})),
                )])
            }
            DeliveryAction::RequestReviewers { number, reviewers } => {
                let ids=reviewers.iter().map(|name|name.parse::<u64>().ok().filter(|id|*id>0).ok_or_else(||invalid("GitLab reviewer selection uses positive user IDs; obtain them from the project's member list."))).collect::<DeliveryResult<Vec<_>>>()?;
                Ok(vec![api(
                    repo,
                    "PUT",
                    &format!("/merge_requests/{number}"),
                    Some(json!({"reviewer_ids":ids})),
                )])
            }
            DeliveryAction::Merge {
                number,
                head_oid,
                method,
                auto,
            } => {
                if *method == MergeMethod::Rebase {
                    return Err(invalid("GitLab rebase strategy is a project setting. Use Squash, or configure merge commits and choose Merge; Bruhs will not silently substitute a merge strategy."));
                }
                if *method == MergeMethod::Merge {
                    let project = self.gitlab_json(path, repo, "").await?;
                    if !matches!(
                        project["merge_method"].as_str(),
                        Some("merge" | "rebase_merge")
                    ) {
                        return Err(invalid("This GitLab project does not use merge commits. Choose Squash or update its merge-method setting in GitLab."));
                    }
                }
                Ok(vec![api(
                    repo,
                    "PUT",
                    &format!("/merge_requests/{number}/merge"),
                    Some(
                        json!({"sha":head_oid,"squash":*method==MergeMethod::Squash,"auto_merge":auto}),
                    ),
                )])
            }
            DeliveryAction::Review {
                number,
                head_oid,
                event,
                body,
                comments,
            } => {
                no_quick_actions(body)?;
                let mr = self
                    .gitlab_json(path, repo, &format!("/merge_requests/{number}"))
                    .await?;
                if mr["sha"] != *head_oid {
                    return Err(invalid(
                        "The GitLab head changed. Reload its diff before reviewing.",
                    ));
                }
                let prefix = format!("/merge_requests/{number}");
                let mut commands = Vec::new();
                for comment in comments {
                    no_quick_actions(&comment.body)?;
                    let refs = &mr["diff_refs"];
                    if ["base_sha", "start_sha", "head_sha"]
                        .iter()
                        .any(|key| refs[*key].as_str().is_none_or(str::is_empty))
                    {
                        return Err(invalid("GitLab is still preparing diff references. Reload the merge request and retry."));
                    }
                    let mut position = json!({"position_type":"text","base_sha":refs["base_sha"],"start_sha":refs["start_sha"],"head_sha":head_oid,"old_path":comment.path,"new_path":comment.path});
                    position[if comment.side == DiffSide::Left {
                        "old_line"
                    } else {
                        "new_line"
                    }] = json!(comment.line);
                    commands.push(api(
                        repo,
                        "POST",
                        &format!("{prefix}/discussions"),
                        Some(json!({"body":comment.body,"position":position})),
                    ));
                }
                if *event == ReviewEvent::RequestChanges {
                    commands.push(api(
                        repo,
                        "POST",
                        &format!("{prefix}/notes"),
                        Some(json!({"body":body})),
                    ));
                    // The documented mutation changes review status only. A /submit_review
                    // quick action would also publish unrelated pending drafts owned by this user.
                    commands.push(Command{program:"glab".into(),args:vec!["api".into(),"graphql".into(),"--hostname".into(),repo.host.clone(),"--method".into(),"POST".into(),"--input".into(),"-".into()],stdin:Some(json!({"query":"mutation BruhsRequestChanges($project: ID!, $iid: String!) { mergeRequestRequestChanges(input: {projectPath: $project, iid: $iid}) { errors mergeRequest { iid } } }","variables":{"project":repo.project,"iid":number.to_string()}}).to_string())});
                } else {
                    if !body.trim().is_empty() {
                        commands.push(api(
                            repo,
                            "POST",
                            &format!("{prefix}/notes"),
                            Some(json!({"body":body})),
                        ));
                    }
                    if *event == ReviewEvent::Approve {
                        commands.push(api(
                            repo,
                            "POST",
                            &format!("{prefix}/approve"),
                            Some(json!({"sha":head_oid})),
                        ));
                    }
                }
                if commands.is_empty() {
                    return Err(invalid("Enter review text or add a line comment."));
                }
                Ok(commands)
            }
            DeliveryAction::MergeStack { .. } | DeliveryAction::RebaseStack { .. } => Ok(common),
            _ => Ok(common),
        }
    }
}

pub(super) fn validate_mutation_output(command: &Command, output: &str) -> DeliveryResult<()> {
    if command.program == "glab"
        && command
            .args
            .get(1)
            .is_some_and(|argument| argument == "graphql")
        && command
            .stdin
            .as_deref()
            .is_some_and(|body| body.contains("mutation BruhsRequestChanges"))
    {
        let value:Value=serde_json::from_str(output).map_err(|_|DeliveryError::Command("GitLab returned an unreadable review-status response. Inspect the MR; earlier comments remain published.".into()))?;
        if value
            .get("errors")
            .and_then(Value::as_array)
            .is_some_and(|errors| !errors.is_empty())
            || value["data"]["mergeRequestRequestChanges"]["errors"]
                .as_array()
                .is_none_or(|errors| !errors.is_empty())
            || value["data"]["mergeRequestRequestChanges"]["mergeRequest"]["iid"]
                .as_str()
                .is_none()
        {
            return Err(DeliveryError::Command("GitLab could not request changes. Check this account's review permission and this GitLab version's mergeRequestRequestChanges support; earlier comments remain published.".into()));
        }
    }
    Ok(())
}
