//! Small string helpers.

/// Cut `s` to at most `max` bytes without splitting a UTF-8 sequence.
/// Moved from bruhs-core `git.rs`.
pub fn truncate_utf8(s: &mut String, max: usize) {
    if s.len() <= max {
        return;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s.truncate(end);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_utf8_respects_char_boundaries() {
        let mut s = "héllo".to_string();
        truncate_utf8(&mut s, 2);
        assert_eq!(s, "h");
        let mut ok = "abc".to_string();
        truncate_utf8(&mut ok, 10);
        assert_eq!(ok, "abc");
        let mut crab = "🦀🦀".to_string();
        truncate_utf8(&mut crab, 5);
        assert_eq!(crab, "🦀");
        truncate_utf8(&mut crab, 0);
        assert_eq!(crab, "");
    }
}
