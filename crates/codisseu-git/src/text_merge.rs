//! Independent three-way line merge model. Computed in the Git worker, never GUI.
//! It proposes no filesystem/index writes and preserves exact source bytes.
use crate::{GitError, GitErrorKind, process};
use std::{ffi::OsString, ops::Range, sync::atomic::AtomicBool, time::Duration};

pub const MAX_MERGE_LINES: usize = 100_000;
pub const MAX_MERGE_CHUNKS: usize = 32_768;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeKind {
    Unchanged,
    OursOnly,
    TheirsOnly,
    BothSame,
    Conflict,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeChoice {
    Base,
    Ours,
    Theirs,
    Both,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergeChunk {
    pub kind: MergeKind,
    pub base: String,
    pub ours: String,
    pub theirs: String,
    /// Zero-based, half-open ranges in each original version, including empties.
    pub base_lines: Range<usize>,
    pub ours_lines: Range<usize>,
    pub theirs_lines: Range<usize>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextMerge {
    pub chunks: Vec<MergeChunk>,
}
impl TextMerge {
    /// Pending changes retain Base; equal changes on both sides are unambiguous.
    /// Both means the exact Ours bytes followed by the exact Theirs bytes.
    pub fn result(&self, choices: &[Option<MergeChoice>]) -> String {
        let mut out = String::new();
        for (index, chunk) in self.chunks.iter().enumerate() {
            if chunk.kind == MergeKind::Unchanged {
                out.push_str(&chunk.base);
                continue;
            }
            if chunk.kind == MergeKind::BothSame {
                out.push_str(&chunk.ours);
                continue;
            }
            match choices
                .get(index)
                .copied()
                .flatten()
                .unwrap_or(MergeChoice::Base)
            {
                MergeChoice::Base => out.push_str(&chunk.base),
                MergeChoice::Ours => out.push_str(&chunk.ours),
                MergeChoice::Theirs => out.push_str(&chunk.theirs),
                MergeChoice::Both => {
                    out.push_str(&chunk.ours);
                    out.push_str(&chunk.theirs);
                }
            }
        }
        out
    }
    pub fn build(
        base: &str,
        ours: &str,
        theirs: &str,
        cancel: &AtomicBool,
    ) -> Result<Self, GitError> {
        for text in [base, ours, theirs] {
            if text.len() as u64 > crate::conflicts::MAX_CONFLICT_TEXT || text.contains('\0') {
                return Err(GitError::invalid(
                    "Comparação de merge limitada a texto UTF-8 sem NUL até4MiB por lado",
                ));
            }
        }
        let base_lines = lines(base)?;
        let ours_lines = lines(ours)?;
        let theirs_lines = lines(theirs)?;
        let directory = tempfile::tempdir()?;
        let base_path = directory.path().join("base.txt");
        let ours_path = directory.path().join("ours.txt");
        let theirs_path = directory.path().join("theirs.txt");
        std::fs::write(&base_path, base)?;
        std::fs::write(&ours_path, ours)?;
        std::fs::write(&theirs_path, theirs)?;
        let ours_edits = edits(
            directory.path(),
            &base_path,
            &ours_path,
            base_lines.len(),
            ours_lines.len(),
            cancel,
        )?;
        let theirs_edits = edits(
            directory.path(),
            &base_path,
            &theirs_path,
            base_lines.len(),
            theirs_lines.len(),
            cancel,
        )?;
        assemble(
            &base_lines,
            &ours_lines,
            &theirs_lines,
            ours_edits,
            theirs_edits,
            cancel,
        )
    }
}

/// Resolve only two disjoint, contiguous character edits against the same Base.
/// This is deliberately conservative: overlapping edits (including insertions at
/// the same position), other chunk kinds and sources above 64 KiB are refused.
/// No line endings, separators or source bytes are synthesized or normalized.
pub fn simple_resolution(chunk: &MergeChunk) -> Option<String> {
    const MAX_SIMPLE_BYTES: usize = 64 * 1024;
    if chunk.kind != MergeKind::Conflict
        || [&chunk.base, &chunk.ours, &chunk.theirs]
            .iter()
            .any(|source| source.len() > MAX_SIMPLE_BYTES)
    {
        return None;
    }
    let mut edits = [
        contiguous_edit(&chunk.base, &chunk.ours),
        contiguous_edit(&chunk.base, &chunk.theirs),
    ];
    if edits
        .iter()
        .any(|(range, replacement)| range.is_empty() && replacement.is_empty())
        || overlaps(&edits[0].0, &edits[1].0)
    {
        return None;
    }
    edits.sort_by_key(|(range, _)| (range.start, range.end));
    let [(first, first_text), (second, second_text)] = edits;
    let mut result = String::with_capacity(chunk.base.len() + first_text.len() + second_text.len());
    result.push_str(&chunk.base[..first.start]);
    result.push_str(first_text);
    result.push_str(&chunk.base[first.end..second.start]);
    result.push_str(second_text);
    result.push_str(&chunk.base[second.end..]);
    Some(result)
}

fn contiguous_edit<'a>(base: &str, side: &'a str) -> (Range<usize>, &'a str) {
    let prefix = base
        .chars()
        .zip(side.chars())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .sum::<usize>();
    let suffix = base[prefix..]
        .chars()
        .rev()
        .zip(side[prefix..].chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .sum::<usize>();
    (
        prefix..base.len() - suffix,
        &side[prefix..side.len() - suffix],
    )
}

fn lines(text: &str) -> Result<Vec<&str>, GitError> {
    let mut result = Vec::new();
    for line in text.split_inclusive('\n') {
        if result.len() >= MAX_MERGE_LINES {
            return Err(GitError::invalid(
                "Comparação por trechos limitada a100000linhas por lado; editor de texto permanece disponível",
            ));
        }
        result.push(line);
    }
    Ok(result)
}
#[derive(Clone, Debug)]
pub(crate) struct Edit {
    pub(crate) base: Range<usize>,
    pub(crate) side: Range<usize>,
}
fn spec(spec: &str, prefix: char) -> Result<Range<usize>, GitError> {
    let spec = spec
        .strip_prefix(prefix)
        .ok_or_else(|| GitError::invalid("Range de diff inválido"))?;
    let (start, count) = spec.split_once(',').map_or((spec, "1"), |v| v);
    let start: usize = start
        .parse()
        .map_err(|_| GitError::invalid("Início de range Git inválido"))?;
    let count: usize = count
        .parse()
        .map_err(|_| GitError::invalid("Contagem de range Git inválida"))?;
    let start = if count == 0 {
        start
    } else {
        start
            .checked_sub(1)
            .ok_or_else(|| GitError::invalid("Linha Git inválida"))?
    };
    let end = start
        .checked_add(count)
        .ok_or_else(|| GitError::invalid("Range Git excedido"))?;
    Ok(start..end)
}
pub(crate) fn edits(
    cwd: &std::path::Path,
    base: &std::path::Path,
    side: &std::path::Path,
    base_len: usize,
    side_len: usize,
    cancel: &AtomicBool,
) -> Result<Vec<Edit>, GitError> {
    let mut args: Vec<OsString> = [
        "diff",
        "--no-index",
        "--no-ext-diff",
        "--no-textconv",
        "--text",
        "--unified=0",
        "--inter-hunk-context=0",
        "--no-indent-heuristic",
        "--diff-algorithm=myers",
        "--no-color",
        "--",
    ]
    .map(OsString::from)
    .into();
    args.push(base.as_os_str().into());
    args.push(side.as_os_str().into());
    let output = process::run(
        cwd,
        &args,
        None,
        cancel,
        Duration::from_secs(20),
        &[(OsString::from("GIT_DIFF_OPTS"), OsString::new())],
    )?;
    if !output.status.success() && output.status.code() != Some(1) {
        return Err(GitError::new(
            GitErrorKind::Git,
            crate::redact(&output.stderr),
        ));
    }
    let patch = std::str::from_utf8(&output.stdout)
        .map_err(|_| GitError::invalid("Diff de texto não Unicode"))?;
    let mut result: Vec<Edit> = Vec::new();
    for line in patch.lines().filter(|line| line.starts_with("@@ ")) {
        let mut fields = line.split_whitespace();
        fields.next();
        let base = spec(
            fields
                .next()
                .ok_or_else(|| GitError::invalid("Diff sem range base"))?,
            '-',
        )?;
        let side = spec(
            fields
                .next()
                .ok_or_else(|| GitError::invalid("Diff sem range de origem"))?,
            '+',
        )?;
        if base.end > base_len
            || side.end > side_len
            || result
                .last()
                .is_some_and(|e| e.base.end > base.start || e.side.end > side.start)
        {
            return Err(GitError::invalid("Ranges Git fora de ordem ou do conteúdo"));
        }
        if result.len() >= MAX_MERGE_CHUNKS {
            return Err(GitError::invalid("Comparação de merge excede32768trechos"));
        }
        result.push(Edit { base, side });
    }
    Ok(result)
}
#[derive(Clone)]
struct Tagged {
    edit: Edit,
    side: usize,
}
fn overlaps(a: &Range<usize>, b: &Range<usize>) -> bool {
    if a.is_empty() && b.is_empty() {
        return a.start == b.start;
    }
    if a.is_empty() {
        return b.start < a.start && a.start < b.end;
    }
    if b.is_empty() {
        return a.start < b.start && b.start < a.end;
    }
    a.start < b.end && b.start < a.end
}
fn push_chunk(
    chunks: &mut Vec<MergeChunk>,
    sources: [&[&str]; 3],
    ranges: [Range<usize>; 3],
) -> Result<(), GitError> {
    if chunks.len() >= MAX_MERGE_CHUNKS {
        return Err(GitError::invalid("Comparação de merge excede32768trechos"));
    }
    let texts =
        std::array::from_fn::<_, 3, _>(|index| sources[index][ranges[index].clone()].concat());
    let kind = if texts[0] == texts[1] && texts[0] == texts[2] {
        MergeKind::Unchanged
    } else if texts[1] == texts[2] {
        MergeKind::BothSame
    } else if texts[0] == texts[2] {
        MergeKind::OursOnly
    } else if texts[0] == texts[1] {
        MergeKind::TheirsOnly
    } else {
        MergeKind::Conflict
    };
    let [base, ours, theirs] = texts;
    let [base_lines, ours_lines, theirs_lines] = ranges;
    chunks.push(MergeChunk {
        kind,
        base,
        ours,
        theirs,
        base_lines,
        ours_lines,
        theirs_lines,
    });
    Ok(())
}
fn assemble(
    base: &[&str],
    ours: &[&str],
    theirs: &[&str],
    ours_edits: Vec<Edit>,
    theirs_edits: Vec<Edit>,
    cancel: &AtomicBool,
) -> Result<TextMerge, GitError> {
    let mut events: Vec<Tagged> = ours_edits
        .into_iter()
        .map(|edit| Tagged { edit, side: 0 })
        .chain(
            theirs_edits
                .into_iter()
                .map(|edit| Tagged { edit, side: 1 }),
        )
        .collect();
    events.sort_by_key(|v| (v.edit.base.start, v.edit.base.end, v.side));
    let sources = [base, ours, theirs];
    let mut cursor = [0usize; 3];
    let mut chunks = Vec::new();
    let mut index = 0;
    while index < events.len() {
        crate::repository::cancelled(cancel)?;
        let start = events[index].edit.base.start;
        if start < cursor[0] {
            return Err(GitError::invalid("Trechos Git sobrepostos fora do grupo"));
        }
        let context = start - cursor[0];
        if context > 0 {
            let ends = std::array::from_fn::<_, 3, _>(|i| cursor[i] + context);
            if ends
                .iter()
                .zip(sources)
                .any(|(end, source)| *end > source.len())
            {
                return Err(GitError::invalid("Contexto Git fora do conteúdo"));
            }
            push_chunk(
                &mut chunks,
                sources,
                std::array::from_fn(|i| cursor[i]..ends[i]),
            )?;
            if chunks.last().unwrap().kind != MergeKind::Unchanged {
                return Err(GitError::invalid("Contexto Git divergente"));
            }
            cursor = ends;
        }
        let mut group_range = events[index].edit.base.clone();
        let group_start = index;
        index += 1;
        while index < events.len() && overlaps(&group_range, &events[index].edit.base) {
            group_range.end = group_range.end.max(events[index].edit.base.end);
            index += 1;
        }
        let mut ends = [
            group_range.end,
            cursor[1] + group_range.len(),
            cursor[2] + group_range.len(),
        ];
        for tagged in &events[group_start..index] {
            let removed = tagged.edit.base.len();
            let inserted = tagged.edit.side.len();
            ends[tagged.side + 1] = ends[tagged.side + 1]
                .checked_sub(removed)
                .and_then(|v| v.checked_add(inserted))
                .ok_or_else(|| GitError::invalid("Delta de linhas Git inválido"))?;
        }
        if ends
            .iter()
            .zip(sources)
            .any(|(end, source)| *end > source.len())
        {
            return Err(GitError::invalid("Trecho Git fora do conteúdo"));
        }
        push_chunk(
            &mut chunks,
            sources,
            std::array::from_fn(|i| cursor[i]..ends[i]),
        )?;
        cursor = ends;
    }
    let remaining = base.len() - cursor[0];
    if cursor[1] + remaining != ours.len() || cursor[2] + remaining != theirs.len() {
        return Err(GitError::invalid(
            "Diff incompleto para as versões de merge",
        ));
    }
    if remaining > 0 {
        push_chunk(
            &mut chunks,
            sources,
            std::array::from_fn(|i| cursor[i]..sources[i].len()),
        )?;
        if chunks.last().unwrap().kind != MergeKind::Unchanged {
            return Err(GitError::invalid("Contexto final Git divergente"));
        }
    }
    Ok(TextMerge { chunks })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn model(base: &str, ours: &str, theirs: &str) -> TextMerge {
        TextMerge::build(base, ours, theirs, &AtomicBool::new(false)).unwrap()
    }
    fn reconstruct(model: &TextMerge, field: usize) -> String {
        model
            .chunks
            .iter()
            .map(|c| match field {
                0 => c.base.as_str(),
                1 => c.ours.as_str(),
                _ => c.theirs.as_str(),
            })
            .collect()
    }
    #[test]
    fn independent_and_adjacent_changes_keep_separate_choices_and_exact_ranges() {
        let m = model("a\nb\nc\nd\n", "A\nb\nc\nd\n", "a\nB\nc\nD\n");
        let changes: Vec<_> = m
            .chunks
            .iter()
            .filter(|c| c.kind != MergeKind::Unchanged)
            .collect();
        assert_eq!(changes.len(), 3);
        assert_eq!(changes[0].kind, MergeKind::OursOnly);
        assert_eq!(changes[1].kind, MergeKind::TheirsOnly);
        assert_eq!(changes[2].base_lines, 3..4);
        let choices = m
            .chunks
            .iter()
            .map(|c| {
                Some(if c.kind == MergeKind::OursOnly {
                    MergeChoice::Ours
                } else {
                    MergeChoice::Theirs
                })
            })
            .collect::<Vec<_>>();
        assert_eq!(m.result(&choices), "A\nB\nc\nD\n");
    }
    #[test]
    fn overlap_and_same_point_insertions_are_conflicts_but_boundary_insertions_are_independent() {
        let m = model("a\nb\nc\n", "a\nours\nc\n", "a\ntheirs\nc\n");
        let n = m
            .chunks
            .iter()
            .position(|c| c.kind == MergeKind::Conflict)
            .unwrap();
        let mut choices = vec![None; m.chunks.len()];
        choices[n] = Some(MergeChoice::Both);
        assert_eq!(m.result(&choices), "a\nours\ntheirs\nc\n");
        let m = model("a\nb\n", "a\ninsert\nb\n", "a\nother\nb\n");
        let c = m
            .chunks
            .iter()
            .find(|c| c.kind == MergeKind::Conflict)
            .unwrap();
        assert_eq!(c.base_lines, 1..1);
        assert_eq!(c.ours_lines, 1..2);
        assert_eq!(c.theirs_lines, 1..2);
        let m = model("a\nb\n", "a\ninsert\nb\n", "a\nB\n");
        assert!(!m.chunks.iter().any(|c| c.kind == MergeKind::Conflict));
        let choices = m
            .chunks
            .iter()
            .map(|c| {
                Some(if c.kind == MergeKind::OursOnly {
                    MergeChoice::Ours
                } else {
                    MergeChoice::Theirs
                })
            })
            .collect::<Vec<_>>();
        assert_eq!(m.result(&choices), "a\ninsert\nB\n");
    }
    #[test]
    fn unicode_crlf_no_final_newline_add_add_and_modify_delete_preserve_all_versions() {
        for (base, ours, theirs) in [
            ("a\r\nb😀\r\nc", "a\r\nOURS😀\r\nc", "a\r\nTHEIRS日本\r\nc"),
            ("", "new ours", "new theirs"),
            ("old\n", "", "edited\n"),
            ("keep", "keep\n", "keep"),
            ("old\n", "same\n", "same\n"),
        ] {
            let m = model(base, ours, theirs);
            assert_eq!(reconstruct(&m, 0), base);
            assert_eq!(reconstruct(&m, 1), ours);
            assert_eq!(reconstruct(&m, 2), theirs);
            let ours_choices = vec![Some(MergeChoice::Ours); m.chunks.len()];
            let theirs_choices = vec![Some(MergeChoice::Theirs); m.chunks.len()];
            assert_eq!(m.result(&ours_choices), ours);
            assert_eq!(m.result(&theirs_choices), theirs);
        }
    }
    #[test]
    fn seeded_line_edits_reconstruct_every_source_and_do_not_move_ranges() {
        let base = (0..30)
            .map(|n| format!("line {n} ação 😀\r\n"))
            .collect::<String>();
        for seed in 0..32usize {
            let change = |salt: usize| -> String {
                let mut out = String::new();
                for (n, line) in base.split_inclusive('\n').enumerate() {
                    let value = (seed * 17 + n * 11 + salt * 7) % 19;
                    if value == 0 {
                        out.push_str(&format!("inserted {salt} {n}\r\n"));
                    }
                    if value == 1 {
                        continue;
                    }
                    if value == 2 {
                        out.push_str(&format!("changed {salt} {n}\r\n"));
                    } else {
                        out.push_str(line);
                    }
                }
                if seed % 3 == 0 {
                    out.push_str("last without newline");
                }
                out
            };
            let ours = change(1);
            let theirs = change(2);
            let m = model(&base, &ours, &theirs);
            let inputs = [base.as_str(), ours.as_str(), theirs.as_str()];
            for (side, input) in inputs.iter().enumerate() {
                assert_eq!(reconstruct(&m, side), *input);
            }
            let line_sources = inputs.map(|s| s.split_inclusive('\n').collect::<Vec<_>>());
            for c in &m.chunks {
                for (side, (text, range)) in [
                    (&c.base, &c.base_lines),
                    (&c.ours, &c.ours_lines),
                    (&c.theirs, &c.theirs_lines),
                ]
                .iter()
                .enumerate()
                {
                    assert_eq!(line_sources[side][(*range).clone()].concat(), **text);
                }
            }
            assert_eq!(
                m.result(&vec![Some(MergeChoice::Ours); m.chunks.len()]),
                ours
            );
            assert_eq!(
                m.result(&vec![Some(MergeChoice::Theirs); m.chunks.len()]),
                theirs
            );
        }
    }
    fn simple_chunk(base: &str, ours: &str, theirs: &str) -> MergeChunk {
        MergeChunk {
            kind: MergeKind::Conflict,
            base: base.into(),
            ours: ours.into(),
            theirs: theirs.into(),
            base_lines: 0..1,
            ours_lines: 0..1,
            theirs_lines: 0..1,
        }
    }
    #[test]
    fn simple_conflicts_combine_disjoint_prefix_suffix_edits_without_repeating_context() {
        for (base, ours, theirs, expected) in [
            ("abc", "Xabc", "abcY", "XabcY"),
            (
                "function oldName() { return 1; }\n",
                "function newName() { return 1; }\n",
                "function oldName() { return 2; }\n",
                "function newName() { return 2; }\n",
            ),
            (
                "left right\r\n",
                "LEFT right\r\n",
                "left RIGHT\r\n",
                "LEFT RIGHT\r\n",
            ),
            ("left right", "left RIGHT", "LEFT right", "LEFT RIGHT"),
            ("abc", "bc", "ab", "b"),
        ] {
            assert_eq!(
                simple_resolution(&simple_chunk(base, ours, theirs)).as_deref(),
                Some(expected)
            );
        }
    }
    #[test]
    fn simple_conflicts_respect_unicode_boundaries_and_preserve_combining_marks() {
        for (base, ours, theirs, expected) in [
            ("😀ação 日本", "😃ação 日本", "😀ação 韓国", "😃ação 韓国"),
            ("αβ", "αγ", "Αβ", "Αγ"),
            (
                "e\u{301} end",
                "e\u{302} end",
                "e\u{301} END",
                "e\u{302} END",
            ),
        ] {
            assert_eq!(
                simple_resolution(&simple_chunk(base, ours, theirs)).as_deref(),
                Some(expected)
            );
        }
    }
    #[test]
    fn simple_conflicts_refuse_overlap_same_point_insertions_and_other_kinds() {
        for (base, ours, theirs) in [
            ("abcdef", "abXdef", "abYdef"),
            ("ac", "abc", "aXc"),
            ("abcdef", "aXYef", "abcZef"),
            ("", "ours", "theirs"),
        ] {
            assert!(simple_resolution(&simple_chunk(base, ours, theirs)).is_none());
        }
        let mut c = simple_chunk("abc", "Abc", "abC");
        for kind in [
            MergeKind::Unchanged,
            MergeKind::OursOnly,
            MergeKind::TheirsOnly,
            MergeKind::BothSame,
        ] {
            c.kind = kind;
            assert!(simple_resolution(&c).is_none());
        }
    }
    #[test]
    fn simple_conflict_limit_is_in_bytes_for_each_of_the_three_versions() {
        let base = format!("a{}z", "x".repeat(64 * 1024 - 2));
        let ours = format!("b{}z", "x".repeat(64 * 1024 - 2));
        let theirs = format!("a{}y", "x".repeat(64 * 1024 - 2));
        let expected = format!("b{}y", "x".repeat(64 * 1024 - 2));
        assert_eq!(
            simple_resolution(&simple_chunk(&base, &ours, &theirs)),
            Some(expected)
        );
        for side in 0..3 {
            let mut c = simple_chunk("abc", "Abc", "abC");
            match side {
                0 => c.base = "x".repeat(64 * 1024 + 1),
                1 => c.ours = "x".repeat(64 * 1024 + 1),
                _ => c.theirs = "x".repeat(64 * 1024 + 1),
            }
            assert!(simple_resolution(&c).is_none());
        }
    }
    #[test]
    fn cancellation_and_limits_are_explicit_and_equal_changes_do_not_need_choices() {
        assert!(TextMerge::build("a", "b", "c", &AtomicBool::new(true)).is_err());
        assert!(
            TextMerge::build(
                &"\n".repeat(MAX_MERGE_LINES + 1),
                "",
                "",
                &AtomicBool::new(false)
            )
            .is_err()
        );
        let m = model("old\n", "same\n", "same\n");
        assert_eq!(m.result(&[]), "same\n");
    }
}
