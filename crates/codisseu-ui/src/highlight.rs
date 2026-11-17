//! Lexical highlighting for the shared native editor; not a parser or LSP.
//! No semantic diagnostics, interpolation analysis, heredocs, JS regex literals
//! or embedded-language routing. Every UTF-8 byte is retained in the LayoutJob.
//! Colors are read from the centralized public OSS scheme excerpt, with source,
//! revision and attribution recorded alongside that JSON. They are not local
//! measurements of a proprietary IDE installation.
use egui::{Color32, FontId, TextFormat, text::LayoutJob};
use std::sync::OnceLock;
pub(super) mod extended;

/// A filename can carry the language even when Path::extension is absent or misleading.
pub(super) fn syntax_key(path: &std::path::Path) -> String {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match name.as_str() {
        ".env" => "env".into(),
        ".gitignore" | ".dockerignore" | ".ignore" => "ignore".into(),
        ".gitattributes" => "attributes".into(),
        ".editorconfig" | ".gitconfig" | ".npmrc" | ".yarnrc" => "ini".into(),
        ".babelrc" | ".eslintrc" | ".prettierrc" | ".bowerrc" | "composer.lock" => "json".into(),
        "tsconfig.json" | "jsconfig.json" => "jsonc".into(),
        "cargo.lock" => "toml".into(),
        ".bashrc" | ".bash_profile" | ".profile" | ".zshrc" => "sh".into(),
        "dockerfile" => "dockerfile".into(),
        _ if name.starts_with(".env.") => "env".into(),
        _ if name.starts_with("dockerfile.") || name.ends_with(".dockerfile") => {
            "dockerfile".into()
        }
        _ => path
            .extension()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_ascii_lowercase(),
    }
}

const MAX_HIGHLIGHT_BYTES: usize = 256 * 1024;
const MAX_SPANS: usize = 32_768;

struct Palette {
    text: Color32,
    keyword: Color32,
    string: Color32,
    number: Color32,
    comment: Color32,
    doc_comment: Color32,
    variable: Color32,
    function: Color32,
}

fn palette() -> &'static Palette {
    static PALETTE: OnceLock<Palette> = OnceLock::new();
    PALETTE.get_or_init(|| {
        let source: serde_json::Value = serde_json::from_str(include_str!(
            "../../../reference/public/intellij-oss/palette-excerpt.json"
        ))
        .expect("versioned OSS palette JSON");
        let foreground = |attribute: &str| {
            let hex = source["editor_attributes"][attribute]["FOREGROUND"]
                .as_str()
                .expect("versioned editor foreground");
            let rgb = u32::from_str_radix(hex, 16).expect("versioned RGB hex");
            Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
        };
        Palette {
            text: foreground("TEXT"),
            keyword: foreground("DEFAULT_KEYWORD"),
            string: foreground("DEFAULT_STRING"),
            number: foreground("DEFAULT_NUMBER"),
            comment: foreground("DEFAULT_LINE_COMMENT"),
            doc_comment: super::color(super::Tokens::load().editor_phpdoc),
            variable: super::color(super::Tokens::load().editor_php_variable),
            function: foreground("DEFAULT_FUNCTION_DECLARATION"),
        }
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Language {
    Php,
    JavaScript,
    Sql,
    Plain,
}

impl Language {
    fn from_extension(extension: &str) -> Self {
        match extension
            .trim_start_matches('.')
            .to_ascii_lowercase()
            .as_str()
        {
            "php" | "phtml" => Self::Php,
            "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "mts" | "cts" => Self::JavaScript,
            "sql" => Self::Sql,
            _ => Self::Plain,
        }
    }

    fn keyword(self, word: &str) -> bool {
        let words: &[&str] = match self {
            Self::Php => &[
                "int",
                "float",
                "bool",
                "string",
                "void",
                "mixed",
                "never",
                "object",
                "php",
                "abstract",
                "and",
                "array",
                "as",
                "break",
                "callable",
                "case",
                "catch",
                "class",
                "clone",
                "const",
                "continue",
                "declare",
                "default",
                "die",
                "do",
                "echo",
                "else",
                "elseif",
                "empty",
                "enddeclare",
                "endfor",
                "endforeach",
                "endif",
                "endswitch",
                "endwhile",
                "enum",
                "eval",
                "exit",
                "extends",
                "false",
                "final",
                "finally",
                "fn",
                "for",
                "foreach",
                "function",
                "global",
                "goto",
                "if",
                "implements",
                "include",
                "include_once",
                "instanceof",
                "insteadof",
                "interface",
                "isset",
                "list",
                "match",
                "namespace",
                "new",
                "null",
                "or",
                "parent",
                "print",
                "private",
                "protected",
                "public",
                "readonly",
                "require",
                "require_once",
                "return",
                "self",
                "static",
                "switch",
                "throw",
                "trait",
                "true",
                "try",
                "unset",
                "use",
                "var",
                "while",
                "xor",
                "yield",
            ],
            Self::JavaScript => &[
                "abstract",
                "any",
                "as",
                "asserts",
                "async",
                "await",
                "bigint",
                "boolean",
                "break",
                "case",
                "catch",
                "class",
                "const",
                "constructor",
                "continue",
                "debugger",
                "declare",
                "default",
                "delete",
                "do",
                "else",
                "enum",
                "export",
                "extends",
                "false",
                "finally",
                "for",
                "from",
                "function",
                "get",
                "if",
                "implements",
                "import",
                "in",
                "infer",
                "instanceof",
                "interface",
                "is",
                "keyof",
                "let",
                "module",
                "namespace",
                "never",
                "new",
                "null",
                "number",
                "object",
                "of",
                "override",
                "private",
                "protected",
                "public",
                "readonly",
                "return",
                "satisfies",
                "set",
                "static",
                "string",
                "super",
                "switch",
                "symbol",
                "this",
                "throw",
                "true",
                "try",
                "type",
                "typeof",
                "undefined",
                "unique",
                "unknown",
                "var",
                "void",
                "while",
                "with",
                "yield",
            ],
            Self::Sql => &[
                "ALL",
                "ALTER",
                "AND",
                "AS",
                "ASC",
                "BEGIN",
                "BETWEEN",
                "BY",
                "CASE",
                "CAST",
                "CHECK",
                "COLLATE",
                "COLUMN",
                "COMMIT",
                "CONSTRAINT",
                "CREATE",
                "CROSS",
                "CURRENT_DATE",
                "CURRENT_TIME",
                "CURRENT_TIMESTAMP",
                "DATABASE",
                "DEFAULT",
                "DELETE",
                "DESC",
                "DISTINCT",
                "DROP",
                "ELSE",
                "END",
                "ESCAPE",
                "EXCEPT",
                "EXISTS",
                "FALSE",
                "FOREIGN",
                "FROM",
                "FULL",
                "GROUP",
                "HAVING",
                "IN",
                "INDEX",
                "INNER",
                "INSERT",
                "INTERSECT",
                "INTO",
                "IS",
                "JOIN",
                "KEY",
                "LEFT",
                "LIKE",
                "LIMIT",
                "NOT",
                "NULL",
                "OFFSET",
                "ON",
                "OR",
                "ORDER",
                "OUTER",
                "OVER",
                "PARTITION",
                "PRIMARY",
                "RECURSIVE",
                "REFERENCES",
                "RETURNING",
                "RIGHT",
                "ROLLBACK",
                "ROW",
                "ROWS",
                "SELECT",
                "SET",
                "TABLE",
                "THEN",
                "TRUE",
                "UNION",
                "UNIQUE",
                "UPDATE",
                "USING",
                "VALUES",
                "VIEW",
                "WHEN",
                "WHERE",
                "WINDOW",
                "WITH",
            ],
            Self::Plain => &[],
        };
        if self == Self::JavaScript {
            words.contains(&word)
        } else {
            words
                .iter()
                .any(|candidate| word.eq_ignore_ascii_case(candidate))
        }
    }
}

/// Root's layouter caches this by text, extension and font; no I/O is performed.
pub(super) fn layout(text: &str, extension: &str, font: FontId) -> LayoutJob {
    if let Some(language) = extended::Language::for_key(extension) {
        return extended::layout(text, language, font, extended::State::default());
    }
    let language = Language::from_extension(extension);
    let palette = palette();
    let mut job = LayoutJob::default();
    if language == Language::Plain || text.len() > MAX_HIGHLIGHT_BYTES {
        append(&mut job, text, &font, palette.text);
        return job;
    }
    let mut position = 0;
    let mut function_name = false;
    while position < text.len() {
        // Leave one section for the remainder, preserving bytes and bounding
        // spans even for adversarial punctuation/identifier alternation.
        if job.sections.len() >= MAX_SPANS - 1 {
            append(&mut job, &text[position..], &font, palette.text);
            break;
        }
        let start = position;
        let tail = &text[start..];
        let first = tail.chars().next().expect("nonempty UTF-8 tail");
        let color = if let Some(comment) = tail.strip_prefix("/*") {
            position = start + comment.find("*/").map_or(tail.len(), |end| end + 4);
            if language == Language::Php && tail.starts_with("/**") {
                palette.doc_comment
            } else {
                palette.comment
            }
        } else if (language == Language::Sql && tail.starts_with("--"))
            || (language != Language::Sql && tail.starts_with("//"))
            || (language == Language::Php && first == '#')
        {
            position = start + tail.find('\n').unwrap_or(tail.len());
            palette.comment
        } else if matches!(first, '\'' | '"' | '`') || (language == Language::Sql && first == '[') {
            function_name = false;
            position = string_end(text, start, first, language);
            palette.string
        } else if first.is_ascii_digit()
            || (first == '.' && tail.as_bytes().get(1).is_some_and(u8::is_ascii_digit))
        {
            position = number_end(text, start, language);
            palette.number
        } else if identifier_start(first, language) {
            position += first.len_utf8();
            while let Some(character) = text[position..].chars().next() {
                if !identifier_continue(character, language) {
                    break;
                }
                position += character.len_utf8();
            }
            let word = &text[start..position];
            let declaration = function_name;
            function_name = language != Language::Sql && word == "function";
            if language == Language::Php && word.starts_with('$') {
                palette.variable
            } else if language.keyword(word) {
                palette.keyword
            } else if declaration {
                palette.function
            } else {
                palette.text
            }
        } else {
            if !first.is_whitespace() && !matches!(first, '*' | '&') || first == '\n' {
                function_name = false;
            }
            position += first.len_utf8();
            // Group whitespace; punctuation remains separate but is bounded.
            if first.is_whitespace() {
                while let Some(character) = text[position..].chars().next() {
                    if !character.is_whitespace() {
                        break;
                    }
                    position += character.len_utf8();
                }
            }
            palette.text
        };
        if text[start..position].contains('\n') {
            function_name = false;
        }
        let mut span_font = font.clone();
        if language == Language::Php && color == palette.doc_comment {
            span_font.family = egui::FontFamily::Name("codisseu.mono.italic".into());
        }
        append(&mut job, &text[start..position], &span_font, color);
    }
    job
}

fn append(job: &mut LayoutJob, text: &str, font: &FontId, color: Color32) {
    job.append(
        text,
        0.0,
        TextFormat {
            font_id: font.clone(),
            color,
            ..Default::default()
        },
    );
}

fn identifier_start(character: char, language: Language) -> bool {
    character == '_' || character.is_alphabetic() || (language != Language::Sql && character == '$')
}

fn identifier_continue(character: char, language: Language) -> bool {
    identifier_start(character, language)
        || character.is_alphanumeric()
        || matches!(character, '\u{0300}'..='\u{036f}')
}

fn string_end(text: &str, start: usize, quote: char, language: Language) -> usize {
    let closing = if quote == '[' { ']' } else { quote };
    let mut position = start + quote.len_utf8();
    while let Some(character) = text[position..].chars().next() {
        position += character.len_utf8();
        if character == '\\' && language != Language::Sql {
            if let Some(escaped) = text[position..].chars().next() {
                position += escaped.len_utf8();
            }
        } else if character == closing {
            if language == Language::Sql && text[position..].starts_with(closing) {
                position += closing.len_utf8();
            } else {
                break;
            }
        }
    }
    position
}

fn number_end(text: &str, start: usize, language: Language) -> usize {
    let bytes = text.as_bytes();
    let mut position = start;
    if bytes.get(start) == Some(&b'0')
        && bytes
            .get(start + 1)
            .is_some_and(|b| matches!(b, b'x' | b'X' | b'b' | b'B' | b'o' | b'O'))
    {
        let radix = bytes[start + 1].to_ascii_lowercase();
        position += 2;
        while bytes.get(position).is_some_and(|b| {
            *b == b'_'
                || match radix {
                    b'x' => b.is_ascii_hexdigit(),
                    b'b' => matches!(b, b'0' | b'1'),
                    _ => matches!(b, b'0'..=b'7'),
                }
        }) {
            position += 1;
        }
    } else {
        while bytes
            .get(position)
            .is_some_and(|b| b.is_ascii_digit() || *b == b'_')
        {
            position += 1;
        }
        if bytes.get(position) == Some(&b'.') {
            position += 1;
            while bytes
                .get(position)
                .is_some_and(|b| b.is_ascii_digit() || *b == b'_')
            {
                position += 1;
            }
        }
        if bytes
            .get(position)
            .is_some_and(|b| matches!(b, b'e' | b'E'))
        {
            let mut exponent = position + 1;
            if bytes
                .get(exponent)
                .is_some_and(|b| matches!(b, b'+' | b'-'))
            {
                exponent += 1;
            }
            if bytes.get(exponent).is_some_and(u8::is_ascii_digit) {
                position = exponent + 1;
                while bytes.get(position).is_some_and(u8::is_ascii_digit) {
                    position += 1;
                }
            }
        }
    }
    if language == Language::JavaScript && bytes.get(position) == Some(&b'n') {
        position += 1;
    }
    position
}

/// Shared public-scheme colors for the native Markdown source renderer.
pub(super) fn source_colors() -> [Color32; 5] {
    let p = palette();
    [p.text, p.keyword, p.string, p.comment, p.function]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_preserved(source: &str, extension: &str) -> LayoutJob {
        let job = layout(source, extension, FontId::monospace(15.0));
        assert_eq!(job.text, source);
        assert!(job.sections.len() <= MAX_SPANS);
        let mut end = 0;
        for section in &job.sections {
            assert_eq!(usize::from(section.byte_range.start), end);
            assert!(source.is_char_boundary(section.byte_range.start.into()));
            assert!(source.is_char_boundary(section.byte_range.end.into()));
            end = section.byte_range.end.into();
        }
        assert_eq!(end, source.len());
        job
    }

    fn color_of(job: &LayoutJob, needle: &str) -> Color32 {
        let byte = job.text.find(needle).unwrap();
        job.sections
            .iter()
            .find(|section| section.byte_range.contains(&byte.into()))
            .unwrap()
            .format
            .color
    }

    #[test]
    fn declarations_variables_and_doc_comments_preserve_lexical_boundaries() {
        let php = assert_preserved(
            "<?php /** documentation */\nfunction café(int $value): string { return $value; }\n// function fake $value\n\"function fake $value\";",
            "php",
        );
        assert_eq!(color_of(&php, "documentation"), palette().doc_comment);
        assert_eq!(color_of(&php, "café"), palette().function);
        assert_eq!(color_of(&php, "int"), palette().keyword);
        assert_eq!(color_of(&php, "$value"), palette().variable);
        assert_eq!(color_of(&php, "// function"), palette().comment);
        assert_eq!(color_of(&php, "\"function"), palette().string);
        let ts = assert_preserved(
            "export function sample(value: number) {}\nconst dollar = '$value';",
            "ts",
        );
        assert_eq!(color_of(&ts, "sample"), palette().function);
        assert_eq!(color_of(&ts, "$value"), palette().string);
        // A new line, anonymous function or member call is not a declaration name.
        for source in [
            "function\nvalue",
            "function () { value }",
            "function(){}; obj.value",
        ] {
            let job = assert_preserved(source, "ts");
            assert_eq!(color_of(&job, "value"), palette().text);
        }
    }

    #[test]
    fn unicode_crlf_combining_marks_and_no_final_newline_are_unchanged() {
        for extension in ["php", "ts", "js", "sql", "txt"] {
            assert_preserved(
                "\u{feff}const ação = '😀 e\u{301} 日本';\r\n// café\r\n42",
                extension,
            );
        }
        assert_preserved("", "php");
    }

    #[test]
    fn escaped_strings_do_not_start_comments_or_keywords() {
        let php = assert_preserved(
            "<?php echo \"a\\\" // return 😀\"; // comentário\nreturn 2;",
            "php",
        );
        assert_eq!(color_of(&php, "// return"), palette().string);
        assert_eq!(color_of(&php, "// comentário"), palette().comment);
        assert_eq!(color_of(&php, "echo"), palette().keyword);
        assert_eq!(color_of(&php, "2;"), palette().number);
        let ts = assert_preserved(
            "const text = `return ${value} // 😀`; /* import */ let n = 0xff;",
            "ts",
        );
        assert_eq!(color_of(&ts, "return"), palette().string);
        assert_eq!(color_of(&ts, "import"), palette().comment);
        assert_eq!(color_of(&ts, "const"), palette().keyword);
        assert_eq!(color_of(&ts, "0xff"), palette().number);
    }

    #[test]
    fn sql_doubled_quotes_semicolons_and_comments_are_lexed_without_splitting() {
        let job = assert_preserved(
            "sElEcT 'it''s; -- text', \"odd;name\", [select] FROM t; -- comentário\nWHERE n = .5e+2;",
            "sql",
        );
        assert_eq!(color_of(&job, "sElEcT"), palette().keyword);
        assert_eq!(color_of(&job, "-- text"), palette().string);
        assert_eq!(color_of(&job, "odd;name"), palette().string);
        assert_eq!(color_of(&job, "-- comentário"), palette().comment);
        assert_eq!(color_of(&job, ".5e+2"), palette().number);
    }

    #[test]
    fn unterminated_tokens_and_resource_limits_preserve_remaining_text() {
        assert_preserved("SELECT '😀 unfinished", "sql");
        assert_preserved("/* unfinished 😀", "ts");
        let large = "const 😀\n".repeat(MAX_HIGHLIGHT_BYTES / 8);
        let job = assert_preserved(&large, "ts");
        assert_eq!(job.sections.len(), 1);
        let alternating = "if;".repeat(MAX_SPANS);
        let job = assert_preserved(&alternating, "ts");
        assert_eq!(job.sections.len(), MAX_SPANS);
        assert_eq!(job.sections.last().unwrap().format.color, palette().text);
    }
}
