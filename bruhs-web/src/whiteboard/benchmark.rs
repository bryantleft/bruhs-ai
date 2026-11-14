//! Opt-in adapter around the production canvas and shared Rust editor.
use super::canvas::BoardCanvas;
use bruhs_models::whiteboard::*;
use std::{cell::RefCell, rc::Rc};
use wasm_bindgen::{prelude::*, JsCast};

struct State {
    editor: Option<BoardEditor>,
    canvas: BoardCanvas,
    updates: u64,
}
pub fn mount() -> Result<(), JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("Window unavailable"))?;
    let document = window
        .document()
        .ok_or_else(|| JsValue::from_str("Document unavailable"))?;
    let canvas = document
        .create_element("canvas")?
        .dyn_into::<web_sys::HtmlCanvasElement>()?;
    canvas.set_id("whiteboard-canvas");
    canvas.set_attribute("aria-label", "Whiteboard benchmark")?;
    document
        .body()
        .ok_or_else(|| JsValue::from_str("Body unavailable"))?
        .append_child(&canvas)?;
    let state = Rc::new(RefCell::new(State {
        editor: None,
        canvas: BoardCanvas::new(
            canvas,
            window
                .inner_width()?
                .as_f64()
                .ok_or_else(|| JsValue::from_str("Invalid width"))?,
            window
                .inner_height()?
                .as_f64()
                .ok_or_else(|| JsValue::from_str("Invalid height"))?,
            window.device_pixel_ratio(),
        )?,
        updates: 0,
    }));
    let api = js_sys::Object::new();
    let target = state.clone();
    let load = Closure::wrap(Box::new(move |value: JsValue| -> Result<JsValue, JsValue> {
        let doc: BoardDocument =
            serde_wasm_bindgen::from_value(value).map_err(|e| JsValue::from_str(&e.to_string()))?;
        let editor = BoardEditor::new(doc).map_err(error)?;
        let mut state = target.borrow_mut();
        state.editor = Some(editor);
        state.updates = 0;
        paint(&mut state)?;
        stats(&state)
    })
        as Box<dyn FnMut(JsValue) -> Result<JsValue, JsValue>>);
    js_sys::Reflect::set(&api, &"load".into(), load.as_ref())?;
    load.forget();
    let target = state.clone();
    let camera = Closure::wrap(Box::new(move |value: JsValue| -> Result<(), JsValue> {
        let camera: Camera =
            serde_wasm_bindgen::from_value(value).map_err(|e| JsValue::from_str(&e.to_string()))?;
        if ![camera.x, camera.y, camera.zoom]
            .into_iter()
            .all(f64::is_finite)
            || camera.zoom <= 0.0
        {
            return Err(JsValue::from_str("Invalid camera"));
        }
        let mut state = target.borrow_mut();
        state
            .editor
            .as_mut()
            .ok_or_else(|| JsValue::from_str("Scene not loaded"))?
            .camera = camera;
        state.updates += 1;
        paint(&mut state)
    }) as Box<dyn FnMut(JsValue) -> Result<(), JsValue>>);
    js_sys::Reflect::set(&api, &"camera".into(), camera.as_ref())?;
    camera.forget();
    let target = state.clone();
    let begin = Closure::wrap(Box::new(move |value: JsValue| -> Result<(), JsValue> {
        let ids: Vec<String> =
            serde_wasm_bindgen::from_value(value).map_err(|e| JsValue::from_str(&e.to_string()))?;
        let mut state = target.borrow_mut();
        let editor = state
            .editor
            .as_mut()
            .ok_or_else(|| JsValue::from_str("Scene not loaded"))?;
        editor.selection = ids.into_iter().collect();
        editor.start_move();
        paint(&mut state)
    }) as Box<dyn FnMut(JsValue) -> Result<(), JsValue>>);
    js_sys::Reflect::set(&api, &"begin".into(), begin.as_ref())?;
    begin.forget();
    let target = state.clone();
    let movement = Closure::wrap(Box::new(move |value: JsValue| -> Result<(), JsValue> {
        let delta: Point =
            serde_wasm_bindgen::from_value(value).map_err(|e| JsValue::from_str(&e.to_string()))?;
        let mut state = target.borrow_mut();
        state
            .editor
            .as_mut()
            .ok_or_else(|| JsValue::from_str("Scene not loaded"))?
            .preview_move(delta)
            .map_err(error)?;
        state.updates += 1;
        paint(&mut state)
    }) as Box<dyn FnMut(JsValue) -> Result<(), JsValue>>);
    js_sys::Reflect::set(&api, &"movement".into(), movement.as_ref())?;
    movement.forget();
    let target = state.clone();
    let end = Closure::wrap(Box::new(move || -> Result<(), JsValue> {
        let mut state = target.borrow_mut();
        state
            .editor
            .as_mut()
            .ok_or_else(|| JsValue::from_str("Scene not loaded"))?
            .commit_move(0)
            .map_err(error)?;
        paint(&mut state)
    }) as Box<dyn FnMut() -> Result<(), JsValue>>);
    js_sys::Reflect::set(&api, &"end".into(), end.as_ref())?;
    end.forget();
    let target = state;
    let get_stats =
        Closure::wrap(
            Box::new(move || -> Result<JsValue, JsValue> { stats(&target.borrow()) })
                as Box<dyn FnMut() -> Result<JsValue, JsValue>>,
        );
    js_sys::Reflect::set(&api, &"stats".into(), get_stats.as_ref())?;
    get_stats.forget();
    let ready = Closure::wrap(Box::new(|| true) as Box<dyn FnMut() -> bool>);
    js_sys::Reflect::set(&api, &"ready".into(), ready.as_ref())?;
    ready.forget();
    js_sys::Reflect::set(&window, &"__boardBench".into(), &api)?;
    Ok(())
}
fn error(error: BoardError) -> JsValue {
    JsValue::from_str(&error.to_string())
}
fn paint(state: &mut State) -> Result<(), JsValue> {
    let State { editor, canvas, .. } = state;
    if let Some(editor) = editor {
        canvas.draw(editor)?;
    }
    Ok(())
}
fn stats(state: &State) -> Result<JsValue, JsValue> {
    let editor = state
        .editor
        .as_ref()
        .ok_or_else(|| JsValue::from_str("Scene not loaded"))?;
    let doc = editor.document();
    let viewport = editor
        .camera
        .viewport(state.canvas.width, state.canvas.height);
    let value = serde_json::json!({"renderer":"bruhs-rust-canvas","shapes":doc.shapes.len(),"bindings":doc.shapes.values().filter_map(|shape|if let ShapeKind::Arrow{start,end}=&shape.kind{Some(usize::from(start.binding.is_some())+usize::from(end.binding.is_some()))}else{None}).sum::<usize>(),"updates":state.updates,"camera":editor.camera,"viewport":viewport,"draws":state.canvas.draws,"painted":state.canvas.painted,"visible":editor.ordered().filter(|shape|bounds(doc,shape).intersects(viewport)).count(),"arrows":editor.ordered().filter_map(|shape|if let ShapeKind::Arrow{start,end}=&shape.kind{Some(serde_json::json!({"id":shape.id,"start":display_endpoint(editor,shape,start),"end":display_endpoint(editor,shape,end)}))}else{None}).collect::<Vec<_>>(),"geometry":editor.ordered().filter(|shape|!matches!(shape.kind,ShapeKind::Arrow{..})).map(|shape|serde_json::json!({"id":shape.id,"x":shape.rect.x,"y":shape.rect.y})).collect::<Vec<_>>()});
    js_sys::JSON::parse(&value.to_string())
}
