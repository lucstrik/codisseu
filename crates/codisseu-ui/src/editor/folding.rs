//! Bounded, view-only folding. Source coordinates never change and no text is removed.
use codisseu_core::{Selection, TextSnapshot};
use std::{
    collections::BTreeSet,
    ops::Range,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
const MAX_BYTES: usize = 1024 * 1024;
const MAX_LINES: usize = 20000;
const MAX_RANGES: usize = 4096;
const MAX_JSON_DEPTH: usize = 256;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FoldKind {
    Braces,
    Indentation,
    Comment,
    Region,
    Markdown,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FoldRange {
    pub start: usize,
    pub end: usize,
    pub kind: FoldKind,
    pub level: usize,
}
#[derive(Clone, Debug, Default)]
pub struct FoldInspection {
    pub ranges: Vec<FoldRange>,
    pub truncated: bool,
}
#[derive(Clone, Debug)]
struct Span {
    source: Range<usize>,
    visual: usize,
}
#[derive(Clone, Debug)]
pub struct FoldMap {
    layout_epoch: u64,
    revision: u64,
    line_count: usize,
    ranges: Vec<FoldRange>,
    collapsed: BTreeSet<usize>,
    spans: Vec<Span>,
    visible: usize,
}
impl Default for FoldMap {
    fn default() -> Self {
        Self::new(1, vec![])
    }
}
impl FoldMap {
    pub fn new(line_count: usize, ranges: Vec<FoldRange>) -> Self {
        let line_count = line_count.max(1);
        let mut map = Self {
            layout_epoch: 0,
            revision: 0,
            line_count,
            ranges: normalize(line_count, ranges),
            collapsed: BTreeSet::new(),
            spans: vec![],
            visible: line_count,
        };
        map.rebuild();
        map
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn layout_epoch(&self) -> u64 {
        self.layout_epoch
    }
    pub fn line_count(&self) -> usize {
        self.line_count
    }
    pub fn ranges(&self) -> &[FoldRange] {
        &self.ranges
    }
    /// Editing invalidates old coordinates immediately. Never hide text using stale ranges.
    pub fn sync(&mut self, revision: u64, line_count: usize) -> bool {
        if self.revision == revision && self.line_count == line_count.max(1) {
            return false;
        }
        self.revision = revision;
        self.line_count = line_count.max(1);
        self.ranges.clear();
        self.collapsed.clear();
        self.rebuild();
        true
    }
    /// Accept only inspection of this exact revision; preserve state on same-revision refresh.
    pub fn replace_ranges(
        &mut self,
        revision: u64,
        line_count: usize,
        ranges: Vec<FoldRange>,
    ) -> bool {
        if revision != self.revision || line_count.max(1) != self.line_count {
            return false;
        }
        self.ranges = normalize(self.line_count, ranges);
        self.collapsed
            .retain(|start| self.ranges.iter().any(|range| range.start == *start));
        self.rebuild();
        true
    }
    pub fn range_at(&self, line: usize) -> Option<&FoldRange> {
        self.ranges
            .iter()
            .find(|range| range.start == line)
            .or_else(|| {
                self.ranges
                    .iter()
                    .rev()
                    .find(|range| range.start < line && line <= range.end)
            })
    }
    pub fn collapsed_at(&self, line: usize) -> bool {
        self.collapsed.contains(&line)
    }
    pub fn fold_at(&mut self, line: usize) -> bool {
        let Some(start) = self.range_at(line).map(|range| range.start) else {
            return false;
        };
        let changed = self.collapsed.insert(start);
        if changed {
            self.rebuild();
        }
        changed
    }
    pub fn unfold_at(&mut self, line: usize) -> bool {
        let start = self
            .ranges
            .iter()
            .rev()
            .find(|range| {
                range.start <= line && line <= range.end && self.collapsed.contains(&range.start)
            })
            .map(|range| range.start);
        let changed = start.is_some_and(|start| self.collapsed.remove(&start));
        if changed {
            self.rebuild();
        }
        changed
    }
    pub fn toggle_at(&mut self, line: usize) -> bool {
        if self.collapsed_at(line) {
            self.unfold_at(line)
        } else {
            self.fold_at(line)
        }
    }
    pub fn fold_all(&mut self) {
        self.collapsed = self.ranges.iter().map(|range| range.start).collect();
        self.rebuild();
    }
    pub fn unfold_all(&mut self) {
        self.collapsed.clear();
        self.rebuild();
    }
    pub fn fold_level(&mut self, level: usize) {
        for range in &self.ranges {
            if range.level == level {
                self.collapsed.insert(range.start);
            }
        }
        self.rebuild();
    }
    pub fn visible_line_count(&self) -> usize {
        self.visible
    }
    pub fn is_hidden(&self, line: usize) -> bool {
        let line = line.min(self.line_count - 1);
        let i = self.spans.partition_point(|span| span.source.end <= line);
        self.spans
            .get(i)
            .is_none_or(|span| line < span.source.start)
    }
    /// A hidden source line projects to its nearest visible header.
    pub fn visual_row(&self, line: usize) -> usize {
        let line = line.min(self.line_count - 1);
        let i = self.spans.partition_point(|span| span.source.end <= line);
        if let Some(span) = self.spans.get(i)
            && line >= span.source.start
        {
            return span.visual + line - span.source.start;
        }
        i.checked_sub(1)
            .map_or(0, |i| self.spans[i].visual + self.spans[i].source.len() - 1)
    }
    pub fn source_line(&self, row: usize) -> usize {
        let row = row.min(self.visible.saturating_sub(1));
        let i = self
            .spans
            .partition_point(|span| span.visual + span.source.len() <= row);
        let span = &self.spans[i.min(self.spans.len() - 1)];
        span.source.start + row - span.visual
    }
    /// Reveal endpoints without rewriting directed scalar selections or history.
    pub fn reveal_selection(&mut self, text: &TextSnapshot, selection: Selection) -> bool {
        let lines = [
            text.char_to_line(selection.anchor.min(text.len_chars())),
            text.char_to_line(selection.head.min(text.len_chars())),
        ];
        let remove = self
            .ranges
            .iter()
            .filter(|range| {
                lines
                    .iter()
                    .any(|line| range.start < *line && *line <= range.end)
            })
            .map(|range| range.start)
            .collect::<Vec<_>>();
        let mut changed = false;
        for start in remove {
            changed |= self.collapsed.remove(&start);
        }
        if changed {
            self.rebuild();
        }
        changed
    }
    fn rebuild(&mut self) {
        self.layout_epoch = self.layout_epoch.wrapping_add(1);
        let mut hidden: Vec<Range<usize>> = Vec::new();
        for range in &self.ranges {
            if self.collapsed.contains(&range.start) {
                let next = range.start + 1..range.end + 1;
                if let Some(previous) = hidden.last_mut()
                    && next.start <= previous.end
                {
                    previous.end = previous.end.max(next.end);
                } else {
                    hidden.push(next);
                }
            }
        }
        self.spans.clear();
        let mut source = 0;
        let mut visual = 0;
        for range in hidden {
            if source < range.start {
                self.spans.push(Span {
                    source: source..range.start,
                    visual,
                });
                visual += range.start - source;
            }
            source = range.end;
        }
        if source < self.line_count {
            self.spans.push(Span {
                source: source..self.line_count,
                visual,
            });
            visual += self.line_count - source;
        }
        self.visible = visual;
    }
}
fn normalize(line_count: usize, mut ranges: Vec<FoldRange>) -> Vec<FoldRange> {
    ranges.retain(|range| range.start < range.end && range.end < line_count);
    ranges.sort_by_key(|range| {
        (
            range.start,
            std::cmp::Reverse(range.end),
            match range.kind {
                FoldKind::Region => 0,
                FoldKind::Comment => 1,
                FoldKind::Braces => 2,
                FoldKind::Markdown => 3,
                FoldKind::Indentation => 4,
            },
        )
    });
    let mut result: Vec<FoldRange> = Vec::new();
    let mut parents: Vec<usize> = Vec::new();
    for mut range in ranges {
        if result
            .last()
            .is_some_and(|previous| previous.start == range.start)
        {
            continue;
        }
        while parents.last().is_some_and(|end| *end < range.start) {
            parents.pop();
        }
        if parents.last().is_some_and(|end| range.end > *end) {
            continue;
        }
        range.level = parents.len() + 1;
        parents.push(range.end);
        result.push(range);
        if result.len() == MAX_RANGES {
            break;
        }
    }
    result
}
/// Own conservative text inspector. It is not a parser/LSP semantic folding provider.
/// Files beyond1MiB/20000lines remain expanded with `truncated=true`.
pub fn inspect(
    text: &TextSnapshot,
    extension: &str,
    cancel: &dyn Fn() -> bool,
) -> Result<FoldInspection, String> {
    if cancel() {
        return Err("Inspeção de folding cancelada".into());
    }
    if text.len_bytes() > MAX_BYTES || text.len_lines() > MAX_LINES {
        return Ok(FoldInspection {
            ranges: vec![],
            truncated: true,
        });
    }
    // JSON has structural ranges rather than indentation guesses: whitespace and
    // text inside strings must never create extra fold headers.
    if matches!(extension, "json" | "jsonc") {
        return inspect_json(text, extension == "jsonc", cancel);
    }
    let began = Instant::now();
    let mut ranges = Vec::new();
    let mut truncated = false;
    let mut indentation: Vec<(usize, usize)> = Vec::new();
    let mut previous: Option<(usize, usize)> = None;
    let mut regions = Vec::new();
    let markdown = matches!(extension, "md" | "markdown");
    let mut headers: Vec<(usize, usize)> = Vec::new();
    let mut fence: Option<(usize, String)> = None;
    let mut inspected_end = 0;
    for (line, slice) in text.lines().enumerate() {
        if cancel() {
            return Err("Inspeção de folding cancelada".into());
        }
        if began.elapsed() > Duration::from_millis(500) || ranges.len() >= MAX_RANGES {
            truncated = true;
            break;
        }
        inspected_end = line;
        let raw = slice.to_string();
        let body = raw.trim_end_matches(['\r', '\n']);
        let content = body.trim_start();
        if markdown {
            if content.starts_with("```") || content.starts_with("~~~") {
                let marker = content
                    .chars()
                    .take_while(|c| *c == content.chars().next().unwrap())
                    .collect::<String>();
                if let Some((first, open)) = &fence {
                    if marker.starts_with(open) {
                        ranges.push(FoldRange {
                            start: *first,
                            end: line,
                            kind: FoldKind::Markdown,
                            level: 0,
                        });
                        fence = None;
                    }
                } else {
                    fence = Some((line, marker));
                }
                continue;
            }
            if fence.is_some() {
                continue;
            }
            let level = content.chars().take_while(|c| *c == '#').count();
            if (1..=6).contains(&level) && content.as_bytes().get(level) == Some(&b' ') {
                while headers
                    .last()
                    .is_some_and(|(_, previous)| *previous >= level)
                {
                    let (start, _) = headers.pop().unwrap();
                    if line > start + 1 {
                        ranges.push(FoldRange {
                            start,
                            end: line - 1,
                            kind: FoldKind::Markdown,
                            level: 0,
                        });
                    }
                }
                headers.push((line, level));
            }
        }
        let marker = content
            .trim_start_matches(['/', '*', '#', '-', '<', '!', ' '])
            .trim_start();
        if marker.starts_with("endregion") {
            if let Some(start) = regions.pop()
                && start < line
            {
                ranges.push(FoldRange {
                    start,
                    end: line,
                    kind: FoldKind::Region,
                    level: 0,
                });
            }
        } else if marker.starts_with("region") {
            regions.push(line);
        }
        if content.is_empty() {
            continue;
        }
        let indent = body
            .chars()
            .take_while(|c| matches!(c, ' ' | '\t'))
            .fold(0, |col, c| {
                if c == '\t' {
                    col + 4 - col % 4
                } else {
                    col + 1
                }
            });
        while indentation
            .last()
            .is_some_and(|(_, column)| indent <= *column)
        {
            let (start, _) = indentation.pop().unwrap();
            let end = previous.map_or(start, |(line, _)| line);
            if start < end {
                ranges.push(FoldRange {
                    start,
                    end,
                    kind: FoldKind::Indentation,
                    level: 0,
                });
            }
        }
        if let Some((line, previous_indent)) = previous
            && indent > previous_indent
        {
            indentation.push((line, previous_indent));
        }
        previous = Some((line, indent));
    }
    while let Some((start, _)) = indentation.pop() {
        let end = previous.map_or(start, |(line, _)| line);
        if start < end {
            ranges.push(FoldRange {
                start,
                end,
                kind: FoldKind::Indentation,
                level: 0,
            });
        }
    }
    while let Some((start, _)) = headers.pop() {
        let end = inspected_end;
        if start < end {
            ranges.push(FoldRange {
                start,
                end,
                kind: FoldKind::Markdown,
                level: 0,
            });
        }
    }
    if !markdown
        && matches!(
            extension,
            "php" | "js" | "jsx" | "ts" | "tsx" | "css" | "scss" | "sql"
        )
    {
        let source = text.to_string();
        let mut braces = Vec::new();
        let mut quote = None;
        let mut escaped = false;
        let mut block = None;
        let mut line_comment = false;
        let mut line = 0;
        let mut chars = source.chars().peekable();
        let mut scanned = 0;
        let mut unsafe_braces = false;
        let mut brace_ranges = Vec::new();
        while let Some(c) = chars.next() {
            scanned += 1;
            if scanned % 256 == 0 && cancel() {
                return Err("Inspeção de folding cancelada".into());
            }
            if began.elapsed() > Duration::from_millis(500)
                || ranges.len() + brace_ranges.len() >= MAX_RANGES
            {
                truncated = true;
                break;
            }
            if c == '\n' || (c == '\r' && chars.peek() != Some(&'\n')) {
                line += 1;
                line_comment = false;
            }
            if line_comment {
                continue;
            }
            if let Some(first) = block {
                if c == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    if first < line {
                        ranges.push(FoldRange {
                            start: first,
                            end: line,
                            kind: FoldKind::Comment,
                            level: 0,
                        });
                    }
                    block = None;
                }
                continue;
            }
            if let Some(q) = quote {
                if escaped {
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == q {
                    quote = None;
                }
                continue;
            }
            if matches!(c, '\'' | '"' | '`') {
                quote = Some(c);
                continue;
            }
            if c == '/' && chars.peek() == Some(&'/') {
                chars.next();
                line_comment = true;
                continue;
            }
            if c == '/' && chars.peek() == Some(&'*') {
                chars.next();
                block = Some(line);
                continue;
            }
            if (c == '#' && extension == "php")
                || (c == '-' && extension == "sql" && chars.peek() == Some(&'-'))
            {
                line_comment = true;
                continue;
            }
            // JS regex literals and PHP heredoc require a grammar: omit brace guesses.
            if (c == '/' && matches!(extension, "js" | "jsx" | "ts" | "tsx"))
                || (c == '<' && extension == "php" && chars.peek() == Some(&'<'))
            {
                unsafe_braces = true;
            }
            if matches!(c, '{' | '[' | '(') {
                if braces.len() == 256 {
                    unsafe_braces = true;
                    truncated = true;
                } else {
                    braces.push((c, line));
                }
            }
            if matches!(c, '}' | ']' | ')') {
                let expected = match c {
                    '}' => '{',
                    ']' => '[',
                    _ => '(',
                };
                if let Some((open, start)) = braces.pop() {
                    if open == expected && start < line {
                        brace_ranges.push(FoldRange {
                            start,
                            end: line,
                            kind: FoldKind::Braces,
                            level: 0,
                        });
                    } else if open != expected {
                        unsafe_braces = true;
                    }
                }
            }
        }
        if !unsafe_braces {
            ranges.extend(brace_ranges);
        }
    }
    if ranges.len() > MAX_RANGES {
        truncated = true;
    }
    Ok(FoldInspection {
        ranges: normalize(text.len_lines(), ranges),
        truncated,
    })
}

/// Bounded lexical JSON/JSONC folding. Only completed, matching objects/arrays
/// produce ranges; the delimiter's closing line stays visible when collapsed.
fn inspect_json(
    text: &TextSnapshot,
    comments: bool,
    cancel: &dyn Fn() -> bool,
) -> Result<FoldInspection, String> {
    let began = Instant::now();
    let mut ranges = Vec::new();
    let mut stack = Vec::new();
    let mut chars = text.chars().peekable();
    let mut line = 0;
    let mut quoted = false;
    let mut escaped = false;
    let mut line_comment = false;
    let mut block_comment = false;
    let mut truncated = false;
    let mut scanned = 0usize;
    while let Some(c) = chars.next() {
        scanned += 1;
        if scanned.is_multiple_of(256) {
            if cancel() {
                return Err("Inspeção de folding cancelada".into());
            }
            if began.elapsed() > Duration::from_millis(500) {
                truncated = true;
                break;
            }
        }
        // Count CRLF once, and lone CR consistently with Rope line coordinates.
        if c == '\n' || (c == '\r' && chars.peek() != Some(&'\n')) {
            line += 1;
            line_comment = false;
        }
        if line_comment {
            continue;
        }
        if block_comment {
            if c == '*' && chars.peek() == Some(&'/') {
                chars.next();
                block_comment = false;
            }
            continue;
        }
        if quoted {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                quoted = false;
            }
            continue;
        }
        match c {
            '"' => quoted = true,
            '/' if comments && chars.peek() == Some(&'/') => {
                chars.next();
                line_comment = true;
            }
            '/' if comments && chars.peek() == Some(&'*') => {
                chars.next();
                block_comment = true;
            }
            '{' | '[' => {
                if stack.len() == MAX_JSON_DEPTH {
                    truncated = true;
                    break;
                }
                stack.push((c, line));
            }
            '}' | ']' => {
                let expected = if c == '}' { '{' } else { '[' };
                if let Some((open, start)) = stack.pop() {
                    if open != expected {
                        // Broken input must not pair an outer opener across a
                        // mismatched inner delimiter. Complete siblings survive.
                        stack.clear();
                    } else if line > start + 1 {
                        if ranges.len() == MAX_RANGES {
                            truncated = true;
                            break;
                        }
                        ranges.push(FoldRange {
                            start,
                            end: line - 1,
                            kind: FoldKind::Braces,
                            level: 0,
                        });
                    }
                }
            }
            _ => {}
        }
    }
    if cancel() {
        return Err("Inspeção de folding cancelada".into());
    }
    Ok(FoldInspection {
        ranges: normalize(text.len_lines(), ranges),
        truncated,
    })
}
#[derive(Clone, Debug)]
pub struct FoldResult {
    pub revision: u64,
    pub line_count: usize,
    pub inspection: FoldInspection,
}
struct Request {
    generation: u64,
    revision: u64,
    text: TextSnapshot,
    extension: String,
}
type Reply = Option<(u64, Result<FoldResult, String>)>;
struct Mailbox {
    request: Mutex<Option<Request>>,
    ready: Condvar,
    stopped: AtomicBool,
}
pub struct FoldService {
    mailbox: Arc<Mailbox>,
    started: bool,
    epoch: Arc<AtomicU64>,
    reply: Arc<Mutex<Reply>>,
    generation: u64,
    pending: bool,
    pub error: Option<String>,
}
impl Default for FoldService {
    fn default() -> Self {
        Self::new()
    }
}
impl FoldService {
    /// No thread is created until the first actual request.
    pub fn new() -> Self {
        Self {
            mailbox: Arc::new(Mailbox {
                request: Mutex::new(None),
                ready: Condvar::new(),
                stopped: AtomicBool::new(false),
            }),
            started: false,
            epoch: Arc::new(AtomicU64::new(0)),
            reply: Arc::new(Mutex::new(None)),
            generation: 0,
            pending: false,
            error: None,
        }
    }
    fn start(&mut self) -> Result<(), String> {
        if self.started {
            return Ok(());
        }
        let mailbox = self.mailbox.clone();
        let epoch = self.epoch.clone();
        let reply = self.reply.clone();
        std::thread::Builder::new()
            .name("codisseu-folding".into())
            .spawn(move || {
                loop {
                    let request = {
                        let Ok(mut slot) = mailbox.request.lock() else {
                            return;
                        };
                        while slot.is_none() && !mailbox.stopped.load(Ordering::Relaxed) {
                            slot = match mailbox.ready.wait(slot) {
                                Ok(slot) => slot,
                                Err(_) => return,
                            };
                        }
                        if mailbox.stopped.load(Ordering::Relaxed) {
                            return;
                        }
                        let Some(request) = slot.take() else {
                            continue;
                        };
                        request
                    };
                    let cancelled = || epoch.load(Ordering::Relaxed) != request.generation;
                    let result =
                        inspect(&request.text, &request.extension, &cancelled).map(|inspection| {
                            FoldResult {
                                revision: request.revision,
                                line_count: request.text.len_lines(),
                                inspection,
                            }
                        });
                    if !cancelled()
                        && let Ok(mut slot) = reply.lock()
                    {
                        *slot = Some((request.generation, result));
                    }
                }
            })
            .map_err(|error| format!("Não foi possível iniciar inspeção de folding: {error}"))?;
        self.started = true;
        Ok(())
    }
    pub fn request(
        &mut self,
        revision: u64,
        text: TextSnapshot,
        extension: String,
    ) -> Result<(), String> {
        if let Err(error) = self.start() {
            self.error = Some(error.clone());
            return Err(error);
        }
        self.generation = self.epoch.fetch_add(1, Ordering::Relaxed) + 1;
        self.error = None;
        self.pending = false;
        if let Ok(mut slot) = self.reply.lock() {
            *slot = None;
        }
        let mut slot = self
            .mailbox
            .request
            .lock()
            .map_err(|_| "Fila de folding indisponível".to_owned())?;
        *slot = Some(Request {
            generation: self.generation,
            revision,
            text,
            extension,
        });
        self.pending = true;
        self.mailbox.ready.notify_one();
        Ok(())
    }
    pub fn pending(&self) -> bool {
        self.pending
    }
    pub fn cancel(&mut self) {
        self.generation = self.epoch.fetch_add(1, Ordering::Relaxed) + 1;
        if let Ok(mut slot) = self.reply.lock() {
            *slot = None;
        }
        self.pending = false;
    }
    pub fn poll(&mut self) -> Option<FoldResult> {
        let reply = self.reply.lock().ok()?.take()?;
        if reply.0 != self.generation {
            return None;
        }
        self.pending = false;
        match reply.1 {
            Ok(result) => Some(result),
            Err(error) => {
                self.error = Some(error);
                None
            }
        }
    }
}
impl Drop for FoldService {
    fn drop(&mut self) {
        self.epoch.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut slot) = self.mailbox.request.lock() {
            self.mailbox.stopped.store(true, Ordering::Relaxed);
            *slot = None;
            self.mailbox.ready.notify_one();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn json_ranges(source: &str, extension: &str) -> FoldInspection {
        inspect(&TextSnapshot::from_str(source), extension, &|| false).unwrap()
    }
    #[test]
    fn json_objects_arrays_nest_and_keep_closing_delimiters_visible() {
        let text = TextSnapshot::from_str(
            "{\n  \"list\": [\n    {\n      \"value\": 1\n    },\n    2\n  ],\n  \"other\": true\n}\n",
        );
        let result = inspect(&text, "json", &|| false).unwrap();
        assert!(!result.truncated);
        assert_eq!(
            result
                .ranges
                .iter()
                .map(|r| (r.start, r.end, r.level))
                .collect::<Vec<_>>(),
            [(0, 7, 1), (1, 5, 2), (2, 3, 3)]
        );
        assert!(result.ranges.iter().all(|r| r.kind == FoldKind::Braces));
        let mut map = FoldMap::new(text.len_lines(), result.ranges);
        map.fold_at(2);
        assert!(map.is_hidden(3));
        assert!(!map.is_hidden(4));
        assert_eq!(map.source_line(3), 4);
        map.fold_at(1);
        assert!(map.is_hidden(5));
        assert!(!map.is_hidden(6));
        map.fold_at(0);
        assert!(map.is_hidden(7));
        assert!(!map.is_hidden(8));
        assert_eq!(map.source_line(1), 8);
        map.unfold_at(0);
        assert!(map.collapsed_at(1));
        assert!(map.collapsed_at(2));
    }
    #[test]
    fn json_ignores_string_delimiters_and_escaped_quotes_without_indentation_folds() {
        let source = concat!(
            "{\n",
            "  \"text\": \"{ [ } ] // /* region \\\" still string\",\n",
            "  \"backslash\": \"\\\\\",\n",
            "  \"single\": \"'\",\n",
            "  \"tick\": \"`\",\n",
            "  \"empty\": {},\n",
            "  \"list\": []\n",
            "}"
        );
        let result = json_ranges(source, "json");
        assert_eq!(result.ranges, [range(0, 6).with_level(1)]);
        // Empty/multiline-empty values and one-line values have no hidden body.
        assert!(
            json_ranges("{\n}\n[\n]\n{\"x\": {}}", "json")
                .ranges
                .is_empty()
        );
        // Extra indentation and words resembling region markers are plain data.
        assert!(
            json_ranges("\"region\"\n    \"value\"\n\"endregion\"", "json")
                .ranges
                .is_empty()
        );
    }
    #[test]
    fn jsonc_skips_line_and_block_comments_but_json_does_not_invent_comments() {
        let source =
            "{\r\n // { [ \"\r\n /* }\r\n    ] \" */\r\n \"list\": [\r\n  1\r\n ]\r\n}\r\n";
        let result = json_ranges(source, "jsonc");
        assert_eq!(
            result.ranges,
            [range(0, 6).with_level(1), range(4, 5).with_level(2)]
        );
        assert!(!result.truncated);
        let source = "/**\n{\n  1\n}\n*/";
        let strict = json_ranges(source, "json");
        assert_eq!(strict.ranges, [range(1, 2).with_level(1)]);
        assert!(json_ranges(source, "jsonc").ranges.is_empty());
    }
    #[test]
    fn json_incomplete_and_mismatched_input_only_retains_completed_ranges() {
        assert!(json_ranges("{\n  \"value\": 1", "json").ranges.is_empty());
        assert!(json_ranges("{\n  [\n  }\n]\n}", "json").ranges.is_empty());
        let partial = json_ranges(
            "{\n \"ok\": [\n  1\n ],\n \"text\": \"unterminated }\n}\n",
            "json",
        );
        assert_eq!(partial.ranges, [range(1, 2).with_level(1)]);
        assert!(
            json_ranges("{\n /* unfinished {\n}\n", "jsonc")
                .ranges
                .is_empty()
        );
    }
    #[test]
    fn json_crlf_and_lone_cr_use_source_line_coordinates() {
        for newline in ["\n", "\r\n", "\r"] {
            let source = ["{", " \"á👩‍💻\": [", "  1", " ]", "}"].join(newline);
            assert_eq!(
                json_ranges(&source, "json").ranges,
                [range(0, 3).with_level(1), range(1, 2).with_level(2)]
            );
        }
    }
    #[test]
    fn json_metadata_and_document_limits_are_bounded_and_report_truncation() {
        let many = "[\n 1\n]\n".repeat(MAX_RANGES + 1);
        let result = json_ranges(&many, "json");
        assert_eq!(result.ranges.len(), MAX_RANGES);
        assert!(result.truncated);
        let deep = format!(
            "{}\n1\n{}",
            "[".repeat(MAX_JSON_DEPTH + 1),
            "]".repeat(MAX_JSON_DEPTH + 1)
        );
        let result = json_ranges(&deep, "json");
        assert!(result.ranges.is_empty());
        assert!(result.truncated);
        for source in [" ".repeat(MAX_BYTES + 1), "\n".repeat(MAX_LINES)] {
            let result = json_ranges(&source, "json");
            assert!(result.ranges.is_empty());
            assert!(result.truncated);
        }
    }
    #[test]
    fn json_cancel_is_checked_during_long_strings_and_after_scanning() {
        let text = TextSnapshot::from_str(&format!("{{\n\"text\": \"{}\"\n}}", "a".repeat(1000)));
        let calls = std::cell::Cell::new(0);
        assert!(
            inspect(&text, "json", &|| {
                calls.set(calls.get() + 1);
                calls.get() >= 3
            })
            .is_err()
        );
        assert_eq!(calls.get(), 3);
        let calls = std::cell::Cell::new(0);
        assert!(
            inspect(&TextSnapshot::from_str("{}"), "json", &|| {
                calls.set(calls.get() + 1);
                calls.get() == 2
            })
            .is_err()
        );
    }
    #[test]
    fn php_js_ts_main_blocks_fold_nested_classes_functions_objects_and_interfaces() {
        for (extension, source, expected) in [
            (
                "php",
                "<?php\nclass Service {\n public function run() {\n  $text = \"escaped \\\" }\";\n  // } false close\n  if (true) {\n   return ['value' => '{'];\n  }\n }\n}\n",
                vec![(1, 9), (2, 8), (5, 7)],
            ),
            (
                "js",
                "function run() {\n const obj = {\n  value: `escaped \\` }`,\n  // } false close\n  nested: {\n   value: '}'\n  }\n };\n return obj;\n}\n",
                vec![(0, 9), (1, 7), (4, 6)],
            ),
            (
                "ts",
                "interface Result {\n value: string;\n}\nclass Service {\n run(): Result {\n  /* } false close */\n  return {\n   value: \"}\"\n  };\n }\n}\n",
                vec![(0, 2), (3, 10), (4, 9), (6, 8)],
            ),
        ] {
            let text = TextSnapshot::from_str(source);
            let result = inspect(&text, extension, &|| false).unwrap();
            assert!(!result.truncated, "{extension}");
            let blocks = result
                .ranges
                .iter()
                .filter(|r| r.kind == FoldKind::Braces)
                .map(|r| (r.start, r.end))
                .collect::<Vec<_>>();
            assert_eq!(blocks, expected, "{extension}");
            let mut map = FoldMap::new(text.len_lines(), result.ranges);
            assert!(map.fold_at(expected[0].0));
            assert!(map.is_hidden(expected[0].1));
            assert!(map.unfold_at(expected[0].0));
            assert!(!map.is_hidden(expected[0].1));
        }
    }
    #[test]
    fn php_js_ts_ambiguous_regex_heredoc_and_excessive_depth_do_not_guess_braces() {
        for (extension, source) in [
            ("php", "function run() {\n $x = <<<TEXT\n }\nTEXT;\n}\n"),
            ("js", "function run() {\n const r = /[{}]/;\n}\n"),
            ("ts", "function run() {\n const r: RegExp = /[{}]/;\n}\n"),
        ] {
            let result = inspect(&TextSnapshot::from_str(source), extension, &|| false).unwrap();
            assert!(
                result.ranges.iter().all(|r| r.kind != FoldKind::Braces),
                "{extension}"
            );
        }
        let deep = TextSnapshot::from_str(&format!("{}\n1\n{}", "{".repeat(257), "}".repeat(257)));
        for extension in ["php", "js", "ts"] {
            let result = inspect(&deep, extension, &|| false).unwrap();
            assert!(result.truncated);
            assert!(result.ranges.iter().all(|r| r.kind != FoldKind::Braces));
        }
    }
    #[test]
    fn json_revision_changes_expand_and_refuse_old_ranges_without_changing_selection() {
        let text = TextSnapshot::from_str("{\n \"list\": [\n  1\n ]\n}\n");
        let ranges = inspect(&text, "json", &|| false).unwrap().ranges;
        let selection = Selection {
            anchor: 16,
            head: 2,
        };
        let mut map = FoldMap::new(text.len_lines(), ranges.clone());
        map.fold_all();
        assert!(map.is_hidden(2));
        map.sync(1, text.len_lines());
        assert!(!map.is_hidden(2));
        assert!(!map.replace_ranges(0, text.len_lines(), ranges.clone()));
        assert!(map.ranges().is_empty());
        assert!(map.replace_ranges(1, text.len_lines(), ranges));
        map.fold_all();
        assert!(map.reveal_selection(&text, selection));
        assert!(!map.is_hidden(text.char_to_line(selection.head)));
        assert!(!map.is_hidden(text.char_to_line(selection.anchor)));
        assert_eq!(
            selection,
            Selection {
                anchor: 16,
                head: 2
            }
        );
    }
    impl FoldRange {
        fn with_level(mut self, level: usize) -> Self {
            self.level = level;
            self
        }
    }
    fn range(start: usize, end: usize) -> FoldRange {
        FoldRange {
            start,
            end,
            kind: FoldKind::Braces,
            level: 0,
        }
    }
    #[test]
    fn nested_folds_project_both_directions_without_dense_line_storage() {
        let mut map = FoldMap::new(1_000_000, vec![range(2, 10), range(4, 7), range(20, 30)]);
        map.fold_at(4);
        assert_eq!(map.visible_line_count(), 999997);
        assert_eq!(map.visual_row(7), 4);
        assert_eq!(map.source_line(5), 8);
        map.fold_at(2);
        assert_eq!(map.visible_line_count(), 999992);
        assert_eq!(map.visual_row(10), 2);
        assert_eq!(map.source_line(3), 11);
        map.unfold_at(2);
        assert!(map.collapsed_at(4));
        assert_eq!(map.source_line(5), 8);
        map.fold_all();
        assert!(map.spans.len() < 10);
        for row in [0, 2, 3, 10, 100, 999000] {
            let line = map.source_line(row);
            assert!(!map.is_hidden(line));
            assert_eq!(map.visual_row(line), row.min(map.visible_line_count() - 1));
        }
    }
    #[test]
    fn edit_invalidation_and_reveal_preserve_directed_unicode_source_selection() {
        let text = TextSnapshot::from_str("{\r\n雨\r\n👩‍💻\r\n}\r\n");
        let selection = Selection { anchor: 9, head: 3 };
        let mut map = FoldMap::new(text.len_lines(), vec![range(0, 3)]);
        map.fold_all();
        assert!(map.reveal_selection(&text, selection));
        assert!(!map.is_hidden(1));
        assert_eq!(selection, Selection { anchor: 9, head: 3 });
        map.fold_all();
        assert!(map.sync(1, text.len_lines() + 1));
        assert!(map.ranges().is_empty());
        assert_eq!(map.visible_line_count(), text.len_lines() + 1);
        assert!(!map.replace_ranges(0, text.len_lines(), vec![range(0, 3)]));
    }
    #[test]
    fn inspect_ignores_strings_comments_and_regex_brace_guesses_and_supports_markdown() {
        let text = TextSnapshot::from_str(
            "function f() {\n  const s = '}';\n  /* block\n     comment */\n  if (yes) {\n    run();\n  }\n}\n",
        );
        let result = inspect(&text, "js", &|| false).unwrap();
        assert!(!result.truncated);
        assert!(
            result
                .ranges
                .iter()
                .any(|range| range.kind == FoldKind::Comment && range.start == 2)
        );
        assert!(result.ranges.iter().any(|r| r.start == 0 && r.end == 7));
        assert!(result.ranges.iter().any(|r| r.start == 4 && r.end == 6));
        let regex = inspect(
            &TextSnapshot::from_str("const p = /{/;\nconst x = {\na: 1\n};"),
            "js",
            &|| false,
        )
        .unwrap();
        assert!(!regex.ranges.iter().any(|r| r.kind == FoldKind::Braces));
        let markdown = inspect(
            &TextSnapshot::from_str("# First\nbody\n## Child\ntext\n```js\n{}\n```\n# Last\nend"),
            "md",
            &|| false,
        )
        .unwrap();
        assert!(markdown.ranges.iter().any(|r| r.start == 0 && r.end == 6));
        assert!(markdown.ranges.iter().any(|r| r.start == 4 && r.end == 6));
        assert!(inspect(&text, "js", &|| true).is_err());
    }
    #[test]
    fn worker_is_versioned_bounded_and_latest_result_only() {
        let mut service = FoldService::new();
        assert!(!service.started);
        service
            .request(3, TextSnapshot::from_str("a {\n b\n}\n"), "js".into())
            .unwrap();
        let start = Instant::now();
        let result = loop {
            if let Some(result) = service.poll() {
                break result;
            }
            assert!(start.elapsed() < Duration::from_secs(2));
            std::thread::sleep(Duration::from_millis(1));
        };
        assert_eq!(result.revision, 3);
        assert!(!service.pending());
        for revision in 10..30 {
            service
                .request(
                    revision,
                    TextSnapshot::from_str("latest {\n body\n}"),
                    "js".into(),
                )
                .unwrap();
        }
        let began = Instant::now();
        let result = loop {
            if let Some(result) = service.poll() {
                break result;
            }
            assert!(began.elapsed() < Duration::from_secs(2));
            std::thread::sleep(Duration::from_millis(1));
        };
        assert_eq!(result.revision, 29);
        service
            .request(4, TextSnapshot::from_str("x"), "js".into())
            .unwrap();
        service.cancel();
        std::thread::sleep(Duration::from_millis(5));
        assert!(service.poll().is_none());
        let large = TextSnapshot::from_str(&"a".repeat(MAX_BYTES + 1));
        assert!(inspect(&large, "js", &|| false).unwrap().truncated);
    }
}
