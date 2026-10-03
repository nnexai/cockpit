/// Plain text for widget chrome and choices, bounded in Unicode scalar values.
pub fn sanitize(input: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let mut output = String::with_capacity(input.len().min(max_chars.saturating_mul(4)));
    let mut count = 0;
    let mut space = false;
    for ch in input.chars() {
        if ch.is_whitespace() {
            space = !output.is_empty();
            continue;
        }
        if ch.is_control() || matches!(ch, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') {
            continue;
        }
        if space {
            if count == max_chars {
                output.pop();
                output.push('…');
                return output;
            }
            output.push(' ');
            count += 1;
            space = false;
        }
        if count == max_chars {
            output.pop();
            output.push('…');
            return output;
        }
        output.push(ch);
        count += 1;
    }
    output
}

/// Widget and choice ids are short, lowercase ASCII slugs.
pub fn valid_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    (1..=48).contains(&bytes.len())
        && (bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit())
        && bytes
            .iter()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, b'_' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_controls_and_bidi_without_losing_word_boundaries() {
        assert_eq!(
            sanitize(
                " \tHello\n\r world\u{0000}\u{0080}\u{202a} safe\u{202e}\u{2066}!\u{2069} ",
                80
            ),
            "Hello world safe!"
        );
        assert_eq!(
            sanitize("a\u{202b}\u{202c}\u{202d}\u{2067}\u{2068}b", 80),
            "ab"
        );
    }

    #[test]
    fn truncates_unicode_and_counts_the_ellipsis_in_the_limit() {
        let text = sanitize(&"界".repeat(81), 80);
        assert_eq!(text, format!("{}…", "界".repeat(79)));
        assert_eq!(sanitize(&"界".repeat(80), 80), "界".repeat(80));
        assert_eq!(sanitize("abc", 1), "…");
        assert_eq!(sanitize("abc", 0), "");
        assert_eq!(sanitize("ab  ", 2), "ab");
        assert_eq!(sanitize("ab c", 3), "ab…");
    }

    #[test]
    fn validates_slug_boundaries() {
        for valid in ["a", "0", "view_1-b", &"a".repeat(48)] {
            assert!(valid_id(valid), "{valid}");
        }
        for invalid in ["", "-a", "_a", "A", "é", "a.b", "a b", &"a".repeat(49)] {
            assert!(!valid_id(invalid), "{invalid}");
        }
    }
}
