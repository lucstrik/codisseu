use serde_json::Value;
use std::{
    io::{self, Read, Write},
    path::{Path, PathBuf},
};
pub const MAX_MESSAGE: usize = 4 * 1024 * 1024;
pub fn read_frame(reader: &mut impl Read) -> Result<Value, String> {
    let mut header = Vec::new();
    while !header.ends_with(b"\r\n\r\n") {
        if header.len() >= 8192 {
            return Err("LSP header exceeds 8192 bytes".into());
        }
        let mut byte = [0];
        reader.read_exact(&mut byte).map_err(|e| e.to_string())?;
        header.push(byte[0]);
    }
    let header = std::str::from_utf8(&header).map_err(|e| e.to_string())?;
    let mut length = None;
    for line in header.split("\r\n").filter(|line| !line.is_empty()) {
        let (key, value) = line.split_once(':').ok_or("Malformed LSP header")?;
        if key.eq_ignore_ascii_case("Content-Length") {
            if length.is_some() {
                return Err("Duplicate Content-Length".into());
            }
            length = Some(value.trim().parse::<usize>().map_err(|e| e.to_string())?);
        }
    }
    let length = length.ok_or("Missing Content-Length")?;
    if length > MAX_MESSAGE {
        return Err("LSP message exceeds 4 MiB".into());
    }
    let mut data = vec![0; length];
    reader.read_exact(&mut data).map_err(|e| e.to_string())?;
    serde_json::from_slice(&data).map_err(|e| e.to_string())
}
pub fn write_frame(writer: &mut impl Write, value: &Value) -> io::Result<()> {
    let body = serde_json::to_vec(value)?;
    if body.len() > MAX_MESSAGE {
        return Err(io::Error::other("LSP message exceeds 4 MiB"));
    }
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
    writer.write_all(&body)?;
    writer.flush()
}
pub fn file_uri(path: &Path) -> Result<String, String> {
    let raw = path
        .to_str()
        .ok_or("Non UTF-8 path unsupported by LSP")?
        .to_string();
    #[cfg(windows)]
    let raw = raw.replace('\\', "/");
    let mut result = String::from(if raw.starts_with('/') {
        "file://"
    } else {
        "file:///"
    });
    for byte in raw.bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~:".contains(&byte) {
            result.push(byte as char);
        } else {
            result.push_str(&format!("%{byte:02X}"));
        }
    }
    Ok(result)
}
pub fn uri_path(uri: &str) -> Option<PathBuf> {
    let raw = uri.strip_prefix("file://")?;
    if !raw.starts_with('/') {
        return None;
    }
    let bytes = raw.as_bytes();
    let mut decoded = Vec::new();
    let mut pos = 0;
    while pos < bytes.len() {
        if bytes[pos] == b'%' {
            let hex = std::str::from_utf8(bytes.get(pos + 1..pos + 3)?).ok()?;
            decoded.push(u8::from_str_radix(hex, 16).ok()?);
            pos += 3;
        } else {
            decoded.push(bytes[pos]);
            pos += 1;
        }
    }
    let text = String::from_utf8(decoded).ok()?;
    #[cfg(windows)]
    let text = text.strip_prefix('/').unwrap_or(&text).to_string();
    Some(PathBuf::from(text))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn framing_and_unicode_uri_roundtrip() {
        let value = serde_json::json!({"text":"Olá 🌧"});
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &value).unwrap();
        assert_eq!(read_frame(&mut bytes.as_slice()).unwrap(), value);
        let path = Path::new("/tmp/ação #🌧.ts");
        assert_eq!(uri_path(&file_uri(path).unwrap()).unwrap(), path);
        #[cfg(unix)]
        {
            let backslash = Path::new("/tmp/with\\backslash.ts");
            assert_eq!(uri_path(&file_uri(backslash).unwrap()).unwrap(), backslash);
        }
    }
    #[test]
    fn malformed_frames_rejected() {
        for input in [
            b"Content-Length: 99999999\r\n\r\n".as_slice(),
            b"Content-Length: 2\r\nContent-Length: 2\r\n\r\n{}",
            b"X: x\r\n\r\n{}",
            b"Content-Length: 3\r\n\r\n{}",
        ] {
            assert!(read_frame(&mut &*input).is_err());
        }
    }
    #[test]
    fn utf16_positions_do_not_split_surrogates() {
        assert_eq!(crate::utf16_byte_offset("a🌧é\r\nnext", 0, 3), Some(5));
        assert_eq!(crate::utf16_byte_offset("a🌧é", 0, 2), None);
        assert_eq!(crate::utf16_byte_offset("a🌧é\r\nnext", 1, 2), Some(11));
    }
}
