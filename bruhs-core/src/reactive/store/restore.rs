//! Pausing a restored crew in its staged database (D14, ruling F18).
//!
//! A backup's bruhs shouldn't act on a new computer before the owner looks,
//! and their provider sessions live in folders a backup doesn't carry. This
//! runs on the staged copy only, never on live data, and isn't a command:
//! unlike `StopAll` it also pauses managed presets, and it keeps every
//! approval grant and open approval the owner backed up.

use super::{
    always_on, cancel_runs, crew, rooms, runs_where, save_run, transaction, ReactiveError,
};
use bruhs_models::{reactive::RunStatus, EpochMs};
use rusqlite::{Connection, OptionalExtension};

const RESTORED: &str =
    "Stopped when this data was restored. Review it before starting another run.";
/// What a run the restore's halt ended says: the words a server stop leaves
/// (`store::open`), since the server stops for the restore right after.
const INTERRUPTED: &str = "The server stopped during this run. Review its output before explicitly retrying; work was not replayed.";

/// In one transaction: pause every agent's profile (managed presets
/// included; an earlier pause keeps its time), cancel its pending and
/// running runs, forget every provider session and empty
/// `reactive_session_stamps`. Grants stay. A database without the agent
/// and run tables has no crew and is left as it is.
pub(crate) fn pause_all_for_restore(connection: &mut Connection) -> Result<(), ReactiveError> {
    if !has_table(connection, "reactive_agents")? || !has_table(connection, "reactive_runs")? {
        return Ok(());
    }
    transaction(connection, |connection| {
        // An older Bruhs may lack the tables written below; these are the
        // statements the live open runs.
        always_on::initialize(connection)?;
        crew::initialize(connection)?;
        rooms::initialize(connection)?;
        let ids = {
            let mut statement = connection.prepare("SELECT id FROM reactive_agents ORDER BY id")?;
            let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        let now = EpochMs::now();
        for id in &ids {
            cancel_runs(connection, "agent_id", id, RESTORED)?;
            let mut profile = crew::profile(connection, id)?;
            if profile.paused_at.is_none() {
                profile.paused_at = Some(now);
                crew::save(connection, id, &profile)?;
            }
        }
        connection.execute("UPDATE reactive_agents SET session_id=NULL", [])?;
        connection.execute("DELETE FROM reactive_session_stamps", [])?;
        Ok(())
    })
}

/// In one transaction: every running run of the live crew becomes
/// interrupted, its waiting approvals expired, as the next open after a
/// server stop would leave it (ruling F19). Nothing is paused or
/// cancelled, and no grant or other approval changes, so the data a
/// restore keeps aside puts back as the owner left it.
pub(in crate::reactive) fn interrupt_running(
    connection: &mut Connection,
) -> Result<(), ReactiveError> {
    transaction(connection, |connection| {
        for mut run in runs_where(connection, "status='running'", None)? {
            run.status = RunStatus::Interrupted;
            run.error = Some(INTERRUPTED.into());
            run.updated_at = EpochMs::now();
            save_run(connection, &run)?;
            crew::expire_run_approvals(connection, &run.id)?;
        }
        Ok(())
    })
}

fn has_table(connection: &Connection, name: &str) -> Result<bool, ReactiveError> {
    Ok(connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
            [name],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}
