//! Background line-state continuation for the existing lexical highlighter.
//! This does not claim parsing, semantic tokens or embedded-language support.
use super::*;
use codisseu_core::{TextChange, TextSnapshot};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
    mpsc::{self, SyncSender},
};
const MAX_STATE_RUNS: usize = 65_536;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Language {
    Plain,
    Php,
    JavaScript,
    Sql,
    Extended(highlight::extended::Language),
}
impl Language {
    fn for_extension(ext: &str) -> Self {
        if let Some(language) = highlight::extended::Language::for_key(ext) {
            return Self::Extended(language);
        }
        match ext.trim_start_matches('.').to_ascii_lowercase().as_str() {
            "php" | "phtml" => Self::Php,
            "ts" | "tsx" | "js" | "jsx" | "mts" | "cts" | "mjs" | "cjs" => Self::JavaScript,
            "sql" => Self::Sql,
            _ => Self::Plain,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum State {
    Unsupported,
    Normal,
    BlockComment,
    DocComment,
    LineComment,
    Quoted { closing: char, escaped: bool },
    Extended(highlight::extended::State),
}
#[derive(Clone, Debug)]
struct Run {
    line: usize,
    state: State,
}
#[derive(Debug)]
struct States {
    revision: u64,
    generation: u64,
    language: Language,
    runs: Vec<Run>,
    known_until: usize,
    lines: usize,
    error: Option<String>,
}
impl States {
    fn at(&self, line: usize) -> Option<State> {
        if line >= self.known_until {
            return None;
        }
        let index = self
            .runs
            .partition_point(|run| run.line <= line)
            .saturating_sub(1);
        self.runs.get(index).map(|run| run.state)
    }
}
struct Job {
    snapshot: TextSnapshot,
    revision: u64,
    generation: u64,
    language: Language,
    change: Option<TextChange>,
}
#[derive(Default)]
struct Shared {
    job: Mutex<Option<Job>>,
    result: Mutex<Option<Arc<States>>>,
    generation: AtomicU64,
}
pub(super) struct Lexer {
    shared: Arc<Shared>,
    wake: Option<SyncSender<()>>,
    revision: Option<u64>,
    language: Language,
    accepted: Option<Arc<States>>,
    pending_from: usize,
    pending: bool,
    error: Option<String>,
}
impl Default for Lexer {
    fn default() -> Self {
        Self {
            shared: Arc::default(),
            wake: None,
            revision: None,
            language: Language::Plain,
            accepted: None,
            pending_from: 0,
            pending: false,
            error: None,
        }
    }
}
impl Lexer {
    /// Returns the earliest cached line whose colors must be discarded.
    pub fn sync(
        &mut self,
        snapshot: TextSnapshot,
        revision: u64,
        change: Option<&TextChange>,
        ext: &str,
    ) -> Option<usize> {
        let language = Language::for_extension(ext);
        let mut invalidate = None;
        if self.revision != Some(revision) || self.language != language {
            let sequential = self
                .accepted
                .as_ref()
                .is_some_and(|old| old.revision + 1 == revision)
                && self.revision.is_some_and(|old| old + 1 == revision)
                && self.language == language;
            self.pending_from = if sequential {
                change
                    .filter(|change| change.revision == revision)
                    .map_or(0, |change| change.new_lines.start)
            } else {
                0
            };
            self.revision = Some(revision);
            self.language = language;
            self.error = None;
            invalidate = Some(self.pending_from);
            let generation = self.shared.generation.fetch_add(1, Ordering::SeqCst) + 1;
            self.pending = language != Language::Plain;
            if self.pending {
                if self.wake.is_none() {
                    let (shared, (wake, receive)) = (self.shared.clone(), mpsc::sync_channel(1));
                    self.wake = Some(wake);
                    let spawned = std::thread::Builder::new()
                        .name("codisseu-editor-lexical".into())
                        .spawn(move || {
                            let mut previous = None;
                            while receive.recv().is_ok() {
                                let job = shared.job.lock().unwrap().take();
                                let Some(job) = job else {
                                    continue;
                                };
                                if let Some(result) = scan(&job, previous.as_deref(), || {
                                    shared.generation.load(Ordering::SeqCst) != job.generation
                                }) {
                                    let result = Arc::new(result);
                                    if shared.generation.load(Ordering::SeqCst) == job.generation {
                                        *shared.result.lock().unwrap() = Some(result.clone());
                                        previous = Some(result);
                                    }
                                }
                            }
                        });
                    if let Err(error) = spawned {
                        self.wake = None;
                        self.pending = false;
                        self.error = Some(format!("Destaque lexical indisponível: {error}"));
                        return invalidate;
                    }
                }
                *self.shared.job.lock().unwrap() = Some(Job {
                    snapshot,
                    revision,
                    generation,
                    language,
                    change: change.cloned(),
                });
                let _ = self.wake.as_ref().unwrap().try_send(());
            } else {
                self.accepted = None;
            }
        }
        let result = self.shared.result.lock().unwrap().take();
        if let Some(result) = result
            && Some(result.revision) == self.revision
            && result.language == self.language
            && result.generation == self.shared.generation.load(Ordering::SeqCst)
        {
            self.error = result.error.clone();
            self.accepted = Some(result);
            self.pending = false;
            invalidate = Some(self.pending_from);
        }
        invalidate
    }
    pub fn state(&self, line: usize) -> Option<State> {
        if self.language == Language::Plain {
            return Some(State::Normal);
        }
        if self.pending && line >= self.pending_from {
            return None;
        }
        self.accepted.as_ref()?.at(line)
    }
    pub fn context_state(&self, line: usize, ext: &str) -> Option<State> {
        if self.revision.is_none() || self.language != Language::for_extension(ext) {
            return None;
        }
        self.state(line)
    }
    pub fn pending(&self) -> bool {
        self.pending
    }
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}
impl Drop for Lexer {
    fn drop(&mut self) {
        self.shared.generation.fetch_add(1, Ordering::SeqCst);
    }
}

fn scan(
    job: &Job,
    previous: Option<&States>,
    mut cancelled: impl FnMut() -> bool,
) -> Option<States> {
    let lines = job.snapshot.len_lines();
    let incremental = previous.filter(|previous| {
        previous.revision + 1 == job.revision
            && previous.language == job.language
            && previous.error.is_none()
    });
    let change = job
        .change
        .as_ref()
        .filter(|change| change.revision == job.revision);
    let incremental = incremental.zip(change);
    let start = incremental.map_or(0, |(_, change)| change.new_lines.start.min(lines - 1));
    let mut state = incremental
        .and_then(|(old, _)| old.at(start))
        .unwrap_or_else(|| match job.language {
            Language::Extended(_) => State::Extended(Default::default()),
            _ => State::Normal,
        });
    let mut runs: Vec<Run> = incremental.map_or_else(Vec::new, |(old, _)| {
        old.runs
            .iter()
            .take_while(|run| run.line < start)
            .cloned()
            .collect()
    });
    let mut known_until = lines;
    let mut error = None;
    for (offset, line_text) in job.snapshot.lines_at(start).enumerate() {
        let line = start + offset;
        if cancelled() {
            return None;
        }
        if let Some((old, change)) = incremental
            && line >= change.new_lines.end
        {
            let shift = change.new_lines.len() as isize - change.old_lines.len() as isize;
            let old_line = line.checked_add_signed(-shift)?;
            if old_line < old.lines && old.at(old_line) == Some(state) {
                if runs.last().is_none_or(|last| last.state != state) {
                    runs.push(Run { line, state });
                }
                for run in old.runs.iter().filter(|run| run.line > old_line) {
                    let mapped = run.line.checked_add_signed(shift)?;
                    if runs.last().is_none_or(|last| last.state != run.state) {
                        if runs.len() >= MAX_STATE_RUNS {
                            known_until = mapped;
                            error=Some("Destaque lexical limitado a 65536 mudanças de estado; restante em texto simples".into());
                            break;
                        }
                        runs.push(Run {
                            line: mapped,
                            state: run.state,
                        });
                    }
                }
                break;
            }
        }
        if runs.last().is_none_or(|run| run.state != state) {
            if runs.len() >= MAX_STATE_RUNS {
                known_until = line;
                error=Some("Destaque lexical limitado a 65536 mudanças de estado; restante em texto simples".into());
                break;
            }
            runs.push(Run { line, state });
        }
        state = scan_line(line_text.chars(), state, job.language, &mut cancelled)?;
    }
    Some(States {
        revision: job.revision,
        generation: job.generation,
        language: job.language,
        runs,
        known_until,
        lines,
        error,
    })
}
fn scan_line(
    chars: impl Iterator<Item = char>,
    mut state: State,
    language: Language,
    cancelled: &mut impl FnMut() -> bool,
) -> Option<State> {
    if let Language::Extended(language) = language {
        let state = match state {
            State::Extended(state) => state,
            _ => Default::default(),
        };
        return highlight::extended::scan(chars, language, state, |_, _| {}, cancelled)
            .map(State::Extended);
    }
    let mut chars = chars.peekable();
    let mut count = 0;
    while let Some(c) = chars.next() {
        count += 1;
        if count % 4096 == 0 && cancelled() {
            return None;
        }
        match state {
            State::Extended(_) => return None,
            State::Unsupported => {}
            State::Normal => {
                if language == Language::Php && c == '<' && chars.peek() == Some(&'<') {
                    state = State::Unsupported;
                    continue;
                }
                if c == '/' && chars.peek() == Some(&'*') {
                    chars.next();
                    state = if language == Language::Php && chars.peek() == Some(&'*') {
                        State::DocComment
                    } else {
                        State::BlockComment
                    };
                } else if (language != Language::Sql && c == '/' && chars.peek() == Some(&'/'))
                    || (language == Language::Sql && c == '-' && chars.peek() == Some(&'-'))
                {
                    chars.next();
                    state = State::LineComment;
                } else if language == Language::Php && c == '#' {
                    state = State::LineComment;
                } else if matches!(c, '\'' | '"' | '`') {
                    state = State::Quoted {
                        closing: c,
                        escaped: false,
                    };
                } else if language == Language::Sql && c == '[' {
                    state = State::Quoted {
                        closing: ']',
                        escaped: false,
                    };
                }
            }
            State::BlockComment | State::DocComment => {
                if c == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    state = State::Normal;
                }
            }
            State::LineComment => {
                if c == '\n' {
                    state = State::Normal;
                }
            }
            State::Quoted { closing, escaped } => {
                if escaped {
                    state = State::Quoted {
                        closing,
                        escaped: false,
                    };
                } else if c == '\\' && language != Language::Sql {
                    state = State::Quoted {
                        closing,
                        escaped: true,
                    };
                } else if c == closing {
                    if language == Language::Sql && chars.peek() == Some(&closing) {
                        chars.next();
                    } else {
                        state = State::Normal;
                    }
                }
            }
        }
    }
    Some(state)
}
pub(super) fn preview_advance(text: &str, ext: &str, state: State) -> State {
    scan_line(
        text.chars().chain(std::iter::once('\n')),
        state,
        Language::for_extension(ext),
        &mut || false,
    )
    .unwrap_or(State::Unsupported)
}
/// Continue the existing highlighter using virtual context, then remove all
/// virtual bytes and sections before shaping. It never draws that prefix.
#[cfg(test)]
pub fn layout(text: &str, ext: &str, font: FontId, state: Option<State>) -> egui::text::LayoutJob {
    layout_themed(
        text,
        ext,
        font,
        state,
        codisseu_core::ThemePreference::Classic,
    )
}

pub fn layout_themed(
    text: &str,
    ext: &str,
    font: FontId,
    state: Option<State>,
    theme: codisseu_core::ThemePreference,
) -> egui::text::LayoutJob {
    if matches!(ext, "md" | "markdown" | "mdown" | "mkd") {
        return crate::markdown_source::layout_themed(text, font, theme);
    }
    let Some(state) = state else {
        return highlight::layout_themed(text, "", font, theme);
    };
    let prefix = match state {
        State::Extended(state) => {
            return highlight::extended::layout_themed(
                text,
                highlight::extended::Language::for_key(ext).expect("extended language state"),
                font,
                state,
                theme,
            );
        }
        State::Unsupported => return highlight::layout_themed(text, "", font, theme),
        State::Normal => return highlight::layout_themed(text, ext, font, theme),
        // A separating space prevents a line beginning with '*' from turning
        // a plain block continuation into a synthetic PHPDoc opener.
        State::BlockComment => "/* ".into(),
        State::DocComment => "/** ".into(),
        State::LineComment => {
            if Language::for_extension(ext) == Language::Sql {
                "--".into()
            } else {
                "//".into()
            }
        }
        State::Quoted { closing, escaped } => {
            let mut prefix = if closing == ']' {
                "[".into()
            } else {
                closing.to_string()
            };
            if escaped {
                prefix.push('\\');
            }
            prefix
        }
    };
    let prefix: String = prefix;
    let length = prefix.len();
    let mut job = highlight::layout_themed(&format!("{prefix}{text}"), ext, font, theme);
    job.text = text.into();
    job.sections = job
        .sections
        .into_iter()
        .filter_map(|mut section| {
            let start: usize = section.byte_range.start.into();
            let end: usize = section.byte_range.end.into();
            if end <= length {
                return None;
            }
            section.byte_range = start.saturating_sub(length).into()..(end - length).into();
            Some(section)
        })
        .collect();
    job
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    fn scanned(
        text: &str,
        extension: &str,
        revision: u64,
        previous: Option<&States>,
        change: Option<TextChange>,
    ) -> States {
        scan(
            &Job {
                snapshot: TextSnapshot::from_str(text),
                revision,
                generation: revision,
                language: Language::for_extension(extension),
                change,
            },
            previous,
            || false,
        )
        .unwrap()
    }
    fn colors(job: &egui::text::LayoutJob) -> Vec<egui::Color32> {
        let mut values = vec![egui::Color32::TRANSPARENT; job.text.len()];
        for section in &job.sections {
            for value in &mut values
                [usize::from(section.byte_range.start)..usize::from(section.byte_range.end)]
            {
                *value = section.format.color;
            }
        }
        values
    }
    #[test]
    fn multiline_continuations_have_the_existing_whole_document_colors_and_bytes() {
        for (ext, source) in [
            (
                "php",
                "<?php\r\n/* comment\r\nreturn 2;\r\n*/ echo \"multi\r\nreturn 3;\r\nend\";\r\n// return\r\necho 4;",
            ),
            (
                "ts",
                "const s = `multi\ne\\`scape\nreturn 2;`; /* multi\nclass X {}\n*/ let n = 3;",
            ),
            (
                "sql",
                "SELECT 'first\r\nsecond''quoted\r\nlast', [multi\nname] FROM t; -- comment\nSELECT 2;",
            ),
        ] {
            let snapshot = TextSnapshot::from_str(source);
            let states = scanned(source, ext, 0, None, None);
            let whole = highlight::layout(source, ext, FontId::monospace(13.));
            let whole_colors = colors(&whole);
            let mut offset = 0;
            for line in 0..snapshot.len_lines() {
                let text = snapshot.line(line).to_string();
                let job = layout(&text, ext, FontId::monospace(13.), states.at(line));
                assert_eq!(job.text, text);
                assert_eq!(
                    colors(&job),
                    whole_colors[offset..offset + text.len()],
                    "{ext} line{line}"
                );
                offset += text.len();
            }
        }
    }
    #[test]
    fn incremental_state_changes_and_line_shifts_match_a_fresh_scan() {
        let mut text =
            "head\n/* open\ninside\nclose */\nlet a = 2;\n\"start\nend\"\ntail\n".to_owned();
        let mut previous = scanned(&text, "ts", 0, None, None);
        let cases = [
            (1..4, "// single\ninside\nclose */\n"),
            (1..4, "/* open\ninside\nclose */\n"),
            (0..1, "extra\nhead\n"),
            (5..7, "\"changed\nstill quoted\nend\"\n"),
        ];
        for (index, (old_lines, replacement)) in cases.into_iter().enumerate() {
            let snapshot = TextSnapshot::from_str(&text);
            let start = snapshot.line_to_byte(old_lines.start);
            let end = snapshot.line_to_byte(old_lines.end);
            text.replace_range(start..end, replacement);
            let count = replacement.chars().filter(|&c| c == '\n').count();
            let change = TextChange {
                revision: index as u64 + 1,
                old_chars: snapshot.byte_to_char(start)..snapshot.byte_to_char(end),
                new_chars: snapshot.byte_to_char(start)
                    ..snapshot.byte_to_char(start) + replacement.chars().count(),
                old_lines: old_lines.clone(),
                new_lines: old_lines.start..old_lines.start + count,
            };
            let incremental = scanned(&text, "ts", index as u64 + 1, Some(&previous), Some(change));
            let fresh = scanned(&text, "ts", index as u64 + 1, None, None);
            for line in 0..fresh.lines {
                assert_eq!(
                    incremental.at(line),
                    fresh.at(line),
                    "edit{index} line{line}"
                );
            }
            previous = incremental;
        }
    }
    #[test]
    fn cancellation_coalescing_and_unknown_prefix_never_expose_stale_colors() {
        let mut lexer = Lexer::default();
        let first = TextSnapshot::from_str("/*\ninside\n*/\ntail");
        lexer.sync(first.clone(), 0, None, "ts");
        if lexer.pending() {
            assert_eq!(lexer.state(1), None);
        } else {
            assert_eq!(lexer.state(1), Some(State::BlockComment));
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        while lexer.pending() {
            lexer.sync(first.clone(), 0, None, "ts");
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(lexer.state(1), Some(State::BlockComment));
        let second = TextSnapshot::from_str("//\ninside\n*/\ntail");
        lexer.sync(
            second,
            1,
            Some(&TextChange {
                revision: 1,
                old_chars: 0..0,
                new_chars: 0..0,
                old_lines: 0..1,
                new_lines: 0..1,
            }),
            "ts",
        );
        let third = TextSnapshot::from_str("//\ninside\n*/\nchanged");
        lexer.sync(
            third.clone(),
            2,
            Some(&TextChange {
                revision: 2,
                old_chars: 0..0,
                new_chars: 0..0,
                old_lines: 3..4,
                new_lines: 3..4,
            }),
            "ts",
        );
        if lexer.pending() {
            assert_eq!(
                lexer.state(1),
                None,
                "unaccepted intermediate edit invalidates the prefix too"
            );
        } else {
            assert_eq!(
                lexer.state(1),
                Some(State::Normal),
                "a completed current result may already be accepted"
            );
        }
        while lexer.pending() {
            lexer.sync(third.clone(), 2, None, "ts");
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(lexer.state(1), Some(State::Normal));
        assert_eq!(lexer.accepted.as_ref().unwrap().revision, 2);
        let job = Job {
            snapshot: TextSnapshot::from_str(&"a".repeat(16 * 1024 * 1024)),
            revision: 3,
            generation: 3,
            language: Language::JavaScript,
            change: None,
        };
        let mut checks = 0;
        assert!(
            scan(&job, None, || {
                checks += 1;
                checks > 1
            })
            .is_none()
        );
        assert!(checks <= 3);
    }
    #[test]
    fn metadata_limit_is_explicit_and_unknown_lines_remain_plain() {
        let source = "/*\n*/\n".repeat(MAX_STATE_RUNS / 2 + 1);
        let states = scanned(&source, "php", 0, None, None);
        assert!(states.error.as_ref().unwrap().contains("65536"));
        assert_eq!(states.runs.len(), MAX_STATE_RUNS);
        assert_eq!(states.at(states.known_until), None);
        let job = layout("return 2;", "php", FontId::monospace(13.), None);
        assert_eq!(job.text, "return 2;");
        assert_eq!(job.sections.len(), 1);
    }
    #[test]
    fn seeded_incremental_mutations_match_full_reanalysis_and_existing_lexer() {
        for extension in [
            "php", "ts", "sql", "json", "jsonc", "py", "rs", "xml", "yaml", "toml", "env", "ini",
            "ignore", "sh",
        ] {
            let mut snapshot = TextSnapshot::from_str(
                "head\n/* start\ninside\nend */\n'quoted\nline'\n// tail\nSELECT [name], \"value\";\n",
            );
            let mut old = scanned(&snapshot.to_string(), extension, 0, None, None);
            let mut seed = 0x51eed_u64;
            for revision in 1..=100 {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                let start = (seed as usize) % (snapshot.len_chars() + 1);
                let end = (start + (seed as usize >> 24) % 4).min(snapshot.len_chars());
                let inserts = [
                    "/*", "*/", "//", "#", "--", "\n", "'", "\"", "`", "[", "]", "e\u{301}", "😀",
                    "\\", "\r\n", "text",
                ];
                let insert = inserts[(seed as usize >> 32) % inserts.len()];
                let old_lines =
                    snapshot.char_to_line(start.saturating_sub(1))..snapshot.char_to_line(end) + 1;
                snapshot.remove(start..end);
                snapshot.insert(start, insert);
                let new_lines =
                    old_lines.start..snapshot.char_to_line(start + insert.chars().count()) + 1;
                let change = TextChange {
                    revision,
                    old_chars: start..end,
                    new_chars: start..start + insert.chars().count(),
                    old_lines,
                    new_lines,
                };
                let source = snapshot.to_string();
                let actual = scanned(&source, extension, revision, Some(&old), Some(change));
                let full = scanned(&source, extension, revision, None, None);
                for line in 0..full.lines {
                    assert_eq!(
                        actual.at(line),
                        full.at(line),
                        "{extension} rev{revision} line{line}"
                    );
                }
                if revision % 10 == 0 {
                    let expected = colors(&highlight::layout(
                        &source,
                        extension,
                        FontId::monospace(13.),
                    ));
                    let mut offset = 0;
                    for line in 0..full.lines {
                        let text = snapshot.line(line).to_string();
                        let job = layout(&text, extension, FontId::monospace(13.), actual.at(line));
                        assert_eq!(
                            colors(&job),
                            expected[offset..offset + text.len()],
                            "{extension} rev{revision} line{line}"
                        );
                        offset += text.len();
                    }
                }
                old = actual;
            }
        }
    }

    #[test]
    fn additional_languages_keep_multiline_colors_at_scrolled_viewports() {
        for (ext, source) in [
            (
                "py",
                "def café():\n    text = \"\"\"line\n# not comment\nreturn 42\n\"\"\"\n    return True\n",
            ),
            (
                "rs",
                "fn main() {\n/* outer\n/* inner */\nstill outer */ let s = r##\"raw\n// string\nend\"##;\n}\n",
            ),
            (
                "xml",
                "<root key=\"multi\nvalue\">\n<!-- line\n<fake> -->\n<![CDATA[line\n<tag>]]>\n</root>",
            ),
            (
                "toml",
                "[package]\ntext = \"\"\"multi\n# string\nend\"\"\"\ncount = 42\n",
            ),
            (
                "json",
                "{\r\n  \"name\": \"ação😀\",\r\n  \"items\": [true, null, -3.2e+4]\r\n}\r\n",
            ),
            ("jsonc", "/* comment\nnot code\n*/\n{\"ok\": true}\n"),
            (
                "env",
                "# own\nAPP_URL=https://example/#fragment\nTEXT=\"multi\n# string\nend\"\nDEBUG=true # comment\n",
            ),
            ("yaml", "# own\nname: \"value\"\ncount: 42\n"),
        ] {
            let snapshot = TextSnapshot::from_str(source);
            let states = scanned(source, ext, 0, None, None);
            let whole = colors(&highlight::layout(source, ext, FontId::monospace(13.)));
            let mut offset = 0;
            for line in 0..snapshot.len_lines() {
                let text = snapshot.line(line).to_string();
                let job = layout(&text, ext, FontId::monospace(13.), states.at(line));
                assert_eq!(job.text, text);
                assert_eq!(
                    colors(&job),
                    whole[offset..offset + text.len()],
                    "{ext} line {line}"
                );
                offset += text.len();
            }
        }
    }
}
