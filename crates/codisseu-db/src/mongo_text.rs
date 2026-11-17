//! Locate one Mongo JSON object or literal shell read call without evaluating code.
use std::ops::Range;
struct Block {
    bytes: Range<usize>,
    error: Option<&'static str>,
}
pub fn execution_range(
    text: &str,
    cursor: usize,
    selection: Option<Range<usize>>,
) -> Result<Range<usize>, String> {
    let length = text.chars().count();
    if let Some(range) = selection {
        if range.start > range.end || range.end > length {
            return Err("Seleção Mongo fora do documento".into());
        }
        if !range.is_empty() {
            return Ok(range);
        }
    }
    let caret = if cursor == length {
        text.len()
    } else {
        text.char_indices()
            .nth(cursor)
            .map(|(i, _)| i)
            .ok_or("Cursor Mongo fora do documento")?
    };
    let b = text.as_bytes();
    let mut blocks = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_whitespace() || b[i] == b';' {
            i += 1;
            continue;
        }
        if b[i..].starts_with(b"//") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if b[i..].starts_with(b"/*") {
            let start = i;
            i += 2;
            while i + 1 < b.len() && !b[i..].starts_with(b"*/") {
                i += 1;
            }
            if i + 1 >= b.len() {
                blocks.push(Block {
                    bytes: start..b.len(),
                    error: Some("Comentário Mongo incompleto"),
                });
                break;
            }
            i += 2;
            continue;
        }
        let start = i;
        let shell = b[i..].starts_with(b"db.")
            || b[i..].starts_with(b"db ")
            || b[i..].starts_with(b"db\n")
            || b[i..].starts_with(b"db\r");
        if b[i] != b'{' && !shell {
            blocks.push(Block {
                bytes: start..b.len(),
                error: Some("Use JSON de consulta ou db.getCollection(...).find/aggregate(...)"),
            });
            break;
        }
        let mut stack = Vec::new();
        let mut quote = None;
        let mut error = None;
        while i < b.len() {
            let c = b[i];
            if let Some(q) = quote {
                if c == b'\\' {
                    i += 1;
                } else if c == q {
                    quote = None;
                }
            } else if b[i..].starts_with(b"//") {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
                continue;
            } else if b[i..].starts_with(b"/*") {
                if let Some(end) = text[i + 2..].find("*/") {
                    i += end + 4;
                    continue;
                }
                error = Some("Comentário Mongo incompleto");
                i = b.len();
                break;
            } else {
                match c {
                    b'"' | b'\'' => quote = Some(c),
                    b'{' => stack.push(b'}'),
                    b'[' => stack.push(b']'),
                    b'(' => stack.push(b')'),
                    b'}' | b']' | b')' => {
                        if stack.pop() != Some(c) {
                            error = Some("Delimitadores Mongo incompatíveis");
                            i = b.len();
                            break;
                        }
                    }
                    b';' if stack.is_empty() => break,
                    b'\n' | b'\r' if shell && stack.is_empty() => {
                        // Chains can wrap before a dot, including comments between calls.
                        let mut tail = &text[i..];
                        loop {
                            tail = tail.trim_start();
                            if tail.starts_with("//") {
                                tail = tail.find('\n').map_or("", |n| &tail[n..]);
                            } else if tail.starts_with("/*") {
                                if let Some(end) = tail.find("*/") {
                                    tail = &tail[end + 2..];
                                } else {
                                    break;
                                }
                            } else {
                                break;
                            }
                        }
                        if !tail.starts_with('.') {
                            break;
                        }
                    }
                    _ => {}
                }
            }
            i += 1;
            if !shell && stack.is_empty() && quote.is_none() {
                break;
            }
        }
        if !stack.is_empty() || quote.is_some() {
            error = Some("Consulta Mongo incompleta");
        }
        blocks.push(Block {
            bytes: start..i.min(b.len()),
            error,
        });
    }
    let Some(first) = blocks.first() else {
        return Err("Nenhuma consulta Mongo sob o cursor".into());
    };
    let mut target = first;
    for (index, block) in blocks.iter().enumerate() {
        if caret < block.bytes.start {
            if index > 0 {
                let previous = &blocks[index - 1];
                let gap = &text[previous.bytes.end..caret];
                if !gap.contains(['\n', '\r']) {
                    target = previous;
                    break;
                }
            }
            target = block;
            break;
        }
        target = block;
        if caret < block.bytes.end {
            break;
        }
    }
    if let Some(error) = target.error {
        return Err(format!(
            "{error}; selecione explicitamente o trecho desejado"
        ));
    }
    let start = text[..target.bytes.start].chars().count();
    Ok(start..start + text[target.bytes.clone()].chars().count())
}
