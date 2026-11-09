//! Indentation decorations belong to the editor, so they scroll, wrap and undo with its text.
use gpui_kit::component::input::TextDecoration;
use gpui_kit::{HighlightStyle, rgba};
use std::ops::Range;

pub fn width(text: &str) -> usize {
    if text.lines().any(|line| {
        let spaces = line.bytes().take_while(|&c| c == b' ').count();
        spaces > 0 && spaces % 4 == 2
    }) {
        2
    } else {
        4
    }
}

pub fn spans(text: &str) -> Vec<(Range<usize>, usize)> {
    let unit = width(text);
    let mut spans = vec![];
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let mut column = 0;
        let mut start = 0;
        let mut level = 0;
        for (i, c) in line.char_indices() {
            match c {
                ' ' => column += 1,
                '\t' => column += unit - column % unit,
                _ => break,
            }
            let end = i + c.len_utf8();
            if column >= (level + 1) * unit {
                spans.push((offset + start..offset + end, level));
                start = end;
                level += 1;
            }
        }
        // Color a remaining partial indentation, without touching its following source code.
        let end = line
            .bytes()
            .take_while(|&b| b == b' ' || b == b'\t')
            .count();
        if end > start {
            spans.push((offset + start..offset + end, level));
        }
        offset += line.len();
    }
    spans
}

pub fn decorations(text: &str, dark: bool) -> Vec<TextDecoration> {
    let colors = if dark {
        [
            0xE59B7340, 0xE8CC7540, 0x84C99C40, 0x7BBADB40, 0xBA9FE040, 0xDE97B840,
        ]
    } else {
        [
            0xD67B4828, 0xBA9A2228, 0x449B6528, 0x388EBD28, 0x8D65BE28, 0xB9598828,
        ]
    };
    spans(text)
        .into_iter()
        .map(|(range, depth)| {
            TextDecoration::new(
                range,
                HighlightStyle {
                    background_color: Some(rgba(colors[depth % colors.len()]).into()),
                    ..Default::default()
                },
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::spans;
    #[test]
    fn levels_repeat_consistently_and_only_cover_leading_whitespace() {
        let text = "  first\n    deeper\n  sibling\nplain  spaces\n";
        let runs = spans(text);
        assert_eq!(
            runs.iter().map(|(_, depth)| *depth).collect::<Vec<_>>(),
            [0, 0, 1, 0]
        );
        for (range, _) in &runs {
            assert!(text[range.clone()].bytes().all(|b| b == b' '));
        }
    }
    #[test]
    fn tabs_crlf_and_unicode_keep_valid_byte_ranges() {
        let text = "\t\t🦀\r\n    終\n   partial\n";
        let runs = spans(text);
        assert_eq!(&runs[..2], &[(0..1, 0), (1..2, 1)]);
        for (range, _) in runs {
            assert!(text.is_char_boundary(range.start) && text.is_char_boundary(range.end));
            assert!(text[range].bytes().all(|b| b == b' ' || b == b'\t'));
        }
    }
}
