//! Post-render pass for assistant markdown: highlight.js for fenced code,
//! KaTeX auto-render for math, Mermaid for diagrams. Runs when a stream
//! settles so partial blocks never flash half-highlighted, and only over
//! prose blocks it has not processed yet (each is marked once done), so a
//! settled turn does not re-highlight the whole thread.

use leptos::prelude::*;
use wasm_bindgen::{JsCast, JsValue};

use crate::state::{AgentMessage, MessagePart};

/// Marks a `.prose-msg` block the pass has already processed. A block whose
/// markdown changes is a new element (its row re-renders), so it is
/// processed again.
const DONE_ATTR: &str = "data-render-pass";

/// Install the effect once per chat page. KaTeX, Mermaid, and highlight.js
/// are lazy-loaded through `window.ensureMath()` / `window.ensureMermaid()` /
/// `window.ensureHighlight()` (see `js/bruhs-glue.js`) the first time a
/// message actually needs them.
pub fn install_render_pass(
    messages: ReadSignal<Vec<AgentMessage>>,
    is_streaming: ReadSignal<bool>,
) {
    Effect::new(move |_| {
        if is_streaming.get() {
            return;
        }
        let (needs_math, needs_mermaid) = messages.with(|msgs| {
            let texts = || {
                msgs.iter().flat_map(|m| {
                    m.parts.iter().filter_map(|p| match p {
                        MessagePart::Text(t) => Some(t.as_str()),
                        _ => None,
                    })
                })
            };
            (
                texts().any(has_math),
                texts().any(|t| t.contains("```mermaid")),
            )
        });

        gloo_timers::callback::Timeout::new(0, move || {
            wasm_bindgen_futures::spawn_local(async move {
                run_pass(needs_math, needs_mermaid).await;
            });
        })
        .forget();
    });
}

/// Whether `markdown` has math KaTeX would render: `$$…$$`, `\[…\]`,
/// `\(…\)`, or inline `$…$` whose content starts and ends with a non-space
/// and whose closing `$` is not followed by a digit (so `$5 and $10` is
/// prices, not math). Code spans and fenced blocks do not count.
pub fn has_math(markdown: &str) -> bool {
    let prose = without_code(markdown);
    if prose.contains("\\[") && prose.contains("\\]") {
        return true;
    }
    if prose.contains("\\(") && prose.contains("\\)") {
        return true;
    }
    let chars: Vec<char> = prose.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '$' || (i > 0 && chars[i - 1] == '\\') {
            i += 1;
            continue;
        }
        let display = chars.get(i + 1) == Some(&'$');
        let open = if display { i + 2 } else { i + 1 };
        let Some(close) = (open..chars.len()).find(|&j| chars[j] == '$' && chars[j - 1] != '\\')
        else {
            return false;
        };
        let body = &chars[open..close];
        if display {
            if chars.get(close + 1) == Some(&'$') && body.iter().any(|c| !c.is_whitespace()) {
                return true;
            }
        } else if !body.is_empty()
            && !body[0].is_whitespace()
            && !body[body.len() - 1].is_whitespace()
            && !chars.get(close + 1).is_some_and(char::is_ascii_digit)
        {
            return true;
        }
        // Not math here; the closing `$` may open the next candidate.
        i = if display { close + 1 } else { close };
    }
    false
}

/// `markdown` with fenced code blocks and inline code spans removed.
fn without_code(markdown: &str) -> String {
    let mut out = String::with_capacity(markdown.len());
    let mut in_fence = false;
    for line in markdown.lines() {
        if line.trim_start().starts_with("```") || line.trim_start().starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        let mut in_span = false;
        for c in line.chars() {
            if c == '`' {
                in_span = !in_span;
            } else if !in_span {
                out.push(c);
            }
        }
        out.push('\n');
    }
    out
}

/// Call `window[name]()` and await the promise it returns, if any.
async fn call_loader(window: &web_sys::Window, name: &str) {
    let Ok(loader) = js_sys::Reflect::get(window, &JsValue::from_str(name)) else {
        return;
    };
    let Ok(loader) = loader.dyn_into::<js_sys::Function>() else {
        return;
    };
    match loader.call0(&JsValue::NULL) {
        Ok(promise) => {
            if let Ok(promise) = promise.dyn_into::<js_sys::Promise>() {
                if let Err(error) = wasm_bindgen_futures::JsFuture::from(promise).await {
                    log::warn!("render pass: {name} failed to load: {error:?}");
                }
            }
        }
        Err(error) => log::warn!("render pass: {name} threw: {error:?}"),
    }
}

/// `window[name]` as a function, if it is one.
fn global_fn(target: &JsValue, name: &str) -> Option<js_sys::Function> {
    js_sys::Reflect::get(target, &JsValue::from_str(name))
        .ok()?
        .dyn_into::<js_sys::Function>()
        .ok()
}

fn katex_options() -> js_sys::Object {
    let set = |target: &js_sys::Object, key: &str, value: &JsValue| {
        // Setting a property on a fresh plain object cannot fail.
        let _ = js_sys::Reflect::set(target, &JsValue::from_str(key), value);
    };
    let opts = js_sys::Object::new();
    set(&opts, "throwOnError", &JsValue::FALSE);
    let delims = js_sys::Array::new();
    for (left, right, display) in [
        ("$$", "$$", true),
        ("\\[", "\\]", true),
        ("$", "$", false),
        ("\\(", "\\)", false),
    ] {
        let d = js_sys::Object::new();
        set(&d, "left", &JsValue::from_str(left));
        set(&d, "right", &JsValue::from_str(right));
        set(&d, "display", &JsValue::from_bool(display));
        delims.push(&d);
    }
    set(&opts, "delimiters", &delims);
    opts
}

async fn run_pass(needs_math: bool, needs_mermaid: bool) {
    let Some(window) = web_sys::window() else {
        return;
    };
    if needs_math {
        call_loader(&window, "ensureMath").await;
    }
    if needs_mermaid {
        call_loader(&window, "ensureMermaid").await;
    }
    let Some(document) = window.document() else {
        return;
    };
    let Ok(blocks) = document.query_selector_all(&format!(".prose-msg:not([{DONE_ATTR}])")) else {
        return;
    };
    if blocks.length() == 0 {
        return;
    }

    let mut needs_highlight = false;
    for i in 0..blocks.length() {
        let Some(block) = blocks
            .item(i)
            .and_then(|node| node.dyn_into::<web_sys::Element>().ok())
        else {
            continue;
        };
        if block.query_selector("pre code").ok().flatten().is_some() {
            needs_highlight = true;
            break;
        }
    }
    if needs_highlight {
        call_loader(&window, "ensureHighlight").await;
    }

    let hljs = js_sys::Reflect::get(&window, &JsValue::from_str("hljs"))
        .ok()
        .filter(|h| !h.is_undefined() && !h.is_null());
    let highlight = hljs
        .as_ref()
        .and_then(|h| global_fn(h, "highlightElement").map(|f| (h.clone(), f)));
    let render_math = global_fn(&window, "renderMathInElement");
    let mermaid = js_sys::Reflect::get(&window, &JsValue::from_str("mermaid"))
        .ok()
        .filter(|m| !m.is_undefined() && !m.is_null());
    let mut diagrams = false;

    for i in 0..blocks.length() {
        let Some(block) = blocks
            .item(i)
            .and_then(|n| n.dyn_into::<web_sys::Element>().ok())
        else {
            continue;
        };
        // 1. Mermaid first, so its sources are not highlighted as code.
        if mermaid.is_some() {
            diagrams |= convert_mermaid(&document, &block);
        }
        // 2. highlight.js on this block's fenced code.
        if let Some((hljs, highlight)) = &highlight {
            if let Ok(codes) = block.query_selector_all("pre code") {
                for j in 0..codes.length() {
                    if let Some(code) = codes.item(j) {
                        if let Err(error) = highlight.call1(hljs, &code) {
                            log::warn!("render pass: highlight failed: {error:?}");
                        }
                    }
                }
            }
        }
        // 3. KaTeX auto-render over this block's text.
        if let Some(render_math) = &render_math {
            if let Err(error) = render_math.call2(&JsValue::NULL, &block, &katex_options()) {
                log::warn!("render pass: math rendering failed: {error:?}");
            }
        }
        if let Err(error) = block.set_attribute(DONE_ATTR, "") {
            log::warn!("render pass: could not mark a block done: {error:?}");
        }
    }

    if let (true, Some(mermaid)) = (diagrams, mermaid) {
        if let Some(run) = global_fn(&mermaid, "run") {
            if let Err(error) = run.call0(&mermaid) {
                log::warn!("render pass: mermaid failed: {error:?}");
            }
        }
    }
}

/// Replace each `pre > code.language-mermaid` in `block` with a
/// `<div class="mermaid">` holding its source. `true` if any was replaced.
fn convert_mermaid(document: &web_sys::Document, block: &web_sys::Element) -> bool {
    let Ok(codes) = block.query_selector_all("pre > code.language-mermaid") else {
        return false;
    };
    let mut converted = false;
    for i in 0..codes.length() {
        let Some(code) = codes.item(i) else {
            continue;
        };
        let (Some(pre), Ok(div)) = (code.parent_element(), document.create_element("div")) else {
            continue;
        };
        let Some(parent) = pre.parent_element() else {
            continue;
        };
        if div.set_attribute("class", "mermaid").is_err() {
            continue;
        }
        div.set_text_content(code.text_content().as_deref());
        if parent.replace_child(&div, &pre).is_ok() {
            converted = true;
        }
    }
    converted
}

#[cfg(test)]
mod tests {
    use super::has_math;

    #[test]
    fn math_needs_real_delimiters() {
        for math in [
            "Euler: $e^{i\\pi} + 1 = 0$.",
            "$$\\sum_{i=1}^n i$$",
            "Display:\n$$\nx^2\n$$",
            "\\[ a = b \\]",
            "inline \\(x\\) here",
            "$x$",
            "costs $5 but $x_1$ is math",
        ] {
            assert!(has_math(math), "{math:?}");
        }
        for prose in [
            "It costs $5.",
            "From $5 to $10.",
            "Pay $5 and $ 10",
            "$ x $",
            "a lone $ sign",
            "escaped \\$5 and \\$6",
            "`$x$` in code",
            "```\nlet s = \"$a$\";\n```",
            "$$ $$",
            "",
        ] {
            assert!(!has_math(prose), "{prose:?}");
        }
    }
}
