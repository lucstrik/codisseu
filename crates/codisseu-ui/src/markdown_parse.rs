//! Bounded CommonMark/GFM model. No HTML, browser, network or project code runs here.
use super::*;
use pulldown_cmark::{Alignment, CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use std::{
    fs,
    io::Read,
    ops::Range,
    path::{Component, Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

pub(super) const MAX_SOURCE: usize = 2 * 1024 * 1024;
const MAX_EVENTS: usize = 120_000;
const MAX_BLOCKS: usize = 12_000;
const MAX_DEPTH: usize = 64;
const MAX_IMAGE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_PIXELS: u64 = 4 * 1024 * 1024;
const MAX_IMAGE_TOTAL: usize = 32 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum LinkTarget {
    Anchor(String),
    Local {
        path: PathBuf,
        anchor: Option<String>,
    },
    External(String),
    Blocked(String),
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct InlineStyle {
    pub strong: bool,
    pub emphasis: bool,
    pub strike: bool,
    pub code: bool,
}
#[derive(Clone, Debug)]
pub(super) struct Span {
    pub text: String,
    pub style: InlineStyle,
    pub link: Option<Arc<LinkTarget>>,
}
#[derive(Clone, Debug)]
pub(super) struct Task {
    pub checked: bool,
    pub byte_offset: usize,
}
#[derive(Debug)]
pub(super) enum BlockKind {
    Rich {
        spans: Vec<Span>,
        heading: u8,
        anchor: Option<String>,
        marker: Option<String>,
        task: Option<Task>,
        loose: bool,
    },
    Code {
        language: String,
        text: String,
        job: egui::text::LayoutJob,
    },
    Table {
        align: Vec<Alignment>,
        rows: Vec<Vec<Vec<Span>>>,
    },
    Image {
        alt: String,
        destination: String,
        target: LinkTarget,
    },
    Rule,
}
#[derive(Debug)]
pub(super) struct Block {
    pub range: Range<usize>,
    pub line: usize,
    pub indent: usize,
    pub quote: usize,
    pub kind: BlockKind,
}
pub(super) struct Parsed {
    pub read_only: bool,
    pub blocks: Vec<Block>,
    pub images: HashMap<String, Result<Arc<egui::ColorImage>, String>>,
    pub warnings: Vec<String>,
    pub cost_bytes: usize,
}
#[derive(Default)]
struct Rich {
    spans: Vec<Span>,
    range: Range<usize>,
    heading: u8,
    anchor: Option<String>,
    marker: Option<String>,
    task: Option<Task>,
    loose: bool,
}
struct List {
    next: Option<u64>,
}
struct Table {
    align: Vec<Alignment>,
    rows: Vec<Vec<Vec<Span>>>,
    row: Vec<Vec<Span>>,
    cell: Vec<Span>,
    range: Range<usize>,
}

pub(super) fn parse(
    source: &str,
    path: &Path,
    workspace: Option<&Path>,
    cancel: &AtomicBool,
) -> Result<Parsed, String> {
    parse_inner(source, path, workspace, cancel, false)
}
pub(super) fn parse_remote(source: &str, cancel: &AtomicBool) -> Result<Parsed, String> {
    parse_inner(source, Path::new("remote-jira.md"), None, cancel, true)
}
fn parse_inner(
    source: &str,
    path: &Path,
    workspace: Option<&Path>,
    cancel: &AtomicBool,
    remote: bool,
) -> Result<Parsed, String> {
    let resolve_target = |s: &str| {
        if remote {
            remote_link(s)
        } else {
            resolve_link(s, path, workspace)
        }
    };
    if source.len() > MAX_SOURCE {
        return Err(
            "Markdown preview is limited to 2 MiB; the source editor remains available".into(),
        );
    }
    let started = Instant::now();
    let mut options = Options::empty();
    options.insert(
        Options::ENABLE_TABLES
            | Options::ENABLE_TASKLISTS
            | Options::ENABLE_STRIKETHROUGH
            | Options::ENABLE_GFM
            | Options::ENABLE_HEADING_ATTRIBUTES,
    );
    let mut lines = vec![0];
    let mut chars = source.char_indices().peekable();
    while let Some((index, ch)) = chars.next() {
        if ch == '\r' && chars.peek().is_some_and(|(_, next)| *next == '\n') {
            let (next, _) = chars.next().unwrap();
            lines.push(next + 1);
        } else if matches!(ch, '\n' | '\r') {
            lines.push(index + ch.len_utf8());
        }
    }
    let mut blocks = Vec::new();
    let mut rich = Rich::default();
    let mut style = InlineStyle::default();
    let mut emphasis = 0usize;
    let mut strong = 0usize;
    let mut strike = 0usize;
    let mut link = None;
    let mut link_cache: HashMap<String, Arc<LinkTarget>> = HashMap::new();
    let mut link_bytes = 0usize;
    let mut lists: Vec<List> = Vec::new();
    let mut quote = 0usize;
    let mut depth = 0usize;
    let mut code: Option<(String, String, Range<usize>)> = None;
    let mut image: Option<(String, String, Range<usize>)> = None;
    let mut table: Option<Table> = None;
    let mut warnings = Vec::new();
    let mut anchors: HashMap<String, usize> = HashMap::new();
    for (count, (event, range)) in Parser::new_ext(source, options)
        .into_offset_iter()
        .enumerate()
    {
        if count % 128 == 0
            && (cancel.load(Ordering::Relaxed) || started.elapsed() > Duration::from_secs(2))
        {
            return Err(if cancel.load(Ordering::Relaxed) {
                "Markdown preview cancelled"
            } else {
                "Markdown preview exceeded its 2-second parsing budget"
            }
            .into());
        }
        if count >= MAX_EVENTS || blocks.len() >= MAX_BLOCKS {
            return Err(
                "Markdown structure exceeds the preview budget; the source remains available"
                    .into(),
            );
        }
        if matches!(event, Event::Start(_)) {
            depth += 1;
            if depth > MAX_DEPTH {
                return Err("Markdown nesting exceeds 64 levels".into());
            }
        }
        if matches!(event, Event::End(_)) {
            depth = depth.saturating_sub(1);
        }
        let flush = |rich: &mut Rich,
                     blocks: &mut Vec<Block>,
                     lists: &[List],
                     quote,
                     anchors: &mut HashMap<String, usize>| {
            if rich.spans.is_empty() && rich.task.is_none() {
                return;
            }
            let mut value = std::mem::take(rich);
            if value.heading > 0 {
                let base = value.anchor.take().unwrap_or_else(|| {
                    slug(
                        &value
                            .spans
                            .iter()
                            .map(|s| s.text.as_str())
                            .collect::<String>(),
                    )
                });
                let mut ordinal = *anchors.get(&base).unwrap_or(&0);
                let mut candidate = if ordinal == 0 {
                    base.clone()
                } else {
                    format!("{base}-{ordinal}")
                };
                while anchors.contains_key(&candidate) {
                    ordinal += 1;
                    candidate = format!("{base}-{ordinal}");
                }
                anchors.insert(base, ordinal + 1);
                anchors.insert(candidate.clone(), 1);
                value.anchor = Some(candidate);
            }
            blocks.push(Block {
                line: lines
                    .partition_point(|v| *v <= value.range.start)
                    .saturating_sub(1),
                range: value.range,
                indent: lists.len().saturating_sub(1),
                quote,
                kind: BlockKind::Rich {
                    spans: value.spans,
                    heading: value.heading,
                    anchor: value.anchor,
                    marker: value.marker,
                    task: value.task,
                    loose: value.loose,
                },
            });
        };
        match event {
            Event::Start(Tag::Paragraph) => {
                rich.loose = true;
                if rich.spans.is_empty() {
                    rich.range = range;
                }
            }
            Event::Start(Tag::Heading { level, id, .. }) => {
                flush(&mut rich, &mut blocks, &lists, quote, &mut anchors);
                rich.range = range;
                rich.heading = level as u8;
                rich.anchor = id.map(|id| id.into_string());
            }
            Event::End(TagEnd::Paragraph | TagEnd::Heading(_)) => {
                flush(&mut rich, &mut blocks, &lists, quote, &mut anchors)
            }
            Event::Start(Tag::List(next)) => {
                flush(&mut rich, &mut blocks, &lists, quote, &mut anchors);
                lists.push(List { next });
            }
            Event::End(TagEnd::List(_)) => {
                flush(&mut rich, &mut blocks, &lists, quote, &mut anchors);
                lists.pop();
            }
            Event::Start(Tag::Item) => {
                flush(&mut rich, &mut blocks, &lists, quote, &mut anchors);
                rich.range = range;
                rich.marker = lists.last_mut().map(|list| match &mut list.next {
                    Some(n) => {
                        let text = format!("{n}.");
                        *n = n.saturating_add(1);
                        text
                    }
                    None => "•".into(),
                });
            }
            Event::End(TagEnd::Item) => flush(&mut rich, &mut blocks, &lists, quote, &mut anchors),
            Event::Start(Tag::BlockQuote(_)) => {
                flush(&mut rich, &mut blocks, &lists, quote, &mut anchors);
                quote += 1;
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                flush(&mut rich, &mut blocks, &lists, quote, &mut anchors);
                quote = quote.saturating_sub(1);
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                flush(&mut rich, &mut blocks, &lists, quote, &mut anchors);
                code = Some((
                    match kind {
                        CodeBlockKind::Fenced(s) => s
                            .split_whitespace()
                            .next()
                            .unwrap_or("")
                            .chars()
                            .take(32)
                            .collect(),
                        _ => String::new(),
                    },
                    String::new(),
                    range,
                ));
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some((language, text, range)) = code.take() {
                    blocks.push(Block {
                        line: lines
                            .partition_point(|v| *v <= range.start)
                            .saturating_sub(1),
                        range,
                        indent: lists.len().saturating_sub(1),
                        quote,
                        kind: BlockKind::Code {
                            job: super::super::highlight::layout(
                                &text,
                                match language.as_str() {
                                    "javascript" => "js",
                                    "typescript" => "ts",
                                    "postgresql" | "mysql" => "sql",
                                    other => other,
                                },
                                FontId::monospace(13.),
                            ),
                            language,
                            text,
                        },
                    });
                }
            }
            Event::Start(Tag::Table(align)) => {
                flush(&mut rich, &mut blocks, &lists, quote, &mut anchors);
                if align.len() > 64 {
                    return Err("Markdown table exceeds 64 columns".into());
                }
                table = Some(Table {
                    align,
                    rows: Vec::new(),
                    row: Vec::new(),
                    cell: Vec::new(),
                    range,
                });
            }
            Event::Start(Tag::TableCell) => {
                if let Some(t) = &mut table {
                    t.cell.clear();
                }
            }
            Event::End(TagEnd::TableCell) => {
                if let Some(t) = &mut table {
                    t.row.push(std::mem::take(&mut t.cell));
                }
            }
            Event::End(TagEnd::TableHead | TagEnd::TableRow) => {
                if let Some(t) = &mut table {
                    if t.rows.len() >= 2000 {
                        return Err("Markdown table exceeds 2,000 rows".into());
                    }
                    t.rows.push(std::mem::take(&mut t.row));
                }
            }
            Event::End(TagEnd::Table) => {
                if let Some(t) = table.take() {
                    blocks.push(Block {
                        line: lines
                            .partition_point(|v| *v <= t.range.start)
                            .saturating_sub(1),
                        range: t.range,
                        indent: lists.len().saturating_sub(1),
                        quote,
                        kind: BlockKind::Table {
                            align: t.align,
                            rows: t.rows,
                        },
                    });
                }
            }
            Event::Start(Tag::Image { dest_url, .. }) => {
                if table.is_none() {
                    flush(&mut rich, &mut blocks, &lists, quote, &mut anchors);
                }
                image = Some((dest_url.into_string(), String::new(), range));
            }
            Event::End(TagEnd::Image) => {
                if let Some((destination, alt, range)) = image.take() {
                    if let Some(t) = &mut table {
                        t.cell.push(Span {
                            text: format!("[Image: {alt}]"),
                            style,
                            link: None,
                        });
                    } else {
                        let target = resolve_target(&destination);
                        blocks.push(Block {
                            line: lines
                                .partition_point(|v| *v <= range.start)
                                .saturating_sub(1),
                            range,
                            indent: lists.len().saturating_sub(1),
                            quote,
                            kind: BlockKind::Image {
                                alt,
                                destination,
                                target,
                            },
                        });
                    }
                }
            }
            Event::Start(Tag::Emphasis) => {
                emphasis += 1;
                style.emphasis = true;
            }
            Event::End(TagEnd::Emphasis) => {
                emphasis = emphasis.saturating_sub(1);
                style.emphasis = emphasis > 0;
            }
            Event::Start(Tag::Strong) => {
                strong += 1;
                style.strong = true;
            }
            Event::End(TagEnd::Strong) => {
                strong = strong.saturating_sub(1);
                style.strong = strong > 0;
            }
            Event::Start(Tag::Strikethrough) => {
                strike += 1;
                style.strike = true;
            }
            Event::End(TagEnd::Strikethrough) => {
                strike = strike.saturating_sub(1);
                style.strike = strike > 0;
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                if let Some(target) = link_cache.get(dest_url.as_ref()) {
                    link = Some(target.clone());
                } else {
                    if link_cache.len() >= 4096 || link_bytes + dest_url.len() > 4 * 1024 * 1024 {
                        return Err("Markdown links exceed their preview budget".into());
                    }
                    let target = Arc::new(resolve_target(&dest_url));
                    link_bytes += dest_url.len();
                    link_cache.insert(dest_url.into_string(), target.clone());
                    link = Some(target);
                }
            }
            Event::End(TagEnd::Link) => {
                link = None;
            }
            Event::TaskListMarker(checked) => {
                let marker = source.get(range.clone()).unwrap_or("");
                if let Some(index) = marker.find('[') {
                    let offset = range.start + index + 1;
                    if matches!(source.as_bytes().get(offset), Some(b' ' | b'x' | b'X')) {
                        rich.task = Some(Task {
                            checked,
                            byte_offset: offset,
                        });
                        rich.marker = None;
                    }
                }
            }
            Event::Rule => {
                flush(&mut rich, &mut blocks, &lists, quote, &mut anchors);
                blocks.push(Block {
                    line: lines
                        .partition_point(|v| *v <= range.start)
                        .saturating_sub(1),
                    range,
                    indent: 0,
                    quote,
                    kind: BlockKind::Rule,
                });
            }
            Event::Text(text) | Event::Code(text) | Event::InlineHtml(text) | Event::Html(text) => {
                let mut current_style = style;
                // HTML is rendered literally. It is never interpreted or executed.
                current_style.code = matches!(source.as_bytes().get(range.start), Some(b'`'));
                if let Some((_, code, _)) = &mut code {
                    if code.len() + text.len() > 65536 {
                        return Err(
                            "A Markdown code block exceeds 64 KiB; source remains available".into(),
                        );
                    }
                    code.push_str(&text);
                    continue;
                }
                if let Some((_, alt, _)) = &mut image {
                    alt.push_str(&text);
                    continue;
                }
                if rich.spans.is_empty() && rich.range.is_empty() {
                    rich.range = range.clone();
                }
                rich.range.end = rich.range.end.max(range.end);
                if text.len() > 65536
                    || rich.spans.iter().map(|s| s.text.len()).sum::<usize>() + text.len() > 65536
                {
                    return Err(
                        "A Markdown paragraph exceeds 64 KiB; source remains available".into(),
                    );
                }
                if table.as_ref().is_some_and(|t| {
                    t.cell.iter().map(|s| s.text.len()).sum::<usize>() + text.len() > 8192
                }) {
                    return Err("A Markdown table cell exceeds 8 KiB".into());
                }
                let span = Span {
                    text: text.into_string(),
                    style: current_style,
                    link: link.clone(),
                };
                if let Some(t) = &mut table {
                    t.cell.push(span);
                } else {
                    rich.spans.push(span);
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                let span = Span {
                    text: if matches!(source.as_bytes().get(range.start), Some(b'\\' | b' ')) {
                        "\n"
                    } else {
                        " "
                    }
                    .into(),
                    style,
                    link: link.clone(),
                };
                if let Some((_, alt, _)) = &mut image {
                    alt.push(' ');
                } else if let Some(t) = &mut table {
                    t.cell.push(span);
                } else {
                    rich.spans.push(span);
                }
            }
            _ => {}
        }
    }
    if !rich.spans.is_empty() {
        blocks.push(Block {
            line: lines
                .partition_point(|v| *v <= rich.range.start)
                .saturating_sub(1),
            range: rich.range,
            indent: lists.len().saturating_sub(1),
            quote,
            kind: BlockKind::Rich {
                spans: rich.spans,
                heading: rich.heading,
                anchor: rich.anchor,
                marker: rich.marker,
                task: rich.task,
                loose: rich.loose,
            },
        });
    }
    let mut images = HashMap::new();
    let mut decoded = 0;
    for block in &blocks {
        if cancel.load(Ordering::Relaxed) {
            return Err("Markdown preview cancelled".into());
        }
        if let BlockKind::Image {
            destination,
            target,
            ..
        } = &block.kind
        {
            if images.contains_key(destination) {
                continue;
            }
            if images.len() >= 32 || decoded >= MAX_IMAGE_TOTAL {
                warnings.push("Additional images exceed the 32-image/32-MiB preview budget".into());
                break;
            }
            let result = match target {
                LinkTarget::Local { path, .. } => load_image(
                    path,
                    workspace.or_else(|| path.parent()),
                    MAX_IMAGE_TOTAL - decoded,
                ),
                LinkTarget::External(_) => {
                    Err("Remote image is blocked; preview never fetches the network".into())
                }
                LinkTarget::Blocked(reason) => Err(reason.clone()),
                LinkTarget::Anchor(_) => Err("An image must reference a local file".into()),
            };
            if let Ok(image) = &result {
                decoded += image.pixels.len() * 4;
            }
            images.insert(destination.clone(), result.map(Arc::new));
        }
    }
    let span_count = blocks
        .iter()
        .map(|block| match &block.kind {
            BlockKind::Rich { spans, .. } => spans.len(),
            BlockKind::Table { rows, .. } => rows.iter().flatten().map(Vec::len).sum(),
            _ => 0,
        })
        .sum::<usize>();
    let cost_bytes = source.len() * 4
        + decoded
        + link_bytes * 2
        + span_count * std::mem::size_of::<Span>()
        + blocks.len() * std::mem::size_of::<Block>();
    Ok(Parsed {
        read_only: remote,
        blocks,
        images,
        warnings,
        cost_bytes,
    })
}

pub(super) fn slug(text: &str) -> String {
    let mut value = String::new();
    for ch in text.chars().flat_map(char::to_lowercase) {
        if ch.is_alphanumeric() || ch == '_' || ch == '-' {
            value.push(ch);
        } else if ch.is_whitespace() {
            value.push('-');
        }
    }
    value
}
fn decode(value: &str) -> Result<String, String> {
    let mut bytes = Vec::with_capacity(value.len());
    let mut i = 0;
    let raw = value.as_bytes();
    while i < raw.len() {
        if raw[i] == b'%' {
            if i + 2 >= raw.len() {
                return Err("Invalid escaped link".into());
            }
            let pair =
                std::str::from_utf8(&raw[i + 1..i + 3]).map_err(|_| "Invalid escaped link")?;
            bytes.push(u8::from_str_radix(pair, 16).map_err(|_| "Invalid escaped link")?);
            i += 3;
        } else {
            bytes.push(raw[i]);
            i += 1;
        }
    }
    let text = String::from_utf8(bytes).map_err(|_| "Link is not UTF-8")?;
    if text.chars().any(char::is_control) {
        return Err("Control characters are not allowed in links".into());
    }
    Ok(text)
}
pub(super) fn resolve_link(value: &str, path: &Path, workspace: Option<&Path>) -> LinkTarget {
    match resolve(value, path, workspace) {
        Ok(target) => target,
        Err(reason) => LinkTarget::Blocked(reason),
    }
}
fn remote_link(s: &str) -> LinkTarget {
    let lower = s.to_ascii_lowercase();
    if (lower.starts_with("https://")
        || lower.starts_with("http://")
        || lower.starts_with("mailto:"))
        && !s.chars().any(char::is_control)
    {
        LinkTarget::External(s.into())
    } else if let Some(a) = s.strip_prefix('#') {
        LinkTarget::Anchor(a.into())
    } else {
        LinkTarget::Blocked("Conteúdo Jira não acessa arquivos locais nem executa comandos".into())
    }
}
pub(super) fn validate_local_path(
    target: &Path,
    document: &Path,
    workspace: Option<&Path>,
) -> Result<PathBuf, String> {
    let boundary = workspace
        .or_else(|| document.parent())
        .ok_or("No Markdown resource boundary")?
        .canonicalize()
        .map_err(|_| "Markdown resource boundary is unavailable")?;
    let actual = target
        .canonicalize()
        .map_err(|_| "Local Markdown resource is unavailable")?;
    if !actual.starts_with(&boundary) || !actual.is_file() {
        return Err("Local Markdown resource is outside the project".into());
    }
    let mut current = PathBuf::new();
    for component in target.components() {
        current.push(component.as_os_str());
        if fs::symlink_metadata(&current).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            return Err("Local Markdown resource became a symbolic link".into());
        }
    }
    Ok(actual)
}
fn resolve(value: &str, path: &Path, workspace: Option<&Path>) -> Result<LinkTarget, String> {
    if value.len() > 8192 {
        return Err("Link exceeds 8 KiB".into());
    }
    let value = value.trim();
    if let Some(anchor) = value.strip_prefix('#') {
        return Ok(LinkTarget::Anchor(decode(anchor)?));
    }
    let lower = value.to_ascii_lowercase();
    if lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("mailto:")
    {
        if value.chars().any(char::is_control) || value.contains('\\') {
            return Err("Invalid external URL".into());
        }
        return Ok(LinkTarget::External(value.into()));
    }
    if value.starts_with("//") || value.contains(':') {
        return Err("This link scheme is not allowed".into());
    }
    let (name, anchor) = value
        .split_once('#')
        .map_or((value, None), |(p, a)| (p, Some(a)));
    if name.contains('?') {
        return Err("Local link query strings are not supported".into());
    }
    let name = decode(name)?;
    let candidate = Path::new(&name);
    if name.is_empty() || candidate.is_absolute() || name.contains('\\') {
        return Err("Local links must use a relative project path".into());
    }
    let boundary = workspace
        .or_else(|| path.parent())
        .ok_or("No Markdown directory is available")?
        .canonicalize()
        .map_err(|_| "Markdown directory is unavailable")?;
    let candidate = path
        .parent()
        .ok_or("No Markdown directory")?
        .join(candidate);
    // Canonical containment is checked after resolution, including ../ and links.
    let canonical = candidate
        .canonicalize()
        .map_err(|_| "Local resource is unavailable")?;
    if !canonical.starts_with(&boundary) || !canonical.is_file() {
        return Err("Local resource is outside the project or is not a regular file".into());
    }
    // Reject symlink components instead of silently reading a mutable alias.
    let mut current = boundary.clone();
    for component in canonical
        .strip_prefix(&boundary)
        .map_err(|_| "Resource escaped the project")?
        .components()
    {
        if let Component::Normal(part) = component {
            current.push(part);
            if fs::symlink_metadata(&current)
                .map_err(|_| "Local resource changed")?
                .file_type()
                .is_symlink()
            {
                return Err("Symbolic link resources are not loaded".into());
            }
        }
    }
    // Check the originally requested spelling too, before canonicalization removed links.
    let mut current = PathBuf::new();
    for component in candidate.components() {
        current.push(component.as_os_str());
        if fs::symlink_metadata(&current).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err("Symbolic link resources are not loaded".into());
        }
    }
    Ok(LinkTarget::Local {
        path: canonical,
        anchor: anchor.map(decode).transpose()?,
    })
}
fn load_image(
    path: &Path,
    boundary: Option<&Path>,
    remaining: usize,
) -> Result<egui::ColorImage, String> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK);
    }
    let file = options
        .open(path)
        .map_err(|_| "Cannot open the local image safely")?;
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;
        let actual = PathBuf::from(format!("/proc/self/fd/{}", file.as_raw_fd()))
            .canonicalize()
            .map_err(|_| "Cannot verify the opened image")?;
        let boundary = boundary
            .ok_or("No Markdown resource boundary")?
            .canonicalize()
            .map_err(|_| "Resource boundary is unavailable")?;
        if !actual.starts_with(boundary) {
            return Err("Opened image escaped the project".into());
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = boundary;
    let metadata = file
        .metadata()
        .map_err(|_| "Cannot inspect the local image")?;
    if !metadata.is_file() || metadata.len() > MAX_IMAGE_BYTES {
        return Err("Image exceeds 8 MiB or is not a regular file".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_IMAGE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read the local image")?;
    if bytes.len() as u64 > MAX_IMAGE_BYTES {
        return Err("Image grew beyond the preview budget".into());
    }
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| "Unknown image format")?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some((remaining as u64).min(MAX_PIXELS * 4));
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|_| "Image format is unavailable, damaged, or exceeds the decoded budget")?;
    if u64::from(image.width()) * u64::from(image.height()) > MAX_PIXELS {
        return Err("Image exceeds 4 million pixels".into());
    }
    let image = image.to_rgba8();
    if image.len() > remaining {
        return Err("Decoded images exceed 32 MiB".into());
    }
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        [image.width() as usize, image.height() as usize],
        image.as_raw(),
    ))
}
