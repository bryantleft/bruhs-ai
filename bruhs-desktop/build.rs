include!("../bruhs-core/build_info.rs");

fn main() {
    emit_build_info();
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rerun-if-changed=src/snapshot_text.m");
        println!("cargo:rerun-if-changed=src/snapshot_shortcut.m");
        println!("cargo:rustc-link-lib=framework=Carbon");
        println!("cargo:rerun-if-changed=src/snapshot_window.m");
        println!("cargo:rustc-link-lib=framework=ApplicationServices");
        println!("cargo:rustc-link-lib=framework=QuartzCore");
        cc::Build::new()
            .file("src/snapshot_window.m")
            .file("src/snapshot_text.m")
            .file("src/snapshot_shortcut.m")
            .flag("-fobjc-arc")
            .compile("snapshot_text");
        println!("cargo:rustc-link-lib=framework=Vision");
        println!("cargo:rustc-link-lib=framework=Foundation");
    }
}
