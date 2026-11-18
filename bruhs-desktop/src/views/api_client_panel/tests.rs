use super::{set, value, ApiAttachment, ApiClientPanel};
use crate::{server_client::ServerClient, theme};
use bruhs_models::api_client::*;
use bruhs_ui::zoom::px;
use gpui::{point, size, AppContext, Modifiers, TestAppContext, VisualTestContext};
use std::{cell::RefCell, rc::Rc};
use uuid::Uuid;

#[gpui::test]
fn api_client_native_controls_import_tree_and_unsent_attachment(cx: &mut TestAppContext) {
    cx.update(|cx| {
        bruhs_ui::init(cx);
        theme::apply_bruhs_theme(cx);
        cx.set_global(crate::app_state::DesktopState::new().unwrap());
    });
    let holder = Rc::new(RefCell::new(None));
    let capture = Rc::clone(&holder);
    let (_, cx) = cx.add_window_view(|window, cx| {
        let client = ServerClient::new("http://127.0.0.1:0", "http://127.0.0.1:0").unwrap();
        let panel = cx.new(|cx| {
            ApiClientPanel::with_client("/synthetic/api-project".into(), client, window, cx)
        });
        *capture.borrow_mut() = Some(panel.clone());
        bruhs_ui::Root::new(panel, window, cx)
    });
    cx.simulate_resize(size(px(640.0), px(2600.0)));
    let panel = holder.borrow().clone().unwrap();
    let attached = Rc::new(RefCell::new(Vec::<ApiAttachment>::new()));
    let capture = Rc::clone(&attached);
    let subscription = cx.update(|_, cx| {
        cx.subscribe(&panel, move |_, event: &ApiAttachment, _| {
            capture.borrow_mut().push(event.clone())
        })
    });
    panel.update_in(cx, |panel, window, cx| {
        panel.generation = panel.generation.wrapping_add(1);
        panel.busy = false;
        panel.error = None;
        set(
            &panel.curl,
            "curl 'https://fixture.invalid/graphql' --json '{\"query\":\"{ viewer { name } }\"}'",
            window,
            cx,
        );
        cx.notify();
    });
    cx.run_until_parked();
    click(cx, "api-client-Import into draft");
    panel.update_in(cx, |panel, _, cx| {
        assert_eq!(value(&panel.method, cx), "POST");
        assert_eq!(value(&panel.url, cx), "https://fixture.invalid/graphql");
        assert!(value(&panel.body, cx).contains("viewer"));
        assert!(value(&panel.headers, cx).contains("application/json"));
        assert_eq!(value(&panel.curl, cx), "");
        panel.response = Some(ApiResponse {
            request_id: Uuid::new_v4(),
            status: 201,
            elapsed_ms: 3,
            size_bytes: 40,
            headers: vec![ApiHeader {
                name: "content-type".into(),
                value: "application/json".into(),
            }],
            body: r#"{"data":{"name":"Ada"},"html":"<img src=x>"}"#.into(),
            redacted: false,
        });
        cx.notify();
    });
    cx.run_until_parked();
    assert!(attached.borrow().is_empty());
    click(cx, "api-client-▸ data: {1 keys}");
    panel.update_in(cx, |panel, _, _| assert!(panel.expanded.contains("/data")));
    click(cx, "api-client-Headers");
    panel.update_in(cx, |panel, _, _| assert_eq!(panel.response_tab, 2));
    click(cx, "api-client-Add response to draft");
    assert_eq!(attached.borrow().len(), 1);
    assert!(attached.borrow()[0].text.contains("Ada"));
    assert_eq!(attached.borrow()[0].project_path, "/synthetic/api-project");
    panel.update_in(cx, |panel, window, cx| {
        let bad = "curl https://fixture.invalid/ --config /tmp/never-read";
        set(&panel.curl, bad, window, cx);
        cx.notify();
    });
    click(cx, "api-client-Import into draft");
    panel.update_in(cx, |panel, _, cx| {
        assert!(panel.error.as_deref().unwrap().contains("Unsupported curl"));
        assert!(value(&panel.curl, cx).contains("--config"));
        assert_eq!(value(&panel.url, cx), "https://fixture.invalid/graphql");
    });
    panel.update_in(cx, |panel, window, cx| {
        let method = value(&panel.method, cx);
        let body = value(&panel.body, cx);
        let generation = panel.generation;
        set(
            &panel.credential_value,
            "synthetic-private-input",
            window,
            cx,
        );
        panel.active = Some(Uuid::new_v4());
        panel.visible = true;
        panel.set_visible(false, window, cx);
        assert!(panel.active.is_none());
        assert!(panel.generation > generation);
        assert_eq!(value(&panel.credential_value, cx), "");
        assert_eq!(value(&panel.method, cx), method);
        assert_eq!(value(&panel.body, cx), body);
        panel.set_visible(true, window, cx);
    });
    drop(subscription);
}
fn click(cx: &mut VisualTestContext, selector: &'static str) {
    cx.run_until_parked();
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("Missing native control {selector}"));
    cx.simulate_click(
        point(bounds.center().x, bounds.center().y),
        Modifiers::default(),
    );
    cx.run_until_parked();
}
