//! Syntax highlighting for diff rows via highlight.js, loaded on demand by
//! `window.ensureHighlight()` in `js/bruhs-glue.js`. Pure language detection
//! here; the browser call is a thin `Reflect` wrapper that degrades to plain
//! text when hljs is missing.

#[cfg(target_arch = "wasm32")]
use leptos::prelude::Set;

/// highlight.js language id for `path`, by extension or well-known basename.
pub fn language_for_path(path: &str) -> Option<&'static str> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let lower = name.to_ascii_lowercase();
    match lower.as_str() {
        "dockerfile" | "containerfile" => return Some("dockerfile"),
        "makefile" | "gnumakefile" => return Some("makefile"),
        "cmakelists.txt" => return Some("cmake"),
        ".zshrc" | ".bashrc" | ".bash_profile" | ".profile" => return Some("bash"),
        _ => {}
    }
    let ext = lower.rsplit_once('.').map(|(_, e)| e)?;
    Some(match ext {
        "rs" => "rust",
        "ts" | "tsx" | "mts" | "cts" => "typescript",
        "js" | "jsx" | "mjs" | "cjs" => "javascript",
        "py" | "pyi" => "python",
        "go" => "go",
        "rb" => "ruby",
        "java" => "java",
        "kt" | "kts" => "kotlin",
        "swift" => "swift",
        "c" | "h" => "c",
        "cc" | "cpp" | "cxx" | "hpp" | "hh" => "cpp",
        "cs" => "csharp",
        "php" => "php",
        "sh" | "bash" | "zsh" => "bash",
        "fish" => "bash",
        "json" | "jsonc" => "json",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "md" | "markdown" | "mdx" => "markdown",
        "html" | "htm" => "xml",
        "xml" | "svg" | "plist" => "xml",
        "css" => "css",
        "scss" => "scss",
        "less" => "less",
        "sql" => "sql",
        "graphql" | "gql" => "graphql",
        "lua" => "lua",
        "dart" => "dart",
        "scala" => "scala",
        "hs" => "haskell",
        "ex" | "exs" => "elixir",
        "erl" => "erlang",
        "clj" | "cljs" => "clojure",
        "vue" => "xml",
        "ini" | "cfg" | "conf" => "ini",
        "diff" | "patch" => "diff",
        "proto" => "protobuf",
        "tf" => "hcl",
        "nix" => "nix",
        "zig" => "zig",
        "r" => "r",
        "pl" | "pm" => "perl",
        "ps1" => "powershell",
        "bat" | "cmd" => "dos",
        "txt" | "log" | "lock" => return None,
        _ => return None,
    })
}

/// Highlighted HTML for one line of `lang`, or `None` when highlight.js is
/// not available, does not know `lang`, or throws.
///
/// The result goes into `inner_html`: highlight.js HTML-escapes the source
/// text and only adds its own `<span class="hljs-…">` wrappers, so file
/// contents cannot inject markup.
#[cfg(target_arch = "wasm32")]
pub fn highlight_html(lang: &str, text: &str) -> Option<String> {
    use wasm_bindgen::JsCast;

    let window = web_sys::window()?;
    let hljs = js_sys::Reflect::get(&window, &"hljs".into()).ok()?;
    if hljs.is_undefined() || hljs.is_null() {
        return None;
    }
    let highlight = js_sys::Reflect::get(&hljs, &"highlight".into())
        .ok()?
        .dyn_into::<js_sys::Function>()
        .ok()?;
    let opts = js_sys::Object::new();
    js_sys::Reflect::set(&opts, &"language".into(), &lang.into()).ok()?;
    js_sys::Reflect::set(&opts, &"ignoreIllegals".into(), &true.into()).ok()?;
    let result = highlight.call2(&hljs, &text.into(), &opts).ok()?;
    js_sys::Reflect::get(&result, &"value".into())
        .ok()?
        .as_string()
}

#[cfg(not(target_arch = "wasm32"))]
pub fn highlight_html(_lang: &str, _text: &str) -> Option<String> {
    None
}

/// Load highlight.js once, then set `ready` so diff rows re-render in color.
/// Host builds leave the signal alone and rows stay plain text.
pub fn arm_highlight(ready: leptos::prelude::RwSignal<bool>) {
    #[cfg(target_arch = "wasm32")]
    arm_highlight_wasm(ready);
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = ready;
    }
}

#[cfg(target_arch = "wasm32")]
fn hljs_loaded() -> bool {
    web_sys::window()
        .and_then(|window| js_sys::Reflect::get(&window, &"hljs".into()).ok())
        .is_some_and(|hljs| !hljs.is_undefined() && !hljs.is_null())
}

#[cfg(target_arch = "wasm32")]
fn request_highlight_bundle() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use wasm_bindgen::JsCast;

    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::Relaxed) {
        return;
    }
    wasm_bindgen_futures::spawn_local(async {
        let Some(window) = web_sys::window() else {
            return;
        };
        let Ok(loader) = js_sys::Reflect::get(&window, &"ensureHighlight".into()) else {
            return;
        };
        let Ok(loader) = loader.dyn_into::<js_sys::Function>() else {
            return;
        };
        if let Ok(promise) = loader.call0(&wasm_bindgen::JsValue::NULL) {
            if let Ok(promise) = promise.dyn_into::<js_sys::Promise>() {
                let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
            }
        }
    });
}

#[cfg(target_arch = "wasm32")]
fn poll_highlight(ready: leptos::prelude::RwSignal<bool>, left: u8) {
    if hljs_loaded() {
        ready.set(true);
        return;
    }
    if left == 0 {
        return;
    }
    gloo_timers::callback::Timeout::new(100, move || poll_highlight(ready, left - 1)).forget();
}

#[cfg(target_arch = "wasm32")]
fn arm_highlight_wasm(ready: leptos::prelude::RwSignal<bool>) {
    if hljs_loaded() {
        ready.set(true);
        return;
    }
    request_highlight_bundle();
    poll_highlight(ready, 100);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_common_extensions() {
        assert_eq!(language_for_path("src/main.rs"), Some("rust"));
        assert_eq!(language_for_path("app/App.tsx"), Some("typescript"));
        assert_eq!(language_for_path("cdp.mjs"), Some("javascript"));
        assert_eq!(language_for_path("tool.py"), Some("python"));
        assert_eq!(language_for_path("Dockerfile"), Some("dockerfile"));
        assert_eq!(language_for_path("build/Makefile"), Some("makefile"));
        assert_eq!(language_for_path("ci.yml"), Some("yaml"));
        assert_eq!(language_for_path("Cargo.toml"), Some("toml"));
        assert_eq!(language_for_path("index.html"), Some("xml"));
    }

    #[test]
    fn unknown_extension_is_none() {
        assert_eq!(language_for_path("notes.txt"), None);
        assert_eq!(language_for_path("Cargo.lock"), None);
        assert_eq!(language_for_path("LICENSE"), None);
        assert_eq!(language_for_path("weird.zzz"), None);
    }

    #[test]
    fn case_insensitive() {
        assert_eq!(language_for_path("README.MD"), Some("markdown"));
        assert_eq!(language_for_path("DOCKERFILE"), Some("dockerfile"));
        assert_eq!(language_for_path("Main.RS"), Some("rust"));
    }

    #[test]
    fn host_build_never_highlights() {
        assert_eq!(highlight_html("rust", "fn main() {}"), None);
    }
}
