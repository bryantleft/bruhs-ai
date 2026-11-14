//! Auth injection is installed only by the explicit, temporary native startup fixture.
use gpui::{App, Global};
use std::cell::{Cell, RefCell};

type DeliveryCallback = Box<dyn FnOnce(&mut App)>;

pub(super) struct AuthFixture {
    pub(super) program: String,
    pub(super) original: bool,
    pub(super) expected: usize,
    pub(super) requested: Cell<usize>,
    pub(super) started: Cell<usize>,
    pub(super) delivered: Cell<usize>,
    pub(super) on_delivered: RefCell<Option<DeliveryCallback>>,
}

impl Global for AuthFixture {}

pub(crate) fn auth_fixture_program(cx: &App) -> Option<String> {
    let fixture = cx.try_global::<AuthFixture>()?;
    fixture.requested.set(fixture.requested.get() + 1);
    Some(fixture.program.clone())
}

pub(crate) fn auth_fixture_original(cx: &App) -> bool {
    cx.try_global::<AuthFixture>()
        .is_some_and(|fixture| fixture.original)
}

pub(crate) fn auth_probe_started(cx: &App) {
    if let Some(fixture) = cx.try_global::<AuthFixture>() {
        fixture.started.set(fixture.started.get() + 1);
    }
}

pub(crate) fn auth_status_delivered(cx: &mut App) {
    let callback = {
        let Some(fixture) = cx.try_global::<AuthFixture>() else {
            return;
        };
        let delivered = fixture.delivered.get() + 1;
        assert!(
            delivered <= fixture.expected,
            "unexpected native fixture auth delivery"
        );
        fixture.delivered.set(delivered);
        if delivered == fixture.expected {
            Some(
                fixture
                    .on_delivered
                    .borrow_mut()
                    .take()
                    .expect("one auth completion callback"),
            )
        } else {
            None
        }
    };
    if let Some(callback) = callback {
        callback(cx);
    }
}
