use super::{BoardError, BoardResult, Point, Rect, BOARD_FORMAT_VERSION};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub type ShapeId = String;
pub type AssetId = String;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoardDocument {
    pub format: u32,
    pub id: String,
    pub project_path: String,
    pub title: String,
    pub revision: u64,
    #[serde(default = "paper_color")]
    pub background: Color,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    #[serde(default)]
    pub agent_read: bool,
    pub shapes: BTreeMap<ShapeId, Shape>,
    #[serde(default)]
    pub assets: BTreeMap<AssetId, Asset>,
    #[serde(default)]
    pub sources: BTreeMap<String, Source>,
}
impl BoardDocument {
    pub fn empty(id: String, project_path: String, title: String, now_ms: i64) -> Self {
        Self {
            format: BOARD_FORMAT_VERSION,
            id,
            project_path,
            title,
            revision: 0,
            background: paper_color(),
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
            agent_read: false,
            shapes: BTreeMap::new(),
            assets: BTreeMap::new(),
            sources: BTreeMap::new(),
        }
    }
    pub fn summary(&self) -> BoardSummary {
        BoardSummary {
            id: self.id.clone(),
            project_path: self.project_path.clone(),
            title: self.title.clone(),
            revision: self.revision,
            updated_at_ms: self.updated_at_ms,
            shape_count: self.shapes.len(),
            agent_read: self.agent_read,
        }
    }
    pub fn validate(&self) -> BoardResult<()> {
        if self.format != BOARD_FORMAT_VERSION {
            return Err(BoardError::Invalid(
                "This board format is unsupported. Open it with the version that saved it.".into(),
            ));
        }
        validate_id(&self.id)?;
        uuid::Uuid::parse_str(&self.id)
            .map_err(|_| BoardError::Invalid("A board identity must be a UUID.".into()))?;
        if self.title.trim().is_empty() {
            return Err(BoardError::Invalid("Give the board a title.".into()));
        }
        for (id, shape) in &self.shapes {
            validate_id(id)?;
            if id != &shape.id {
                return Err(BoardError::Invalid(
                    "A shape key does not match its identity.".into(),
                ));
            }
            shape.validate()?;
            if shape
                .source_id
                .as_ref()
                .is_some_and(|id| !self.sources.contains_key(id))
            {
                return Err(BoardError::Invalid(
                    "A shape source reference is missing.".into(),
                ));
            }
            if let Some(parent) = &shape.parent {
                if parent == id
                    || !self
                        .shapes
                        .get(parent)
                        .is_some_and(|p| matches!(p.kind, ShapeKind::Frame | ShapeKind::Group))
                {
                    return Err(BoardError::Invalid(
                        "A shape parent must be an existing frame or group.".into(),
                    ));
                }
            }
            if let ShapeKind::Arrow { start, end } = &shape.kind {
                for endpoint in [start, end] {
                    if let Some(binding) = &endpoint.binding {
                        if &binding.shape_id == id
                            || !self
                                .shapes
                                .get(&binding.shape_id)
                                .is_some_and(Shape::bindable)
                        {
                            return Err(BoardError::Invalid(
                                "Arrow bindings must refer to existing diagram nodes.".into(),
                            ));
                        }
                    }
                }
            }
            if let ShapeKind::Image { asset_id } = &shape.kind {
                if !self.assets.contains_key(asset_id) {
                    return Err(BoardError::Invalid(
                        "This image asset is missing. Import it before adding the image.".into(),
                    ));
                }
            }
        }
        // An iterative walk avoids stack exhaustion on deeply nested imported frames.
        let mut complete = std::collections::BTreeSet::new();
        for id in self.shapes.keys() {
            let mut path = std::collections::BTreeSet::new();
            let mut current = Some(id);
            while let Some(id) = current {
                if complete.contains(id) {
                    break;
                }
                if !path.insert(id.clone()) {
                    return Err(BoardError::Invalid(
                        "Frame/group parents contain a cycle.".into(),
                    ));
                }
                current = self.shapes.get(id).and_then(|shape| shape.parent.as_ref());
            }
            complete.extend(path);
        }
        let mut aliases = std::collections::BTreeSet::new();
        let mut message_order = std::collections::BTreeSet::new();
        for shape in self.shapes.values() {
            match &shape.semantic {
                Some(ShapeSemantic::SequenceParticipant { name, .. })
                    if !shape.bindable()
                        || name.is_empty()
                        || !name
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
                        || !aliases.insert(name) =>
                {
                    return Err(BoardError::Invalid(
                        "Sequence participants need unique, simple aliases and bindable shapes."
                            .into(),
                    ));
                }
                Some(ShapeSemantic::SequenceParticipant { .. }) => {}
                Some(ShapeSemantic::SequenceMessage {
                    from,
                    to,
                    index,
                    dashed,
                }) => {
                    let valid = if let ShapeKind::Arrow { start, end } = &shape.kind {
                        start.binding.as_ref().is_some_and(|b| &b.shape_id == from)
                            && end.binding.as_ref().is_some_and(|b| &b.shape_id == to)
                    } else {
                        false
                    };
                    if !valid
                        || from == to
                        || *dashed != shape.style.dashed
                        || !message_order.insert(*index)
                        || ![from, to].into_iter().all(|id| {
                            self.shapes.get(id).is_some_and(|s| {
                                matches!(
                                    s.semantic,
                                    Some(ShapeSemantic::SequenceParticipant { .. })
                                )
                            })
                        })
                    {
                        return Err(BoardError::Invalid("Sequence messages must match their bound participants, dash style and unique order.".into()));
                    }
                }
                None => {}
            }
        }
        for (id, asset) in &self.assets {
            validate_id(id)?;
            if asset.id != *id
                || asset.width == 0
                || asset.height == 0
                || !matches!(
                    asset.mime.as_str(),
                    "image/png" | "image/jpeg" | "image/webp" | "image/gif"
                )
                || asset.sha256.len() != 64
                || asset.bytes == 0
                || u64::from(asset.width)
                    .checked_mul(u64::from(asset.height))
                    .and_then(|pixels| pixels.checked_mul(4))
                    .is_none_or(|bytes| bytes > crate::logic::attachments::MAX_IMAGE_TOTAL_BYTES)
                || !asset.sha256.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err(BoardError::Invalid(
                    "The local raster asset metadata is invalid.".into(),
                ));
            }
            if asset.bytes > crate::logic::attachments::MAX_IMAGE_BYTES {
                return Err(BoardError::Invalid(
                    "This image exceeds the existing attachment image budget.".into(),
                ));
            }
        }
        let pixels = self
            .assets
            .values()
            .try_fold(0u64, |total, asset| {
                u64::from(asset.width)
                    .checked_mul(u64::from(asset.height))
                    .and_then(|n| n.checked_mul(4))
                    .and_then(|n| total.checked_add(n))
            })
            .ok_or_else(|| BoardError::Invalid("The image budget overflowed.".into()))?;
        if pixels > crate::logic::attachments::MAX_IMAGE_TOTAL_BYTES {
            return Err(BoardError::Invalid(
                "This board exceeds the existing decoded image budget.".into(),
            ));
        }
        for (id, source) in &self.sources {
            validate_id(id)?;
            validate_relative_path(&source.path)?;
            if source.id != *id
                || source.sha256.len() != 64
                || !source.sha256.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err(BoardError::Invalid("Source fingerprints must contain an identity, relative path and SHA-256 digest.".into()));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoardSummary {
    pub id: String,
    pub project_path: String,
    pub title: String,
    pub revision: u64,
    pub updated_at_ms: i64,
    pub shape_count: usize,
    pub agent_read: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Shape {
    pub id: ShapeId,
    pub kind: ShapeKind,
    pub rect: Rect,
    #[serde(default)]
    pub rotation: f64,
    pub z: i64,
    #[serde(default)]
    pub parent: Option<ShapeId>,
    #[serde(default)]
    pub text: String,
    pub style: Style,
    #[serde(default)]
    pub links: Vec<BoardLink>,
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic: Option<ShapeSemantic>,
}
impl Shape {
    pub fn node(id: String, kind: ShapeKind, rect: Rect, z: i64, text: String) -> Self {
        Self {
            id,
            kind,
            rect,
            rotation: 0.0,
            z,
            parent: None,
            text,
            style: Style::default(),
            links: Vec::new(),
            source_id: None,
            semantic: None,
        }
    }
    pub fn bindable(&self) -> bool {
        matches!(
            self.kind,
            ShapeKind::Rectangle
                | ShapeKind::Ellipse
                | ShapeKind::Text
                | ShapeKind::Note
                | ShapeKind::Frame
                | ShapeKind::Image { .. }
        )
    }
    pub fn validate(&self) -> BoardResult<()> {
        validate_id(&self.id)?;
        self.rect.validate()?;
        if !self.rotation.is_finite()
            || !self.style.stroke_width.is_finite()
            || self.style.stroke_width < 0.0
            || !self.style.font_size.is_finite()
            || self.style.font_size <= 0.0
        {
            return Err(BoardError::Invalid(
                "Shape rotation, stroke and text size must be finite and valid.".into(),
            ));
        }
        match &self.kind {
            ShapeKind::Arrow { start, end } => {
                start.validate()?;
                end.validate()?;
            }
            ShapeKind::Freehand { points }
                if points.len() < 2 || points.iter().any(|p| !p.finite()) =>
            {
                return Err(BoardError::Invalid(
                    "A freehand path needs two finite points.".into(),
                ));
            }
            ShapeKind::Image { asset_id } => validate_id(asset_id)?,
            _ => {}
        }
        for link in &self.links {
            link.validate()?;
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ShapeSemantic {
    SequenceParticipant {
        name: String,
        actor: bool,
    },
    SequenceMessage {
        from: ShapeId,
        to: ShapeId,
        index: u32,
        dashed: bool,
    },
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ShapeKind {
    Rectangle,
    Ellipse,
    Text,
    Note,
    Frame,
    Group,
    Arrow { start: Endpoint, end: Endpoint },
    Freehand { points: Vec<Point> },
    Image { asset_id: AssetId },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}
pub fn paper_color() -> Color {
    Color::rgb(16, 17, 15)
}
impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }
    pub fn css(self) -> String {
        format!("#{:02x}{:02x}{:02x}{:02x}", self.r, self.g, self.b, self.a)
    }
    pub fn packed(self) -> u32 {
        (u32::from(self.r) << 16) | (u32::from(self.g) << 8) | u32::from(self.b)
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Style {
    pub stroke: Color,
    pub fill: Option<Color>,
    pub stroke_width: f64,
    pub font_size: f64,
    #[serde(default)]
    pub dashed: bool,
}
impl Default for Style {
    fn default() -> Self {
        Self {
            stroke: Color::rgb(220, 222, 211),
            fill: Some(Color::rgb(38, 42, 33)),
            stroke_width: 2.0,
            font_size: 18.0,
            dashed: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Endpoint {
    pub point: Point,
    #[serde(default)]
    pub binding: Option<Binding>,
}
impl Endpoint {
    pub fn free(point: Point) -> Self {
        Self {
            point,
            binding: None,
        }
    }
    pub fn bound(shape_id: String, anchor: Point) -> Self {
        Self {
            point: Point::default(),
            binding: Some(Binding {
                shape_id,
                anchor,
                offset: Point::default(),
            }),
        }
    }
    fn validate(&self) -> BoardResult<()> {
        if !self.point.finite() {
            return Err(BoardError::Invalid(
                "Arrow endpoints must be finite.".into(),
            ));
        }
        if let Some(binding) = &self.binding {
            validate_id(&binding.shape_id)?;
            if !binding.anchor.finite()
                || !binding.offset.finite()
                || !(0.0..=1.0).contains(&binding.anchor.x)
                || !(0.0..=1.0).contains(&binding.anchor.y)
            {
                return Err(BoardError::Invalid(
                    "A bound arrow anchor must be inside its target.".into(),
                ));
            }
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub shape_id: ShapeId,
    pub anchor: Point,
    #[serde(default)]
    pub offset: Point,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum BoardLink {
    Thread {
        id: String,
    },
    File {
        path: String,
        start_line: u32,
        end_line: u32,
    },
    Task {
        id: String,
    },
    PullRequest {
        url: String,
    },
    Meeting {
        id: String,
    },
    Person {
        id: String,
    },
    Document {
        id: String,
    },
}
impl BoardLink {
    pub fn label(&self) -> String {
        match self {
            Self::Thread { id } => format!("Thread {id}"),
            Self::File {
                path,
                start_line,
                end_line,
            } => format!("{path}:{start_line}–{end_line}"),
            Self::Task { id } => format!("Task {id}"),
            Self::PullRequest { url } => url.clone(),
            Self::Meeting { id } => format!("Meeting {id}"),
            Self::Person { id } => format!("Person {id}"),
            Self::Document { id } => format!("Document {id}"),
        }
    }
    pub fn validate(&self) -> BoardResult<()> {
        match self {
            Self::File {
                path,
                start_line,
                end_line,
            } => {
                validate_relative_path(path)?;
                if *start_line == 0 || end_line < start_line {
                    return Err(BoardError::Invalid(
                        "File links need a positive, ordered line range.".into(),
                    ));
                }
            }
            Self::PullRequest { url } => {
                if !url.starts_with("https://") || url.chars().any(char::is_control) {
                    return Err(BoardError::Invalid(
                        "Pull request links must use HTTPS.".into(),
                    ));
                }
            }
            Self::Thread { id }
            | Self::Task { id }
            | Self::Meeting { id }
            | Self::Person { id }
            | Self::Document { id } => validate_id(id)?,
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Asset {
    pub id: AssetId,
    pub mime: String,
    pub bytes: u64,
    pub width: u32,
    pub height: u32,
    pub sha256: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub id: String,
    pub path: String,
    pub sha256: String,
    #[serde(default)]
    pub mermaid: Option<String>,
    #[serde(default)]
    pub stale: bool,
}
pub fn validate_id(id: &str) -> BoardResult<()> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'))
        || matches!(id, "." | "..")
    {
        Err(BoardError::Invalid(
            "Board identities must be visible strings of 1–128 bytes.".into(),
        ))
    } else {
        Ok(())
    }
}
pub fn validate_relative_path(path: &str) -> BoardResult<()> {
    if path.is_empty()
        || path.as_bytes().get(1) == Some(&b':')
        || path.contains(['\\', '\0'])
        || path.starts_with('/')
        || path
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
        || path.chars().any(char::is_control)
    {
        Err(BoardError::Invalid(
            "Use a relative project file path without parent traversal.".into(),
        ))
    } else {
        Ok(())
    }
}
