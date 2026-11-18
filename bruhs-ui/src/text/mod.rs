//! Rich text: Markdown (and an HTML subset) drawn as gpui elements, with a
//! parsed state that follows streamed text and a mouse selection.
//!
//! - [`TextView`] draws a document; [`TextViewStyle`] sets its look (the
//!   web's chat prose by default).
//! - [`TextViewState`] holds the source, the parsed [`document`] and the
//!   selection; [`TextSelection::selected_text`] reads the app's selection.
//! - Call [`init`] (or [`crate::init`]) for ⌘C on selected text.

pub mod document;
mod state;
mod style;
mod view;

pub use document::Link;
pub use state::{init, CopySelection, TextFormat, TextPosition, TextSelection, TextViewState};
pub use style::TextViewStyle;
pub use view::TextView;
