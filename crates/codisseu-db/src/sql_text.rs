//! Statement targeting for the editor. This is a lexical boundary scanner, not
//! a SQL validator: the selected text still passes the driver's normal guards.
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SqlDialect {
    Sqlite,
    Postgres,
    Mysql,
}

#[derive(Debug)]
struct Statement {
    bytes: Range<usize>,
    code: bool,
    error: Option<&'static str>,
}

/// All public positions are Unicode scalar indices, as in the editor buffer.
/// An explicit nonempty selection is returned exactly, without expanding it or
/// changing the SQL. The backend remains responsible for validating it.
pub fn execution_range(
    text: &str,
    cursor: usize,
    selection: Option<Range<usize>>,
    dialect: SqlDialect,
) -> Result<Range<usize>, String> {
    if let Some(range) = selection {
        if range.start > range.end || range.end > text.chars().count() {
            return Err("Seleção SQL fora do documento".into());
        }
        if !range.is_empty() {
            return Ok(range);
        }
    }
    statement_range(text, cursor, dialect)
}

/// Locate one statement under the caret, respecting dialect quote/comment
/// boundaries. A separator belongs to the preceding statement. In the gap
/// after it, trailing spaces on its line keep that statement; a new line targets
/// the next one. At EOF, trailing whitespace targets the last statement.
pub fn statement_range(
    text: &str,
    cursor: usize,
    dialect: SqlDialect,
) -> Result<Range<usize>, String> {
    let cursor_byte =
        scalar_to_byte(text, cursor).ok_or_else(|| "Cursor SQL fora do documento".to_owned())?;
    let statements = scan(text, dialect);
    let statements: Vec<_> = statements.iter().filter(|s| s.code).collect();
    let Some(first) = statements.first() else {
        return Err("Nenhuma instrução SQL sob o cursor".into());
    };
    let mut selected = *first;
    for (index, statement) in statements.iter().enumerate() {
        if cursor_byte < statement.bytes.start {
            if index > 0 {
                let previous = statements[index - 1];
                let gap = &text[previous.bytes.end..cursor_byte];
                if !gap.contains(['\n', '\r']) {
                    selected = previous;
                    break;
                }
            }
            selected = statement;
            break;
        }
        selected = statement;
        if cursor_byte < statement.bytes.end {
            break;
        }
    }
    if let Some(error) = selected.error {
        return Err(format!(
            "{error}; selecione explicitamente o trecho desejado"
        ));
    }
    let start = text[..selected.bytes.start].chars().count();
    Ok(start..start + text[selected.bytes.clone()].chars().count())
}

fn scalar_to_byte(text: &str, scalar: usize) -> Option<usize> {
    if scalar == text.chars().count() {
        Some(text.len())
    } else {
        text.char_indices().nth(scalar).map(|(byte, _)| byte)
    }
}

fn identifier(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$' || byte >= 128
}

fn dollar_delimiter(bytes: &[u8], offset: usize) -> Option<&[u8]> {
    if offset > 0 && identifier(bytes[offset - 1]) {
        return None;
    }
    let mut end = offset + 1;
    if bytes.get(end) == Some(&b'$') {
        return Some(&bytes[offset..=end]);
    }
    if !bytes
        .get(end)
        .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_' || *b >= 128)
    {
        return None;
    }
    end += 1;
    while bytes
        .get(end)
        .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_' || *b >= 128)
    {
        end += 1;
    }
    (bytes.get(end) == Some(&b'$')).then(|| &bytes[offset..=end])
}

fn scan(text: &str, dialect: SqlDialect) -> Vec<Statement> {
    let bytes = text.as_bytes();
    let mut statements = Vec::new();
    let (mut start, mut i, mut code, mut error) = (0, 0, false, None);
    while i < bytes.len() {
        let byte = bytes[i];
        if byte == b';' {
            push_statement(&mut statements, text, start..i + 1, code, error);
            i += 1;
            start = i;
            code = false;
            error = None;
        } else if byte.is_ascii_whitespace() {
            i += 1;
        } else if (byte == b'-'
            && bytes.get(i + 1) == Some(&b'-')
            && (dialect != SqlDialect::Mysql || bytes.get(i + 2).is_none_or(|b| *b <= b' ')))
            || (byte == b'#' && dialect == SqlDialect::Mysql)
        {
            while i < bytes.len() && !matches!(bytes[i], b'\r' | b'\n') {
                i += 1;
            }
        } else if byte == b'/' && bytes.get(i + 1) == Some(&b'*') {
            if dialect == SqlDialect::Mysql && bytes.get(i + 2) == Some(&b'!') {
                code = true;
                error = Some("Comentário executável MySQL exige seleção explícita");
            }
            let mut depth = 1;
            i += 2;
            while i < bytes.len() && depth > 0 {
                if bytes.get(i..i + 2) == Some(b"*/") {
                    depth -= 1;
                    i += 2;
                } else if dialect == SqlDialect::Postgres && bytes.get(i..i + 2) == Some(b"/*") {
                    depth += 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            if depth > 0 {
                code = true;
                error = Some("Comentário SQL não foi fechado");
            }
        } else if byte == b'$'
            && dialect == SqlDialect::Postgres
            && let Some(delimiter) = dollar_delimiter(bytes, i)
        {
            code = true;
            let after = i + delimiter.len();
            if let Some(relative) = text[after..].find(std::str::from_utf8(delimiter).unwrap()) {
                i = after + relative + delimiter.len();
            } else {
                i = bytes.len();
                error = Some("String dollar-quoted PostgreSQL não foi fechada");
            }
        } else if matches!(byte, b'\'' | b'"')
            || (byte == b'`' && dialect != SqlDialect::Postgres)
            || (byte == b'[' && dialect == SqlDialect::Sqlite)
        {
            code = true;
            let quote = if byte == b'[' { b']' } else { byte };
            let escaped = dialect == SqlDialect::Postgres
                && byte == b'\''
                && i > 0
                && matches!(bytes[i - 1], b'e' | b'E')
                && (i < 2 || !identifier(bytes[i - 2]));
            i += 1;
            let mut closed = false;
            while i < bytes.len() {
                if bytes[i] == quote {
                    if quote != b']' && bytes.get(i + 1) == Some(&quote) {
                        i += 2;
                    } else {
                        i += 1;
                        closed = true;
                        break;
                    }
                } else if bytes[i] == b'\\' && quote != b']' {
                    if escaped {
                        i = (i + 2).min(bytes.len());
                    } else {
                        // Server sql_mode / standard_conforming_strings can
                        // change which quote closes the string. Do not guess
                        // a boundary that could execute a different statement.
                        if (dialect == SqlDialect::Mysql
                            || (dialect == SqlDialect::Postgres && quote == b'\''))
                            && bytes.get(i + 1) == Some(&quote)
                        {
                            error =
                                Some("Aspa com barra invertida depende do modo SQL do servidor");
                            // Everything after this point has uncertain
                            // boundaries, including later apparent statements.
                            i = bytes.len();
                            break;
                        }
                        i += 1;
                    }
                } else {
                    i += 1;
                }
            }
            if !closed {
                error.get_or_insert("String ou identificador SQL não foi fechado");
            }
        } else {
            code = true;
            i += 1;
        }
    }
    push_statement(&mut statements, text, start..bytes.len(), code, error);
    statements
}

fn push_statement(
    statements: &mut Vec<Statement>,
    text: &str,
    bytes: Range<usize>,
    code: bool,
    error: Option<&'static str>,
) {
    let raw = &text[bytes.clone()];
    let trimmed = raw.trim();
    let start = bytes.start + raw.len() - raw.trim_start().len();
    statements.push(Statement {
        bytes: start..start + trimmed.len(),
        code,
        error,
    });
}
