//! Private, bounded metadata logs; no filesystem writes in the normal GUI sink.
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::mpsc::{self, SyncSender},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
const MAX_BYTES: usize = 1024 * 1024;
enum Message {
    Line(String),
    Flush(mpsc::Sender<()>),
}
#[derive(Clone)]
pub(super) struct Log {
    sender: SyncSender<Message>,
    directory: PathBuf,
}
fn stamp() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}
fn private_file(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}
impl Log {
    pub(super) fn new(session: &Path) -> std::io::Result<Self> {
        let directory = session.parent().unwrap_or(Path::new(".")).join("logs");
        if directory.is_symlink() {
            return Err(std::io::Error::other("Diagnostic directory is a symlink"));
        }
        fs::create_dir_all(&directory)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
        }
        prune(&directory);
        let path = directory.join(format!("runtime-{}-{}.log", std::process::id(), stamp()));
        let mut file = private_file(&path)?;
        let (sender, receiver) = mpsc::sync_channel(64);
        std::thread::Builder::new()
            .name("codisseu-diagnostics".into())
            .spawn(move || {
                let mut bytes = 0;
                while let Ok(message) = receiver.recv() {
                    match message {
                        Message::Line(line) if bytes < MAX_BYTES => {
                            let line = format!("unix_ms={} {line}\n", stamp() / 1_000_000);
                            const LIMIT: &[u8] =
                                b"Runtime log limit reached; panic records remain separate.\n";
                            if bytes + line.len() + LIMIT.len() > MAX_BYTES {
                                let _ = file.write_all(LIMIT);
                                bytes = MAX_BYTES;
                                continue;
                            }
                            if file.write_all(line.as_bytes()).is_err() {
                                break;
                            }
                            bytes += line.len();
                        }
                        Message::Line(_) => (),
                        Message::Flush(done) => {
                            let _ = file.flush();
                            let _ = done.send(());
                        }
                    }
                }
            })?;
        let logger = Self { sender, directory };
        logger.record(&format!(
            "startup pid={} version={} os={} arch={}",
            std::process::id(),
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH
        ));
        Ok(logger)
    }
    pub(super) fn record(&self, message: &str) {
        let line: String = message
            .chars()
            .take(4096)
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();
        let _ = self.sender.try_send(Message::Line(line));
    }
    pub(super) fn install_panic_hook(&self) {
        let directory = self.directory.clone();
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let path = directory.join(format!("panic-{}-{}.log", std::process::id(), stamp()));
            if let Ok(mut file) = private_file(&path) {
                let thread = std::thread::current();
                let _ = writeln!(
                    file,
                    "pid={} unix_ms={} thread={} location={:?}\nPanic payload omitted; inspect the source location and backtrace.\n{}",
                    std::process::id(),
                    stamp() / 1_000_000,
                    thread.name().unwrap_or("unnamed"),
                    info.location(),
                    std::backtrace::Backtrace::force_capture()
                );
                let _ = file.sync_all();
            }
            previous(info);
        }));
    }
    pub(super) fn flush(&self) {
        let (done, receiver) = mpsc::channel();
        if self.sender.try_send(Message::Flush(done)).is_ok() {
            let _ = receiver.recv_timeout(Duration::from_millis(500));
        }
    }
}
#[cfg(not(target_os = "linux"))]
fn prune(_: &Path) {}
#[cfg(target_os = "linux")]
fn prune(directory: &Path) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let mut old = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some((kind, rest)) = name.split_once('-') else {
            continue;
        };
        if !matches!(kind, "runtime" | "panic") || !rest.ends_with(".log") {
            continue;
        }
        let Some((pid, time)) = rest.trim_end_matches(".log").split_once('-') else {
            continue;
        };
        let Ok(pid) = pid.parse::<u32>() else {
            continue;
        };
        if time.parse::<u128>().is_err() {
            continue;
        }
        if pid == std::process::id() {
            continue;
        }
        #[cfg(target_os = "linux")]
        if Path::new("/proc").join(pid.to_string()).exists() {
            continue;
        }
        #[cfg(target_os = "linux")]
        if let Ok(metadata) = fs::symlink_metadata(&path)
            && metadata.is_file()
            && !metadata.file_type().is_symlink()
            && let Ok(modified) = metadata.modified()
        {
            old.push((modified, path));
        }
    }
    old.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    for (_, path) in old.into_iter().skip(19) {
        let _ = fs::remove_file(path);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "codisseu-diagnostic-test-{}-{}",
            std::process::id(),
            stamp()
        ));
        fs::create_dir(&path).unwrap();
        path
    }
    #[test]
    fn metadata_writer_flushes_bounded_private_logs() {
        let root = fixture();
        let log = Log::new(&root.join("session.json")).unwrap();
        log.record("shutdown waiting=language,history");
        log.flush();
        let path = fs::read_dir(root.join("logs"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("shutdown waiting=language,history"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        for _ in 0..16 {
            for _ in 0..32 {
                log.record(&"x".repeat(8192));
            }
            log.flush();
        }
        log.flush();
        assert!(fs::metadata(&path).unwrap().len() <= MAX_BYTES as u64);
        assert!(
            fs::read_to_string(path)
                .unwrap()
                .contains("Runtime log limit reached")
        );
        drop(log);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn pruning_preserves_active_logs_foreign_files_and_symlinks() {
        let root = fixture();
        let directory = root.join("logs");
        fs::create_dir(&directory).unwrap();
        for n in 0..25 {
            fs::write(directory.join(format!("runtime-4294967295-{n}.log")), "old").unwrap();
        }
        let active = directory.join(format!("runtime-{}-0.log", std::process::id()));
        fs::write(&active, "active").unwrap();
        let foreign = directory.join("my-notes.log");
        fs::write(&foreign, "keep").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&foreign, directory.join("panic-4294967295-link.log"))
                .unwrap();
        }
        prune(&directory);
        assert_eq!(fs::read_to_string(active).unwrap(), "active");
        assert_eq!(fs::read_to_string(foreign).unwrap(), "keep");
        #[cfg(target_os = "linux")]
        assert_eq!(
            fs::read_dir(&directory)
                .unwrap()
                .filter_map(Result::ok)
                .filter(|e| e
                    .file_name()
                    .to_string_lossy()
                    .starts_with("runtime-4294967295-"))
                .count(),
            19
        );
        fs::remove_dir_all(root).unwrap();
    }
}
