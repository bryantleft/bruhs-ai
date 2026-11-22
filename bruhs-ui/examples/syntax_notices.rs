//! Print the syntax definitions' notices for the packaged desktop app.
fn main() {
    let listing = two_face::acknowledgement::listing();
    if std::env::args().nth(1).as_deref() == Some("--json") {
        println!(
            "{}",
            serde_json::to_string(listing.for_syntaxes()).expect("syntax notices")
        );
        return;
    }
    let mut notices = String::new();
    for license in listing.for_syntaxes() {
        license.write_md(&mut notices);
    }
    print!("{notices}");
}
