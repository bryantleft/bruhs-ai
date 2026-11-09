//! Bruhs.app starts this daemon with `BRUHS_EXIT_WITH_PARENT=1`; if the app
//! dies without stopping it (a crash, a force quit), the daemon is
//! reparented and exits instead of holding the port for the next launch.

use std::time::Duration;

pub fn exit_with_parent() {
    if std::env::var_os("BRUHS_EXIT_WITH_PARENT").is_none_or(|v| v != "1") {
        return;
    }
    #[cfg(unix)]
    {
        let parent = std::os::unix::process::parent_id();
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_secs(1));
            if std::os::unix::process::parent_id() != parent {
                tracing::info!("the app that started bruhs-capture is gone; exiting");
                std::process::exit(0);
            }
        });
    }
}
