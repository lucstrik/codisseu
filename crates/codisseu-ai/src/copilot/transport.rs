use serde_json::Value;
use std::io::{Read, Write};
pub fn read(reader: &mut impl Read) -> Result<Value, String> {
    let mut header = Vec::new();
    while !header.ends_with(b"\r\n\r\n") {
        if header.len() > 8192 {
            return Err("Cabeçalho Copilot excede o limite".into());
        }
        let mut b = [0];
        reader.read_exact(&mut b).map_err(|e| e.to_string())?;
        header.push(b[0]);
    }
    let h = std::str::from_utf8(&header).map_err(|e| e.to_string())?;
    let mut len = None;
    for line in h.split("\r\n").filter(|s| !s.is_empty()) {
        let (k, v) = line.split_once(':').ok_or("Cabeçalho inválido")?;
        if k.eq_ignore_ascii_case("Content-Length") {
            if len.is_some() {
                return Err("Comprimento duplicado".into());
            }
            len = Some(v.trim().parse::<usize>().map_err(|e| e.to_string())?);
        }
    }
    let n = len.ok_or("Comprimento ausente")?;
    if n > 4 * 1024 * 1024 {
        return Err("Mensagem Copilot excede4MiB".into());
    }
    let mut body = vec![0; n];
    reader.read_exact(&mut body).map_err(|e| e.to_string())?;
    serde_json::from_slice(&body).map_err(|e| e.to_string())
}
pub fn write(writer: &mut impl Write, value: &Value) -> Result<(), String> {
    let body = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    if body.len() > 4 * 1024 * 1024 {
        return Err("Mensagem Copilot excede4MiB".into());
    }
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())
        .and_then(|_| writer.write_all(&body))
        .and_then(|_| writer.flush())
        .map_err(|e| e.to_string())
}
