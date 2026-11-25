use std::io::Read as _;
use std::path::{Path, PathBuf};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use bruhs_models::logic::attachments::{kind_for_name, mime_for, MAX_FILE_BYTES};
use cssparser::{Parser, ParserInput, ToCss as _, Token};

use crate::html::{Document, NodeId};

use super::ArtifactError;

fn invalid(message: impl Into<String>) -> ArtifactError {
    ArtifactError::Invalid(message.into())
}

struct Bundler {
    roots: Vec<PathBuf>,
    bytes_read: u64,
    css_stack: Vec<PathBuf>,
}

pub(super) fn bundle(path: &str, project: Option<&str>) -> Result<(String, String), ArtifactError> {
    let mut roots = Vec::new();
    let project = project
        .filter(|p| !p.is_empty())
        .map(std::fs::canonicalize)
        .transpose()?;
    if let Some(project) = &project {
        roots.push(project.clone());
    }
    for root in [
        crate::uploads::media_output_directory()?,
        std::env::temp_dir().join("bruhs-attachments"),
    ] {
        if let Ok(root) = root.canonicalize() {
            if !roots.contains(&root) {
                roots.push(root);
            }
        }
    }
    let base = project.as_deref().unwrap_or_else(|| Path::new("/"));
    let mut bundler = Bundler {
        roots,
        bytes_read: 0,
        css_stack: Vec::new(),
    };
    let source = bundler.resolve(path, base)?;
    let html = bundler.read_text(&source)?;
    let base = source
        .parent()
        .ok_or_else(|| invalid("The artifact has no containing directory."))?;
    let mut dom = Document::parse(&html);
    let mut title = source.file_stem().map_or_else(
        || "Interactive response".into(),
        |s| s.to_string_lossy().into_owned(),
    );
    let mut body: Option<NodeId> = None;
    let mut deferred_scripts: Vec<NodeId> = Vec::new();

    for node in dom.elements_in_order(Document::ROOT) {
        let Some(tag) = dom.tag_name(node).map(str::to_owned) else {
            continue;
        };
        if tag == "body" {
            body = Some(node);
        }
        if tag == "title" {
            let content = dom.text_content(node);
            if !content.trim().is_empty() {
                title = content.trim().into();
            }
        }
        if matches!(tag.as_str(), "base" | "iframe" | "object" | "embed") {
            return Err(invalid(format!(
                "Artifact <{tag}> elements are not supported in an isolated response. Inline their content before publishing."
            )));
        }
        if tag == "meta"
            && dom
                .attr(node, "http-equiv")
                .is_some_and(|value| value.eq_ignore_ascii_case("refresh"))
        {
            return Err(invalid(
                "Artifact meta refresh navigation is not supported.",
            ));
        }
        if dom.has_attr(node, "srcset") {
            return Err(invalid(
                "Choose one local image src instead of srcset before publishing a self-contained artifact.",
            ));
        }
        if tag == "form"
            && dom
                .attr(node, "action")
                .is_some_and(|value| !value.trim().is_empty())
        {
            return Err(invalid(
                "Artifact forms must handle input locally; remove the action URL.",
            ));
        }
        if let Some(css) = dom.attr(node, "style").map(str::to_owned) {
            let rewritten = bundler.css(&css, base)?;
            dom.set_attr(node, "style", rewritten);
        }
        if tag == "style" {
            let rewritten = bundler.css(&dom.text_content(node), base)?;
            dom.set_text(node, &safe_rawtext(&rewritten, "style"));
        }
        if tag == "script" {
            let kind = dom
                .attr(node, "type")
                .unwrap_or_default()
                .to_ascii_lowercase();
            if kind == "importmap" {
                return Err(invalid(
                    "JavaScript import maps must be prebundled into a standalone script before publishing.",
                ));
            }
            let script = if let Some(src) = dom.attr(node, "src").map(str::to_owned) {
                let path = bundler.resolve(&src, base)?;
                let script = bundler.read_text(&path)?;
                dom.remove_attr(node, "src");
                if kind != "module" && dom.has_attr(node, "defer") {
                    deferred_scripts.push(node);
                }
                Some(script)
            } else {
                None
            };
            let content = script.unwrap_or_else(|| dom.text_content(node));
            if matches!(
                kind.as_str(),
                "" | "module"
                    | "text/javascript"
                    | "application/javascript"
                    | "text/ecmascript"
                    | "application/ecmascript"
            ) {
                validate_script(&content)?;
            }
            dom.set_text(node, &safe_rawtext(&content, "script"));
        } else if tag == "link" {
            let rel = dom
                .attr(node, "rel")
                .unwrap_or_default()
                .to_ascii_lowercase();
            if let Some(href) = dom.attr(node, "href").map(str::to_owned) {
                if rel.split_whitespace().any(|part| part == "stylesheet") {
                    let css = bundler.stylesheet(&href, base)?;
                    dom.replace_with_style(node, &safe_rawtext(&css, "style"));
                } else if rel.split_whitespace().any(|part| part == "icon") {
                    let data_uri = bundler.asset(&href, base)?;
                    dom.set_attr(node, "href", data_uri);
                } else {
                    return Err(invalid(format!(
                        "Artifact link rel={rel:?} cannot be bundled. Remove preload/prefetch/import dependencies and publish their local contents instead."
                    )));
                }
            }
        } else {
            for key in ["src", "poster"] {
                if let Some(src) = dom.attr(node, key).map(str::to_owned) {
                    let data_uri = bundler.asset(&src, base)?;
                    dom.set_attr(node, key, data_uri);
                }
            }
            if matches!(tag.as_str(), "image" | "use") {
                if let Some(src) = dom.attr(node, "href").map(str::to_owned) {
                    let data_uri = bundler.asset(&src, base)?;
                    dom.set_attr(node, "href", data_uri);
                }
            }
        }
        dom.retain_attrs(node, |key| !matches!(key, "integrity" | "crossorigin"));
    }

    if !deferred_scripts.is_empty() {
        let target = body.unwrap_or(Document::ROOT);
        // Inline classic scripts ignore defer. Move originally deferred
        // files after the parsed body while preserving their source order.
        for script in deferred_scripts {
            dom.remove_attr(script, "defer");
            dom.move_to_end(script, target);
        }
    }

    let html = dom.serialize();
    if u64::try_from(html.len()).unwrap_or(u64::MAX) > MAX_FILE_BYTES {
        return Err(invalid(
            "The bundled artifact exceeds the app's 50 MiB attachment budget. Show large recordings as separate media blocks.",
        ));
    }
    Ok((html, title))
}

impl Bundler {
    fn resolve(&self, reference: &str, base: &Path) -> Result<PathBuf, ArtifactError> {
        let root = url::Url::from_directory_path(base)
            .map_err(|_| invalid("Artifact resource base must be an absolute directory."))?;
        let url = root.join(reference).map_err(|error| {
            invalid(format!("Invalid artifact resource {reference:?}: {error}"))
        })?;
        if url.scheme() != "file" {
            return Err(invalid(format!(
                "External dependency {reference:?} cannot be fetched while publishing. Download or generate it inside the project and reference the local file."
            )));
        }
        let path = url.to_file_path().map_err(|_| {
            invalid(format!(
                "Artifact resource {reference:?} is not a local file."
            ))
        })?;
        let path = path.canonicalize().map_err(|error| {
            invalid(format!(
                "Artifact resource {} is unavailable: {error}",
                path.display()
            ))
        })?;
        if !self.roots.iter().any(|root| path.starts_with(root)) {
            return Err(invalid(format!(
                "Artifact resource {} is outside the current project and Bruhs attachments.",
                path.display()
            )));
        }
        if !path.is_file() {
            return Err(invalid(format!(
                "Artifact resource {} is not a regular file.",
                path.display()
            )));
        }
        Ok(path)
    }

    fn read(&mut self, path: &Path) -> Result<Vec<u8>, ArtifactError> {
        let file = std::fs::File::open(path)?;
        let budget = MAX_FILE_BYTES.saturating_sub(self.bytes_read);
        if file.metadata()?.len() > budget {
            return Err(invalid(
                "Artifact inputs exceed the app's 50 MiB attachment budget. Show large media separately.",
            ));
        }
        let mut bytes = Vec::new();
        file.take(budget + 1).read_to_end(&mut bytes)?;
        self.bytes_read = self
            .bytes_read
            .saturating_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
        if self.bytes_read > MAX_FILE_BYTES {
            return Err(invalid(
                "Artifact inputs exceed the app's 50 MiB attachment budget.",
            ));
        }
        Ok(bytes)
    }

    fn read_text(&mut self, path: &Path) -> Result<String, ArtifactError> {
        String::from_utf8(self.read(path)?).map_err(|_| {
            invalid(format!(
                "Artifact source {} is not UTF-8 text.",
                path.display()
            ))
        })
    }

    fn asset(&mut self, reference: &str, base: &Path) -> Result<String, ArtifactError> {
        if reference.starts_with('#') || reference.starts_with("data:") {
            return Ok(reference.into());
        }
        let path = self.resolve(reference, base)?;
        let name = path.to_string_lossy();
        let mime = match path.extension().and_then(|e| e.to_str()) {
            Some("woff") => "font/woff",
            Some("woff2") => "font/woff2",
            Some("ttf") => "font/ttf",
            Some("otf") => "font/otf",
            Some("vtt") => "text/vtt",
            _ => mime_for(kind_for_name(&name), &name),
        };
        let bytes = self.read(&path)?;
        let fragment = reference
            .split_once('#')
            .map_or(String::new(), |(_, fragment)| format!("#{fragment}"));
        Ok(format!(
            "data:{mime};base64,{}{fragment}",
            STANDARD.encode(bytes)
        ))
    }

    fn stylesheet(&mut self, reference: &str, base: &Path) -> Result<String, ArtifactError> {
        if reference.starts_with("data:") {
            return Err(invalid(
                "Decode data stylesheet imports into local CSS before publishing.",
            ));
        }
        let path = self.resolve(reference, base)?;
        if self.css_stack.contains(&path) {
            return Err(invalid(format!(
                "Stylesheet import cycle includes {}. Prebundle the stylesheets first.",
                path.display()
            )));
        }
        self.css_stack.push(path.clone());
        let source = self.read_text(&path)?;
        let result = self.css(&source, path.parent().unwrap_or(base));
        self.css_stack.pop();
        result
    }

    fn css(&mut self, source: &str, base: &Path) -> Result<String, ArtifactError> {
        let mut input = ParserInput::new(source);
        self.css_tokens(&mut Parser::new(&mut input), base)
    }

    fn css_tokens(
        &mut self,
        parser: &mut Parser<'_, '_>,
        base: &Path,
    ) -> Result<String, ArtifactError> {
        let mut output = String::new();
        loop {
            let start = parser.position();
            let token = match parser.next_including_whitespace_and_comments() {
                Ok(token) => token.clone(),
                Err(_) => break,
            };
            match token {
                Token::AtKeyword(name) if name.eq_ignore_ascii_case("import") => {
                    let token = parser
                        .next()
                        .map_err(|_| invalid("Stylesheet @import has no URL."))?
                        .clone();
                    let reference = match token {
                        Token::QuotedString(url) | Token::UnquotedUrl(url) => url.to_string(),
                        Token::Function(name) if name.eq_ignore_ascii_case("url") => parser
                            .parse_nested_block(|input| {
                                Ok::<_, cssparser::ParseError<'_, ()>>(
                                    input.expect_string()?.to_string(),
                                )
                            })
                            .map_err(|_| invalid("Stylesheet @import has an invalid URL."))?,
                        _ => return Err(invalid("Stylesheet @import requires a local file URL.")),
                    };
                    let tail_start = parser.position();
                    let mut tail = String::new();
                    while let Ok(token) = parser.next_including_whitespace_and_comments() {
                        if matches!(token, Token::Semicolon) {
                            tail = parser
                                .slice_from(tail_start)
                                .trim_end_matches(';')
                                .trim()
                                .into();
                            break;
                        }
                        tail = parser.slice_from(tail_start).trim().into();
                    }
                    if tail.starts_with("layer") || tail.starts_with("supports") {
                        return Err(invalid(
                            "Stylesheet layer/supports imports must be prebundled before publishing.",
                        ));
                    }
                    let imported = self.stylesheet(&reference, base)?;
                    if tail.is_empty() {
                        output.push_str(&imported);
                    } else {
                        output.push_str(&format!("@media {tail}{{{imported}}}"));
                    }
                }
                Token::UnquotedUrl(reference) => {
                    output.push_str(&css_url(&self.asset(&reference, base)?))
                }
                Token::Function(name) if name.eq_ignore_ascii_case("url") => {
                    let reference = parser
                        .parse_nested_block(|input| {
                            Ok::<_, cssparser::ParseError<'_, ()>>(
                                input.expect_string()?.to_string(),
                            )
                        })
                        .map_err(|_| invalid("Stylesheet url() is invalid."))?;
                    output.push_str(&css_url(&self.asset(&reference, base)?));
                }
                Token::Function(_)
                | Token::CurlyBracketBlock
                | Token::SquareBracketBlock
                | Token::ParenthesisBlock => {
                    let open = parser.slice_from(start).to_string();
                    let close = match token {
                        Token::CurlyBracketBlock => '}',
                        Token::SquareBracketBlock => ']',
                        _ => ')',
                    };
                    let inner = parser
                        .parse_nested_block(|input| {
                            self.css_tokens(input, base).map_err(|error| {
                                input.new_custom_error::<_, String>(error.to_string())
                            })
                        })
                        .map_err(|error| {
                            invalid(format!("Stylesheet could not be bundled: {error:?}"))
                        })?;
                    output.push_str(&open);
                    output.push_str(&inner);
                    output.push(close);
                }
                Token::BadString(_) | Token::BadUrl(_) => {
                    return Err(invalid("Stylesheet contains an invalid string or URL."))
                }
                _ => output.push_str(parser.slice_from(start)),
            }
        }
        Ok(output)
    }
}

fn css_url(reference: &str) -> String {
    format!(
        "url({})",
        Token::QuotedString(reference.into()).to_css_string()
    )
}

fn safe_rawtext(source: &str, tag: &str) -> String {
    let needle = format!("</{tag}");
    let lower = source.to_ascii_lowercase();
    let mut output = String::with_capacity(source.len());
    let mut cursor = 0;
    for (index, _) in lower.match_indices(&needle) {
        output.push_str(&source[cursor..index + 1]);
        output.push('\\');
        cursor = index + 1;
    }
    output.push_str(&source[cursor..]);
    output
}

/// Lean, zero-dependency JavaScript scanner for artifact sandboxing.
///
/// Properly skips comments (`//`, `/* */`), string literals (`'...'`, `"..."`),
/// template literals (while scanning `${...}` interpolations), and regex
/// literals (`/.../flags` with `[...]` character classes), while rejecting
/// `import`/`export` statements and dynamic network/worker/loader APIs.
fn validate_script(source: &str) -> Result<(), ArtifactError> {
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    // Whether `/` at the current position begins a regex literal rather than division.
    let mut regex_allowed = true;
    // Stack of open brace depths for active `${...}` template literal interpolations.
    let mut template_brace_stack: Vec<usize> = Vec::new();
    let mut brace_depth = 0usize;

    while i < len {
        let b = bytes[i];

        // Whitespace
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }

        // Line comment `//`
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
            i += 2;
            while i < len && !matches!(bytes[i], b'\n' | b'\r') {
                i += 1;
            }
            continue;
        }

        // Block comment `/* ... */`
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
            i += 2;
            let mut closed = false;
            while i + 1 < len {
                if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    i += 2;
                    closed = true;
                    break;
                }
                i += 1;
            }
            if !closed {
                return Err(invalid(
                    "JavaScript could not be scanned: unterminated block comment. Fix its syntax or prebundle it before publishing.",
                ));
            }
            continue;
        }

        // Regex literal `/pattern/flags`
        if b == b'/' && regex_allowed {
            i += 1;
            let mut in_class = false;
            let mut closed = false;
            while i < len {
                let c = bytes[i];
                if c == b'\\' {
                    i = (i + 2).min(len);
                    continue;
                }
                if c == b'\n' || c == b'\r' {
                    break;
                }
                if c == b'[' {
                    in_class = true;
                } else if c == b']' {
                    in_class = false;
                } else if c == b'/' && !in_class {
                    i += 1;
                    while i < len && bytes[i].is_ascii_alphabetic() {
                        i += 1;
                    }
                    closed = true;
                    break;
                }
                i += 1;
            }
            if !closed {
                return Err(invalid(
                    "JavaScript could not be scanned: unterminated regular expression. Fix its syntax or prebundle it before publishing.",
                ));
            }
            regex_allowed = false;
            continue;
        }

        // Single or double quoted string
        if b == b'"' || b == b'\'' {
            let quote = b;
            i += 1;
            let mut closed = false;
            while i < len {
                let c = bytes[i];
                if c == b'\\' {
                    i = (i + 2).min(len);
                    continue;
                }
                if c == quote {
                    i += 1;
                    closed = true;
                    break;
                }
                if c == b'\n' || c == b'\r' {
                    break;
                }
                i += 1;
            }
            if !closed {
                return Err(invalid(
                    "JavaScript could not be scanned: unterminated string literal. Fix its syntax or prebundle it before publishing.",
                ));
            }
            regex_allowed = false;
            continue;
        }

        // Template literal `` `...` `` (re-entered after closing `}` of `${...}`)
        if b == b'`' {
            i = scan_template_chunk(bytes, i + 1, &mut template_brace_stack, brace_depth)?;
            if template_brace_stack.last() == Some(&brace_depth) {
                brace_depth += 1;
                regex_allowed = true;
            } else {
                regex_allowed = false;
            }
            continue;
        }

        // Braces (track `${...}` template interpolation boundaries)
        if b == b'{' {
            brace_depth += 1;
            regex_allowed = true;
            i += 1;
            continue;
        }
        if b == b'}' {
            brace_depth = brace_depth.saturating_sub(1);
            if template_brace_stack.last() == Some(&brace_depth) {
                template_brace_stack.pop();
                i = scan_template_chunk(bytes, i + 1, &mut template_brace_stack, brace_depth)?;
                if template_brace_stack.last() == Some(&brace_depth) {
                    brace_depth += 1;
                    regex_allowed = true;
                } else {
                    regex_allowed = false;
                }
                continue;
            }
            regex_allowed = true;
            i += 1;
            continue;
        }

        // Identifiers and keywords
        if b.is_ascii_alphabetic() || matches!(b, b'_' | b'$') || b >= 0x80 {
            let start = i;
            while i < len {
                let c = bytes[i];
                if c.is_ascii_alphanumeric() || matches!(c, b'_' | b'$') {
                    i += 1;
                } else if c >= 0x80 {
                    let ch = source[i..].chars().next().unwrap_or('\0');
                    if ch.is_alphanumeric() {
                        i += ch.len_utf8();
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            }
            let ident = &source[start..i];
            if matches!(ident, "import" | "export") {
                return Err(invalid(
                    "JavaScript imports must be prebundled into a standalone script before publishing. Local CSS, classic script src files, images and media are bundled automatically.",
                ));
            }
            if matches!(
                ident,
                "fetch"
                    | "XMLHttpRequest"
                    | "WebSocket"
                    | "EventSource"
                    | "Worker"
                    | "SharedWorker"
                    | "importScripts"
                    | "require"
            ) {
                return Err(invalid(
                    "JavaScript requests or module loaders must be replaced with inline data and code before publishing an isolated artifact.",
                ));
            }
            regex_allowed = matches!(
                ident,
                "return"
                    | "throw"
                    | "case"
                    | "in"
                    | "instanceof"
                    | "typeof"
                    | "void"
                    | "delete"
                    | "new"
                    | "do"
                    | "else"
                    | "yield"
                    | "await"
                    | "of"
            );
            continue;
        }

        // Numbers
        if b.is_ascii_digit() {
            while i < len && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'.' | b'_')) {
                i += 1;
            }
            regex_allowed = false;
            continue;
        }

        // Punctuation
        regex_allowed = !matches!(b, b')' | b']');
        i += 1;
    }

    Ok(())
}

fn scan_template_chunk(
    bytes: &[u8],
    mut i: usize,
    template_brace_stack: &mut Vec<usize>,
    brace_depth: usize,
) -> Result<usize, ArtifactError> {
    let len = bytes.len();
    while i < len {
        let c = bytes[i];
        if c == b'\\' {
            i = (i + 2).min(len);
            continue;
        }
        if c == b'`' {
            return Ok(i + 1);
        }
        if c == b'$' && i + 1 < len && bytes[i + 1] == b'{' {
            template_brace_stack.push(brace_depth);
            return Ok(i + 2);
        }
        i += 1;
    }
    Err(invalid(
        "JavaScript could not be scanned: unterminated template literal. Fix its syntax or prebundle it before publishing.",
    ))
}
