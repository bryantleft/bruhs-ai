//! Syntax highlighting for diff rows and code blocks using the shared
//! pure-Rust [`bruhs_models::syntax`] scanner. Zero JS bundles, synchronous on
//! both WASM and host builds.

use leptos::prelude::Set;

/// Syntax language id for `path`, by extension or well-known basename.
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

/// Highlighted HTML for `text` in `lang`, or `None` when `lang` is not
/// supported by the native highlighter.
///
/// The result goes into `inner_html`: [`bruhs_models::syntax::highlight_to_hljs_html`]
/// HTML-escapes every source character and only wraps spans in
/// `<span class="hljs-…">`, so file contents cannot inject markup.
pub fn highlight_html(lang: &str, text: &str) -> Option<String> {
    bruhs_models::syntax::highlight_to_hljs_html(lang, text)
}

/// Mark highlighting ready immediately — the pure-Rust scanner is compiled in
/// and requires no deferred JS bundle load.
pub fn arm_highlight(ready: leptos::prelude::RwSignal<bool>) {
    ready.set(true);
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
    fn highlights_code_and_escapes_html() {
        let html = highlight_html("rust", "fn main() { let x = \"<script>\"; }").unwrap();
        assert!(html.contains("<span class=\"hljs-keyword\">fn</span>"));
        assert!(html.contains("<span class=\"hljs-keyword\">let</span>"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(!html.contains("<script>"));
        assert_eq!(highlight_html("unknown-lang-xyz", "hello"), None);
    }
}
