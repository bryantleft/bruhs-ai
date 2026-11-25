//! Print third-party syntax grammar notices for the packaged desktop app.
//!
//! `bruhs-ui` uses its own first-party Rust highlighter (`input::syntax`) with
//! no bundled third-party grammar definitions, so the notice list is empty.
fn main() {
    if std::env::args().nth(1).as_deref() == Some("--json") {
        println!("[]");
    }
}
