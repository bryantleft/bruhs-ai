//! Include native WebKit controls beside GPUI's virtual accessibility tree.
//!
//! AccessKit subclasses the window content view and supplies its virtual
//! children. A native child WKWebView would otherwise be visually present but
//! absent from that tree. Keep the window's existing children and add only the
//! visible embedded views, restoring every override on hide/drop.

use objc2::{rc::Retained, runtime::AnyObject};
use objc2_app_kit::{NSAccessibility, NSView, NSWindow};
use objc2_foundation::NSArray;
use std::{cell::RefCell, collections::HashMap, ptr::NonNull};
use wry::{WebView, WebViewExtMacOS};

struct ViewEntry {
    view: Retained<NSView>,
    original_parent: Option<Retained<AnyObject>>,
}
struct WindowEntry {
    window: Retained<NSWindow>,
    original_children: Option<Retained<NSArray>>,
    views: HashMap<NonNull<NSView>, ViewEntry>,
}
thread_local! {static WINDOWS:RefCell<HashMap<NonNull<NSWindow>,WindowEntry>>=RefCell::new(HashMap::new());}

impl WindowEntry {
    fn publish(&self) {
        let originals: Vec<Retained<AnyObject>> = self
            .original_children
            .as_ref()
            .into_iter()
            .flat_map(|array| array.iter())
            .collect();
        let mut children: Vec<&AnyObject> = originals.iter().map(std::ops::Deref::deref).collect();
        for entry in self.views.values() {
            children.push(&entry.view);
        }
        let children = NSArray::from_slice(&children);
        // SAFETY: the original children and each NSView are retained AppKit
        // accessibility objects. This host runs only on the UI thread, and
        // NSWindow copies the array before these local references go away.
        unsafe {
            self.window.setAccessibilityChildren(Some(&children));
        }
    }
}

/// Main-thread AppKit references make this guard non-Send. Several visible
/// previews in one window share the override rather than restoring over peers.
pub(super) struct NativeAccessibility {
    window: NonNull<NSWindow>,
    view: NonNull<NSView>,
}
impl NativeAccessibility {
    pub(super) fn new(webview: &WebView) -> Self {
        let window = webview.ns_window();
        let view: Retained<NSView> = webview.webview().into_super().into_super();
        let key = NonNull::from(window.as_ref());
        let view_key = NonNull::from(view.as_ref());
        WINDOWS.with(|windows| {
            let mut windows = windows.borrow_mut();
            let entry = windows.entry(key).or_insert_with(|| WindowEntry {
                original_children: window.accessibilityChildren(),
                window,
                views: HashMap::new(),
            });
            let original_parent = view.accessibilityParent();
            // SAFETY: this visible NSView is now an explicit child of the live,
            // retained NSWindow. Its original parent is retained for restore.
            unsafe {
                view.setAccessibilityParent(Some(&entry.window));
            }
            entry.views.insert(
                view_key,
                ViewEntry {
                    view,
                    original_parent,
                },
            );
            entry.publish();
        });
        Self {
            window: key,
            view: view_key,
        }
    }
}
impl Drop for NativeAccessibility {
    fn drop(&mut self) {
        WINDOWS.with(|windows| {
            let mut windows = windows.borrow_mut();
            let Some(entry) = windows.get_mut(&self.window) else {
                return;
            };
            if let Some(view) = entry.views.remove(&self.view) {
                // SAFETY: both the NSView and its saved AppKit parent remain
                // retained until after the original relationship is restored.
                unsafe {
                    view.view
                        .setAccessibilityParent(view.original_parent.as_deref());
                }
            }
            if entry.views.is_empty() {
                // SAFETY: restore exactly the saved, retained accessibility
                // object array on the still-retained main-thread NSWindow.
                unsafe {
                    entry
                        .window
                        .setAccessibilityChildren(entry.original_children.as_deref());
                }
                windows.remove(&self.window);
            } else {
                entry.publish();
            }
        });
    }
}

/// The first responder may be WebKit's internal NSView rather than the outer
/// WKWebView. Check the complete visible subtree before routing native editing.
pub(super) fn has_focused_view() -> bool {
    WINDOWS.with(|windows| {
        windows.borrow().values().any(|entry| {
            entry.window.isKeyWindow()
                && entry.window.firstResponder().is_some_and(|responder| {
                    let object: &AnyObject = &responder;
                    object.downcast_ref::<NSView>().is_some_and(|view| {
                        entry
                            .views
                            .values()
                            .any(|entry| view.isDescendantOf(&entry.view))
                    })
                })
        })
    })
}
