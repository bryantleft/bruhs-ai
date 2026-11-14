//! Canvas painter shared by the editor and its opt-in rendering benchmark.
use bruhs_models::{num::ToIntSaturating, whiteboard::*};
use std::{collections::BTreeMap, rc::Rc};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement};

struct Raster {
    image: web_sys::HtmlImageElement,
    _load: Closure<dyn FnMut(web_sys::Event)>,
    _error: Closure<dyn FnMut(web_sys::Event)>,
}
impl Drop for Raster {
    fn drop(&mut self) {
        self.image.set_onload(None);
        self.image.set_onerror(None);
    }
}

pub(crate) struct BoardCanvas {
    pub element: HtmlCanvasElement,
    context: CanvasRenderingContext2d,
    pub width: f64,
    pub height: f64,
    pub dpr: f64,
    pub draws: u64,
    pub painted: usize,
    assets: BTreeMap<String, Raster>,
}
impl BoardCanvas {
    pub fn new(
        element: HtmlCanvasElement,
        width: f64,
        height: f64,
        dpr: f64,
    ) -> Result<Self, JsValue> {
        let context = element
            .get_context("2d")?
            .ok_or_else(|| JsValue::from_str("Canvas 2D is unavailable"))?
            .dyn_into::<CanvasRenderingContext2d>()?;
        let mut this = Self {
            element,
            context,
            width,
            height,
            dpr,
            draws: 0,
            painted: 0,
            assets: BTreeMap::new(),
        };
        this.resize(width, height, dpr)?;
        Ok(this)
    }
    pub fn install_assets(
        &mut self,
        board: &BoardDocument,
        payloads: &BTreeMap<String, AssetPayload>,
        changed: Rc<dyn Fn()>,
    ) -> Result<(), JsValue> {
        for asset in board.assets.values() {
            if self.assets.contains_key(&asset.sha256) {
                continue;
            }
            let payload = payloads
                .get(&asset.sha256)
                .ok_or_else(|| JsValue::from_str("Local image bytes are missing"))?;
            let url = payload
                .data_url(asset)
                .map_err(|e| JsValue::from_str(&e.to_string()))?;
            let image = web_sys::HtmlImageElement::new()?;
            let ready = changed.clone();
            let load =
                Closure::wrap(Box::new(move |_: web_sys::Event| ready()) as Box<dyn FnMut(_)>);
            let ready = changed.clone();
            let error =
                Closure::wrap(Box::new(move |_: web_sys::Event| ready()) as Box<dyn FnMut(_)>);
            image.set_onload(Some(load.as_ref().unchecked_ref()));
            image.set_onerror(Some(error.as_ref().unchecked_ref()));
            image.set_src(&url);
            self.assets.insert(
                asset.sha256.clone(),
                Raster {
                    image,
                    _load: load,
                    _error: error,
                },
            );
        }
        Ok(())
    }
    pub async fn wait_images(&self) -> Result<(), JsValue> {
        for raster in self.assets.values() {
            wasm_bindgen_futures::JsFuture::from(raster.image.decode()).await?;
        }
        Ok(())
    }
    pub fn failed_images(&self) -> usize {
        self.assets
            .values()
            .filter(|r| r.image.complete() && r.image.natural_width() == 0)
            .count()
    }
    pub fn resize(&mut self, width: f64, height: f64, dpr: f64) -> Result<(), JsValue> {
        if ![width, height, dpr, width * dpr, height * dpr]
            .into_iter()
            .all(f64::is_finite)
            || width <= 0.0
            || height <= 0.0
            || dpr <= 0.0
        {
            return Err(JsValue::from_str(
                "Canvas dimensions must be positive and finite",
            ));
        }
        self.width = width;
        self.height = height;
        self.dpr = dpr;
        self.element
            .set_width((width * dpr).round().to_u32_saturating());
        self.element
            .set_height((height * dpr).round().to_u32_saturating());
        self.element
            .style()
            .set_property("width", &format!("{width}px"))?;
        self.element
            .style()
            .set_property("height", &format!("{height}px"))?;
        Ok(())
    }
    pub fn draw(&mut self, editor: &BoardEditor) -> Result<(), JsValue> {
        let ctx = &self.context;
        let camera = editor.camera;
        let doc = editor.document();
        let viewport = camera.viewport(self.width, self.height);
        ctx.set_transform(self.dpr, 0.0, 0.0, self.dpr, 0.0, 0.0)?;
        ctx.set_fill_style_str(&doc.background.css());
        ctx.fill_rect(0.0, 0.0, self.width, self.height);
        ctx.set_transform(
            camera.zoom * self.dpr,
            0.0,
            0.0,
            camera.zoom * self.dpr,
            -camera.x * camera.zoom * self.dpr,
            -camera.y * camera.zoom * self.dpr,
        )?;
        self.painted = 0;
        for shape in editor.ordered() {
            if matches!(shape.kind, ShapeKind::Group) {
                continue;
            }
            let selected = editor
                .movement
                .as_ref()
                .is_some_and(|m| m.ids.contains(&shape.id));
            let delta = editor
                .movement
                .as_ref()
                .map_or(Point::default(), |m| m.delta);
            let shape_bounds = display_bounds(editor, shape);
            if !viewport
                .expanded(shape.style.stroke_width)
                .intersects(shape_bounds)
            {
                continue;
            }
            self.painted += 1;
            ctx.save();
            ctx.set_line_width(shape.style.stroke_width);
            ctx.set_stroke_style_str(&shape.style.stroke.css());
            ctx.set_line_cap("round");
            ctx.set_line_join("round");
            if shape.style.dashed {
                let dash = js_sys::Array::new();
                dash.push(&6.0.into());
                dash.push(&4.0.into());
                ctx.set_line_dash(&dash)?;
            }
            if !matches!(shape.kind, ShapeKind::Arrow { .. }) {
                if selected {
                    ctx.translate(delta.x, delta.y)?;
                }
                if shape.rotation != 0.0 {
                    let center = shape.rect.center();
                    ctx.translate(center.x, center.y)?;
                    ctx.rotate(shape.rotation)?;
                    ctx.translate(-center.x, -center.y)?;
                }
            }
            match &shape.kind {
                ShapeKind::Image { asset_id } => {
                    if let Some(raster) = doc
                        .assets
                        .get(asset_id)
                        .and_then(|asset| self.assets.get(&asset.sha256))
                        .filter(|r| r.image.complete() && r.image.natural_width() > 0)
                    {
                        ctx.draw_image_with_html_image_element_and_dw_and_dh(
                            &raster.image,
                            shape.rect.x,
                            shape.rect.y,
                            shape.rect.w,
                            shape.rect.h,
                        )?;
                    } else {
                        ctx.stroke_rect(shape.rect.x, shape.rect.y, shape.rect.w, shape.rect.h);
                    }
                }
                ShapeKind::Rectangle | ShapeKind::Note | ShapeKind::Frame => {
                    if !matches!(shape.kind, ShapeKind::Frame) {
                        if let Some(fill) = shape.style.fill {
                            ctx.set_fill_style_str(&fill.css());
                            ctx.fill_rect(shape.rect.x, shape.rect.y, shape.rect.w, shape.rect.h);
                        }
                    }
                    ctx.stroke_rect(shape.rect.x, shape.rect.y, shape.rect.w, shape.rect.h);
                }
                ShapeKind::Ellipse => {
                    ctx.begin_path();
                    ctx.ellipse(
                        shape.rect.center().x,
                        shape.rect.center().y,
                        shape.rect.w / 2.0,
                        shape.rect.h / 2.0,
                        0.0,
                        0.0,
                        std::f64::consts::TAU,
                    )?;
                    if let Some(fill) = shape.style.fill {
                        ctx.set_fill_style_str(&fill.css());
                        ctx.fill();
                    }
                    ctx.stroke();
                }
                ShapeKind::Arrow { start, end } => {
                    let a = display_endpoint(editor, shape, start);
                    let b = display_endpoint(editor, shape, end);
                    ctx.begin_path();
                    ctx.move_to(a.x, a.y);
                    ctx.line_to(b.x, b.y);
                    ctx.stroke();
                    let length = a.distance(b);
                    if length > 0.0 {
                        let head = shape.style.stroke_width * 5.0;
                        let dx = (b.x - a.x) / length;
                        let dy = (b.y - a.y) / length;
                        ctx.begin_path();
                        ctx.move_to(b.x, b.y);
                        ctx.line_to(
                            b.x - dx * head - dy * head / 2.0,
                            b.y - dy * head + dx * head / 2.0,
                        );
                        ctx.line_to(
                            b.x - dx * head + dy * head / 2.0,
                            b.y - dy * head - dx * head / 2.0,
                        );
                        ctx.close_path();
                        ctx.set_fill_style_str(&shape.style.stroke.css());
                        ctx.fill();
                    }
                }
                ShapeKind::Freehand { points } => {
                    ctx.begin_path();
                    for (i, p) in points.iter().enumerate() {
                        if i == 0 {
                            ctx.move_to(p.x, p.y);
                        } else {
                            ctx.line_to(p.x, p.y);
                        }
                    }
                    ctx.stroke();
                }
                _ => {}
            }
            if !shape.text.is_empty() {
                ctx.set_font(&format!(
                    "{}px MonaspaceArgonVariable,monospace",
                    shape.style.font_size
                ));
                ctx.set_fill_style_str(&shape.style.stroke.css());
                ctx.set_text_baseline("middle");
                let centered = !matches!(shape.kind, ShapeKind::Text | ShapeKind::Frame);
                ctx.set_text_align(if centered { "center" } else { "left" });
                let anchor = if matches!(shape.kind, ShapeKind::Arrow { .. }) {
                    display_text_anchor(editor, shape)
                } else {
                    text_anchor(doc, shape)
                };
                let x = anchor.x;
                let y = anchor.y;
                let lines = shape.text.lines();
                let line_height = shape.style.font_size * 1.35;
                let max_lines = (shape.rect.h / line_height)
                    .floor()
                    .max(1.0)
                    .to_usize_saturating();
                for (index, line) in lines.take(max_lines).enumerate() {
                    ctx.fill_text(
                        line,
                        x,
                        y + bruhs_models::num::ToFloatLossy::to_f64_lossy(index) * line_height,
                    )?;
                }
            }
            ctx.restore();
        }
        if let Some(bounds) = editor.selected_bounds() {
            let bounds = editor
                .movement
                .as_ref()
                .map_or(bounds, |m| bounds.translated(m.delta));
            ctx.set_line_width(1.0 / camera.zoom);
            ctx.set_stroke_style_str("#4169e1");
            ctx.stroke_rect(bounds.x, bounds.y, bounds.w, bounds.h);
        }
        if let Some(bounds) = editor.selection_box {
            ctx.set_line_width(1.0 / camera.zoom);
            ctx.set_stroke_style_str("#4169e1");
            ctx.set_fill_style_str("#4169e122");
            ctx.fill_rect(bounds.x, bounds.y, bounds.w, bounds.h);
            ctx.stroke_rect(bounds.x, bounds.y, bounds.w, bounds.h);
        }
        self.draws = self.draws.wrapping_add(1);
        Ok(())
    }
}
