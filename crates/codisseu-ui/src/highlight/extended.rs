//! Own, bounded lexical colors. One streaming scanner serves viewport layout and
//! background line continuation; this is intentionally not a semantic provider.
use super::*;
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Language {
    Json,
    Jsonc,
    Xml,
    Python,
    Rust,
    Yaml,
    Toml,
    Ini,
    Env,
    Ignore,
    Attributes,
    Css,
    Shell,
    Docker,
    C,
    Java,
    Go,
}
impl Language {
    pub fn for_key(key: &str) -> Option<Self> {
        Some(
            match key.trim_start_matches('.').to_ascii_lowercase().as_str() {
                "json" | "jsonl" | "ndjson" => Self::Json,
                "jsonc" | "code-workspace" => Self::Jsonc,
                "xml" | "xsd" | "xsl" | "xslt" | "svg" | "html" | "htm" | "xhtml" => Self::Xml,
                "py" | "pyi" | "pyw" => Self::Python,
                "rs" | "rust" => Self::Rust,
                "yml" | "yaml" => Self::Yaml,
                "toml" => Self::Toml,
                "ini" | "cfg" | "conf" | "properties" | "editorconfig" => Self::Ini,
                "env" => Self::Env,
                "ignore" | "gitignore" | "dockerignore" => Self::Ignore,
                "attributes" | "gitattributes" => Self::Attributes,
                "css" | "scss" | "sass" | "less" => Self::Css,
                "sh" | "bash" | "zsh" | "fish" => Self::Shell,
                "dockerfile" => Self::Docker,
                "c" | "h" | "cc" | "cpp" | "hpp" | "cxx" | "hxx" => Self::C,
                "java" | "kt" | "kts" => Self::Java,
                "go" => Self::Go,
                _ => return None,
            },
        )
    }
    fn config(self) -> bool {
        matches!(self, Self::Yaml | Self::Toml | Self::Ini | Self::Env)
    }
    fn slash_comments(self) -> bool {
        matches!(
            self,
            Self::Jsonc | Self::Rust | Self::C | Self::Java | Self::Go
        )
    }
    fn hash_comments(self) -> bool {
        matches!(
            self,
            Self::Python
                | Self::Yaml
                | Self::Toml
                | Self::Ini
                | Self::Env
                | Self::Shell
                | Self::Docker
                | Self::Ignore
                | Self::Attributes
        )
    }
    fn keyword(self, word: &str) -> bool {
        let words = match self {
            Self::Json | Self::Jsonc => "true false null",
            Self::Python => {
                "False None True and as assert async await break class continue def del elif else except finally for from global if import in is lambda nonlocal not or pass raise return try while with yield match case"
            }
            Self::Rust => {
                "as async await break const continue crate dyn else enum extern false fn for if impl in let loop match mod move mut pub ref return self Self static struct super trait true type union unsafe use where while gen yield try bool char str u8 u16 u32 u64 u128 usize i8 i16 i32 i64 i128 isize f16 f32 f64 f128"
            }
            Self::Yaml => "true false null yes no on off True False Null TRUE FALSE NULL",
            Self::Toml | Self::Ini | Self::Env => "true false null unset export",
            Self::Shell => {
                "if then else elif fi for while until do done case esac in function select time export local readonly declare return break continue source"
            }
            Self::Docker => {
                "FROM RUN CMD LABEL MAINTAINER EXPOSE ENV ADD COPY ENTRYPOINT VOLUME USER WORKDIR ARG ONBUILD STOPSIGNAL HEALTHCHECK SHELL AS"
            }
            Self::C => {
                "auto bool break case char class const constexpr continue default delete do double else enum explicit extern false float for friend if inline int long namespace new nullptr operator private protected public register return short signed sizeof static struct switch template this throw true try typedef typename union unsigned using virtual void volatile while"
            }
            Self::Java => {
                "abstract assert boolean break byte case catch char class const continue default do double else enum extends false final finally float for if implements import instanceof int interface long native new null package private protected public return short static strictfp super switch synchronized this throw throws transient true try var void volatile while fun val data object override when is as companion suspend"
            }
            Self::Go => {
                "break case chan const continue default defer else fallthrough false for func go goto if import interface map nil package range return select struct switch true type var bool byte rune string error int int8 int16 int32 int64 uint uint8 uint16 uint32 uint64 uintptr float32 float64 complex64 complex128"
            }
            _ => "",
        };
        words.split_ascii_whitespace().any(|w| w == word)
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Mode {
    #[default]
    Normal,
    Block(u16),
    XmlComment,
    Cdata,
    Quote {
        quote: char,
        triple: bool,
        escaped: bool,
        raw_hashes: Option<u8>,
    },
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct State {
    mode: Mode,
    tag: bool,
    element: bool,
    declaration: bool,
    type_declaration: bool,
    doc_comment: bool,
    value: bool,
    line_content: bool,
    spaced: bool,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Text,
    Keyword,
    String,
    Number,
    Comment,
    DocComment,
    Key,
    Function,
    Variable,
}
impl Kind {
    fn color(self) -> Color32 {
        let p = palette();
        match self {
            Self::Text => p.text,
            Self::Keyword => p.keyword,
            Self::String => p.string,
            Self::Number => p.number,
            Self::Comment => p.comment,
            Self::DocComment => p.doc_comment,
            Self::Key => p.function,
            Self::Function => p.function,
            Self::Variable => p.variable,
        }
    }
}
struct Input<I> {
    chars: I,
    ahead: VecDeque<char>,
    byte: usize,
    checks: usize,
}
impl<I: Iterator<Item = char>> Input<I> {
    fn peek(&mut self, n: usize) -> Option<char> {
        while self.ahead.len() <= n {
            self.ahead.push_back(self.chars.next()?);
        }
        self.ahead.get(n).copied()
    }
    fn starts(&mut self, s: &str) -> bool {
        s.chars().enumerate().all(|(i, c)| self.peek(i) == Some(c))
    }
    fn take(&mut self) -> Option<char> {
        let c = self.ahead.pop_front().or_else(|| self.chars.next())?;
        self.byte += c.len_utf8();
        self.checks += 1;
        Some(c)
    }
    fn skip(&mut self, count: usize) {
        for _ in 0..count {
            self.take();
        }
    }
    fn following_separator(&mut self) -> Option<char> {
        // Bounded lookahead; no unbounded buffering for adversarial whitespace.
        for i in 0..256 {
            let c = self.peek(i)?;
            if !matches!(c, ' ' | '\t') {
                return Some(c);
            }
        }
        None
    }
}
fn ident(c: char) -> bool {
    c == '_' || c.is_alphanumeric() || matches!(c, '\u{0300}'..='\u{036f}')
}

pub(crate) fn scan(
    chars: impl Iterator<Item = char>,
    language: Language,
    mut state: State,
    mut emit: impl FnMut(std::ops::Range<usize>, Kind),
    mut cancel: impl FnMut() -> bool,
) -> Option<State> {
    let mut input = Input {
        chars,
        ahead: VecDeque::new(),
        byte: 0,
        checks: 0,
    };
    while let Some(c) = input.peek(0) {
        if input.checks >= 4096 {
            input.checks = 0;
            if cancel() {
                return None;
            }
        }
        let start = input.byte;
        let kind = match state.mode {
            Mode::Block(depth) => {
                let doc_comment = state.doc_comment;
                if input.starts("*/") {
                    input.skip(2);
                    state.mode = if depth <= 1 {
                        state.doc_comment = false;
                        Mode::Normal
                    } else {
                        Mode::Block(depth - 1)
                    };
                } else if language == Language::Rust && input.starts("/*") {
                    input.skip(2);
                    state.mode = Mode::Block(depth.saturating_add(1));
                } else {
                    input.take();
                }
                if doc_comment {
                    Kind::DocComment
                } else {
                    Kind::Comment
                }
            }
            Mode::XmlComment | Mode::Cdata => {
                let comment = state.mode == Mode::XmlComment;
                let end = if comment { "-->" } else { "]]>" };
                if input.starts(end) {
                    input.skip(3);
                    state.mode = Mode::Normal;
                } else {
                    input.take();
                }
                if comment { Kind::Comment } else { Kind::String }
            }
            Mode::Quote {
                quote,
                triple,
                escaped,
                raw_hashes,
            } => {
                input.take();
                if escaped {
                    state.mode = Mode::Quote {
                        quote,
                        triple,
                        escaped: false,
                        raw_hashes,
                    };
                } else if c == '\\'
                    && raw_hashes.is_none()
                    && !(language == Language::Toml && quote == '\'')
                    && !(language == Language::Shell && quote == '\'')
                    && language != Language::Xml
                {
                    state.mode = Mode::Quote {
                        quote,
                        triple,
                        escaped: true,
                        raw_hashes,
                    };
                } else if c == quote {
                    let closes = if let Some(hashes) = raw_hashes {
                        (0..hashes as usize).all(|i| input.peek(i) == Some('#'))
                    } else {
                        !triple || input.peek(0) == Some(quote) && input.peek(1) == Some(quote)
                    };
                    if closes {
                        input.skip(raw_hashes.map_or(if triple { 2 } else { 0 }, usize::from));
                        state.mode = Mode::Normal;
                    }
                }
                Kind::String
            }
            Mode::Normal => {
                if c == '\n' || c == '\r' {
                    input.take();
                    state.value = false;
                    state.line_content = false;
                    if language != Language::Rust {
                        state.declaration = false;
                    }
                    state.spaced = true;
                    Kind::Text
                } else if c.is_whitespace() {
                    input.take();
                    state.spaced = true;
                    Kind::Text
                } else if language == Language::Xml && input.starts("<!--") {
                    input.skip(4);
                    state.mode = Mode::XmlComment;
                    Kind::Comment
                } else if language == Language::Xml && input.starts("<![CDATA[") {
                    input.skip(9);
                    state.mode = Mode::Cdata;
                    Kind::String
                } else if language == Language::Xml && c == '<' {
                    input.take();
                    state.tag = true;
                    state.element = true;
                    Kind::Keyword
                } else if language == Language::Xml && c == '>' {
                    input.take();
                    state.tag = false;
                    state.element = false;
                    Kind::Keyword
                } else if language == Language::Xml && !state.tag {
                    input.take();
                    Kind::Text
                } else if (language.slash_comments() || language == Language::Css)
                    && input.starts("/*")
                {
                    state.doc_comment = language == Language::Rust
                        && (input.starts("/**") && !input.starts("/***") || input.starts("/*!"));
                    input.skip(2);
                    state.mode = Mode::Block(1);
                    if state.doc_comment {
                        Kind::DocComment
                    } else {
                        Kind::Comment
                    }
                } else if (language.slash_comments() && input.starts("//"))
                    || (language.hash_comments()
                        && c == '#'
                        && (language != Language::Docker || !state.line_content)
                        && (!matches!(language, Language::Shell | Language::Env | Language::Yaml)
                            || state.spaced
                            || !state.line_content))
                    || (language == Language::Ini && c == ';')
                {
                    let doc_comment = language == Language::Rust
                        && (input.starts("///") && !input.starts("////") || input.starts("//!"));
                    while input.peek(0).is_some_and(|c| c != '\n' && c != '\r') {
                        input.take();
                        if input.checks >= 4096 {
                            input.checks = 0;
                            if cancel() {
                                return None;
                            }
                        }
                    }
                    if doc_comment {
                        Kind::DocComment
                    } else {
                        Kind::Comment
                    }
                } else if matches!(language, Language::Ignore | Language::Attributes) {
                    input.take();
                    state.line_content = true;
                    if matches!(c, '!' | '*' | '?' | '[' | ']' | '=') {
                        Kind::Keyword
                    } else {
                        Kind::String
                    }
                } else if language == Language::Rust
                    && (c == 'r' && matches!(input.peek(1), Some('#' | '"'))
                        || matches!(c, 'b' | 'c')
                            && input.peek(1) == Some('r')
                            && matches!(input.peek(2), Some('#' | '"')))
                {
                    let prefix = if c == 'r' { 1 } else { 2 };
                    let mut hashes = 0;
                    while hashes < 255 && input.peek(hashes + prefix) == Some('#') {
                        hashes += 1;
                    }
                    if input.peek(hashes + prefix) == Some('"') {
                        input.skip(hashes + prefix + 1);
                        state.mode = Mode::Quote {
                            quote: '"',
                            triple: false,
                            escaped: false,
                            raw_hashes: Some(hashes as u8),
                        };
                        Kind::String
                    } else if prefix == 1 && hashes == 1 && input.peek(2).is_some_and(ident) {
                        input.skip(2);
                        while input.peek(0).is_some_and(ident) {
                            input.take();
                            if input.checks >= 4096 {
                                input.checks = 0;
                                if cancel() {
                                    return None;
                                }
                            }
                        }
                        let declaration = state.declaration && !state.type_declaration;
                        state.declaration = false;
                        state.type_declaration = false;
                        if declaration || input.following_separator() == Some('(') {
                            Kind::Function
                        } else {
                            Kind::Text
                        }
                    } else {
                        // An ordinary identifier beginning with r/br/cr must be
                        // scanned as a whole, including function declaration state.
                        input.take();
                        while input.peek(0).is_some_and(ident) {
                            input.take();
                            if input.checks >= 4096 {
                                input.checks = 0;
                                if cancel() {
                                    return None;
                                }
                            }
                        }
                        let declaration = state.declaration && !state.type_declaration;
                        state.declaration = false;
                        state.type_declaration = false;
                        if declaration || input.following_separator() == Some('(') {
                            Kind::Function
                        } else {
                            Kind::Text
                        }
                    }
                } else if language == Language::Rust
                    && matches!(c, 'b' | 'c')
                    && (input.peek(1) == Some('"') || c == 'b' && input.peek(1) == Some('\''))
                {
                    input.take();
                    let quote = input.take().unwrap();
                    state.mode = Mode::Quote {
                        quote,
                        triple: false,
                        escaped: false,
                        raw_hashes: None,
                    };
                    Kind::String
                } else if c == '\''
                    && language == Language::Rust
                    && input.peek(1).is_some_and(ident)
                    && input.peek(2) != Some('\'')
                {
                    input.take();
                    while input.peek(0).is_some_and(ident) {
                        input.take();
                    }
                    Kind::Variable
                } else if c == '"'
                    || c == '\'' && !matches!(language, Language::Json | Language::Jsonc)
                    || c == '`' && matches!(language, Language::Shell | Language::Go)
                {
                    input.take();
                    let triple =
                        matches!(language, Language::Python | Language::Toml | Language::Java)
                            && input.peek(0) == Some(c)
                            && input.peek(1) == Some(c);
                    if triple {
                        input.skip(2);
                    }
                    state.mode = Mode::Quote {
                        quote: c,
                        triple,
                        escaped: false,
                        raw_hashes: if c == '`' { Some(0) } else { None },
                    };
                    state.line_content = true;
                    // Consume the complete token so its JSON/YAML key can have a distinct color.
                    let mut escaped = false;
                    while let Some(next) = input.peek(0) {
                        if !triple
                            && matches!(next, '\n' | '\r')
                            && !matches!(language, Language::Rust | Language::Go)
                        {
                            break;
                        }
                        if input.checks >= 4096 {
                            input.checks = 0;
                            if cancel() {
                                return None;
                            }
                        }
                        input.take();
                        if escaped {
                            escaped = false;
                            continue;
                        }
                        if next == '\\'
                            && c != '`'
                            && language != Language::Xml
                            && !(c == '\'' && matches!(language, Language::Toml | Language::Shell))
                        {
                            escaped = true;
                            continue;
                        }
                        if next == c
                            && (!triple || input.peek(0) == Some(c) && input.peek(1) == Some(c))
                        {
                            if triple {
                                input.skip(2);
                            }
                            state.mode = Mode::Normal;
                            break;
                        }
                    }
                    if let Mode::Quote { escaped: e, .. } = &mut state.mode {
                        *e = escaped;
                    }
                    if state.mode == Mode::Normal
                        && matches!(
                            language,
                            Language::Json | Language::Jsonc | Language::Yaml | Language::Toml
                        )
                        && matches!(input.following_separator(), Some(':' | '='))
                    {
                        Kind::Key
                    } else {
                        Kind::String
                    }
                } else if matches!(language, Language::Toml | Language::Ini)
                    && c == '['
                    && !state.line_content
                {
                    while input.peek(0).is_some_and(|c| c != '\n' && c != '\r') {
                        let c = input.take();
                        if c == Some(']') {
                            break;
                        }
                        if input.checks >= 4096 {
                            input.checks = 0;
                            if cancel() {
                                return None;
                            }
                        }
                    }
                    state.line_content = true;
                    Kind::Key
                } else if c == '$'
                    && matches!(language, Language::Shell | Language::Env | Language::Docker)
                {
                    input.take();
                    if input.peek(0) == Some('{') {
                        input.take();
                    }
                    while input.peek(0).is_some_and(ident) {
                        input.take();
                    }
                    if input.peek(0) == Some('}') {
                        input.take();
                    }
                    Kind::Variable
                } else if (c.is_ascii_digit()
                    || c == '-' && input.peek(1).is_some_and(|c| c.is_ascii_digit()))
                    && !(language.config() && language != Language::Yaml && !state.value)
                {
                    input.take();
                    while input.peek(0).is_some_and(|c| {
                        c.is_ascii_alphanumeric()
                            || c == '_'
                            || c == '.'
                                && (language != Language::Rust
                                    || input.peek(1).is_some_and(|n| n.is_ascii_digit()))
                    }) {
                        let digit = input.take();
                        if matches!(digit, Some('e' | 'E'))
                            && matches!(input.peek(0), Some('+' | '-'))
                        {
                            input.take();
                        }
                        if input.checks >= 4096 {
                            input.checks = 0;
                            if cancel() {
                                return None;
                            }
                        }
                    }
                    state.line_content = true;
                    Kind::Number
                } else if ident(c) || c == '@' || language.config() && matches!(c, '.' | '-' | '*')
                {
                    // Only bounded spelling is retained; long identifiers do not allocate without limit.
                    let mut word = String::new();
                    while input.peek(0).is_some_and(|c| {
                        ident(c)
                            || matches!(language, Language::Xml) && matches!(c, ':' | '-' | '.')
                            || language.config() && matches!(c, '.' | '-' | '*')
                    }) {
                        let c = input.take().unwrap();
                        if word.len() < 1024 {
                            word.push(c);
                        }
                        if input.checks >= 4096 {
                            input.checks = 0;
                            if cancel() {
                                return None;
                            }
                        }
                    }
                    if input.byte == start {
                        input.take();
                    }
                    let declaration = state.declaration;
                    let type_declaration = state.type_declaration;
                    state.type_declaration = language == Language::Rust
                        && matches!(
                            word.as_str(),
                            "struct" | "enum" | "trait" | "type" | "union"
                        );
                    state.declaration = matches!(
                        (language, word.as_str()),
                        (Language::Python, "def" | "class")
                            | (
                                Language::Rust,
                                "fn" | "struct" | "enum" | "trait" | "type" | "union"
                            )
                            | (Language::Go, "func")
                            | (Language::Java, "class" | "interface")
                    );
                    state.line_content = true;
                    if language == Language::Xml {
                        if state.element {
                            state.element = false;
                            Kind::Keyword
                        } else {
                            Kind::Variable
                        }
                    } else if language.keyword(&word)
                        || language == Language::Rust && input.peek(0) == Some('!')
                    {
                        Kind::Keyword
                    } else if declaration && !(language == Language::Rust && type_declaration)
                        || language == Language::Rust
                            && input.following_separator() == Some('(')
                            && !type_declaration
                    {
                        Kind::Function
                    } else if (language.config() && !state.value || language == Language::Css)
                        && matches!(input.following_separator(), Some(':' | '='))
                    {
                        Kind::Key
                    } else if language.config() && state.value {
                        Kind::String
                    } else {
                        Kind::Text
                    }
                } else {
                    input.take();
                    if language.config() && matches!(c, ':' | '=') {
                        state.value = true;
                    }
                    if !matches!(c, '/' | '?' | '!') || language != Language::Xml {
                        state.line_content = true;
                    }
                    Kind::Text
                }
            }
        };
        if !c.is_whitespace() {
            state.spaced = false;
        }
        emit(start..input.byte, kind);
    }
    Some(state)
}

pub(crate) fn layout(text: &str, language: Language, font: FontId, state: State) -> LayoutJob {
    let mut job = LayoutJob::default();
    if text.len() > MAX_HIGHLIGHT_BYTES {
        append(&mut job, text, &font, palette().text);
        return job;
    }
    let mut last: Option<(std::ops::Range<usize>, Kind)> = None;
    let mut limited = false;
    scan(
        text.chars(),
        language,
        state,
        |range, mut kind| {
            if job.sections.len() >= MAX_SPANS - 2 {
                limited = true;
            }
            if limited {
                kind = Kind::Text;
            }
            if let Some((previous, previous_kind)) = &mut last {
                if *previous_kind == kind {
                    previous.end = range.end;
                    return;
                }
                append(
                    &mut job,
                    &text[previous.clone()],
                    &font,
                    previous_kind.color(),
                );
            }
            last = Some((range, kind));
        },
        || false,
    );
    if let Some((range, kind)) = last {
        append(&mut job, &text[range], &font, kind.color());
    }
    job
}

#[cfg(test)]
mod tests {
    use super::*;
    fn job(source: &str, key: &str) -> LayoutJob {
        let job = super::super::layout(source, key, FontId::monospace(13.));
        assert_eq!(job.text, source);
        let mut at = 0;
        for section in &job.sections {
            let range = usize::from(section.byte_range.start)..usize::from(section.byte_range.end);
            assert_eq!(range.start, at);
            assert!(source.is_char_boundary(range.start));
            assert!(source.is_char_boundary(range.end));
            at = range.end;
        }
        assert_eq!(at, source.len());
        assert!(job.sections.len() <= MAX_SPANS);
        job
    }
    fn color(job: &LayoutJob, needle: &str) -> Color32 {
        let at = job.text.find(needle).unwrap();
        job.sections
            .iter()
            .find(|s| s.byte_range.contains(&at.into()))
            .unwrap()
            .format
            .color
    }
    #[test]
    fn special_names_are_recognized_before_extensions() {
        for (name, key) in [
            (".env", "env"),
            (".env.local", "env"),
            (".env.production", "env"),
            (".gitignore", "ignore"),
            (".dockerignore", "ignore"),
            (".gitattributes", "attributes"),
            (".editorconfig", "ini"),
            ("Dockerfile", "dockerfile"),
            ("Dockerfile.dev", "dockerfile"),
            ("a.RS", "rs"),
            ("a.YML", "yml"),
        ] {
            assert_eq!(syntax_key(std::path::Path::new(name)), key);
        }
    }
    #[test]
    fn json_keys_values_literals_escapes_and_exponents_are_distinct() {
        let j = job(
            "{\r\n  \"ação😀\": \"https://example/#x {true} \\\"\",\n  \"n\": -1.5e+2, \"ok\": true, \"missing\": null\n}",
            "json",
        );
        assert_eq!(color(&j, "ação"), palette().function);
        assert_eq!(color(&j, "https"), palette().string);
        assert_eq!(color(&j, "-1.5e+2"), palette().number);
        assert_eq!(color(&j, "e+2"), palette().number);
        assert_eq!(color(&j, "true, \"missing"), palette().keyword);
        assert_eq!(color(&j, "null"), palette().keyword);
        let j = job("// true\n{\"ok\":false} /* comment */", "jsonc");
        assert_eq!(color(&j, "// true"), palette().comment);
        assert_eq!(color(&j, "/* comment"), palette().comment);
        let j = job("// true", "json");
        assert_ne!(color(&j, "// true"), palette().comment);
    }
    #[test]
    fn python_rust_and_markup_multiline_tokens_ignore_fake_code() {
        let p = job(
            "def café():\n    text = \"\"\"def fake\n# still string\n\"\"\"\n    return True # real comment",
            "py",
        );
        assert_eq!(color(&p, "def café"), palette().keyword);
        assert_eq!(color(&p, "café"), palette().function);
        assert_eq!(color(&p, "def fake"), palette().string);
        assert_eq!(color(&p, "# still"), palette().string);
        assert_eq!(color(&p, "# real"), palette().comment);
        let r = job(
            "pub fn café<'a>(s: &'a str) {\n let x = r##\"first \" // not comment\nend\"##; /* outer /* nested */ still comment */ let n = 42; }",
            "rs",
        );
        assert_eq!(color(&r, "café"), palette().function);
        assert_eq!(color(&r, "'a"), palette().variable);
        assert_eq!(color(&r, "// not"), palette().string);
        assert_eq!(color(&r, "still comment"), palette().comment);
        assert_eq!(color(&r, "42"), palette().number);
        let x = job(
            "<?xml version=\"1.0\"?>\n<root enabled=\"true\"><!-- <fake>\n still comment -->ação<![CDATA[<tag>]]></root>",
            "xml",
        );
        assert_eq!(color(&x, "root"), palette().keyword);
        assert_eq!(color(&x, "enabled"), palette().variable);
        assert_eq!(color(&x, "\"true\""), palette().string);
        assert_eq!(color(&x, "<fake>"), palette().comment);
        assert_eq!(color(&x, "ação"), palette().text);
        assert_eq!(color(&x, "<tag>"), palette().string);
    }
    #[test]
    fn rust_calls_raw_identifiers_byte_strings_ranges_and_doc_comments() {
        let source = concat!(
            "/// Documentação 😀\r\n",
            "pub struct Weather<'a> { value: &'a str }\r\n",
            "pub fn\r\nforecast<T>(value: T) { return; }\r\n",
            "fn r#match() { let r#type = 1..=20; let n = 1.5e+2_f64;\r\n",
            "    forecast(42); weather.forecast(); r#match();\r\n",
            "    let bytes = br##\"a \" // still bytes\r\nend\"##;\r\n",
            "    let c = c\"CString\"; let b = b'X';\r\n",
            "    /** Outer /* nested */ still docs */\r\n",
            "    println!(\"ok\");\r\n",
            "}\r\n"
        );
        let r = job(source, "rs");
        for needle in ["forecast<T>", "forecast(42)", "forecast();", "r#match()"] {
            assert_eq!(color(&r, needle), palette().function, "{needle}");
        }
        assert_eq!(
            color(&r, "Weather"),
            palette().text,
            "type name is not a function"
        );
        assert_eq!(color(&r, "return"), palette().keyword);
        assert_eq!(
            color(&r, "r#type"),
            palette().text,
            "raw identifier is not a keyword"
        );
        assert_eq!(
            color(&r, "..="),
            palette().text,
            "range is not a decimal point"
        );
        assert_eq!(color(&r, "1.5e+2_f64"), palette().number);
        for needle in ["br##", "// still bytes", "end\"##", "c\"CString", "b'X'"] {
            assert_eq!(color(&r, needle), palette().string, "{needle}");
        }
        assert_eq!(color(&r, "Documentação"), palette().doc_comment);
        assert_eq!(color(&r, "still docs"), palette().doc_comment);
        assert_eq!(color(&r, "println!"), palette().keyword);
        // The line-state worker and visible renderer share identical rules.
        let mut state = State::default();
        let mut spans = Vec::new();
        let mut offset = 0;
        for line in source.split_inclusive('\n') {
            state = scan(
                line.chars(),
                Language::Rust,
                state,
                |range, kind| {
                    spans.push((offset + range.start..offset + range.end, kind.color()));
                },
                || false,
            )
            .unwrap();
            offset += line.len();
        }
        for (byte, _) in source.char_indices() {
            let expected = r
                .sections
                .iter()
                .find(|s| s.byte_range.contains(&byte.into()))
                .unwrap()
                .format
                .color;
            assert_eq!(
                spans
                    .iter()
                    .find(|(range, _)| range.contains(&byte))
                    .unwrap()
                    .1,
                expected,
                "at{byte}"
            );
        }
    }
    #[test]
    fn configuration_urls_quoted_hashes_sections_and_globs() {
        let e = job(
            "# demo\nAPP_URL=https://example.test/#fragment\nTOKEN=\"value#kept\" # note\nDEBUG=true\nCOUNT=42\n",
            "env",
        );
        assert_eq!(color(&e, "# demo"), palette().comment);
        assert_eq!(color(&e, "APP_URL"), palette().function);
        assert_eq!(color(&e, "https"), palette().string);
        assert_eq!(color(&e, "#fragment"), palette().text);
        assert_eq!(color(&e, "value#kept"), palette().string);
        assert_eq!(color(&e, "# note"), palette().comment);
        assert_eq!(color(&e, "true"), palette().keyword);
        assert_eq!(color(&e, "42"), palette().number);
        for key in ["yaml", "yml", "toml", "ini"] {
            let c = job(
                "# comment\n[package]\nname = \"Codisseu\"\nenabled: true\n",
                key,
            );
            assert_eq!(color(&c, "comment"), palette().comment);
            assert_eq!(color(&c, "Codisseu"), palette().string);
            assert_eq!(color(&c, "enabled"), palette().function);
        }
        let y = job("ports:\n  - 8080\n  - -5\n", "yaml");
        assert_eq!(color(&y, "8080"), palette().number);
        assert_eq!(color(&y, "-5"), palette().number);
        let i = job("# ignored\nnode_modules/\n!keep.json\n*.log\n", "ignore");
        assert_eq!(color(&i, "ignored"), palette().comment);
        assert_eq!(color(&i, "node_modules"), palette().string);
        assert_eq!(color(&i, "!"), palette().keyword);
        assert_eq!(color(&i, "*."), palette().keyword);
    }
    #[test]
    fn all_formats_preserve_unicode_crlf_and_have_basic_colors() {
        for (key, source) in [
            (
                "css",
                ".card { color: \"red\"; margin: 10px; /* comment */ }",
            ),
            (
                "sh",
                "#!/bin/sh\nexport NAME=\"ação😀\"\nif true; then echo $NAME; fi",
            ),
            ("dockerfile", "# demo\nFROM rust:1 AS build\nWORKDIR /app"),
            ("cpp", "// demo\nclass Test { public: int n = 42; };"),
            ("java", "class Test { public String text = \"ação😀\"; }"),
            ("go", "package main\nfunc main() { var n = 42; }"),
            ("attributes", "# demo\n*.php text eol=lf"),
        ] {
            let j = job(&format!("\u{feff}{source}\r\n"), key);
            assert!(
                j.sections.iter().any(|s| s.format.color != palette().text),
                "{key}"
            );
        }
        for key in [
            "json", "xml", "py", "rs", "yaml", "toml", "ini", "env", "ignore",
        ] {
            job("", key);
            job("\"unterminated😀\r\nα", key);
        }
        assert_eq!(job("def x(): return True", "txt").sections.len(), 1);
    }
    #[test]
    fn limits_and_cancellation_bound_work_without_losing_bytes() {
        let large = "😀".repeat(MAX_HIGHLIGHT_BYTES / 4 + 1);
        assert_eq!(job(&large, "json").sections.len(), 1);
        let alternating = "true;".repeat(MAX_SPANS);
        job(&alternating, "json");
        for (source, key) in [
            ("x".repeat(100_000), "rs"),
            (format!("[{}", "x".repeat(100_000)), "ini"),
            (format!("\"{}", "x".repeat(100_000)), "py"),
        ] {
            let mut checks = 0;
            assert!(
                scan(
                    source.chars(),
                    Language::for_key(key).unwrap(),
                    State::default(),
                    |_, _| {},
                    || {
                        checks += 1;
                        checks > 1
                    }
                )
                .is_none()
            );
            assert!(checks <= 3);
        }
    }
}
