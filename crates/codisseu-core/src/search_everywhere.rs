//! Read-only, bounded Search Everywhere worker. No project code is executed.
use codisseu_editor::TextSnapshot;
pub use codisseu_syntax::search_symbols::Kind as SymbolKind;
use codisseu_syntax::search_symbols::{self, Symbol};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

pub const MAX_RESULTS: usize = 200;
pub const FILES: u8 = 1;
pub const CLASSES: u8 = 2;
pub const SYMBOLS: u8 = 4;
pub const ACTIONS: u8 = 8;
pub const TEXT: u8 = 16;
pub const ALL_CONTRIBUTORS: u8 = 31;
const MAX_FILES: usize = 100_000;
const MAX_SOURCE: usize = 1024 * 1024;
const MAX_BYTES: usize = 64 * 1024 * 1024;
const MAX_SYMBOLS: usize = 100_000;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Category {
    #[default]
    All,
    Classes,
    Files,
    Symbols,
    Actions,
    Text,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FileFilter {
    #[default]
    All,
    Php,
    JavaScript,
    TypeScript,
    Sql,
}
impl FileFilter {
    pub fn accepts(self, path: &Path) -> bool {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        match self {
            Self::All => true,
            Self::Php => matches!(ext.as_str(), "php" | "phtml"),
            Self::JavaScript => matches!(ext.as_str(), "js" | "jsx" | "mjs" | "cjs"),
            Self::TypeScript => matches!(ext.as_str(), "ts" | "tsx" | "mts" | "cts"),
            Self::Sql => ext == "sql",
        }
    }
}
#[derive(Clone)]
pub struct BufferSnapshot {
    pub path: PathBuf,
    pub version: u64,
    pub text: TextSnapshot,
}
#[derive(Clone)]
pub struct Request {
    pub encoding_profile: Option<PathBuf>,
    pub root: Option<PathBuf>,
    pub external_roots: Vec<PathBuf>,
    pub buffers: Vec<BufferSnapshot>,
    pub query: String,
    pub category: Category,
    pub filter: FileFilter,
    pub include_non_project: bool,
    pub only_open: bool,
    pub refresh: u64,
    pub preview: bool,
    pub contributors: u8,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitKind {
    File,
    Class,
    Symbol,
    Text,
}
#[derive(Clone, Debug)]
pub struct Hit {
    pub kind: HitKind,
    pub title: String,
    pub detail: String,
    pub path: PathBuf,
    pub line: u32,
    pub column: u32,
    pub buffer_version: Option<u64>,
    pub symbol_kind: Option<search_symbols::Kind>,
    pub score: usize,
    pub preview: Option<Preview>,
}
#[derive(Clone, Debug)]
pub struct Preview {
    pub first_line: u32,
    pub text: String,
}
/// Load one selected result outside the GUI thread. The caller supplies dirty
/// document snapshots directly instead of using this disk fallback.
pub fn load_preview(
    path: &Path,
    root: Option<&Path>,
    encoding_profile: Option<&Path>,
    cancelled: &dyn Fn() -> bool,
) -> Result<Preview, String> {
    if cancelled() {
        return Err("Search preview cancelled".into());
    }
    let encodings = super::encoding::Resolver::new(encoding_profile)?;
    let text = read_source(path, &encodings, root, cancelled)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Preview requires a regular text file of at most 1 MiB".to_owned())?;
    if cancelled() {
        return Err("Search preview cancelled".into());
    }
    Ok(Preview {
        first_line: 0,
        text,
    })
}
#[derive(Clone, Debug, Default)]
pub struct Response {
    pub hits: Vec<Hit>,
    pub truncated: bool,
    pub scanned_files: usize,
    pub searched_bytes: usize,
    pub elapsed_ms: u128,
}
struct Job {
    generation: u64,
    request: Request,
}
struct Reply {
    generation: u64,
    result: Result<Response, String>,
}

pub struct SearchEverywhereService {
    sender: Option<SyncSender<Job>>,
    receiver: Receiver<Reply>,
    generation: Arc<AtomicU64>,
    pending: Option<Job>,
    running: usize,
    worker: Option<JoinHandle<()>>,
    pub result: Option<Response>,
    pub error: Option<String>,
}
impl Default for SearchEverywhereService {
    fn default() -> Self {
        Self::new()
    }
}
impl SearchEverywhereService {
    pub fn new() -> Self {
        let (sender, jobs) = mpsc::sync_channel::<Job>(1);
        let (replies, receiver) = mpsc::sync_channel::<Reply>(2);
        let generation = Arc::new(AtomicU64::new(0));
        let worker_generation = generation.clone();
        let worker = std::thread::Builder::new()
            .name("codisseu-everywhere".into())
            .spawn(move || {
                let mut cache = Cache::default();
                while let Ok(job) = jobs.recv() {
                    let cancelled = || worker_generation.load(Ordering::Acquire) != job.generation;
                    let result = search(&job.request, &mut cache, &cancelled);
                    if replies
                        .send(Reply {
                            generation: job.generation,
                            result,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .expect("Search Everywhere worker");
        Self {
            sender: Some(sender),
            receiver,
            generation,
            pending: None,
            running: 0,
            worker: Some(worker),
            result: None,
            error: None,
        }
    }
    pub fn submit(&mut self, request: Request) -> Result<(), String> {
        if request.query.len() > 512 || request.query.contains('\0') {
            return Err("Search query exceeds 512 bytes or contains NUL".into());
        }
        if request.buffers.len() > 32 || request.external_roots.len() > 16 {
            return Err("Search supports 32 open buffers and 16 explicit external roots".into());
        }
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        self.pending = Some(Job {
            generation,
            request,
        });
        self.result = None;
        self.error = None;
        self.flush();
        Ok(())
    }
    fn flush(&mut self) {
        if let Some(job) = self.pending.take() {
            match self.sender.as_ref().expect("live worker").try_send(job) {
                Ok(()) => self.running += 1,
                Err(mpsc::TrySendError::Full(job)) => self.pending = Some(job),
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    self.error = Some("Search worker unavailable".into())
                }
            }
        }
    }
    pub fn poll(&mut self) {
        while let Ok(reply) = self.receiver.try_recv() {
            self.running = self.running.saturating_sub(1);
            if reply.generation == self.generation.load(Ordering::Acquire) {
                match reply.result {
                    Ok(result) => self.result = Some(result),
                    Err(error) => self.error = Some(error),
                }
            }
        }
        self.flush();
    }
    pub fn pending(&self) -> bool {
        self.running > 0 || self.pending.is_some()
    }
    pub fn cancel(&mut self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        self.pending = None;
        self.result = None;
        self.error = None;
    }
}
impl Drop for SearchEverywhereService {
    fn drop(&mut self) {
        self.cancel();
        self.sender.take(); /* A dropped receiver wakes a reply sender; never wait on filesystem I/O in GUI Drop. */
        self.worker.take();
    }
}

#[derive(Default)]
struct Cache {
    key: Option<(Option<PathBuf>, Vec<PathBuf>, bool, u64)>,
    files: Vec<PathBuf>,
    partial: bool,
    symbols: HashMap<PathBuf, (u64, Vec<Symbol>, bool)>,
    symbol_count: usize,
    symbol_bytes: usize,
}
fn search(
    request: &Request,
    cache: &mut Cache,
    cancelled: &dyn Fn() -> bool,
) -> Result<Response, String> {
    let encodings = super::encoding::Resolver::new(request.encoding_profile.as_deref())?;
    let started = Instant::now();
    let deadline = started + Duration::from_secs(10);
    let stopped = || cancelled() || Instant::now() >= deadline;
    if request.category == Category::Actions
        || (request.category == Category::All
            && request.contributors & (FILES | CLASSES | SYMBOLS | TEXT) == 0)
    {
        return Ok(Response::default());
    }
    let project_roots = match request.root.as_deref() {
        Some(path) if super::workspace_file::is_workspace_file(path) => {
            super::workspace_file::read_folders(path)?
                .into_iter()
                .filter(|f| f.available)
                .map(|f| f.path)
                .collect::<Vec<_>>()
        }
        _ => request.root.iter().cloned().collect(),
    };
    let key = (
        request.root.clone(),
        request.external_roots.clone(),
        request.include_non_project,
        request.refresh,
    );
    if !request.only_open && cache.key.as_ref() != Some(&key) {
        cache.key = None;
        cache.files.clear();
        cache.symbols.clear();
        cache.symbol_count = 0;
        cache.symbol_bytes = 0;
        cache.partial = false;
        let mut roots = project_roots.clone();
        if request.include_non_project {
            roots.extend(request.external_roots.iter().cloned());
        }
        let mut walker = Walker {
            dependencies: request.include_non_project,
            stopped: &stopped,
            seen: HashSet::new(),
            files: &mut cache.files,
            partial: &mut cache.partial,
            path_bytes: 0,
        };
        for root in roots {
            walker.walk(&root, 0);
            if stopped() || walker.files.len() >= MAX_FILES {
                break;
            }
        }
        if cancelled() {
            return Err("Search cancelled".into());
        }
        cache.files.sort();
        if !stopped() {
            cache.key = Some(key);
        }
    }
    let mut out = Response {
        truncated: !request.only_open && cache.partial,
        ..Response::default()
    };
    if request.category == Category::Actions {
        return Ok(out);
    }
    let buffers = request
        .buffers
        .iter()
        .map(|b| (&b.path, b))
        .collect::<HashMap<_, _>>();
    let mut paths = request
        .buffers
        .iter()
        .map(|b| b.path.clone())
        .collect::<Vec<_>>();
    if !request.only_open {
        paths.extend(cache.files.iter().cloned());
    }
    let mut seen = HashSet::new();
    let query = if request.category == Category::Text {
        request.query.as_str()
    } else {
        request.query.trim()
    };
    for path in paths {
        if stopped() {
            out.truncated = true;
            break;
        }
        if !seen.insert(path.clone()) || !request.filter.accepts(&path) {
            continue;
        }
        let in_project = project_roots.iter().any(|r| path.starts_with(r));
        if !in_project && !request.include_non_project && request.root.is_some() {
            continue;
        }
        out.scanned_files += 1;
        let buffer = buffers.get(&path).copied();
        let version = buffer.map(|b| b.version);
        let detail = request
            .root
            .as_ref()
            .and_then(|r| path.strip_prefix(r).ok())
            .unwrap_or(&path)
            .display()
            .to_string();
        let title = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let first_hit = out.hits.len();
        if (request.category == Category::Files
            || (request.category == Category::All && request.contributors & FILES != 0))
            && let Some(rank) =
                fuzzy_score(&title, query).or_else(|| fuzzy_score(&detail, query).map(|n| n + 30))
        {
            out.hits.push(Hit {
                kind: HitKind::File,
                title,
                detail: detail.clone(),
                path: path.clone(),
                line: 0,
                column: 0,
                buffer_version: version,
                symbol_kind: None,
                score: rank
                    + if query.is_empty() && buffer.is_none() {
                        100
                    } else {
                        0
                    },
                preview: None,
            });
        }
        let file_match = out.hits.len() != first_hit;
        if (query.is_empty() || request.category == Category::Files)
            && !(request.preview && file_match)
        {
            continue;
        }
        if out.searched_bytes >= MAX_BYTES {
            out.truncated = true;
            continue;
        }
        let text = if let Some(buffer) = buffer {
            if buffer.text.len_bytes() > MAX_SOURCE {
                out.truncated = true;
                continue;
            }
            buffer.text.to_string()
        } else {
            match read_source(
                &path,
                &encodings,
                project_roots
                    .iter()
                    .find(|root| path.starts_with(root))
                    .map(|p| p.as_path()),
                &stopped,
            ) {
                Ok(Some(text)) => text,
                Ok(None) => {
                    if fs::metadata(&path).is_ok_and(|m| m.len() > MAX_SOURCE as u64) {
                        out.truncated = true;
                    }
                    continue;
                }
                Err(_) => {
                    out.truncated = true;
                    continue;
                }
            }
        };
        if out.searched_bytes + text.len() > MAX_BYTES {
            out.truncated = true;
            continue;
        }
        out.searched_bytes += text.len();
        if request.preview && file_match {
            out.hits[first_hit].preview = Some(preview(&text, 0));
        }
        if (matches!(request.category, Category::Classes | Category::Symbols)
            || (request.category == Category::All
                && request.contributors & (CLASSES | SYMBOLS) != 0))
            && !query.is_empty()
            && supported_source(&path)
        {
            let hash = fingerprint(text.as_bytes());
            let fresh = cache
                .symbols
                .get(&path)
                .is_some_and(|(old, _, _)| *old == hash);
            if !fresh {
                let parsed = search_symbols::declarations(&path, &text, &stopped);
                // An interrupted parse belongs to this request only. Caching
                // it by fingerprint would hide declarations from later queries
                // even after cancellation or the request deadline has passed.
                if stopped() {
                    out.truncated = true;
                    break;
                }
                if let Some((_, old, _)) = cache.symbols.remove(&path) {
                    cache.symbol_count = cache.symbol_count.saturating_sub(old.len());
                    cache.symbol_bytes = cache.symbol_bytes.saturating_sub(symbol_bytes(&old));
                }
                let bytes = symbol_bytes(&parsed.symbols);
                if cache.symbol_count + parsed.symbols.len() <= MAX_SYMBOLS
                    && cache.symbol_bytes + bytes <= 32 * 1024 * 1024
                {
                    cache.symbol_count += parsed.symbols.len();
                    cache.symbol_bytes += bytes;
                    cache
                        .symbols
                        .insert(path.clone(), (hash, parsed.symbols, parsed.truncated));
                } else {
                    out.truncated = true;
                }
            }
            if let Some((_, declarations, partial)) = cache.symbols.get(&path) {
                out.truncated |= *partial;
                for symbol in declarations {
                    if request.category == Category::Classes && !symbol.kind.is_class() {
                        continue;
                    }
                    if request.category == Category::All
                        && ((symbol.kind.is_class() && request.contributors & CLASSES == 0)
                            || (!symbol.kind.is_class() && request.contributors & SYMBOLS == 0))
                    {
                        continue;
                    }
                    if let Some(score) = fuzzy_score(&symbol.name, query) {
                        out.hits.push(Hit {
                            kind: if symbol.kind.is_class() {
                                HitKind::Class
                            } else {
                                HitKind::Symbol
                            },
                            title: symbol.name.clone(),
                            detail: if symbol.container.is_empty() {
                                detail.clone()
                            } else {
                                format!("{} — {}", symbol.container, detail)
                            },
                            path: path.clone(),
                            line: symbol.line,
                            column: symbol.column,
                            buffer_version: version,
                            symbol_kind: Some(symbol.kind),
                            score,
                            preview: request.preview.then(|| preview(&text, symbol.line)),
                        });
                    }
                }
            }
        }
        if !query.is_empty()
            && (request.category == Category::Text
                || (request.category == Category::All && request.contributors & TEXT != 0))
        {
            let folded = text.to_lowercase();
            let needle = query.to_lowercase();
            // Unicode lowercasing can expand scalars: match per original line and
            // map folded offsets back to original scalar positions explicitly.
            if folded.contains(&needle) {
                let mut matches = 0;
                for (line, source) in text.lines().enumerate() {
                    if stopped() {
                        break;
                    }
                    let mut folded_line = String::new();
                    let mut columns = Vec::new();
                    for (column, ch) in source.chars().enumerate() {
                        for lower in ch.to_lowercase() {
                            for _ in 0..lower.len_utf8() {
                                columns.push(column);
                            }
                            folded_line.push(lower);
                        }
                    }
                    if let Some(at) = folded_line.find(&needle) {
                        let title = source.trim().chars().take(180).collect::<String>();
                        out.hits.push(Hit {
                            kind: HitKind::Text,
                            title,
                            detail: format!("{detail}:{}", line + 1),
                            path: path.clone(),
                            line: line as u32,
                            column: columns.get(at).copied().unwrap_or(0) as u32,
                            buffer_version: version,
                            symbol_kind: None,
                            score: 500 + line,
                            preview: request.preview.then(|| preview(&text, line as u32)),
                        });
                        matches += 1;
                        if matches >= 32 {
                            out.truncated = true;
                            break;
                        }
                    }
                }
            }
        }
        if out.hits.len() > 2000 {
            out.truncated = true;
            trim_hits(&mut out.hits, 1000);
        }
    }
    if cancelled() {
        return Err("Search cancelled".into());
    }
    out.truncated |= out.hits.len() > MAX_RESULTS;
    trim_hits(&mut out.hits, MAX_RESULTS);
    out.elapsed_ms = started.elapsed().as_millis();
    Ok(out)
}
fn trim_hits(hits: &mut Vec<Hit>, limit: usize) {
    hits.sort_by(|a, b| {
        a.score
            .cmp(&b.score)
            .then_with(|| a.title.cmp(&b.title))
            .then_with(|| a.path.cmp(&b.path))
            .then_with(|| a.line.cmp(&b.line))
    });
    hits.truncate(limit);
}
fn preview(text: &str, line: u32) -> Preview {
    let first = line.saturating_sub(5);
    Preview {
        first_line: first,
        text: text
            .lines()
            .skip(first as usize)
            .take(18)
            .map(|s| s.chars().take(160).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n"),
    }
}
fn symbol_bytes(symbols: &[Symbol]) -> usize {
    symbols
        .iter()
        .map(|s| s.name.len() + s.container.len() + std::mem::size_of::<Symbol>())
        .sum()
}
fn supported_source(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "php" | "phtml" | "ts" | "tsx" | "mts" | "cts" | "js" | "jsx" | "mjs" | "cjs"
    )
}
fn read_source(
    path: &Path,
    encodings: &super::encoding::Resolver,
    root: Option<&Path>,
    cancelled: &dyn Fn() -> bool,
) -> Result<Option<String>, std::io::Error> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > MAX_SOURCE as u64 {
        return Ok(None);
    }
    let mut bytes = Vec::with_capacity((meta.len() as usize).min(MAX_SOURCE));
    let mut file = fs::File::open(path)?.take((MAX_SOURCE + 1) as u64);
    let mut chunk = [0; 8192];
    loop {
        if cancelled() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "Search preview cancelled",
            ));
        }
        let read = file.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..read]);
    }
    if bytes.len() > MAX_SOURCE || bytes.contains(&0) {
        return Ok(None);
    }
    let encoding = encodings
        .resolve(path, root)
        .map_err(std::io::Error::other)?;
    let (text, _) = encoding.decode(&bytes).map_err(std::io::Error::other)?;
    if text.len() > MAX_SOURCE {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Decoded source exceeds 1 MiB",
        ));
    }
    Ok(Some(text))
}
struct Walker<'a> {
    dependencies: bool,
    stopped: &'a dyn Fn() -> bool,
    seen: HashSet<PathBuf>,
    files: &'a mut Vec<PathBuf>,
    partial: &'a mut bool,
    path_bytes: usize,
}
impl Walker<'_> {
    fn walk(&mut self, path: &Path, depth: usize) {
        if (self.stopped)() || self.files.len() >= MAX_FILES || depth > 32 {
            *self.partial = true;
            return;
        }
        let Ok(metadata) = fs::symlink_metadata(path) else {
            *self.partial = true;
            return;
        };
        if metadata.file_type().is_symlink() {
            return;
        }
        if path.as_os_str().len() > 4096 {
            *self.partial = true;
            return;
        }
        if metadata.is_file() {
            if self.path_bytes + path.as_os_str().len() > 32 * 1024 * 1024 {
                *self.partial = true;
                return;
            }
            if self.seen.insert(path.to_path_buf()) {
                self.path_bytes += path.as_os_str().len();
                self.files.push(path.to_path_buf());
            }
            return;
        }
        if !metadata.is_dir() {
            return;
        }
        let Ok(entries) = fs::read_dir(path) else {
            *self.partial = true;
            return;
        };
        for entry in entries {
            if (self.stopped)() || self.files.len() >= MAX_FILES {
                *self.partial = true;
                break;
            }
            let Ok(entry) = entry else {
                *self.partial = true;
                continue;
            };
            let name = entry.file_name();
            if matches!(name.to_str(), Some(".git" | "target" | ".codisseu"))
                || (!self.dependencies
                    && matches!(
                        name.to_str(),
                        Some("node_modules" | "vendor" | ".venv" | "venv" | "dist" | "build")
                    ))
            {
                continue;
            }
            self.walk(&entry.path(), depth + 1);
        }
    }
}
/// Unicode subsequence ranking, with exact/prefix/path matches ahead of fuzzy.
pub fn fuzzy_score(candidate: &str, query: &str) -> Option<usize> {
    let candidate = candidate.to_lowercase();
    let query = query.to_lowercase();
    if query.is_empty() || candidate == query {
        return Some(0);
    }
    if candidate.starts_with(&query) {
        return Some(1);
    }
    if let Some(at) = candidate.find(&query) {
        return Some(10 + at);
    }
    let mut chars = candidate.chars().enumerate();
    let mut rank = 100;
    for wanted in query.chars() {
        rank += chars.find(|(_, c)| *c == wanted)?.0;
    }
    Some(rank)
}
fn fingerprint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |n, b| {
        (n ^ u64::from(*b)).wrapping_mul(0x100000001b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn interrupted_ast_does_not_poison_the_next_query_cache() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("classes.ts");
        let text = (0..100)
            .map(|i| format!("export class Class{i} {{ method{i}() {{}} }}\n"))
            .collect::<String>();
        fs::write(&path, text).unwrap();
        let mut request = Request {
            encoding_profile: None,
            root: Some(dir.path().into()),
            external_roots: vec![],
            buffers: vec![],
            query: "Class".into(),
            category: Category::Files,
            filter: FileFilter::All,
            include_non_project: false,
            only_open: false,
            refresh: 1,
            preview: false,
            contributors: ALL_CONTRIBUTORS,
        };
        let mut cache = Cache::default();
        search(&request, &mut cache, &|| false).unwrap();
        request.category = Category::Classes;
        let checks = Cell::new(0);
        let cancelled = || {
            checks.set(checks.get() + 1);
            checks.get() > 15
        };
        assert!(search(&request, &mut cache, &cancelled).is_err());
        assert!(checks.get() > 15);
        let result = search(&request, &mut cache, &|| false).unwrap();
        assert!(!result.truncated);
        assert_eq!(result.hits.len(), 100);
        assert!(result.hits.iter().any(|hit| hit.title == "Class99"));
    }

    #[test]
    fn open_buffer_results_do_not_inherit_a_partial_disk_catalogue() {
        let dir = tempfile::tempdir().unwrap();
        let request = Request {
            encoding_profile: None,
            root: Some(dir.path().into()),
            external_roots: vec![],
            buffers: vec![BufferSnapshot {
                path: dir.path().join("open.ts"),
                version: 4,
                text: codisseu_editor::TextBuffer::new("export class Known {}".into())
                    .unwrap()
                    .snapshot(),
            }],
            query: "Known".into(),
            category: Category::Classes,
            filter: FileFilter::All,
            include_non_project: false,
            only_open: true,
            refresh: 1,
            preview: false,
            contributors: ALL_CONTRIBUTORS,
        };
        let mut cache = Cache {
            partial: true,
            ..Cache::default()
        };
        let found = search(&request, &mut cache, &|| false).unwrap();
        assert_eq!(found.hits.len(), 1);
        assert_eq!(found.hits[0].buffer_version, Some(4));
        assert!(!found.truncated);
    }
}
