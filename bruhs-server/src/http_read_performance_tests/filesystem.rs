use super::*;
use std::path::PathBuf;

pub(super) async fn run(
    result: &mut Vec<Measurement>,
    records: usize,
    reverse: bool,
    warmups: usize,
    samples: usize,
) {
    // Keep the actual HomePath boundary; never substitute HOME or browse it.
    let target = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace")
        .join("target");
    let directory = tempfile::Builder::new()
        .prefix("http-read-")
        .tempdir_in(target)
        .expect("private confined filesystem fixture");
    let base = directory.path().canonicalize().expect("fixture path");
    assert!(base.starts_with(
        std::env::home_dir()
            .expect("real home")
            .canonicalize()
            .expect("canonical home")
    ));
    let root = base.join("entries");
    std::fs::create_dir(&root).expect("fixture entries root");
    let mut names = Vec::new();
    for index in 0..records {
        let name = if index == 0 {
            "Ω space".to_owned()
        } else {
            format!("d{index:05}")
        };
        std::fs::create_dir(root.join(&name)).expect("fixture child directory");
        names.push(name);
    }
    names.sort();
    std::fs::create_dir(root.join(".hidden")).expect("hidden fixture directory");
    std::fs::write(root.join("file.txt"), b"fixture\n").expect("fixture regular file");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(root.join(".hidden"), root.join("linked"))
            .expect("fixture directory symlink");
        std::os::unix::fs::symlink(root.join("missing"), root.join("broken"))
            .expect("fixture broken symlink");
    }
    let repo = base.join("repo");
    std::fs::create_dir(&repo).expect("fixture repo");
    let template = base.join("template");
    std::fs::create_dir(&template).expect("empty git template");
    git(
        &repo,
        &[
            "init",
            "-q",
            "--initial-branch=main",
            &format!("--template={}", template.display()),
        ],
    )
    .await;
    let empty = base.join("empty-config");
    std::fs::write(&empty, b"").expect("fixture empty attributes and excludes");
    for (key, value) in [
        ("core.fsmonitor", "false"),
        ("core.abbrev", "7"),
        ("core.autocrlf", "false"),
        ("diff.algorithm", "myers"),
        ("diff.noprefix", "false"),
        (
            "core.attributesFile",
            empty.to_str().expect("fixture attributes"),
        ),
        (
            "core.excludesFile",
            empty.to_str().expect("fixture excludes"),
        ),
    ] {
        git(&repo, &["config", "--local", key, value]).await;
    }
    std::fs::write(repo.join("tracked.txt"), b"base\n").expect("fixture initial content");
    git(&repo, &["add", "--", "tracked.txt"]).await;
    std::fs::write(repo.join("tracked.txt"), b"changed\n").expect("fixture modified content");
    let host = Host::new(&base, None).await;
    let mut cases = cases(&root, &repo, &names);
    assert_eq!(cases.len(), 9);
    if reverse {
        cases.reverse();
    }
    for mut case in cases {
        case.canonical = canonical_paths(&case.expected, &base);
        result.push(
            measure(
                &host,
                case,
                "filesystem",
                records,
                "mixed-directory-unborn-repo",
                warmups,
                samples,
            )
            .await,
        );
    }
    host.close().await;
    let path = directory.path().to_owned();
    directory
        .close()
        .expect("remove confined filesystem fixture");
    assert!(!path.exists(), "filesystem fixture cleanup");
}

async fn git(repo: &Path, args: &[&str]) {
    let output = tokio::process::Command::new("git")
        .args(args)
        .current_dir(repo)
        .kill_on_drop(true)
        .output()
        .await
        .expect("fixture git command");
    assert!(
        output.status.success(),
        "fixture git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn cases(root: &Path, repo: &Path, names: &[String]) -> Vec<ReadCase> {
    let dirs: Vec<_> = names
        .iter()
        .map(|name| json!({"name":name,"path":root.join(name)}))
        .collect();
    let mut children: Vec<_> = names
        .iter()
        .map(|name| json!({"name":format!("{name}/"),"path":format!("{name}/"),"isDirectory":true}))
        .collect();
    let mut files = vec!["file.txt"];
    #[cfg(unix)]
    files.extend(["broken", "linked"]);
    files.sort_unstable();
    children.extend(
        files
            .into_iter()
            .map(|name| json!({"name":name,"path":name,"isDirectory":false})),
    );
    children.truncate(500);
    vec![
        ReadCase::post(
            "list-dir",
            "POST /api/agent/list-dir",
            json!({"path":root}),
            json!({"current":root,"name":"entries","parent":root.parent(),"dirs":dirs}),
        ),
        ReadCase::post(
            "file-children",
            "POST /api/agent/files/children",
            json!({"path":root,"dir":""}),
            json!({"files":children}),
        ),
        ReadCase::post(
            "validate-directory",
            "POST /api/agent/validate-path",
            json!({"path":root}),
            json!({"valid":true,"name":"entries"}),
        ),
        ReadCase::post(
            "validate-file",
            "POST /api/agent/validate-path",
            json!({"path":root.join("file.txt")}),
            json!({"valid":false,"name":"","error":"Path is not a directory"}),
        ),
        ReadCase::post(
            "validate-missing",
            "POST /api/agent/validate-path",
            json!({"path":root.join("missing")}),
            json!({"valid":false,"name":"","error":"Directory not found"}),
        ),
        ReadCase::post(
            "children-no-project",
            "POST /api/agent/files/children",
            json!({}),
            json!({"error":"Open a project to browse files."}),
        )
        .error(400),
        ReadCase::post(
            "git-branches-unborn",
            "POST /api/agent/git-branches",
            json!({"path":repo}),
            json!({"branches":[],"current":null}),
        ),
        ReadCase::post(
            "git-status-unborn",
            "POST /api/agent/git-status",
            json!({"path":repo}),
            json!({"branch":"main","ahead":0,"behind":0,"dirty":[{"path":"tracked.txt","status":"M","staged":true}]}),
        ),
        // Blob abbreviations independently follow SHA-1("blob <len>\0<content>").
        ReadCase::post(
            "git-diff-unborn",
            "POST /api/agent/git-diff",
            json!({"path":repo}),
            json!({"diff":"diff --git a/tracked.txt b/tracked.txt\nindex df967b9..5ea2ed4 100644\n--- a/tracked.txt\n+++ b/tracked.txt\n@@ -1 +1 @@\n-base\n+changed\n","truncated":false}),
        ),
    ]
}

fn canonical_paths(value: &Value, base: &Path) -> Value {
    match value {
        Value::String(text) => {
            let path = PathBuf::from(text);
            match path.strip_prefix(base) {
                Ok(relative) if path.is_absolute() => {
                    json!(format!("$fixture/{}", relative.display()).trim_end_matches('/'))
                }
                _ => value.clone(),
            }
        }
        Value::Array(values) => Value::Array(
            values
                .iter()
                .map(|value| canonical_paths(value, base))
                .collect(),
        ),
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(key, value)| (key.clone(), canonical_paths(value, base)))
                .collect(),
        ),
        _ => value.clone(),
    }
}

#[tokio::test]
async fn directory_and_path_validation_preserve_the_real_home_boundary() {
    let target = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace")
        .join("target");
    let directory = tempfile::Builder::new()
        .prefix("http-boundary-")
        .tempdir_in(target)
        .expect("private boundary fixture");
    let file = directory.path().join("plain-file");
    std::fs::write(&file, b"fixture").expect("boundary fixture file");
    let open_error = std::fs::read_dir(&file).expect_err("regular file is not a directory");
    let host = Host::new(directory.path(), None).await;
    for case in [
        ReadCase::post(
            "outside-list",
            "POST /api/agent/list-dir",
            json!({"path":"/"}),
            json!({"error":"Path must be within your home directory"}),
        )
        .error(403),
        ReadCase::post(
            "outside-validation",
            "POST /api/agent/validate-path",
            json!({"path":"/"}),
            json!({"valid":false,"name":"","error":"Path must be within your home directory"}),
        ),
        ReadCase::post(
            "list-file",
            "POST /api/agent/list-dir",
            json!({"path":file}),
            json!({"error":format!("Cannot read directory: {open_error}")}),
        )
        .error(400),
    ] {
        let _ = measure(&host, case, "filesystem", 0, "boundary", 0, 1).await;
    }
    host.close().await;
}
