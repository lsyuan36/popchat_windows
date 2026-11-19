//! Match displayed text and return byte ranges in the original UTF-8 string.
use std::ops::Range;

// Lowercasing can expand one scalar. Folded byte offsets must never be used
// directly to slice the user's text.
fn folded_with_ranges(text: &str) -> (String, Vec<Range<usize>>) {
    use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};
    let mut folded = String::new();
    let mut ranges = Vec::new();
    for (offset, character) in text.char_indices() {
        for decomposed in character
            .to_string()
            .nfd()
            .filter(|c| !is_combining_mark(*c))
        {
            for lower in decomposed.to_lowercase() {
                folded.push(lower);
                ranges.extend(std::iter::repeat_n(
                    offset..offset + character.len_utf8(),
                    lower.len_utf8(),
                ));
            }
        }
    }
    (folded, ranges)
}

pub fn find_ranges(text: &str, query: &str) -> Vec<Range<usize>> {
    let (needle, _) = folded_with_ranges(query);
    if needle.is_empty() {
        return Vec::new();
    }
    let (haystack, mapping) = folded_with_ranges(text);
    let mut visible: Vec<Range<usize>> = Vec::new();
    for (start, matched) in haystack.match_indices(&needle) {
        let range = mapping[start].start..mapping[start + matched.len() - 1].end;
        // Distinct folded matches can share a source character (e.g. jamo on
        // either side of a Hangul syllable). Highlight that visible span once;
        // overlapping ranges cannot be used to slice a LayoutJob in order.
        if let Some(last) = visible.last_mut().filter(|last| range.start < last.end) {
            last.end = last.end.max(range.end);
        } else {
            visible.push(range);
        }
    }
    visible
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folded_matches_that_share_one_displayed_character_form_one_visible_hit() {
        let text = "\u{1161}가\u{1100}"; // NFD: medial, leading, medial, leading
        assert_eq!(find_ranges(text, "\u{1161}\u{1100}"), vec![0..text.len()]);
        // Adjacent but independently selectable words still count separately.
        assert_eq!(find_ranges("éÉ", "e"), vec![0..2, 2..4]);
    }
}
