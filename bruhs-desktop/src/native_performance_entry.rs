// A normal binary deliberately avoids GPUI's dev-dependency test-support feature.
// The shared entry routes only this Cargo binary to the opt-in synthetic harness.
include!("main.rs");
