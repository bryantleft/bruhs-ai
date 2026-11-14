use super::super::canvas::{BoardImages, BoardLayer};
use super::*;
use bruhs_models::num::ToFloatLossy as _;
use image::ImageEncoder as _;
use std::{
    borrow::Cow,
    io::Read as _,
    path::{Path, PathBuf},
};

pub(super) fn decode_images(portable: &PortableBoard) -> Result<BoardImages, String> {
    prepare_images(portable, BoardImages::default())
}
pub(super) fn prepare_images(
    portable: &PortableBoard,
    mut images: BoardImages,
) -> Result<BoardImages, String> {
    let mut total = 0u64;
    for asset in portable.board.assets.values() {
        total = total
            .checked_add(u64::from(asset.width) * u64::from(asset.height) * 4)
            .ok_or("Image budget overflow")?;
        if total > bruhs_models::logic::attachments::MAX_IMAGE_TOTAL_BYTES {
            return Err("This board exceeds the decoded image budget.".into());
        }
        if images.assets.contains_key(&asset.sha256) {
            continue;
        }
        let payload = portable
            .payloads
            .get(&asset.sha256)
            .ok_or("Restore the missing image bytes")?;
        images
            .assets
            .insert(asset.sha256.clone(), decode_raster(asset, payload)?);
    }
    let renderer = SvgRenderer::new(Arc::new(ExportFonts));
    let mut layer_bytes = 0u64;
    for shape in portable.board.shapes.values().filter(|shape| {
        shape.rotation != 0.0
            && (!shape.text.is_empty() || matches!(shape.kind, ShapeKind::Image { .. }))
    }) {
        if images
            .layers
            .get(&shape.id)
            .is_some_and(|layer| layer.matches(shape))
        {
            continue;
        }
        let mut board = BoardDocument::empty(
            uuid::Uuid::new_v4().to_string(),
            portable.board.project_path.clone(),
            "Native layer".into(),
            0,
        );
        board.background = Color {
            r: 0,
            g: 0,
            b: 0,
            a: 0,
        };
        let mut item = shape.clone();
        item.parent = None;
        item.source_id = None;
        item.semantic = None;
        if let ShapeKind::Image { asset_id } = &item.kind {
            let asset = portable
                .board
                .assets
                .get(asset_id)
                .ok_or("Missing layer image")?;
            board.assets.insert(asset_id.clone(), asset.clone());
        }
        board.shapes.insert(item.id.clone(), item);
        let area = bounds(&board, &board.shapes[&shape.id]).expanded(20.0);
        let svg = export_svg(&board, &portable.payloads, None).map_err(|e| e.to_string())?;
        let scale = (bruhs_models::logic::attachments::MAX_IMAGE_TOTAL_BYTES.to_f64_lossy()
            / (area.w * area.h * 4.0))
            .sqrt()
            .min(1.0)
            .min(8192.0 / area.w)
            .min(8192.0 / area.h);
        if !scale.is_finite() || scale <= 0.0 {
            return Err("The rotated layer exceeds native raster limits.".into());
        }
        let image = renderer
            .render_single_frame(svg.as_bytes(), (scale / 2.0).to_f32_lossy())
            .map_err(|e| e.to_string())?;
        layer_bytes = layer_bytes
            .checked_add(
                u64::try_from(
                    image
                        .as_bytes(0)
                        .ok_or("Missing native layer pixels")?
                        .len(),
                )
                .unwrap_or(u64::MAX),
            )
            .ok_or("Native layer budget overflow")?;
        if layer_bytes > bruhs_models::logic::attachments::MAX_IMAGE_TOTAL_BYTES {
            return Err("Rotated labels and images exceed the native layer pixel budget. Split the board or reset their rotation.".into());
        }
        images.layers.insert(
            shape.id.clone(),
            BoardLayer {
                shape: shape.clone(),
                bounds: area,
                image,
            },
        );
    }
    images
        .layers
        .retain(|id, _| portable.board.shapes.contains_key(id));
    Ok(images)
}
fn decode_raster(asset: &Asset, payload: &AssetPayload) -> Result<Arc<RenderImage>, String> {
    let bytes = payload.decode(asset).map_err(|e| e.to_string())?;
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(bruhs_models::logic::attachments::MAX_IMAGE_TOTAL_BYTES);
    reader.limits(limits);
    let mut rgba = reader
        .decode()
        .map_err(|e| format!("The image could not be decoded: {e}"))?
        .to_rgba8();
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    Ok(Arc::new(RenderImage::new(vec![image::Frame::new(rgba)])))
}
fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Choose a regular file.".into());
    }
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > limit {
        return Err("The file exceeds the board content budget.".into());
    }
    Ok(bytes)
}
impl WhiteboardLibrary {
    pub(super) fn pick_file(
        &mut self,
        image: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let chooser = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(
                if image {
                    "Choose local image"
                } else {
                    "Import board"
                }
                .into(),
            ),
        });
        let project = self.project.clone();
        let board_id = self.current.clone();
        cx.spawn_in(window, async move |this, cx| {
            let path = match chooser.await {
                Ok(Ok(Some(paths))) => paths.into_iter().next(),
                Ok(Ok(None)) => None,
                Ok(Err(e)) => {
                    let _ = this.update(cx, |this, cx| this.fail(e, cx));
                    return;
                }
                Err(_) => None,
            };
            let Some(path) = path else {
                return;
            };
            let result = cx
                .background_spawn(async move {
                    let bytes = read_bounded(
                        &path,
                        if image {
                            bruhs_models::logic::attachments::MAX_IMAGE_BYTES
                        } else {
                            MAX_PORTABLE_BYTES
                        },
                    )?;
                    if image {
                        let (asset, payload) =
                            AssetPayload::from_bytes(&bytes).map_err(|e| e.to_string())?;
                        let raster = decode_raster(&asset, &payload)?;
                        Ok::<_, String>(Picked::Image(asset, payload, raster))
                    } else {
                        let text = std::str::from_utf8(&bytes).map_err(|e| e.to_string())?;
                        let imported = import_board_file(
                            text,
                            &path.file_name().unwrap_or_default().to_string_lossy(),
                            project,
                            now(),
                        )
                        .map_err(|e| e.to_string())?;
                        Ok(Picked::Board(imported))
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(Picked::Board(imported)) => {
                    this.import_warnings = imported.warnings;
                    if let Err(e) = this.set_preview(imported.portable, cx) {
                        this.fail(e, cx);
                    }
                }
                Ok(Picked::Image(asset, payload, raster)) => {
                    if let Some(id) = board_id {
                        this.insert_image(&id, asset, payload, raster, cx);
                    }
                }
                Err(e) => this.fail(e, cx),
            });
        })
        .detach();
    }
    fn insert_image(
        &mut self,
        id: &str,
        asset: Asset,
        payload: AssetPayload,
        raster: Arc<RenderImage>,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(seat) = self.seats.get_mut(id) else {
            return;
        };
        let at = seat.view.read(cx).center();
        let hash = asset.sha256.clone();
        let changed = seat.view.update(cx, |view, cx| {
            let before = view.input.borrow().editor.document().revision;
            view.edit(
                |input| {
                    let scale = (480.0 / f64::from(asset.width))
                        .min(320.0 / f64::from(asset.height))
                        .min(1.0);
                    let shape = Shape::node(
                        uuid::Uuid::new_v4().to_string(),
                        ShapeKind::Image {
                            asset_id: asset.id.clone(),
                        },
                        Rect::new(
                            at.x,
                            at.y,
                            f64::from(asset.width) * scale,
                            f64::from(asset.height) * scale,
                        ),
                        input.editor.next_z()?,
                        String::new(),
                    );
                    let id = shape.id.clone();
                    input.editor.edit(
                        vec![
                            BoardOperation::Asset { asset },
                            BoardOperation::shape(shape),
                        ],
                        now(),
                    )?;
                    input.editor.selection = std::collections::BTreeSet::from([id]);
                    Ok(())
                },
                cx,
            );
            let changed = view.input.borrow().editor.document().revision != before;
            if changed {
                let mut images = view.images();
                images.assets.insert(hash, raster);
                view.install_images(images, cx);
            }
            changed
        });
        if changed {
            Arc::make_mut(&mut seat.payloads).insert(payload.sha256.clone(), payload);
        }
    }
    pub(super) fn export(
        &mut self,
        action: BoardAction,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(portable) = self.portable(cx) else {
            return;
        };
        let frame = self.active().and_then(|s| {
            let view = s.view.read(cx);
            let input = view.input.borrow();
            input
                .editor
                .selection
                .iter()
                .any(|id| {
                    input.editor.document().shapes.get(id).is_some_and(|shape| {
                        matches!(shape.kind, ShapeKind::Frame | ShapeKind::Group)
                    })
                })
                .then(|| input.editor.selection.clone())
        });
        let extension = match action {
            BoardAction::ExportSvg => "svg",
            BoardAction::ExportPng => "png",
            BoardAction::ExportMermaid => "mmd",
            _ => "bruhs-board.json",
        };
        let name = format!("whiteboard.{extension}");
        let directory = PathBuf::from(&self.project);
        let chooser = cx.prompt_for_new_path(&directory, Some(&name));
        cx.spawn_in(window, async move |this, cx| {
            let path = match chooser.await {
                Ok(Ok(path)) => path,
                _ => None,
            };
            let Some(path) = path else {
                return;
            };
            let work = cx.background_spawn(async move {
                let bytes = match action {
                    BoardAction::ExportSvg => export_svg(&portable.board, &portable.payloads, None)
                        .map(String::into_bytes)
                        .map_err(|e| e.to_string())?,
                    BoardAction::ExportMermaid => {
                        export_mermaid_selection(&portable.board, frame.as_ref())
                            .map(String::into_bytes)
                            .map_err(|e| e.to_string())?
                    }
                    BoardAction::ExportPng => png(&portable)?,
                    _ => serde_json::to_vec_pretty(&portable).map_err(|e| e.to_string())?,
                };
                std::fs::write(path, bytes).map_err(|e| e.to_string())
            });
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(()) => this.notice = Some("Export saved.".into()),
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub(super) fn check_sources(&mut self, cx: &mut Context<'_, Self>) {
        let Some(board) = self.document(cx) else {
            return;
        };
        if board.sources.is_empty() {
            return;
        }
        let id = board.id;
        let project = PathBuf::from(&self.project);
        let revision = board.revision;
        let work = cx.background_spawn(async move {
            board
                .sources
                .into_values()
                .map(|mut source| {
                    source.stale =
                        bruhs_core::whiteboard::source_fingerprint(&project, &source.path)
                            .map_or(true, |hash| hash != source.sha256);
                    BoardOperation::Source { source }
                })
                .collect::<Vec<_>>()
        });
        cx.spawn(async move|this,cx|{let ops=work.await;let _=this.update(cx,|this,cx|{if let Some(seat)=this.seats.get(&id){seat.view.update(cx,|view,cx|view.edit(|input|{if input.editor.document().revision!=revision{return Err(BoardError::Conflict);}let changed=ops.into_iter().filter(|op|match op{BoardOperation::Source{source}=>input.editor.document().sources.get(&source.id)!=Some(source),_=>true}).collect();input.editor.edit(changed,now())},cx));}this.notice=Some("Source fingerprints checked. Stale diagrams keep your edits; request and review an agent refresh.".into());cx.notify();});}).detach();
    }
}
enum Picked {
    Image(Asset, AssetPayload, Arc<RenderImage>),
    Board(BoardImport),
}
struct ExportFonts;
impl AssetSource for ExportFonts {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        Ok(matches!(
            path,
            "fonts/ibm-plex-sans/IBMPlexSans-Regular.ttf" | "fonts/lilex/Lilex-Regular.ttf"
        )
        .then(|| {
            Cow::Borrowed(
                include_bytes!("../../../../assets/fonts/MonaspaceArgon-Regular.otf").as_slice(),
            )
        }))
    }
    fn list(&self, _: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(Vec::new())
    }
}
fn png(portable: &PortableBoard) -> Result<Vec<u8>, String> {
    let svg = export_svg(&portable.board, &portable.payloads, None).map_err(|e| e.to_string())?;
    let editor = BoardEditor::new(portable.board.clone()).map_err(|e| e.to_string())?;
    let area = editor
        .bounds()
        .unwrap_or(Rect::new(0.0, 0.0, 640.0, 480.0))
        .expanded(20.0);
    let scale = (bruhs_models::logic::attachments::MAX_IMAGE_TOTAL_BYTES.to_f64_lossy()
        / (area.w * area.h * 4.0))
        .sqrt()
        .min(1.0)
        .min(8192.0 / area.w)
        .min(8192.0 / area.h);
    if !scale.is_finite() || scale <= 0.0 {
        return Err("The board cannot be rasterized. Export SVG instead.".into());
    }
    let image = SvgRenderer::new(Arc::new(ExportFonts))
        .render_single_frame(svg.as_bytes(), (scale / 2.0).to_f32_lossy())
        .map_err(|e| e.to_string())?;
    let mut bytes = image
        .as_bytes(0)
        .ok_or("PNG raster is unavailable")?
        .to_vec();
    for pixel in bytes.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let size = image.size(0);
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(
            &bytes,
            u32::try_from(size.width.0).map_err(|_| "Invalid image width")?,
            u32::try_from(size.height.0).map_err(|_| "Invalid image height")?,
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|e| e.to_string())?;
    Ok(png)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[::core::prelude::v1::test]
    fn local_raster_decode_and_png_export_paint_real_image_pixels() {
        let mut imported = import_board_file(
            include_str!("../../../../../bench/whiteboard/fixtures/excalidraw-v2.excalidraw"),
            "fixture.excalidraw",
            "/fixture".into(),
            1,
        )
        .unwrap();
        imported
            .portable
            .board
            .shapes
            .get_mut("a")
            .unwrap()
            .rotation = 0.2;
        let images = decode_images(&imported.portable).unwrap();
        assert_eq!(images.assets.len(), 1);
        assert!(images.layers.contains_key("a"));
        let bytes = png(&imported.portable).unwrap();
        let image = image::load_from_memory(&bytes).unwrap().to_rgba8();
        assert!(image.width() > 0 && image.height() > 0);
        assert!(image
            .pixels()
            .any(|pixel| pixel[0] > 200 && pixel[1] < 30 && pixel[2] < 30 && pixel[3] == 255));
    }
}
