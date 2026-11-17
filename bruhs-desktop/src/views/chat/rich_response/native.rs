use super::*;
use std::cell::RefCell;

#[cfg(any(target_os = "linux", all(test, unix)))]
mod event_waiter;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "windows")]
mod windows;

#[derive(Default)]
pub(super) struct NativeEmbed {
    view: Option<wry::WebView>,
    geometry: Option<(Bounds<Pixels>, Bounds<Pixels>)>,
    visible: bool,
    loaded: Rc<std::cell::Cell<bool>>,
    theme: std::collections::BTreeMap<String, String>,
    scale_factor: f32,
    #[cfg(target_os = "windows")]
    pending: Option<windows::PendingWebView>,
    #[cfg(target_os = "linux")]
    linux_runtime: Option<linux::GtkRuntimeLease>,
    #[cfg(target_os = "linux")]
    linux_child: Option<linux::GtkChild>,
}

impl NativeEmbed {
    pub(super) fn available() -> bool {
        crate::webview_platform::inline_unavailable_reason().is_none()
    }

    pub(super) fn clear(&mut self) {
        #[cfg(target_os = "windows")]
        {
            self.pending = None;
        }
        if let Some(view) = self.view.take() {
            drop(view);
        }
        #[cfg(target_os = "linux")]
        {
            self.linux_child = None;
            self.linux_runtime = None;
        }
        self.geometry = None;
        self.visible = false;
        self.loaded.set(false);
    }

    pub(super) fn hide(&mut self) {
        if self.visible {
            if let Some(view) = &self.view {
                if let Err(error) = view.set_visible(false) {
                    tracing::warn!(
                        "could not hide an inline response; removing its native view: {error}"
                    );
                    self.clear();
                    return;
                }
            }
            self.visible = false;
            #[cfg(target_os = "linux")]
            if let Some(child) = &self.linux_child {
                child.hide();
            }
        }
    }

    pub(super) fn place(
        &mut self,
        shell: &str,
        body: &str,
        frame_url: Option<String>,
        geometry: (Bounds<Pixels>, Bounds<Pixels>),
        widget: HostWidget,
        context: PlacementContext<'_>,
    ) -> Result<(), String> {
        let (full, visible) = geometry;
        let update = geometry_script(full, visible);
        let current_theme = widget.theme.clone();
        #[cfg(target_os = "windows")]
        if let Some(pending) = &self.pending {
            let Some(result) = pending.take_ready() else {
                return Ok(());
            };
            self.pending = None;
            self.view = Some(result?);
        }
        if self.view.is_none() {
            let markup = serde_json::to_string(body).map_err(|error| error.to_string())?;
            let script = widget.html.as_deref().map_or_else(String::new, |html| {
                interactive_host_script(html, &widget.nonce)
            });
            let init = serde_json::json!({"bruhs":1,"type":"init","state":widget.state,"theme":widget.theme});
            let initialization = format!(
                "addEventListener('DOMContentLoaded',()=>{{document.body.innerHTML='<div id=\"bruhs-rich-root\"></div>';document.documentElement.style.cssText='margin:0;background:transparent;overflow:hidden';document.body.style.cssText='margin:0;overflow:hidden';document.getElementById('bruhs-rich-root').innerHTML={markup};{script};if(window.bruhsHostInit)window.bruhsHostInit({init});{update};for(const media of document.querySelectorAll('img,video,audio'))media.addEventListener('error',()=>{{const error=document.createElement('p');error.textContent='This media could not be loaded. Check its source or format.';error.style.color='#e9a89c';media.replaceWith(error)}},{{once:true}})}})"
            );
            #[cfg(debug_assertions)]
            let initialization = if std::env::var("BRUHS_DESKTOP_SMOKE_SCENE")
                .is_ok_and(|scene| scene.starts_with("transcript-rich"))
            {
                format!(
                    "addEventListener('message',event=>{{if(event.source===document.querySelector('iframe')?.contentWindow&&event.data?.tag==='bruhs-rich-response-smoke')window.__bruhsRichSmoke=event.data}});{initialization}"
                )
            } else {
                initialization
            };
            let initialization = trusted_initialization(shell, &initialization)?;
            let shell_url = shell.to_string();
            let loaded = Rc::new(std::cell::Cell::new(false));
            self.loaded = Rc::clone(&loaded);
            let app = RefCell::new(context.app.to_async());
            let entity = context.entity;
            let ipc_app = RefCell::new(context.app.to_async());
            let ipc_chat = context.chat;
            let ipc_shell = shell.to_string();
            let nonce = widget.nonce;
            let key = widget.key;
            let builder = wry::WebViewBuilder::new()
                .with_url(shell)
                .with_visible(false)
                .with_on_page_load_handler(move |event, _| {
                    if matches!(event, wry::PageLoadEvent::Finished) {
                        loaded.set(true);
                        app.borrow_mut().update(|cx| cx.notify(entity));
                    }
                })
                .with_initialization_script(initialization)
                .with_ipc_handler(move |request| {
                    if request.uri().to_string() != ipc_shell {
                        return;
                    }
                    let _ = ipc_chat.update(&mut *ipc_app.borrow_mut(), |this, cx| {
                        this.receive_widget_message(&key, &nonce, request.body(), cx)
                    });
                })
                .with_bounds(native_rect(visible, context.window))
                .with_background_color((0x14, 0x12, 0x10, 0xff))
                .with_incognito(true)
                .with_clipboard(false)
                .with_devtools(false)
                .with_navigation_handler(move |url| {
                    url == shell_url
                        || url == "about:blank"
                        || url == "about:srcdoc"
                        || frame_url.as_deref() == Some(url.as_str())
                })
                .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
                .with_download_started_handler(|_, _| false);
            #[cfg(target_os = "linux")]
            let runtime = linux::GtkRuntimeLease::acquire(context.app)?;
            #[cfg(target_os = "linux")]
            let (view, child) = linux::build_child(builder, context.window)?;
            #[cfg(target_os = "windows")]
            {
                self.pending = Some(windows::PendingWebView::start(
                    builder,
                    context.window,
                    context.app,
                    context.entity,
                )?);
                return Ok(());
            }
            #[cfg(not(any(target_os = "linux", target_os = "windows")))]
            let view = builder
                .build_as_child(context.window)
                .map_err(platform_error)?;
            #[cfg(target_os = "linux")]
            {
                child.resize(
                    &view,
                    native_rect(visible, context.window),
                    context.window.scale_factor(),
                )?;
                self.linux_child = Some(child);
                self.linux_runtime = Some(runtime);
            }
            #[cfg(not(target_os = "windows"))]
            {
                // The page zooms with the window.
                view.zoom(f64::from(context.window.zoom()))
                    .map_err(|error| error.to_string())?;
                self.view = Some(view);
            }
        } else if self.geometry != Some(geometry)
            || self.scale_factor != context.window.scale_factor()
        {
            if let Some(view) = &self.view {
                #[cfg(not(target_os = "linux"))]
                view.set_bounds(native_rect(visible, context.window))
                    .map_err(|error| error.to_string())?;
                #[cfg(target_os = "linux")]
                if let Some(child) = &self.linux_child {
                    child.resize(
                        view,
                        native_rect(visible, context.window),
                        context.window.scale_factor(),
                    )?;
                }
                view.zoom(f64::from(context.window.zoom()))
                    .map_err(|error| error.to_string())?;
                view.evaluate_script(&update)
                    .map_err(|error| error.to_string())?;
            }
        }
        #[cfg(target_os = "linux")]
        if let Some(runtime) = &self.linux_runtime {
            runtime.check()?;
        }
        if self.theme != current_theme {
            if let Some(view) = &self.view {
                let init = serde_json::json!({"bruhs":1,"type":"init","state":widget.state,"theme":current_theme});
                view.evaluate_script(&format!("window.bruhsHostInit?.({init})"))
                    .map_err(|error| error.to_string())?;
            }
            self.theme = current_theme;
        }
        if !self.visible && self.loaded.get() {
            #[cfg(target_os = "linux")]
            if let Some(child) = &self.linux_child {
                child.show();
            }
            if let Some(view) = &self.view {
                view.set_visible(true).map_err(|error| error.to_string())?;
            }
            self.visible = true;
        }
        self.geometry = Some(geometry);
        self.scale_factor = context.window.scale_factor();
        Ok(())
    }

    pub(super) fn flush(&self) {
        if let Some(view) = &self.view {
            let _ = view.evaluate_script("window.bruhsHostFlush?.()");
        }
    }

    pub(super) fn is_live(&self) -> bool {
        self.view.is_some()
    }

    #[cfg(debug_assertions)]
    pub(super) fn smoke_report(&self) {
        if let Some(view) = &self.view {
            #[cfg(target_os = "linux")]
            if let (Some(child), Some((_, visible))) = (&self.linux_child, self.geometry) {
                child.smoke_report(view, visible, self.scale_factor);
            }
            let _ = view.evaluate_script_with_callback("JSON.stringify({root:!!document.getElementById('bruhs-rich-root'),sandbox:document.querySelector('iframe')?.getAttribute('sandbox'),probe:window.__bruhsRichSmoke})", |result| eprintln!("BRUHS_RICH_RESPONSE_SMOKE native={result}"));
        }
    }
}

impl Drop for NativeEmbed {
    fn drop(&mut self) {
        self.clear();
    }
}

fn native_rect(bounds: Bounds<Pixels>, window: &Window) -> wry::Rect {
    let position = wry::dpi::LogicalPosition::new(
        f64::from(f32::from(bounds.origin.x)),
        f64::from(f32::from(bounds.origin.y)),
    );
    let size = wry::dpi::LogicalSize::new(
        f64::from(f32::from(bounds.size.width)),
        f64::from(f32::from(bounds.size.height)),
    );
    #[cfg(target_os = "linux")]
    {
        wry::Rect {
            position: position
                .to_physical::<i32>(f64::from(window.scale_factor()))
                .into(),
            size: size
                .to_physical::<u32>(f64::from(window.scale_factor()))
                .into(),
        }
    }
    // Elsewhere the frame is in window points: the content's zoom times its
    // bounds. (Linux converts with the scale factor, which carries the zoom.)
    #[cfg(not(target_os = "linux"))]
    {
        let zoom = f64::from(window.zoom());
        let position = wry::dpi::LogicalPosition::new(position.x * zoom, position.y * zoom);
        let size = wry::dpi::LogicalSize::new(size.width * zoom, size.height * zoom);
        wry::Rect {
            position: position.into(),
            size: size.into(),
        }
    }
}

fn trusted_initialization(shell: &str, initialization: &str) -> Result<String, String> {
    let shell = serde_json::to_string(shell).map_err(|error| error.to_string())?;
    // WebView2 injects initialization scripts into every frame, even with
    // for_main_frame_only=true. No bridge or nonce may be installed there.
    Ok(format!(
        "(()=>{{if(window!==window.top||location.href!=={shell})return;{initialization}}})()"
    ))
}

#[cfg(not(target_os = "linux"))]
fn platform_error(error: wry::Error) -> String {
    #[cfg(target_os = "windows")]
    {
        format!(
            "{error}. Install or repair the Microsoft Edge WebView2 Runtime, then restart bruhs."
        )
    }
    #[cfg(not(target_os = "windows"))]
    {
        error.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::trusted_initialization;

    #[test]
    fn browser_initialization_requires_the_exact_trusted_main_frame() {
        let source = trusted_initialization(
            "http://127.0.0.1:3001/api/agent/rich-response",
            "secretBridge()",
        )
        .unwrap();
        let guard = source
            .find("if(window!==window.top||location.href!==")
            .unwrap();
        let bridge = source.find("secretBridge()").unwrap();
        assert!(guard < bridge);
        assert!(source.contains("/api/agent/rich-response\")return;"));
    }
}
