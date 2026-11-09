//! The shell around the views: navigation, panels, projects, git and
//! one-shot page requests.

use super::*;

#[derive(Copy, Clone)]
pub struct ActiveView(pub ReadSignal<AppView>);
#[derive(Copy, Clone)]
pub struct SetActiveView(pub WriteSignal<AppView>);

/// A request for a page to do something once (open a dialog, plan the day,
/// save), sent from elsewhere: the command bar, a shortcut. The page may not
/// be mounted yet; the request waits for it. Each request is handled once,
/// so revisiting the page does not act on an old one again.
#[derive(Copy, Clone)]
pub struct OneShot {
    requested: RwSignal<u64>,
    handled: StoredValue<u64>,
}

impl OneShot {
    pub fn new() -> Self {
        Self {
            requested: RwSignal::new(0),
            handled: StoredValue::new(0),
        }
    }

    /// Ask for the action.
    pub fn request(self) {
        self.requested.update(|n| *n = n.wrapping_add(1));
    }

    /// `true` once for each request not yet handled, marking it handled.
    /// Tracks the request, so an effect calling it runs on the next one.
    pub fn take(self) -> bool {
        let requested = self.requested.get();
        let fresh = self.handled.get_value() != requested;
        self.handled.set_value(requested);
        fresh
    }
}

/// ⌘K "Plan my day": Today proposes a plan.
#[derive(Copy, Clone)]
pub struct PlanDayRequest(pub OneShot);

/// ⌘K "Smart scheduling": Calendar opens its scheduling panel.
#[derive(Copy, Clone)]
pub struct SchedulingOpenRequest(pub OneShot);

/// ⌘K "New web clip": Knowledge opens the clip dialog.
#[derive(Copy, Clone)]
pub struct KnowledgeCreateClipRequest(pub OneShot);

/// ⌘S on a document: the document page saves.
#[derive(Copy, Clone)]
pub struct KnowledgeSaveRequest(pub OneShot);

/// ⌘K "Import conversation": Inbox opens the import dialog.
#[derive(Copy, Clone)]
pub struct InboxCreateRequest(pub OneShot);

#[derive(Copy, Clone)]
pub struct CommandBarOpen(pub ReadSignal<bool>);
#[derive(Copy, Clone)]
pub struct SetCommandBarOpen(pub WriteSignal<bool>);

/// Active project — the folder the chat agent runs in. `None` means "server's
/// CWD" (useful for first-run / unconfigured states).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActiveProject {
    pub path: String,
    pub name: String,
}

#[derive(Copy, Clone)]
pub struct ActiveProjectSignal(pub ReadSignal<Option<ActiveProject>>);
#[derive(Copy, Clone)]
pub struct SetActiveProjectSignal(pub WriteSignal<Option<ActiveProject>>);

/// The "Import thread" dialog (⌘K → Import thread...).
#[derive(Copy, Clone)]
pub struct ImportThreadOpen(pub RwSignal<bool>);

#[derive(Copy, Clone)]
pub struct ProjectPickerOpen(pub ReadSignal<bool>);
#[derive(Copy, Clone)]
pub struct SetProjectPickerOpen(pub WriteSignal<bool>);

/// Recent projects list, persisted across sessions.
#[derive(Copy, Clone)]
pub struct RecentProjectsSignal(pub ReadSignal<Vec<ActiveProject>>);
#[derive(Copy, Clone)]
pub struct SetRecentProjectsSignal(pub WriteSignal<Vec<ActiveProject>>);

/// What the project picker does with the chosen folder.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ProjectPickerMode {
    #[default]
    NewThread,
    NewTerminal,
}

#[derive(Copy, Clone)]
pub struct ProjectPickerModeSignal(pub RwSignal<ProjectPickerMode>);

/// Surfaces the right panel can show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PanelSurface {
    Diff,
    Tasks,
    Preview,
    /// The project tree. Not the open attachment.
    Files,
    /// The GitHub pull request for this thread's branch.
    Pull,
    /// The open attachment or a file the agent linked. Not the project tree.
    File,
}

/// The pull request the right panel is showing for the active thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShownBranchPull {
    pub key: String,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub state: String,
}

#[derive(Copy, Clone)]
pub struct BranchPullSignal(pub RwSignal<Option<ShownBranchPull>>);

/// A file the viewer is showing. `source` opens HTML as markup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenFile {
    pub path: String,
    pub project: Option<String>,
    pub source: bool,
    pub label: String,
}

#[derive(Copy, Clone)]
pub struct OpenFileSignal(pub RwSignal<Option<OpenFile>>);

/// Preference key for the last preview URL of a project (`None` = general).
pub fn preview_pref_key(project_path: Option<&str>) -> String {
    format!("bruhs.previewUrl:{}", project_path.unwrap_or(""))
}

pub use bruhs_models::logic::preview::normalize_preview_url;

/// Right-panel open state plus which surface is showing. `surface == None`
/// while open shows the launcher grid.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RightPanel {
    pub open: bool,
    pub surface: Option<PanelSurface>,
}

impl RightPanel {
    /// Toggle a specific surface: opening the panel on it, or closing the
    /// panel when that surface is already showing.
    pub fn toggle_surface(&mut self, surface: PanelSurface) {
        if self.open && self.surface == Some(surface) {
            self.open = false;
        } else {
            self.open = true;
            self.surface = Some(surface);
        }
    }

    /// Toggle the panel itself, keeping the last surface.
    pub fn toggle(&mut self) {
        self.open = !self.open;
    }

    pub fn show(&mut self, surface: PanelSurface) {
        self.open = true;
        self.surface = Some(surface);
    }
}

#[derive(Copy, Clone)]
pub struct RightPanelSignal(pub RwSignal<RightPanel>);

/// Right-panel width in CSS px, persisted to `bruhs.rightPanelWidth`.
#[derive(Copy, Clone)]
pub struct RightPanelWidth(pub RwSignal<f64>);

pub const RIGHT_PANEL_DEFAULT_WIDTH: f64 = 540.0;
pub const RIGHT_PANEL_MIN_WIDTH: f64 = 360.0;
/// The chat column never drops below this while the panel is open.
pub const CHAT_COLUMN_MIN_WIDTH: f64 = 360.0;

/// Clamp a requested panel width: at least the minimum, at most 70% of the
/// viewport, and never so wide the chat column collapses.
pub fn clamp_panel_width(requested: f64, viewport_width: f64) -> f64 {
    let max = (viewport_width * 0.7)
        .min(viewport_width - CHAT_COLUMN_MIN_WIDTH)
        .round();
    if max <= RIGHT_PANEL_MIN_WIDTH {
        return RIGHT_PANEL_MIN_WIDTH;
    }
    requested.clamp(RIGHT_PANEL_MIN_WIDTH, max)
}

/// What the Diff surface is showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffScope {
    /// Edits made by the assistant turn that answered this user message.
    Turn { user_message_id: String },
    /// `git diff` of the project working tree.
    Uncommitted,
}

#[derive(Copy, Clone)]
pub struct DiffScopeSignal(pub RwSignal<Option<DiffScope>>);

/// File the diff panel should scroll to once its scope has loaded (set by
/// the changed-files card); the panel clears it after focusing.
#[derive(Copy, Clone)]
pub struct DiffFocusPath(pub RwSignal<Option<String>>);

/// Sidebar project filter: `None` = all, `Some("")` = general threads only.
#[derive(Copy, Clone)]
pub struct SidebarProjectFilter(pub RwSignal<Option<String>>);

#[derive(Copy, Clone)]
pub struct WorkspaceMode(pub RwSignal<bruhs_models::SidebarTab>);

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct GitDirtyFile {
    pub path: String,
    pub status: String,
    pub staged: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct GitStatus {
    pub branch: Option<String>,
    #[serde(default)]
    pub ahead: u32,
    #[serde(default)]
    pub behind: u32,
    #[serde(default)]
    pub dirty: Vec<GitDirtyFile>,
}

#[derive(Copy, Clone)]
pub struct GitStatusSignal(pub RwSignal<GitStatus>);

/// The shell's state: navigation, panels, projects, git, banners and the
/// one-shot page requests.
pub(super) fn provide() {
    let (sidebar_open, set_sidebar_open) = signal(true);
    let (command_bar_open, set_command_bar_open) = signal(false);
    let (active_view, set_active_view) = signal(AppView::Chat);

    let initial_active_project = load_pref("bruhs.activeProject")
        .and_then(|s| serde_json::from_str::<ActiveProject>(&s).ok());
    let (active_project, set_active_project) =
        signal::<Option<ActiveProject>>(initial_active_project);
    Effect::new(move |_| match active_project.get() {
        Some(p) => {
            if let Ok(s) = serde_json::to_string(&p) {
                save_pref("bruhs.activeProject", &s);
            }
        }
        None => save_pref("bruhs.activeProject", ""),
    });

    let initial_recent_projects = load_pref("bruhs.recentProjects")
        .and_then(|s| serde_json::from_str::<Vec<ActiveProject>>(&s).ok())
        .unwrap_or_default();
    let (recent_projects, set_recent_projects) = signal(initial_recent_projects);
    Effect::new(move |_| {
        if let Ok(s) = serde_json::to_string(&recent_projects.get()) {
            save_pref("bruhs.recentProjects", &s);
        }
    });

    let (project_picker_open, set_project_picker_open) = signal(false);
    let project_picker_mode = RwSignal::new(ProjectPickerMode::NewThread);

    let right_panel = RwSignal::new(RightPanel::default());
    let initial_panel_width = load_pref("bruhs.rightPanelWidth")
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|w| w.is_finite() && *w >= RIGHT_PANEL_MIN_WIDTH)
        .unwrap_or(RIGHT_PANEL_DEFAULT_WIDTH);
    let right_panel_width = RwSignal::new(initial_panel_width);
    Effect::new(move |_| {
        save_pref(
            "bruhs.rightPanelWidth",
            &right_panel_width.get().round().to_string(),
        );
    });

    let initial_filter = load_pref("bruhs.sidebarProjectFilter").filter(|s| s != "*");
    let initial_mode = load_pref("bruhs.workspaceMode").map_or_else(
        || {
            if initial_filter.as_ref().is_some_and(|name| !name.is_empty()) {
                bruhs_models::SidebarTab::Code
            } else {
                bruhs_models::SidebarTab::Chat
            }
        },
        |value| bruhs_models::SidebarTab::from_str(&value),
    );
    let workspace_mode = RwSignal::new(initial_mode);
    Effect::new(move |_| save_pref("bruhs.workspaceMode", workspace_mode.get().as_str()));
    let sidebar_project_filter = RwSignal::new(initial_filter.filter(|name| !name.is_empty()));
    Effect::new(move |_| {
        let v = sidebar_project_filter.get();
        save_pref("bruhs.sidebarProjectFilter", v.as_deref().unwrap_or("*"));
    });

    provide_context(SidebarOpen(sidebar_open));
    provide_context(SetSidebarOpen(set_sidebar_open));
    provide_context(CommandBarOpen(command_bar_open));
    provide_context(SetCommandBarOpen(set_command_bar_open));
    provide_context(ActiveView(active_view));
    provide_context(SetActiveView(set_active_view));
    provide_context(ActiveProjectSignal(active_project));
    provide_context(SetActiveProjectSignal(set_active_project));
    provide_context(RecentProjectsSignal(recent_projects));
    provide_context(SetRecentProjectsSignal(set_recent_projects));
    provide_context(ImportThreadOpen(RwSignal::new(false)));
    provide_context(ProjectPickerOpen(project_picker_open));
    provide_context(SetProjectPickerOpen(set_project_picker_open));
    provide_context(ProjectPickerModeSignal(project_picker_mode));
    provide_context(RightPanelSignal(right_panel));
    provide_context(RightPanelWidth(right_panel_width));
    provide_context(BranchPullSignal(RwSignal::new(None)));
    provide_context(OpenFileSignal(RwSignal::new(None)));
    provide_context(DiffScopeSignal(RwSignal::new(Option::<DiffScope>::None)));
    provide_context(DiffFocusPath(RwSignal::new(Option::<String>::None)));
    provide_context(SidebarProjectFilter(sidebar_project_filter));
    provide_context(WorkspaceMode(workspace_mode));
    provide_context(GitStatusSignal(RwSignal::new(GitStatus::default())));
    provide_context(crate::components::banner::BannerStackSignal(RwSignal::new(
        VecDeque::<crate::components::banner::Banner>::new(),
    )));
    provide_context(PlanDayRequest(OneShot::new()));
    provide_context(SchedulingOpenRequest(OneShot::new()));
    provide_context(KnowledgeCreateClipRequest(OneShot::new()));
    provide_context(KnowledgeSaveRequest(OneShot::new()));
    provide_context(InboxCreateRequest(OneShot::new()));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A request waits for its page and is handled once: remounting the page
    /// (a new consumer effect calling `take`) does not act on it again.
    #[test]
    fn a_one_shot_request_is_handled_once() {
        let owner = Owner::new();
        owner.with(|| {
            let shot = OneShot::new();
            assert!(!shot.take(), "nothing requested yet");
            shot.request();
            assert!(shot.take());
            assert!(!shot.take(), "a revisit does not reopen");
            shot.request();
            shot.request();
            assert!(shot.take(), "two quick requests act once");
            assert!(!shot.take());
        });
    }

    #[test]
    fn preview_url_preference_is_per_project() {
        assert_eq!(preview_pref_key(Some("/p")), "bruhs.previewUrl:/p");
        assert_eq!(preview_pref_key(None), "bruhs.previewUrl:");
    }

    #[test]
    fn right_panel_toggle_surface_semantics() {
        let mut p = RightPanel::default();
        p.toggle_surface(PanelSurface::Diff);
        assert_eq!(
            p,
            RightPanel {
                open: true,
                surface: Some(PanelSurface::Diff)
            }
        );
        p.toggle_surface(PanelSurface::Tasks);
        assert_eq!(p.surface, Some(PanelSurface::Tasks));
        p.toggle_surface(PanelSurface::Tasks);
        assert!(!p.open);
        p.toggle();
        assert!(p.open);
        assert_eq!(p.surface, Some(PanelSurface::Tasks));
        p.toggle_surface(PanelSurface::Files);
        assert_eq!(p.surface, Some(PanelSurface::Files));
        p.toggle_surface(PanelSurface::Pull);
        assert!(p.open);
        assert_eq!(p.surface, Some(PanelSurface::Pull));
    }

    #[test]
    fn panel_width_clamps_to_viewport() {
        assert_eq!(clamp_panel_width(540.0, 1440.0), 540.0);
        assert_eq!(clamp_panel_width(100.0, 1440.0), RIGHT_PANEL_MIN_WIDTH);
        assert_eq!(clamp_panel_width(2000.0, 1440.0), 1008.0);
        // Narrow viewport: cap by the chat column floor, never below the minimum.
        assert_eq!(clamp_panel_width(600.0, 800.0), 440.0);
        assert_eq!(clamp_panel_width(600.0, 600.0), RIGHT_PANEL_MIN_WIDTH);
    }
}
