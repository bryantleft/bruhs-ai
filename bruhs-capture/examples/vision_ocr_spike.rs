//! Spike: prove cidre + Vision framework can OCR a static image.
//!
//! Run from `bruhs-capture/`:
//!   `cargo run --example vision_ocr_spike <path_to_image>`
//!
//! Loads a JPEG/PNG from disk, runs `VNRecognizeTextRequest` with
//! `.accurate` recognition level + language correction, and prints each
//! detected text observation as:
//!
//!   <text>\t<x>,<y>,<w>,<h>\tconf=<f32>
//!
//! Bounding boxes are normalized [0..1] in the image's coordinate space
//! (origin = bottom-left, Vision convention).

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("Vision OCR spike is macOS-only.");
    std::process::exit(1);
}

#[cfg(target_os = "macos")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use cidre::{ns, vn};

    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: vision_ocr_spike <path_to_image>");
        std::process::exit(2);
    }
    let image_path = &args[1];

    // Build a file:// URL for the image. `with_fs_path_str` handles path
    // escaping correctly (spaces, unicode, etc.).
    let url = ns::Url::with_fs_path_str(image_path, false);

    // ImageRequestHandler retains the image source for its lifetime and
    // never modifies it. URL-based construction is the simplest path —
    // Vision decodes the file itself.
    let handler = vn::ImageRequestHandler::with_url(&url, None);

    // Configure the OCR request: accurate + language correction + English.
    // (Auto-detect would also work on macOS 13+, but explicit is clearer.)
    let mut request = vn::RecognizeTextRequest::new();
    request.set_recognition_level(vn::RequestTextRecognitionLevel::Accurate);
    request.set_uses_lang_correction(true);
    let en = ns::String::with_str("en-US");
    let langs = ns::Array::<ns::String>::from_slice(&[&en]);
    request.set_recognition_langs(&langs);

    // Perform synchronously. `&request` of type `&arc::R<RecognizeTextRequest>`
    // auto-derefs through `RecognizeTextRequest -> ImageBasedRequest -> Request`
    // in the slice's known element type.
    let requests = ns::Array::<vn::Request>::from_slice(&[&request]);
    if let Err(err) = handler.perform(&requests) {
        eprintln!(
            "VNImageRequestHandler.perform failed: domain={:?} code={}",
            err.domain(),
            err.code() as i64,
        );
        std::process::exit(1);
    }

    // Collect + print observations. `results()` is `Option<NSArray>` —
    // None means the request wasn't executed (shouldn't happen after a
    // successful `perform`); empty means no text detected.
    let Some(results) = request.results() else {
        eprintln!("No results returned by VNRecognizeTextRequest.");
        return Ok(());
    };

    let count = results.len();
    eprintln!("Detected {} text observation(s) in {}", count, image_path);
    eprintln!("Format: <text>\\t<x>,<y>,<w>,<h>\\tconf=<f32>");
    eprintln!("---");

    for i in 0..count {
        let obs = match results.get(i) {
            Ok(o) => o,
            Err(_) => continue,
        };

        // Top candidate string + its own confidence (per-candidate score,
        // distinct from the observation-level confidence). We print the
        // candidate's confidence since it directly corresponds to the
        // string we're emitting.
        let candidates = obs.top_candidates(1);
        let Ok(top) = candidates.get(0) else {
            continue;
        };
        let text = top.string().to_string();
        let conf = top.confidence();

        let bbox = obs.bounding_box();
        // Vision normalized rect: origin bottom-left, x/y/w/h all [0..1].
        println!(
            "{}\t{:.4},{:.4},{:.4},{:.4}\tconf={:.3}",
            text, bbox.origin.x, bbox.origin.y, bbox.size.width, bbox.size.height, conf,
        );
    }

    Ok(())
}
