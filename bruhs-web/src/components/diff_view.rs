//! Inline line diff for Edit/Write tool bodies, rendered from
//! `bruhs_models::logic::diff::lcs_diff` with old/new line-number gutters.

use leptos::prelude::*;

use crate::highlight::{highlight_html, language_for_path};
use bruhs_models::logic::diff::{lcs_diff, stats, DiffLine, LineKind};

/// Row classes and sign glyph for a diff line.
pub fn line_style(kind: LineKind) -> (&'static str, &'static str) {
    match kind {
        LineKind::Add => (
            "bg-pandan-400/10 text-pandan-200 [&_.diff-code]:text-rambutan-200",
            "+",
        ),
        LineKind::Remove => (
            "bg-lychee-400/10 text-lychee-200 [&_.diff-code]:text-rambutan-300",
            "−",
        ),
        LineKind::Context => ("text-rambutan-400", " "),
    }
}

/// One rendered diff line: two gutters plus the text. `wrap` (default on)
/// soft-wraps long lines; off keeps them on one line for horizontal scroll.
/// `lang` (a highlight.js id) colours tokens; without it, or when hljs is
/// not loaded, the text renders plain.
#[component]
pub fn DiffRow(
    line: DiffLine,
    #[prop(optional)] wrap: Option<RwSignal<bool>>,
    #[prop(optional_no_strip)] lang: Option<&'static str>,
    /// Flips true after highlight.js loads so this row re-renders in color.
    #[prop(optional, into)]
    highlight_ready: Option<Signal<bool>>,
) -> impl IntoView {
    let (row_cls, sign) = line_style(line.kind);
    let old_no = line.old_no.map(|n| n.to_string()).unwrap_or_default();
    let new_no = line.new_no.map(|n| n.to_string()).unwrap_or_default();
    let text_cls = move || match wrap {
        Some(w) if !w.get() => "whitespace-pre pr-3",
        _ => "whitespace-pre-wrap break-all pr-3",
    };
    let armed = highlight_ready.is_none_or(|ready| ready.get());
    let highlighted = lang
        .filter(|_| armed)
        .and_then(|language| highlight_html(language, &line.text));
    let body = match highlighted {
        Some(html) => view! { <span class="diff-code" inner_html=html></span> }.into_any(),
        None => view! { <span>{line.text}</span> }.into_any(),
    };
    view! {
        <div class=format!("grid grid-cols-[2.75rem_2.75rem_1fr] {row_cls}")>
            <span class="select-none pr-2 text-right tabular-nums text-rambutan-500">{old_no}</span>
            <span class="select-none pr-2 text-right tabular-nums text-rambutan-500">{new_no}</span>
            <span class=text_cls>
                <span class="select-none opacity-60">{sign}" "</span>
                {body}
            </span>
        </div>
    }
}

/// Inline diff of `old` → `new`, with an optional file header showing the
/// path and `+N −N` counts.
#[component]
pub fn DiffView(
    #[prop(optional, into)] file_path: String,
    old: String,
    new: String,
) -> impl IntoView {
    let lines = lcs_diff(&old, &new);
    let st = stats(&lines);
    let show_header = !file_path.is_empty();
    let lang = language_for_path(&file_path);
    let highlight_ready = RwSignal::new(false);
    crate::highlight::arm_highlight(highlight_ready);
    let ready = Signal::derive(move || highlight_ready.get());

    view! {
        <div class="overflow-hidden rounded-grape bg-longan-950 text-code-sm inset-ring inset-ring-white/5">
            {show_header.then(|| view! {
                <div class="flex items-center justify-between gap-2 border-b border-longan-800/50 px-3 py-1">
                    <span class="min-w-0 truncate text-rambutan-400" title=file_path.clone()>{file_path.clone()}</span>
                    <span class="flex shrink-0 items-center gap-1.5 tabular-nums">
                        <span class="text-pandan-400">{format!("+{}", st.additions)}</span>
                        <span class="text-lychee-400">{format!("−{}", st.deletions)}</span>
                    </span>
                </div>
            })}
            <div class="max-h-64 overflow-auto py-1">
                {lines.into_iter().map(|line| view! { <DiffRow line=line lang=lang highlight_ready=ready /> }).collect_view()}
            </div>
        </div>
    }
}
