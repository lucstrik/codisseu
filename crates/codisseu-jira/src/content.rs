//! ADF → safe Markdown. No HTML evaluation, local resource reads or remote images.
use serde_json::Value;
const MAX: usize = 512 * 1024;
pub fn markdown(v: &Value) -> String {
    if let Some(s) = v.as_str() {
        return s.chars().take(MAX).collect();
    }
    let mut out = String::new();
    let mut nodes = 0;
    node(v, &mut out, 0, &mut nodes);
    if out.len() > MAX {
        let mut n = MAX;
        while !out.is_char_boundary(n) {
            n -= 1
        }
        out.truncate(n);
        out.push_str("\n\n[Conteúdo limitado a512KiB]")
    }
    out
}
fn escape(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        if "\\`*_{}[]<>|#".contains(c) {
            o.push('\\')
        }
        o.push(c)
    }
    o
}
fn node(v: &Value, o: &mut String, depth: usize, n: &mut usize) {
    if depth > 48 || *n > 20000 || o.len() > MAX {
        return;
    }
    *n += 1;
    let t = v["type"].as_str().unwrap_or("");
    if t == "text" {
        let s = v["text"].as_str().unwrap_or("");
        let marks = v["marks"].as_array().map(Vec::as_slice).unwrap_or(&[]);
        let code = marks.iter().any(|m| m["type"] == "code");
        let mut text = if code {
            format!("<code>{}</code>", escape(s))
        } else {
            escape(s)
        };
        if code {
            let fence = "`".repeat(s.split('`').map(str::len).count().max(1) + 1);
            text = format!("{fence} {s} {fence}");
        }
        for m in marks {
            match m["type"].as_str().unwrap_or("") {
                "strong" => text = format!("**{text}**"),
                "em" => text = format!("*{text}*"),
                "strike" => text = format!("~~{text}~~"),
                "link" => {
                    if let Some(u) = safe_url(&m["attrs"]["href"]) {
                        text = format!("[{text}](<{u}>)")
                    }
                }
                _ => (),
            }
        }
        o.push_str(&text);
        return;
    }
    match t {
        "hardBreak" => {
            o.push_str("  \n");
            return;
        }
        "rule" => {
            o.push_str("\n---\n");
            return;
        }
        "mention" | "emoji" | "status" | "date" => {
            let a = &v["attrs"];
            let text = a["text"]
                .as_str()
                .or_else(|| a["shortName"].as_str())
                .or_else(|| a["timestamp"].as_str())
                .unwrap_or("[menção]");
            o.push_str(&escape(text));
            return;
        }
        "inlineCard" | "blockCard" => {
            if let Some(u) = safe_url(&v["attrs"]["url"]) {
                o.push_str(&format!("[Abrir link](<{u}>)"))
            }
            return;
        }
        "media" => {
            if let Some(u) = safe_url(&v["attrs"]["url"]) {
                o.push_str(&format!("![Imagem](<{u}>)"));
            } else {
                o.push_str("[Anexo disponível na seção Anexos do card]");
            }
            return;
        }
        "heading" => {
            o.push('\n');
            o.push_str(&"#".repeat(v["attrs"]["level"].as_u64().unwrap_or(2).clamp(1, 6) as usize));
            o.push(' ')
        }
        "codeBlock" => {
            let mut s = String::new();
            plain(v, &mut s, 0);
            let mut fence = "```".to_string();
            while s.contains(&fence) {
                fence.push('`')
            }
            o.push_str(&format!("\n{fence}\n{s}\n{fence}\n\n"));
            return;
        }
        "bulletList" | "orderedList" => {
            if let Some(items) = v["content"].as_array() {
                for (i, item) in items.iter().enumerate() {
                    let mut s = String::new();
                    node(item, &mut s, depth + 1, n);
                    let marker = if t == "orderedList" {
                        format!(
                            "{}. ",
                            i + v["attrs"]["order"].as_u64().unwrap_or(1) as usize
                        )
                    } else {
                        "- ".into()
                    };
                    for (j, line) in s.trim().lines().enumerate() {
                        o.push_str(if j == 0 { &marker } else { "    " });
                        o.push_str(line);
                        o.push('\n')
                    }
                }
                o.push('\n')
            }
            return;
        }
        "blockquote" | "panel" => {
            let mut s = String::new();
            children(v, &mut s, depth, n);
            for l in s.trim().lines() {
                o.push_str("> ");
                o.push_str(l);
                o.push('\n')
            }
            o.push('\n');
            return;
        }
        "table" => {
            if let Some(rows) = v["content"].as_array() {
                for (i, row) in rows.iter().take(200).enumerate() {
                    o.push('|');
                    if let Some(cells) = row["content"].as_array() {
                        for cell in cells.iter().take(32) {
                            let mut s = String::new();
                            children(cell, &mut s, depth + 1, n);
                            o.push_str(&s.trim().replace('\n', " "));
                            o.push('|')
                        }
                        o.push('\n');
                        if i == 0 {
                            o.push('|');
                            for _ in cells.iter().take(32) {
                                o.push_str(" --- |")
                            }
                            o.push('\n')
                        }
                    }
                }
                o.push('\n')
            }
            return;
        }
        _ => (),
    }
    children(v, o, depth, n);
    if matches!(t, "paragraph" | "heading" | "expand" | "nestedExpand") {
        o.push_str("\n\n")
    }
}
fn children(v: &Value, o: &mut String, d: usize, n: &mut usize) {
    if let Some(a) = v["content"].as_array() {
        for v in a {
            node(v, o, d + 1, n)
        }
    }
}
fn plain(v: &Value, o: &mut String, d: usize) {
    if d > 48 || o.len() > MAX {
        return;
    }
    if let Some(s) = v["text"].as_str() {
        o.push_str(s)
    }
    if let Some(a) = v["content"].as_array() {
        for c in a {
            plain(c, o, d + 1)
        }
    }
}
fn safe_url(v: &Value) -> Option<String> {
    let s = v.as_str()?;
    let u = url::Url::parse(s).ok()?;
    if !matches!(u.scheme(), "http" | "https" | "mailto")
        || s.contains(['<', '>', '\n', '\r'])
        || !u.username().is_empty()
        || u.password().is_some()
    {
        None
    } else {
        Some(u.to_string())
    }
}
