//! Cold SVG work belongs to workers. Entries are keyed by exact source, never
//! a message index, so a delayed result cannot overwrite a newer block.
use super::{
    desktop_math_color, desktop_mermaid_theme, render_desktop_math_colored,
    render_desktop_mermaid_themed, ChatView, DesktopMathRender, DesktopMermaidRender, PreparedSvg,
};
use gpui::{AppContext as _, Context, SharedString, Task};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, OnceLock,
};
use tokio::sync::Semaphore;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum SvgKind {
    Math { display: bool },
    Mermaid,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct SvgKey {
    kind: SvgKind,
    source: SharedString,
    palette_sig: SharedString,
}

enum SvgJobTheme {
    Math(String),
    Mermaid(Box<bruhs_models::diagram::Theme>),
}

impl SvgKey {
    fn render(&self, job_theme: &SvgJobTheme) -> Result<String, String> {
        match (self.kind, job_theme) {
            (SvgKind::Math { display }, SvgJobTheme::Math(color)) => {
                match render_desktop_math_colored(&self.source, display, color) {
                    DesktopMathRender::Svg(svg) => Ok(svg),
                    DesktopMathRender::Error(error) => Err(error),
                }
            }
            (SvgKind::Mermaid, SvgJobTheme::Mermaid(theme)) => {
                match render_desktop_mermaid_themed(&self.source, theme) {
                    DesktopMermaidRender::Svg(svg) => Ok(svg),
                    DesktopMermaidRender::Error(error) => Err(error),
                }
            }
            _ => Err("mismatched SVG render job".to_string()),
        }
    }
}

#[derive(Clone)]
pub(super) struct RenderGates {
    pub math: Arc<Semaphore>,
    pub diagrams: Arc<Semaphore>,
}

impl Default for RenderGates {
    fn default() -> Self {
        static GATES: OnceLock<RenderGates> = OnceLock::new();
        GATES
            .get_or_init(|| {
                // Both math and diagram renderers are pure Rust and thread-safe;
                // cap concurrency to leave CPU headroom for foreground UI work.
                let workers = std::thread::available_parallelism()
                    .map_or(1, |count| count.get().saturating_sub(2).max(1));
                Self {
                    math: Arc::new(Semaphore::new(workers)),
                    diagrams: Arc::new(Semaphore::new(workers)),
                }
            })
            .clone()
    }
}

struct SvgEntry {
    value: RefCell<PreparedSvg>,
    cancelled: Arc<AtomicBool>,
    _task: RefCell<Option<Task<()>>>,
}

impl Drop for SvgEntry {
    fn drop(&mut self) {
        // Dropping the foreground task also cancels a queued background future.
        // A renderer already inside a synchronous library call can finish, but
        // its cancelled result will never be adopted by the UI.
        self.cancelled.store(true, Ordering::Release);
    }
}

pub(super) struct SvgCache {
    // Keep every source used by the current prepared messages, like the old
    // per-message SVG blocks. Evicting live sources at a fixed count would
    // continually restart a large message's jobs. ContentBlockCache removes
    // unreferenced sources after replacement/rewind; global renderer LRUs keep
    // their existing limits for reuse beyond the current content.
    entries: RefCell<HashMap<SvgKey, Rc<SvgEntry>>>,
    gates: RenderGates,
    #[cfg(test)]
    pub requests: std::cell::Cell<usize>,
}

impl Default for SvgCache {
    fn default() -> Self {
        Self::with_gates(RenderGates::default())
    }
}

impl SvgCache {
    pub(super) fn with_gates(gates: RenderGates) -> Self {
        Self {
            entries: RefCell::new(HashMap::new()),
            gates,
            #[cfg(test)]
            requests: std::cell::Cell::new(0),
        }
    }

    pub(super) fn request(
        &self,
        kind: SvgKind,
        source: &SharedString,
        cx: &Context<'_, ChatView>,
    ) -> PreparedSvg {
        let (palette_sig, job_theme) = match kind {
            SvgKind::Math { .. } => {
                let color = desktop_math_color();
                (SharedString::from(color.clone()), SvgJobTheme::Math(color))
            }
            SvgKind::Mermaid => {
                let theme = desktop_mermaid_theme();
                let sig = SharedString::from(format!(
                    "{}:{}",
                    theme.background, theme.primary_text_color
                ));
                (sig, SvgJobTheme::Mermaid(Box::new(theme)))
            }
        };
        let key = SvgKey {
            kind,
            source: source.clone(),
            palette_sig,
        };
        if let Some(entry) = self.entries.borrow().get(&key) {
            return entry.value.borrow().clone();
        }

        let entry = Rc::new(SvgEntry {
            value: RefCell::new(PreparedSvg::Pending),
            cancelled: Arc::new(AtomicBool::new(false)),
            _task: RefCell::new(None),
        });
        self.entries
            .borrow_mut()
            .insert(key.clone(), Rc::clone(&entry));
        #[cfg(test)]
        self.requests.set(self.requests.get() + 1);

        let gate = match kind {
            SvgKind::Math { .. } => Arc::clone(&self.gates.math),
            SvgKind::Mermaid => Arc::clone(&self.gates.diagrams),
        };
        let cancelled = Arc::clone(&entry.cancelled);
        let work = cx.background_spawn(async move {
            let permit = gate.acquire().await.ok()?;
            if cancelled.load(Ordering::Acquire) {
                return None;
            }
            // Image construction hashes the SVG bytes. Keep that work on the
            // worker too; the foreground only adopts the finished image/error.
            let result = match key.render(&job_theme) {
                Ok(svg) => PreparedSvg::from_svg(svg),
                Err(error) => PreparedSvg::Error(error.into()),
            };
            drop(permit);
            (!cancelled.load(Ordering::Acquire)).then_some(result)
        });
        let weak_entry = Rc::downgrade(&entry);
        let task = cx.spawn(async move |view, cx| {
            let Some(result) = work.await else { return };
            let Some(entry) = weak_entry.upgrade() else {
                return;
            };
            if entry.cancelled.load(Ordering::Acquire) {
                return;
            }
            *entry.value.borrow_mut() = result;
            // Chat's viewport remeasures its visible range on notification.
            // Do not scan or invalidate an entire history for a shared formula.
            let _ = view.update(cx, |_, cx| cx.notify());
        });
        *entry._task.borrow_mut() = Some(task);
        PreparedSvg::Pending
    }

    pub(super) fn clear(&self) {
        self.entries.borrow_mut().clear();
    }

    pub(super) fn retain_sources(
        &self,
        mut referenced: impl FnMut(SvgKind, &SharedString) -> bool,
    ) {
        self.entries
            .borrow_mut()
            .retain(|key, _| referenced(key.kind, &key.source));
    }

    #[cfg(test)]
    pub(super) fn pending(&self) -> usize {
        self.entries
            .borrow()
            .values()
            .filter(|entry| matches!(*entry.value.borrow(), PreparedSvg::Pending))
            .count()
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.entries.borrow().len()
    }
}
