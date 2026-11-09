// Shared by the build scripts of the app's binaries (bruhs-server,
// bruhs-desktop, bruhs-capture): `include!("../bruhs-core/build_info.rs")`.
// Emits BRUHS_BUILD_COMMIT (the checkout's HEAD) and BRUHS_REPO_ROOT (its
// top level), both empty outside a git checkout, plus BRUHS_RELEASE_VERSION
// (empty for a source build; a release leaves BRUHS_REPO_ROOT empty), and
// reruns when HEAD moves.

fn git_output(args: &[&str]) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(std::env::var_os("CARGO_MANIFEST_DIR")?)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|text| !text.is_empty())
}

fn emit_build_info() {
        let commit = git_output(&["rev-parse", "HEAD"]).unwrap_or_default();
    // A packaged release (scripts/bundle-macos.sh sets the version) updates
    // from GitHub releases; the build machine's checkout means nothing to it.
    let release = std::env::var("BRUHS_RELEASE_VERSION").unwrap_or_default();
    let root = if release.trim().is_empty() {
        git_output(&["rev-parse", "--show-toplevel"]).unwrap_or_default()
    } else {
        String::new()
    };
    println!("cargo:rustc-env=BRUHS_RELEASE_VERSION={}", release.trim());
    println!("cargo:rerun-if-env-changed=BRUHS_RELEASE_VERSION");
    println!("cargo:rustc-env=BRUHS_BUILD_COMMIT={commit}");
    println!("cargo:rustc-env=BRUHS_REPO_ROOT={root}");
    // HEAD (per worktree), the branch it points at, and packed refs.
    let mut watched = vec![git_output(&["rev-parse", "--git-path", "HEAD"])];
    if let Some(branch) = git_output(&["symbolic-ref", "-q", "HEAD"]) {
        watched.push(git_output(&["rev-parse", "--git-path", &branch]));
    }
    watched.push(git_output(&["rev-parse", "--git-path", "packed-refs"]));
    let dir = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default());
    for path in watched.into_iter().flatten() {
        // `--git-path` answers relative to the manifest dir in a plain
        // checkout. A missing path would make cargo rebuild every time
        // (a branch packed by `git gc`, a repo with no packed-refs yet), so
        // only existing files are watched; HEAD always exists, and packing
        // or unpacking a branch touches packed-refs or the loose ref.
        let path = dir.join(path);
        if path.exists() {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
    println!("cargo:rerun-if-changed=build.rs");
}
