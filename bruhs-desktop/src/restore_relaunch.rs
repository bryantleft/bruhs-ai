//! After a restore is scheduled the desktop starts a new instance of
//! itself and quits (D17): the new one waits for this one's instance lock,
//! then applies the restore before it opens anything. Its quit save lands
//! in the data that becomes the pre-restore set, which is fine.

use std::path::Path;
use std::process::{Command, Stdio};

use gpui::App;

/// Starts a new instance of this app, detached from this one, then quits:
/// a packaged app through `open -n <bundle>`, a source build by running
/// its own executable in a process group of its own.
#[cfg_attr(
    test,
    expect(dead_code, reason = "tests hand Settings, Backup a stand-in")
)]
pub fn relaunch_and_quit(cx: &mut App) {
    let bundle = bruhs_core::updates::release::current_app_bundle();
    let exe = std::env::current_exe().ok();
    let spawned = relaunch_command(bundle.as_deref(), exe.as_deref())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    match spawned {
        // `open` returns at once; a source build's child is reaped by init
        // once this app exits.
        Ok(child) => drop(child),
        Err(error) => tracing::warn!("couldn't start Bruhs again: {error}"),
    }
    cx.quit();
}

fn relaunch_command(bundle: Option<&Path>, exe: Option<&Path>) -> Command {
    match (bundle, exe) {
        (Some(bundle), _) => {
            let mut command = Command::new("/usr/bin/open");
            command.arg("-n").arg(bundle);
            command
        }
        (None, exe) => {
            #[cfg_attr(
                not(unix),
                expect(unused_mut, reason = "only unix starts it in a group of its own")
            )]
            let mut command = Command::new(exe.unwrap_or_else(|| Path::new("bruhs-desktop")));
            #[cfg(unix)]
            std::os::unix::process::CommandExt::process_group(&mut command, 0);
            command
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_relaunch_opens_a_new_instance_of_the_bundle_or_runs_the_build() {
        let bundled = relaunch_command(Some(Path::new("/Applications/Bruhs.app")), None);
        assert_eq!(bundled.get_program(), "/usr/bin/open");
        let args: Vec<_> = bundled.get_args().collect();
        assert_eq!(args, ["-n", "/Applications/Bruhs.app"]);
        let source = relaunch_command(None, Some(Path::new("/src/target/debug/bruhs-desktop")));
        assert_eq!(source.get_program(), "/src/target/debug/bruhs-desktop");
        assert_eq!(source.get_args().count(), 0);
    }
}
