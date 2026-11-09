//! Packaged builds update themselves from GitHub releases: check the latest release, download the update zip in
//! the background, verify it, then swap the app and relaunch on restart.
//!
//! Trust: the zip must match the release's `SHA256SUMS`, and the app inside
//! must pass `codesign --verify --deep --strict` against a requirement that
//! pins an Apple-issued Developer ID certificate with the running app's team
//! (checked again by the swap script right before the move). A build without
//! a team (ad-hoc or unsigned) never installs an update in place; it points
//! at the download page.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use bruhs_models::updates::{
    checksum_for, is_newer, release_asset_names, AppUpdate, AppUpdateState, ReleaseUpdate, Version,
    RELEASE_CHECKSUMS,
};
use sha2::{Digest, Sha256};

use super::BuildInfo;

const API_TIMEOUT: Duration = Duration::from_secs(10);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const USER_AGENT: &str = concat!("bruhs-update-check/", env!("CARGO_PKG_VERSION"));

/// The release asset architecture for this binary (`arm64`, `x86_64`).
pub fn release_arch() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "arm64",
        other => other,
    }
}

/// The `.app` bundle this process runs from, if any.
pub fn current_app_bundle() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
    exe.ancestors()
        .find(|p| p.extension().is_some_and(|e| e == "app"))
        .map(Path::to_path_buf)
}

/// What the latest release offers this architecture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LatestRelease {
    pub version: String,
    pub html_url: String,
    pub zip_url: Option<String>,
    pub checksums_url: Option<String>,
}

/// Read GitHub's `releases/latest` JSON.
pub(crate) fn parse_latest(json: &serde_json::Value, arch: &str) -> Result<LatestRelease, String> {
    let tag = json["tag_name"]
        .as_str()
        .ok_or("The latest release has no tag.")?;
    let version = Version::find(tag)
        .ok_or_else(|| format!("The latest release tag {tag:?} is not a version."))?
        .to_string();
    let (_, zip_name) = release_asset_names(arch);
    let asset = |name: &str| {
        json["assets"].as_array().and_then(|assets| {
            assets
                .iter()
                .find(|a| a["name"].as_str() == Some(name))
                .and_then(|a| a["browser_download_url"].as_str())
                .map(String::from)
        })
    };
    Ok(LatestRelease {
        version,
        html_url: json["html_url"].as_str().unwrap_or_default().to_string(),
        zip_url: asset(&zip_name),
        checksums_url: asset(RELEASE_CHECKSUMS),
    })
}

fn client(timeout: Duration) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(timeout)
        .user_agent(USER_AGENT)
        .build()
        .map_err(|error| error.to_string())
}

/// Check the running packaged build against the latest release.
pub(crate) async fn check(build: &BuildInfo, version: &str, online: bool) -> AppUpdate {
    let mut app = AppUpdate {
        state: AppUpdateState::Unknown,
        built_commit: build.commit.clone(),
        remote_commit: None,
        remote_branch: None,
        local_branch: None,
        repo_root: None,
        command: None,
        error: None,
        release: Some(ReleaseUpdate {
            current_version: version.to_string(),
            latest_version: None,
            release_url: None,
            download_url: None,
            sha256: None,
        }),
    };
    if !online {
        return app;
    }
    match latest(&build.release_repo, release_arch()).await {
        Ok((latest, sha256)) => {
            app.state = if is_newer(&latest.version, version) {
                AppUpdateState::Behind
            } else {
                AppUpdateState::UpToDate
            };
            if app.state == AppUpdateState::Behind && (latest.zip_url.is_none() || sha256.is_none())
            {
                app.error = Some(format!(
                    "v{} has no {} update for this Mac yet; download it from the release page.",
                    latest.version,
                    release_arch()
                ));
            }
            app.release = Some(ReleaseUpdate {
                current_version: version.to_string(),
                latest_version: Some(latest.version),
                release_url: Some(latest.html_url),
                download_url: latest.zip_url,
                sha256,
            });
        }
        Err(error) => app.error = Some(error),
    }
    app
}

async fn latest(repo: &str, arch: &str) -> Result<(LatestRelease, Option<String>), String> {
    let client = client(API_TIMEOUT)?;
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let response = client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|error| format!("GitHub could not be reached: {error}"))?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Err("No release has been published yet.".into());
    }
    let json: serde_json::Value = response
        .error_for_status()
        .map_err(|error| format!("GitHub refused the release check: {error}"))?
        .json()
        .await
        .map_err(|error| format!("GitHub sent an unreadable release: {error}"))?;
    let latest = parse_latest(&json, arch)?;
    let (_, zip_name) = release_asset_names(arch);
    let sha256 = match &latest.checksums_url {
        Some(url) => {
            let sums = client
                .get(url)
                .send()
                .await
                .and_then(reqwest::Response::error_for_status)
                .map_err(|error| format!("The release checksums could not be read: {error}"))?
                .text()
                .await
                .map_err(|error| format!("The release checksums could not be read: {error}"))?;
            checksum_for(&sums, &zip_name)
        }
        None => None,
    };
    Ok((latest, sha256))
}

// ---------------------------------------------------------------------------
// Download, verify, stage
// ---------------------------------------------------------------------------

async fn run(program: &str, args: &[&str]) -> Result<std::process::Output, String> {
    tokio::process::Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|error| format!("`{program}` could not run: {error}"))
}

const CODESIGN: &str = "/usr/bin/codesign";
const DITTO: &str = "/usr/bin/ditto";

/// The team id in `codesign -dv` output (it prints on stderr). Only a
/// well-formed Apple team id (ten uppercase letters or digits) counts, since
/// it ends up inside a code requirement and a shell script.
fn parse_team(codesign_stderr: &str) -> Option<String> {
    codesign_stderr
        .lines()
        .find_map(|line| line.strip_prefix("TeamIdentifier="))
        .map(str::trim)
        .filter(|team| {
            team.len() == 10
                && team
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        })
        .map(String::from)
}

/// The Developer ID team that signed `app`, if any.
pub async fn signing_team(app: &Path) -> Option<String> {
    let path = app.to_string_lossy();
    let out = run(CODESIGN, &["-dv", "--verbose=2", &path]).await.ok()?;
    parse_team(&String::from_utf8_lossy(&out.stderr))
}

/// [`signing_team`], for the quit path where no runtime is at hand.
pub fn signing_team_blocking(app: &Path) -> Option<String> {
    let out = std::process::Command::new(CODESIGN)
        .args(["-dv", "--verbose=2"])
        .arg(app)
        .stdin(Stdio::null())
        .output()
        .ok()?;
    parse_team(&String::from_utf8_lossy(&out.stderr))
}

/// A code requirement that holds only for an app signed with an
/// Apple-issued Developer ID Application certificate from `team`.
pub(crate) fn developer_id_requirement(team: &str) -> String {
    format!(
        "=anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] exists \
         and certificate leaf[field.1.2.840.113635.100.6.1.13] exists \
         and certificate leaf[subject.OU] = \"{team}\""
    )
}

/// `codesign --verify` of `app` against [`developer_id_requirement`].
async fn verify_developer_id(app: &Path, team: &str) -> Result<(), String> {
    let requirement = format!("-R{}", developer_id_requirement(team));
    let out = run(
        CODESIGN,
        &[
            "--verify",
            "--deep",
            "--strict",
            &requirement,
            &app.to_string_lossy(),
        ],
    )
    .await?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "The update's signature does not verify: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

async fn bundle_version(app: &Path) -> Option<String> {
    let plist = app.join("Contents/Info.plist");
    let plist = plist.to_string_lossy();
    let out = run(
        "/usr/libexec/PlistBuddy",
        &["-c", "Print :CFBundleShortVersionString", &plist],
    )
    .await
    .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Where updates are staged: beside the app, so the swap is a rename on one
/// volume.
pub fn staging_dir(current_app: &Path) -> Option<PathBuf> {
    current_app.parent().map(|dir| dir.join(".bruhs-update"))
}

/// Download `release`'s update zip, check it, unpack it and verify the app
/// inside. Returns the staged app, ready for [`install_on_exit`].
pub async fn stage_update(release: &ReleaseUpdate, current_app: &Path) -> Result<PathBuf, String> {
    let (Some(url), Some(expected), Some(version)) = (
        release.download_url.as_deref(),
        release.sha256.as_deref(),
        release.latest_version.as_deref(),
    ) else {
        return Err("This release has no update for this Mac.".into());
    };
    let team = signing_team(current_app).await.ok_or(
        "This copy of Bruhs is not signed with a Developer ID, so it cannot update itself. Download the new version instead.",
    )?;
    let stage = staging_dir(current_app).ok_or("The app's folder could not be found.")?;
    let _ = tokio::fs::remove_dir_all(&stage).await;
    let staged = download_and_verify(url, expected, version, &team, &stage).await;
    if staged.is_err() {
        let _ = tokio::fs::remove_dir_all(&stage).await;
    }
    staged
}

async fn download_and_verify(
    url: &str,
    expected: &str,
    version: &str,
    team: &str,
    stage: &Path,
) -> Result<PathBuf, String> {
    tokio::fs::create_dir_all(stage).await.map_err(|error| {
        format!(
            "Bruhs cannot write to {}: {error}. Move it to Applications, or download the new version.",
            stage.parent().map_or_else(String::new, |p| p.display().to_string())
        )
    })?;

    // Download, hashing as it arrives.
    let zip = stage.join("update.zip");
    let mut response = client(DOWNLOAD_TIMEOUT)?
        .get(url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| format!("The update could not be downloaded: {error}"))?;
    let mut file = tokio::fs::File::create(&zip)
        .await
        .map_err(|error| format!("The update could not be saved: {error}"))?;
    let mut hasher = Sha256::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("The download stopped: {error}"))?
    {
        hasher.update(&chunk);
        tokio::io::AsyncWriteExt::write_all(&mut file, &chunk)
            .await
            .map_err(|error| format!("The update could not be saved: {error}"))?;
    }
    tokio::io::AsyncWriteExt::flush(&mut file)
        .await
        .map_err(|error| format!("The update could not be saved: {error}"))?;
    let actual = format!("{:x}", hasher.finalize());
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(
            "The download does not match the release's checksum; nothing was installed.".into(),
        );
    }

    // Unpack and verify.
    let unpacked = stage.join("unpacked");
    let out = run(
        DITTO,
        &[
            "-x",
            "-k",
            &zip.to_string_lossy(),
            &unpacked.to_string_lossy(),
        ],
    )
    .await?;
    if !out.status.success() {
        return Err("The update could not be unpacked.".into());
    }
    let new_app = std::fs::read_dir(&unpacked)
        .map_err(|error| format!("The update could not be read: {error}"))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|e| e == "app"))
        .ok_or("The update has no app inside.")?;
    // A symlinked .app would let the swap move something outside the stage.
    let is_dir = std::fs::symlink_metadata(&new_app).is_ok_and(|meta| meta.file_type().is_dir());
    if !is_dir {
        return Err("The update's app is not a folder; nothing was installed.".into());
    }
    verify_developer_id(&new_app, team).await?;
    if signing_team(&new_app).await.as_deref() != Some(team) {
        return Err("The update is signed by a different developer; nothing was installed.".into());
    }
    if bundle_version(&new_app).await.as_deref() != Some(version) {
        return Err(format!(
            "The update is not v{version}; nothing was installed."
        ));
    }
    let _ = tokio::fs::remove_file(&zip).await;
    Ok(new_app)
}

fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// The shell check the swap script runs on the staged app right before the
/// move, so a bundle changed after staging is not installed.
pub(crate) fn verify_command(staged: &Path, team: &str) -> String {
    format!(
        "{CODESIGN} --verify --deep --strict {} {} >/dev/null 2>&1",
        shell_quote(&format!("-R{}", developer_id_requirement(team))),
        shell_quote(&staged.to_string_lossy()),
    )
}

/// The script that waits for `pid` to exit, re-runs `verify` (a shell
/// command) on the staged app, swaps it in, restores the old one if the swap
/// fails, relaunches, and cleans up. A failed check reopens the old app and
/// drops the stage.
pub(crate) fn swap_script(
    pid: u32,
    current_app: &Path,
    staged: &Path,
    stage: &Path,
    verify: &str,
) -> String {
    let quote = |p: &Path| shell_quote(&p.to_string_lossy());
    let (app, new, dir) = (quote(current_app), quote(staged), quote(stage));
    let backup = quote(&stage.join("previous.app"));
    format!(
        "while kill -0 {pid} 2>/dev/null; do sleep 0.2; done\n\
         if ! {verify}; then /bin/rm -rf {dir}; /usr/bin/open {app}; exit 1; fi\n\
         if /bin/mv {app} {backup}; then\n\
         \x20 if /bin/mv {new} {app}; then /bin/rm -rf {dir}; else /bin/mv {backup} {app}; fi\n\
         fi\n\
         /usr/bin/open {app}\n"
    )
}

/// Swap `staged` into `current_app` after this process exits, then reopen
/// it. The caller quits right after.
pub fn install_on_exit(current_app: &Path, staged: &Path) -> Result<(), String> {
    let stage = staging_dir(current_app).ok_or("The app's folder could not be found.")?;
    let team = signing_team_blocking(current_app)
        .ok_or("This copy of Bruhs is not signed with a Developer ID.")?;
    let verify = verify_command(staged, &team);
    let script = swap_script(std::process::id(), current_app, staged, &stage, &verify);
    let mut command = std::process::Command::new("/bin/sh");
    command
        .arg("-c")
        .arg(script)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    command
        .spawn()
        .map(drop)
        .map_err(|error| format!("The update could not be installed: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latest_release_json_is_read_for_this_arch() {
        let json = serde_json::json!({
            "tag_name": "v0.2.0",
            "html_url": "https://github.com/bryantleft/bruhs-ai/releases/tag/v0.2.0",
            "assets": [
                {"name": "Bruhs-macOS-arm64.dmg", "browser_download_url": "https://x/dmg"},
                {"name": "Bruhs-macOS-arm64.zip", "browser_download_url": "https://x/zip"},
                {"name": "SHA256SUMS", "browser_download_url": "https://x/sums"}
            ]
        });
        let latest = parse_latest(&json, "arm64").unwrap();
        assert_eq!(latest.version, "0.2.0");
        assert_eq!(latest.zip_url.as_deref(), Some("https://x/zip"));
        assert_eq!(latest.checksums_url.as_deref(), Some("https://x/sums"));
        assert_eq!(parse_latest(&json, "x86_64").unwrap().zip_url, None);
        assert!(parse_latest(&serde_json::json!({"tag_name": "nightly"}), "arm64").is_err());
    }

    #[test]
    fn the_swap_script_waits_restores_and_quotes() {
        let staged = Path::new("/Applications/.bruhs-update/unpacked/Bruhs.app");
        let script = swap_script(
            42,
            Path::new("/Applications/Bruhs.app"),
            staged,
            Path::new("/Applications/.bruhs-update"),
            &verify_command(staged, "KW4434WNW9"),
        );
        assert!(script.starts_with("while kill -0 42 "));
        assert!(script
            .contains("if ! /usr/bin/codesign --verify --deep --strict '-R=anchor apple generic"));
        assert!(script.contains(
            r#"leaf[subject.OU] = "KW4434WNW9"' '/Applications/.bruhs-update/unpacked/Bruhs.app'"#
        ));
        assert!(script.contains(
            "/bin/mv '/Applications/Bruhs.app' '/Applications/.bruhs-update/previous.app'"
        ));
        assert!(script.contains(
            "else /bin/mv '/Applications/.bruhs-update/previous.app' '/Applications/Bruhs.app'; fi"
        ));
        assert!(script.ends_with("/usr/bin/open '/Applications/Bruhs.app'\n"));
        let odd = swap_script(
            1,
            Path::new("/x/it's.app"),
            Path::new("/y"),
            Path::new("/z"),
            "true",
        );
        assert!(odd.contains(r"'/x/it'\''s.app'"));
    }

    /// The swap really moves bundles and restores on failure.
    #[test]
    fn the_swap_script_moves_the_bundle() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("Bruhs.app");
        let stage = dir.path().join(".bruhs-update");
        let staged = stage.join("unpacked/Bruhs.app");
        std::fs::create_dir_all(app.join("Contents")).unwrap();
        std::fs::write(app.join("Contents/v"), "old").unwrap();
        std::fs::create_dir_all(staged.join("Contents")).unwrap();
        std::fs::write(staged.join("Contents/v"), "new").unwrap();
        // u32::MAX is no live pid, so the wait ends at once; skip `open`.
        let run = |verify: &str| {
            let script =
                swap_script(u32::MAX, &app, &staged, &stage, verify).replace("open ", "true ");
            std::process::Command::new("/bin/sh")
                .arg("-c")
                .arg(script)
                .status()
                .unwrap()
                .success()
        };
        // A staged app that no longer verifies is dropped; the old one stays.
        assert!(!run("false"));
        assert_eq!(
            std::fs::read_to_string(app.join("Contents/v")).unwrap(),
            "old"
        );
        assert!(!stage.exists());
        std::fs::create_dir_all(staged.join("Contents")).unwrap();
        std::fs::write(staged.join("Contents/v"), "new").unwrap();
        assert!(run("true"));
        assert_eq!(
            std::fs::read_to_string(app.join("Contents/v")).unwrap(),
            "new"
        );
        assert!(!stage.exists());
    }

    #[test]
    fn only_a_well_formed_team_id_is_read() {
        let out = "Executable=/x\nAuthority=Developer ID Application: A (KW4434WNW9)\nTeamIdentifier=KW4434WNW9\n";
        assert_eq!(parse_team(out).as_deref(), Some("KW4434WNW9"));
        assert_eq!(parse_team("TeamIdentifier=not set\n"), None);
        assert_eq!(parse_team("TeamIdentifier=AB\"; x\n"), None);
        assert_eq!(parse_team("Signature=adhoc\n"), None);
    }

    #[test]
    fn the_requirement_pins_apple_developer_id_and_the_team() {
        let requirement = developer_id_requirement("KW4434WNW9");
        assert!(requirement.starts_with("=anchor apple generic and "));
        assert!(requirement.contains("certificate 1[field.1.2.840.113635.100.6.2.6] exists"));
        assert!(requirement.contains("certificate leaf[field.1.2.840.113635.100.6.1.13] exists"));
        assert!(requirement.ends_with(r#"certificate leaf[subject.OU] = "KW4434WNW9""#));
    }
}

/// End-to-end against a real signed build, served locally:
///
/// ```sh
/// scripts/bundle-macos.sh 0.1.0 --sign
/// (cd dist && python3 -m http.server 8124) &
/// BRUHS_TEST_APP=$PWD/dist/Bruhs.app BRUHS_TEST_ZIP_URL=http://127.0.0.1:8124/Bruhs-macOS-arm64.zip \
///   BRUHS_TEST_ZIP_SHA=$(shasum -a 256 dist/Bruhs-macOS-arm64.zip | cut -d' ' -f1) \
///   cargo test -p bruhs-core --lib stage_a_real_signed_update -- --ignored
/// ```
#[cfg(test)]
mod live {
    use super::*;

    #[tokio::test]
    #[ignore = "needs a signed dist/Bruhs.app and a local server; see the doc comment"]
    async fn stage_a_real_signed_update() {
        let app = PathBuf::from(std::env::var("BRUHS_TEST_APP").unwrap());
        let url = std::env::var("BRUHS_TEST_ZIP_URL").unwrap();
        let sha = std::env::var("BRUHS_TEST_ZIP_SHA").unwrap();
        // Stage beside a copy, so the build in dist/ stays as it is.
        let dir = tempfile::tempdir().unwrap();
        let current = dir.path().join("Bruhs.app");
        assert!(std::process::Command::new("ditto")
            .arg(&app)
            .arg(&current)
            .status()
            .unwrap()
            .success());
        let release = |sha: &str| ReleaseUpdate {
            current_version: "0.0.9".into(),
            latest_version: Some("0.1.0".into()),
            release_url: None,
            download_url: Some(url.clone()),
            sha256: Some(sha.into()),
        };
        let bad = stage_update(&release(&"0".repeat(64)), &current)
            .await
            .unwrap_err();
        assert!(bad.contains("checksum"), "{bad}");
        let staged = stage_update(&release(&sha), &current).await.unwrap();
        assert!(staged.ends_with("Bruhs.app"), "{}", staged.display());
        assert_eq!(signing_team(&staged).await, signing_team(&app).await);
        // The swap moves it into place.
        let stage = staging_dir(&current).unwrap();
        let team = signing_team(&app).await.unwrap();
        let script = swap_script(
            u32::MAX,
            &current,
            &staged,
            &stage,
            &verify_command(&staged, &team),
        )
        .replace("open ", "true ");
        assert!(std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(script)
            .status()
            .unwrap()
            .success());
        assert!(current.join("Contents/MacOS/bruhs-desktop").is_file());
        assert!(!stage.exists());
    }
}
