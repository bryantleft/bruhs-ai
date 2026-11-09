use wasm_bindgen::JsCast;

/// Browser selection offsets count UTF-16 code units, not UTF-8 bytes.
/// A position inside a surrogate pair is rounded down to a character boundary.
pub(super) fn byte_offset(text: &str, utf16_offset: usize) -> usize {
    let mut units = 0;
    for (byte, ch) in text.char_indices() {
        units += ch.len_utf16();
        if units > utf16_offset {
            return byte;
        }
    }
    text.len()
}

pub(super) fn utf16_offset(text: &str, byte: usize) -> u32 {
    let mut byte = byte.min(text.len());
    while byte > 0 && !text.is_char_boundary(byte) {
        byte -= 1;
    }
    u32::try_from(text[..byte].encode_utf16().count()).unwrap_or(u32::MAX)
}

pub(super) fn selection_byte_offset(textarea: &web_sys::HtmlTextAreaElement, text: &str) -> usize {
    textarea
        .selection_start()
        .ok()
        .flatten()
        .map_or(text.len(), |offset| byte_offset(text, offset as usize))
}

/// `(on the first visual line, on the last visual line)`, counting wraps.
/// A selection returns `None`. Measurement falls back to logical lines.
pub(super) fn visual_line_edges(textarea: &web_sys::HtmlTextAreaElement) -> Option<(bool, bool)> {
    let value = textarea.value();
    let start = textarea.selection_start().ok().flatten()?;
    let end = textarea.selection_end().ok().flatten()?;
    if start != end {
        return None;
    }
    if value.is_empty() {
        return Some((true, true));
    }
    measure_visual_edges(textarea, &value, start).or_else(|| {
        bruhs_models::logic::recall::logical_line_edges(
            &value,
            byte_offset(&value, start as usize),
            byte_offset(&value, start as usize),
        )
    })
}

fn measure_visual_edges(
    textarea: &web_sys::HtmlTextAreaElement,
    value: &str,
    utf16_caret: u32,
) -> Option<(bool, bool)> {
    let window = web_sys::window()?;
    let document = window.document()?;
    let body = document.body()?;
    let width = textarea.client_width();
    if width <= 0 {
        return None;
    }
    let computed = window.get_computed_style(textarea).ok().flatten()?;
    let mirror = document.create_element("div").ok()?;
    let mirror_el = mirror.dyn_ref::<web_sys::HtmlElement>()?;
    let style = mirror_el.style();
    for name in [
        "font-family",
        "font-size",
        "font-weight",
        "font-style",
        "letter-spacing",
        "line-height",
        "padding-top",
        "padding-right",
        "padding-bottom",
        "padding-left",
        "border-top-width",
        "border-right-width",
        "border-bottom-width",
        "border-left-width",
        "box-sizing",
        "white-space",
        "overflow-wrap",
        "word-break",
        "tab-size",
    ] {
        if let Ok(property) = computed.get_property_value(name) {
            let _ = style.set_property(name, &property);
        }
    }
    let _ = style.set_property("position", "absolute");
    let _ = style.set_property("visibility", "hidden");
    let _ = style.set_property("left", "0");
    let _ = style.set_property("top", "0");
    let _ = style.set_property("height", "auto");
    let _ = style.set_property("overflow", "hidden");
    let _ = style.set_property("width", &format!("{width}px"));
    if style.get_property_value("white-space").ok().as_deref() == Some("") {
        let _ = style.set_property("white-space", "pre-wrap");
    }

    let byte = byte_offset(value, utf16_caret as usize);
    let before = document.create_text_node(&value[..byte]);
    let after = document.create_text_node(&value[byte..]);
    let mark = document.create_element("span").ok()?;
    mark.set_text_content(Some("\u{200b}"));
    mirror.append_child(&before).ok()?;
    mirror.append_child(&mark).ok()?;
    mirror.append_child(&after).ok()?;
    body.append_child(&mirror).ok()?;

    let mirror_box = mirror_el.get_bounding_client_rect();
    let mark_box = mark.get_bounding_client_rect();
    let _ = body.remove_child(&mirror);
    let line = line_height_px(&computed).unwrap_or(16.0);
    // Padding and border sit above the first line. Comparing the marker to
    // the border box made a caret on that line look like it had wrapped.
    let inset_top = css_px(&computed, "border-top-width") + css_px(&computed, "padding-top");
    let inset_bottom =
        css_px(&computed, "border-bottom-width") + css_px(&computed, "padding-bottom");
    let content_delta = mark_box.top() - mirror_box.top() - inset_top;
    let content_height = mirror_box.height() - inset_top - inset_bottom;
    if content_height <= 0.0 || !content_delta.is_finite() {
        return None;
    }
    let on_first = content_delta < line * 0.5;
    let on_last = content_delta + line > content_height - 1.0;
    Some((on_first, on_last))
}

fn css_px(style: &web_sys::CssStyleDeclaration, name: &str) -> f64 {
    style
        .get_property_value(name)
        .ok()
        .and_then(|raw| raw.trim().trim_end_matches("px").parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value >= 0.0)
        .unwrap_or(0.0)
}

fn line_height_px(style: &web_sys::CssStyleDeclaration) -> Option<f64> {
    let raw = style.get_property_value("line-height").ok()?;
    let pixels = raw.trim_end_matches("px").parse::<f64>().ok()?;
    (pixels > 0.0).then_some(pixels)
}

#[cfg(test)]
mod tests {
    use super::byte_offset;

    #[test]
    fn ascii_offsets_are_unchanged() {
        assert_eq!(byte_offset("hi @src", 7), 7);
    }

    #[test]
    fn accented_character_uses_two_utf8_bytes() {
        assert_eq!(byte_offset("éx", 1), 2);
    }

    #[test]
    fn cjk_character_uses_three_utf8_bytes() {
        assert_eq!(byte_offset("界x", 1), 3);
    }

    #[test]
    fn astral_character_uses_two_utf16_units() {
        assert_eq!(byte_offset("🙂x", 2), 4);
    }

    #[test]
    fn partial_surrogate_pair_rounds_down() {
        assert_eq!(byte_offset("a🙂x", 2), 1);
    }

    #[test]
    fn combining_marks_remain_independently_addressable() {
        assert_eq!(byte_offset("e\u{301}x", 2), 3);
    }

    #[test]
    fn mixed_text_offsets_round_trip_at_every_character_boundary() {
        let text = "é界🙂 e\u{301}\n@src";
        for byte in text
            .char_indices()
            .map(|(byte, _)| byte)
            .chain([text.len()])
        {
            assert_eq!(byte_offset(text, text[..byte].encode_utf16().count()), byte);
        }
    }

    #[test]
    fn beyond_end_is_clamped_to_valid_boundary() {
        assert_eq!(byte_offset("🙂", usize::MAX), 4);
    }

    #[test]
    fn empty_text_has_only_zero_offset() {
        assert_eq!(byte_offset("", 42), 0);
    }
}
