//! Semantic decorations consume accepted diagnostics only. No source-name heuristics.
use super::*;
use std::{ops::Range, path::Path};

pub(super) fn unnecessary_ranges(
    language: &codisseu_core::LanguageService,
    path: &Path,
    version: u64,
    snapshot: &TextSnapshot,
    composing: bool,
) -> Vec<Range<usize>> {
    if composing
        || snapshot.len_bytes() > 1024 * 1024
        || language.analyzed_versions.get(path) != Some(&version)
    {
        return Vec::new();
    }
    let diagnostics: Vec<_> = language
        .diagnostics
        .iter()
        .filter(|d| d.path == path)
        .take(200)
        .filter(|d| d.unnecessary && d.version.is_none_or(|v| v == version))
        .collect();
    // Group at most 400 endpoints: each relevant Rope line is traversed once,
    // regardless of the number of overlapping diagnostics on that line.
    let mut requested: BTreeMap<u32, std::collections::BTreeSet<u32>> = BTreeMap::new();
    for d in &diagnostics {
        requested
            .entry(d.start_line)
            .or_default()
            .insert(d.start_character);
        requested
            .entry(d.end_line)
            .or_default()
            .insert(d.end_character);
    }
    let mut positions = BTreeMap::new();
    for (line, columns) in requested {
        if line as usize >= snapshot.len_lines() {
            continue;
        }
        let start = snapshot.line_to_char(line as usize);
        let end = content_end(snapshot, line as usize);
        let mut columns = columns.into_iter().peekable();
        let mut units = 0;
        let mut scalar = start;
        for ch in snapshot.line(line as usize).chars().take(end - start) {
            while let Some(&column) = columns.peek() {
                if column > units {
                    break;
                }
                if column == units {
                    positions.insert((line, column), scalar);
                }
                columns.next();
            }
            if columns.peek().is_none() {
                break;
            }
            units += ch.len_utf16() as u32;
            scalar += 1;
        }
        for column in columns {
            if column == units {
                positions.insert((line, column), scalar);
            }
        }
    }
    let mut ranges: Vec<Range<usize>> = diagnostics
        .into_iter()
        .filter_map(|d| {
            let start = *positions.get(&(d.start_line, d.start_character))?;
            let end = *positions.get(&(d.end_line, d.end_character))?;
            (start < end).then_some(start..end)
        })
        .collect();
    ranges.sort_unstable_by_key(|r| (r.start, r.end));
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
    for range in ranges {
        if let Some(last) = merged.last_mut()
            && range.start <= last.end
        {
            last.end = last.end.max(range.end);
        } else {
            merged.push(range);
        }
    }
    merged
}
pub(super) fn line_ranges(ranges: &[Range<usize>], start: usize, end: usize) -> Vec<Range<usize>> {
    ranges
        .iter()
        .filter_map(|r| {
            let a = start.max(r.start);
            let b = end.min(r.end);
            (a < b).then(|| a - start..b - start)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use codisseu_core::LanguageService;
    #[test]
    fn semantic_spans_require_current_version_and_strict_unicode_positions() {
        let snapshot = TextSnapshot::from_str("🌧 café\r\n\tunused\nlast");
        let mut language = LanguageService::new();
        let path = Path::new("fixture.ts");
        language.diagnostics.push(
            serde_json::from_value(serde_json::json!({
                "path": path, "version": 3, "start_line": 0, "start_character": 3,
                "end_line": 1, "end_character": 7, "severity": 4, "unnecessary": true,
                "message": "unused", "language": "TypeScript",
            }))
            .unwrap(),
        );
        let diagnostic = language.diagnostics[0].clone();
        assert!(unnecessary_ranges(&language, path, 3, &snapshot, false).is_empty());
        language.analyzed_versions.insert(path.into(), 3);
        assert_eq!(
            unnecessary_ranges(&language, path, 3, &snapshot, false),
            vec![2..15]
        );
        assert!(unnecessary_ranges(&language, path, 3, &snapshot, true).is_empty());
        assert!(unnecessary_ranges(&language, path, 4, &snapshot, false).is_empty());
        language.diagnostics[0].start_character = 1; // inside surrogate pair
        assert!(unnecessary_ranges(&language, path, 3, &snapshot, false).is_empty());
        language.diagnostics[0] = diagnostic;
        language.diagnostics[0].end_character = 99;
        assert!(unnecessary_ranges(&language, path, 3, &snapshot, false).is_empty());
    }
    #[test]
    fn multiline_overlap_is_merged_then_intersected_without_newline_or_text_changes() {
        let snapshot = TextSnapshot::from_str("abc\r\ndef\n");
        let mut language = LanguageService::new();
        let path = Path::new("fixture.ts");
        language.analyzed_versions.insert(path.into(), 7);
        for (a, b, c, d) in [(0, 1, 1, 1), (1, 0, 1, 3), (1, 3, 0, 1)] {
            language.diagnostics.push(
                serde_json::from_value(serde_json::json!({
                    "path": path, "version": null, "start_line": a, "start_character": b,
                    "end_line": c, "end_character": d, "severity": 4, "unnecessary": true,
                    "message": "", "language": "TypeScript",
                }))
                .unwrap(),
            );
        }
        let ranges = unnecessary_ranges(&language, path, 7, &snapshot, false);
        assert_eq!(ranges, vec![1..8]);
        assert_eq!(
            line_ranges(&ranges, 0, content_end(&snapshot, 0)),
            vec![1..3]
        );
        assert_eq!(
            line_ranges(&ranges, 5, content_end(&snapshot, 1)),
            vec![0..3]
        );
        assert_eq!(snapshot.to_string(), "abc\r\ndef\n");
    }
}
