//! Restores where backup doesn't run (D27): none is ever scheduled, so
//! there is nothing to apply at start, `bruhs-server restore-apply` has
//! nothing to do and exits 0, and no restore asks this server to stop.

use crate::routes::backup::NoBackup;

/// `bruhs-server restore-apply [--once]`.
const RESTORE_APPLY: &str = "restore-apply";

/// `Some(0)` for `restore-apply`: nothing waits. `None` when the
/// arguments name another command.
pub(crate) fn restore_apply_cli() -> Option<i32> {
    (std::env::args_os().nth(1).as_deref() == Some(RESTORE_APPLY.as_ref())).then_some(0)
}

/// Nothing waits, so the server starts.
pub(crate) async fn apply_at_start() -> Option<i32> {
    None
}

/// Stands in for the restore control the restore routes take: there are
/// none here.
pub(crate) fn real(_backup: &NoBackup) -> NoBackup {
    NoBackup
}

/// Never resolves: no restore asks this server to stop.
pub(crate) async fn exit_requested() {
    std::future::pending::<()>().await;
}
