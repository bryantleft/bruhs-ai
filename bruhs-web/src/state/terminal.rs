//! Terminal tabs and the drawer.

use super::*;

/// Terminal drawer visibility (⌘J).
#[derive(Copy, Clone)]
pub struct TerminalOpen(pub RwSignal<bool>);

/// One shell session. The xterm instance and socket live in JS
/// (`window.bruhsTerminal`) for the life of the page; this is the metadata.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TerminalSession {
    pub id: String,
    /// `None` = home directory.
    pub cwd: Option<String>,
    pub title: String,
    pub project_name: Option<String>,
}

#[derive(Copy, Clone)]
pub struct TerminalSessions(pub RwSignal<Vec<TerminalSession>>);
#[derive(Copy, Clone)]
pub struct ActiveTerminalId(pub RwSignal<Option<String>>);
/// Bumped by the JS status callback so status labels re-render.
#[derive(Copy, Clone)]
pub struct TerminalStatusTick(pub RwSignal<u32>);

pub use bruhs_models::logic::terminal::terminal_title;

/// Which session becomes active after closing `closed`: the active one if
/// it survives, else the next tab, else the previous, else none.
pub fn next_active_after_close(
    sessions: &[TerminalSession],
    closed: &str,
    active: Option<&str>,
) -> Option<String> {
    if active.is_some_and(|a| a != closed) {
        return active.map(str::to_string);
    }
    let pos = sessions.iter().position(|s| s.id == closed)?;
    sessions
        .get(pos + 1)
        .or_else(|| pos.checked_sub(1).and_then(|p| sessions.get(p)))
        .map(|s| s.id.clone())
}

pub const TERMINAL_SESSIONS_PREF: &str = "bruhs.terminalSessions";
pub const ACTIVE_TERMINAL_PREF: &str = "bruhs.activeTerminal";

/// Merge the persisted tab list with what the server still runs: local
/// entries whose shell is gone are dropped (order kept), sessions created
/// elsewhere (another tab, the desktop) are appended.
pub fn reconcile_terminal_sessions(
    local: &[TerminalSession],
    server: &[crate::api::TerminalSessionInfo],
) -> Vec<TerminalSession> {
    let mut out: Vec<TerminalSession> = local
        .iter()
        .filter(|l| server.iter().any(|s| s.id == l.id))
        .cloned()
        .collect();
    for s in server {
        if !out.iter().any(|l| l.id == s.id) {
            out.push(TerminalSession {
                id: s.id.clone(),
                cwd: s.cwd.clone(),
                title: s.title.clone(),
                project_name: s.project_name.clone(),
            });
        }
    }
    out
}

pub const TERMINAL_DEFAULT_HEIGHT: f64 = 280.0;
pub const TERMINAL_MIN_HEIGHT: f64 = 160.0;
pub const TERMINAL_MAX_HEIGHT: f64 = 600.0;

/// Keep a requested drawer height inside the allowed band.
pub fn clamp_terminal_height(px: f64) -> f64 {
    if !px.is_finite() {
        return TERMINAL_DEFAULT_HEIGHT;
    }
    px.clamp(TERMINAL_MIN_HEIGHT, TERMINAL_MAX_HEIGHT)
}

/// Terminal tabs (persisted, then reconciled with the server's shells) and
/// the drawer.
pub(super) fn provide() {
    // Terminal tabs survive a reload: the shells live in the server, this is
    // the tab metadata. Reconciled against the server list right away.
    let terminal_sessions = RwSignal::new(
        load_json_pref::<Vec<TerminalSession>>(TERMINAL_SESSIONS_PREF).unwrap_or_default(),
    );
    let active_terminal_id =
        RwSignal::new(load_pref(ACTIVE_TERMINAL_PREF).filter(|s| !s.is_empty()));
    Effect::new(move |_| terminal_sessions.with(|s| save_json_pref(TERMINAL_SESSIONS_PREF, s)));
    Effect::new(move |_| {
        save_pref(
            ACTIVE_TERMINAL_PREF,
            active_terminal_id.get().as_deref().unwrap_or(""),
        )
    });
    leptos::task::spawn_local(async move {
        // Unreachable server: keep the local tabs rather than wiping them.
        let server = match crate::api::list_terminal_sessions().await {
            Ok(server) => server,
            Err(error) => {
                log::warn!("terminal sessions could not be listed: {error}");
                return;
            }
        };
        let next =
            terminal_sessions.with_untracked(|local| reconcile_terminal_sessions(local, &server));
        let active_ok = active_terminal_id.with_untracked(|a| {
            a.as_ref()
                .is_some_and(|id| next.iter().any(|s| s.id == *id))
        });
        let first = next.first().map(|s| s.id.clone());
        if terminal_sessions.try_set(next).is_none() && !active_ok {
            let _ = active_terminal_id.try_set(first);
        }
    });

    provide_context(TerminalOpen(RwSignal::new(false)));
    provide_context(TerminalSessions(terminal_sessions));
    provide_context(ActiveTerminalId(active_terminal_id));
    provide_context(TerminalStatusTick(RwSignal::new(0u32)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconcile_drops_dead_keeps_order_adopts_new() {
        let local = vec![
            TerminalSession {
                id: "a".into(),
                cwd: Some("/x".into()),
                title: "x".into(),
                project_name: None,
            },
            TerminalSession {
                id: "dead".into(),
                cwd: None,
                title: "Home".into(),
                project_name: None,
            },
            TerminalSession {
                id: "b".into(),
                cwd: None,
                title: "Home".into(),
                project_name: None,
            },
        ];
        let info = |id: &str, title: &str| crate::api::TerminalSessionInfo {
            id: id.into(),
            cwd: None,
            title: title.into(),
            project_name: None,
        };
        let server = vec![info("b", "Home"), info("new", "other"), info("a", "x")];
        let merged = reconcile_terminal_sessions(&local, &server);
        let ids: Vec<&str> = merged.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["a", "b", "new"]);
        assert_eq!(merged[2].title, "other");
        assert!(reconcile_terminal_sessions(&local, &[]).is_empty());
    }

    #[test]
    fn next_active_after_close_picks_neighbours() {
        let s = |id: &str| TerminalSession {
            id: id.to_string(),
            cwd: None,
            title: id.to_string(),
            project_name: None,
        };
        let sessions = vec![s("a"), s("b"), s("c")];
        assert_eq!(
            next_active_after_close(&sessions, "b", Some("a")).as_deref(),
            Some("a")
        );
        assert_eq!(
            next_active_after_close(&sessions, "b", Some("b")).as_deref(),
            Some("c")
        );
        assert_eq!(
            next_active_after_close(&sessions, "c", Some("c")).as_deref(),
            Some("b")
        );
        assert_eq!(
            next_active_after_close(&sessions[..1], "a", Some("a")),
            None
        );
        assert_eq!(next_active_after_close(&sessions, "zz", None), None);
    }
}
