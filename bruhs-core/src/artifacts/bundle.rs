use std::io::Read as _;
use std::path::{Path, PathBuf};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use bruhs_models::logic::attachments::{kind_for_name, mime_for, MAX_FILE_BYTES};
use cssparser::{Parser, ParserInput, ToCss as _, Token};
use html5ever::{parse_document, serialize, tendril::TendrilSink as _};
use markup5ever_rcdom::{Handle, Node, NodeData, RcDom, SerializableHandle};

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
    let dom = parse_document(RcDom::default(), Default::default()).one(html);
    let mut title = source.file_stem().map_or_else(
        || "Interactive response".into(),
        |s| s.to_string_lossy().into_owned(),
    );
    let mut nodes = vec![dom.document.clone()];
    let mut body = None;
    let mut deferred_scripts = Vec::new();
    while let Some(node) = nodes.pop() {
        if let NodeData::Element {
            name,
            attrs,
            template_contents,
            ..
        } = &node.data
        {
            let tag = name.local.as_ref();
            if tag == "body" {
                body = Some(node.clone());
            }
            if tag == "title" {
                let content = text(&node);
                if !content.trim().is_empty() {
                    title = content.trim().into();
                }
            }
            if matches!(tag, "base" | "iframe" | "object" | "embed") {
                return Err(invalid(format!("Artifact <{tag}> elements are not supported in an isolated response. Inline their content before publishing.")));
            }
            if tag == "meta"
                && attribute(&node, "http-equiv")
                    .is_some_and(|value| value.eq_ignore_ascii_case("refresh"))
            {
                return Err(invalid(
                    "Artifact meta refresh navigation is not supported.",
                ));
            }
            if attribute(&node, "srcset").is_some() {
                return Err(invalid("Choose one local image src instead of srcset before publishing a self-contained artifact."));
            }
            if tag == "form"
                && attribute(&node, "action").is_some_and(|value| !value.trim().is_empty())
            {
                return Err(invalid(
                    "Artifact forms must handle input locally; remove the action URL.",
                ));
            }
            if let Some(css) = attribute(&node, "style") {
                set_attribute(&node, "style", bundler.css(&css, base)?);
            }
            if tag == "style" {
                set_text(
                    &node,
                    &safe_rawtext(&bundler.css(&text(&node), base)?, "style"),
                );
            }
            if tag == "script" {
                let kind = attribute(&node, "type")
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                if kind == "importmap" {
                    return Err(invalid("JavaScript import maps must be prebundled into a standalone script before publishing."));
                }
                let script = if let Some(src) = attribute(&node, "src") {
                    let path = bundler.resolve(&src, base)?;
                    let script = bundler.read_text(&path)?;
                    remove_attribute(&node, "src");
                    if kind != "module" && attribute(&node, "defer").is_some() {
                        deferred_scripts.push(node.clone());
                    }
                    Some(script)
                } else {
                    None
                };
                let content = script.unwrap_or_else(|| text(&node));
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
                set_text(&node, &safe_rawtext(&content, "script"));
            } else if tag == "link" {
                let rel = attribute(&node, "rel")
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                if let Some(href) = attribute(&node, "href") {
                    if rel.split_whitespace().any(|part| part == "stylesheet") {
                        let css = bundler.stylesheet(&href, base)?;
                        replace_stylesheet(&node, &safe_rawtext(&css, "style"));
                    } else if rel.split_whitespace().any(|part| part == "icon") {
                        set_attribute(&node, "href", bundler.asset(&href, base)?);
                    } else {
                        return Err(invalid(format!("Artifact link rel={rel:?} cannot be bundled. Remove preload/prefetch/import dependencies and publish their local contents instead.")));
                    }
                }
            } else {
                for key in ["src", "poster"] {
                    if let Some(src) = attribute(&node, key) {
                        set_attribute(&node, key, bundler.asset(&src, base)?);
                    }
                }
                if matches!(tag, "image" | "use") {
                    if let Some(src) = attribute(&node, "href") {
                        set_attribute(&node, "href", bundler.asset(&src, base)?);
                    }
                }
            }
            attrs.borrow_mut().retain(|attribute| {
                !matches!(attribute.name.local.as_ref(), "integrity" | "crossorigin")
            });
            if let Some(contents) = &*template_contents.borrow() {
                nodes.push(contents.clone());
            }
        }
        nodes.extend(node.children.borrow().iter().rev().cloned());
    }
    if let Some(body) = body {
        // Inline classic scripts ignore defer. Move originally deferred
        // files after the parsed body while preserving their source order.
        for script in deferred_scripts {
            if let Some(parent) = script.parent.take().and_then(|parent| parent.upgrade()) {
                parent
                    .children
                    .borrow_mut()
                    .retain(|child| !std::rc::Rc::ptr_eq(child, &script));
            }
            script.parent.set(Some(std::rc::Rc::downgrade(&body)));
            remove_attribute(&script, "defer");
            body.children.borrow_mut().push(script);
        }
    }
    let mut bytes = Vec::new();
    serialize(
        &mut bytes,
        &SerializableHandle::from(dom.document),
        Default::default(),
    )?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_FILE_BYTES {
        return Err(invalid("The bundled artifact exceeds the app's 50 MiB attachment budget. Show large recordings as separate media blocks."));
    }
    let html = String::from_utf8(bytes)
        .map_err(|_| invalid("The bundled artifact is not valid UTF-8."))?;
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
            return Err(invalid(format!("External dependency {reference:?} cannot be fetched while publishing. Download or generate it inside the project and reference the local file.")));
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
            return Err(invalid("Artifact inputs exceed the app's 50 MiB attachment budget. Show large media separately."));
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
                        return Err(invalid("Stylesheet layer/supports imports must be prebundled before publishing."));
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

fn attribute(node: &Handle, key: &str) -> Option<String> {
    if let NodeData::Element { attrs, .. } = &node.data {
        attrs
            .borrow()
            .iter()
            .find(|attribute| attribute.name.local.as_ref() == key)
            .map(|attribute| attribute.value.to_string())
    } else {
        None
    }
}

fn css_url(reference: &str) -> String {
    format!(
        "url({})",
        Token::QuotedString(reference.into()).to_css_string()
    )
}

fn set_attribute(node: &Handle, key: &str, value: String) {
    if let NodeData::Element { attrs, .. } = &node.data {
        if let Some(attribute) = attrs
            .borrow_mut()
            .iter_mut()
            .find(|attribute| attribute.name.local.as_ref() == key)
        {
            attribute.value = value.into();
        }
    }
}

fn remove_attribute(node: &Handle, key: &str) {
    if let NodeData::Element { attrs, .. } = &node.data {
        attrs
            .borrow_mut()
            .retain(|attribute| attribute.name.local.as_ref() != key);
    }
}

fn replace_stylesheet(node: &Handle, css: &str) {
    if let NodeData::Element { name, attrs, .. } = &node.data {
        let mut name = name.clone();
        name.local = "style".into();
        let attrs = attrs
            .borrow()
            .iter()
            .filter(|attr| {
                !matches!(
                    attr.name.local.as_ref(),
                    "href" | "rel" | "integrity" | "crossorigin"
                )
            })
            .cloned()
            .collect();
        let replacement = Node::new(NodeData::Element {
            name,
            attrs: std::cell::RefCell::new(attrs),
            template_contents: std::cell::RefCell::new(None),
            mathml_annotation_xml_integration_point: false,
        });
        set_text(&replacement, css);
        if let Some(parent) = node.parent.take().and_then(|parent| parent.upgrade()) {
            replacement
                .parent
                .set(Some(std::rc::Rc::downgrade(&parent)));
            if let Some(slot) = parent
                .children
                .borrow_mut()
                .iter_mut()
                .find(|child| std::rc::Rc::ptr_eq(child, node))
            {
                *slot = replacement;
            }
        }
    }
}

fn text(node: &Handle) -> String {
    let mut result = String::new();
    let mut nodes: Vec<_> = node.children.borrow().iter().rev().cloned().collect();
    while let Some(node) = nodes.pop() {
        if let NodeData::Text { contents } = &node.data {
            result.push_str(&contents.borrow());
        }
        nodes.extend(node.children.borrow().iter().rev().cloned());
    }
    result
}

fn set_text(node: &Handle, content: &str) {
    let text = Node::new(NodeData::Text {
        contents: std::cell::RefCell::new(content.into()),
    });
    text.parent.set(Some(std::rc::Rc::downgrade(node)));
    *node.children.borrow_mut() = vec![text];
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

fn validate_script(source: &str) -> Result<(), ArtifactError> {
    for token in ress::Scanner::new(source) {
        let item = token.map_err(|error| invalid(format!("JavaScript could not be scanned: {error}. Fix its syntax or prebundle it before publishing.")))?;
        if matches!(&item.token, ress::tokens::Token::Keyword(keyword) if matches!(keyword.as_str(), "import" | "export"))
        {
            return Err(invalid("JavaScript imports must be prebundled into a standalone script before publishing. Local CSS, classic script src files, images and media are bundled automatically."));
        }
        if matches!(&item.token, ress::tokens::Token::Ident(identifier) if ["fetch", "XMLHttpRequest", "WebSocket", "EventSource", "Worker", "SharedWorker", "importScripts", "require"].contains(&identifier.as_ref()))
        {
            return Err(invalid("JavaScript requests or module loaders must be replaced with inline data and code before publishing an isolated artifact."));
        }
    }
    Ok(())
}
