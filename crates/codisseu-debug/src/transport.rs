use serde_json::Value;
use std::{
    io::{BufRead, BufReader, Read, Write},
    sync::mpsc::{self, Receiver, SyncSender},
    thread,
};
const MAX_MESSAGE: usize = 8 * 1024 * 1024;
pub(crate) struct Connection {
    pub writer: Box<dyn Write + Send>,
    pub rx: Receiver<Result<Value, String>>,
    pub seq: u64,
    pub initialized: bool,
    pub configure_sent: bool,
    pub launch: Value,
}
impl Connection {
    pub fn new(
        reader: impl Read + Send + 'static,
        writer: impl Write + Send + 'static,
        launch: Value,
    ) -> Self {
        let (tx, rx) = mpsc::sync_channel(64);
        thread::spawn(move || read_loop(reader, tx));
        Self {
            writer: Box::new(writer),
            rx,
            seq: 0,
            initialized: false,
            configure_sent: false,
            launch,
        }
    }
    pub fn request(&mut self, command: &str, args: Value) -> Result<u64, String> {
        self.seq += 1;
        let seq = self.seq;
        self.write(
            &serde_json::json!({"seq":seq,"type":"request","command":command,"arguments":args}),
        )?;
        Ok(seq)
    }
    pub fn response(&mut self, request: &Value, body: Value, success: bool) -> Result<(), String> {
        self.seq += 1;
        self.write(&serde_json::json!({"seq":self.seq,"type":"response","request_seq":request["seq"],"command":request["command"],"success":success,"body":body}))
    }
    fn write(&mut self, value: &Value) -> Result<(), String> {
        let bytes = serde_json::to_vec(value).map_err(|x| x.to_string())?;
        write!(self.writer, "Content-Length: {}\r\n\r\n", bytes.len())
            .and_then(|()| self.writer.write_all(&bytes))
            .and_then(|()| self.writer.flush())
            .map_err(|x| x.to_string())
    }
}
fn read_loop(reader: impl Read, tx: SyncSender<Result<Value, String>>) {
    let mut reader = BufReader::new(reader);
    loop {
        let result = read_message(&mut reader);
        let failed = result.is_err();
        if tx.send(result).is_err() || failed {
            break;
        }
    }
}
pub(crate) fn read_message(reader: &mut impl BufRead) -> Result<Value, String> {
    let mut size = None;
    let mut total = 0;
    loop {
        let mut line = String::new();
        let count = reader
            .take(8193)
            .read_line(&mut line)
            .map_err(|x| x.to_string())?;
        if count == 0 {
            return Err("Debug adapter closed the connection".into());
        }
        total += count;
        if total > 8192 {
            return Err("Debug adapter header exceeds 8 KiB".into());
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((k, v)) = line.split_once(':')
            && k.eq_ignore_ascii_case("content-length")
        {
            size = Some(
                v.trim()
                    .parse::<usize>()
                    .map_err(|_| "Invalid DAP length")?,
            );
        }
    }
    let size = size.ok_or("Missing DAP Content-Length")?;
    if size > MAX_MESSAGE {
        return Err("DAP message exceeds 8 MiB".into());
    }
    let mut bytes = vec![0; size];
    reader.read_exact(&mut bytes).map_err(|x| x.to_string())?;
    serde_json::from_slice(&bytes).map_err(|x| format!("Invalid DAP JSON: {x}"))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn framing_unicode() {
        let value = serde_json::json!({"message":"á🐛"});
        let s = serde_json::to_vec(&value).unwrap();
        let mut b = format!("Content-Length: {}\r\n\r\n", s.len()).into_bytes();
        b.extend(s);
        assert_eq!(read_message(&mut std::io::Cursor::new(b)).unwrap(), value)
    }
    #[test]
    fn oversized_rejected() {
        assert!(
            read_message(&mut std::io::Cursor::new(
                b"Content-Length: 99999999\r\n\r\n"
            ))
            .is_err()
        )
    }
}
