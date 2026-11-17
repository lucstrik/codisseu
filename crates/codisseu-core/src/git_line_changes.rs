//! Two-view bounded background comparison. Snapshots are COW; text materialization
//! and all Git/filesystem work happen on this worker, never during painting.
use super::*;
use git::line_changes::{Change, MAX_BYTES, MAX_LINES};
use std::{
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};
#[derive(Clone, Debug, PartialEq, Eq)]
struct Key {
    id: DocumentId,
    version: u64,
    path: PathBuf,
    encoding: FileEncoding,
}
struct Input {
    key: Key,
    text: TextSnapshot,
}
struct Request {
    files: Vec<Input>,
    cancel: Arc<AtomicBool>,
}
struct ResultFile {
    key: Key,
    result: Result<Vec<Change>, String>,
}
struct Reply {
    files: Vec<ResultFile>,
}
pub(super) struct Service {
    sender: Option<SyncSender<Request>>,
    receiver: Receiver<Reply>,
    desired: Vec<Key>,
    accepted: Vec<ResultFile>,
    pending: bool,
    cancel: Arc<AtomicBool>,
    changed: Instant,
    refreshed: Instant,
}
impl Service {
    pub fn new() -> Self {
        let (sender, input) = mpsc::sync_channel::<Request>(1);
        let (output, receiver) = mpsc::sync_channel(1);
        let sender = std::thread::Builder::new()
            .name("codisseu-git-lines".into())
            .spawn(move || {
                while let Ok(request) = input.recv() {
                    let mut files = Vec::new();
                    for input in request.files {
                        let result = (|| {
                            if input.text.len_bytes() > MAX_BYTES
                                || input.text.len_lines() > MAX_LINES
                            {
                                return Err("Marcas Git limitadas a4MiB/100000linhas".into());
                            }
                            let baseline =
                                git::line_changes::index_baseline(&input.key.path, &request.cancel)
                                    .map_err(|e| e.to_string())?;
                            let Some(bytes) = baseline else {
                                return Ok(Vec::new());
                            };
                            let base = input.key.encoding.decode(&bytes)?.0;
                            git::line_changes::compare(
                                &base,
                                &input.text.to_string(),
                                &request.cancel,
                            )
                            .map_err(|e| e.to_string())
                        })();
                        files.push(ResultFile {
                            key: input.key,
                            result,
                        });
                    }
                    if output.send(Reply { files }).is_err() {
                        break;
                    }
                }
            })
            .ok()
            .map(|_| sender);
        Self {
            sender,
            receiver,
            desired: Vec::new(),
            accepted: Vec::new(),
            pending: false,
            cancel: Arc::new(AtomicBool::new(false)),
            changed: Instant::now(),
            refreshed: Instant::now() - Duration::from_secs(3),
        }
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
        self.sender.take();
    }
}
impl AppService {
    /// Cancel and drain before normal shutdown so every Git child is reaped.
    pub fn cancel_git_line_changes(&mut self) {
        let state = &mut self.git_line_changes;
        state.cancel.store(true, Ordering::SeqCst);
        state.desired.clear();
        state.accepted.clear();
        while state.receiver.try_recv().is_ok() {
            state.pending = false;
        }
    }
    pub fn git_line_changes_pending(&self) -> bool {
        self.git_line_changes.pending
    }
    pub fn git_line_changes(&self, id: DocumentId, version: u64) -> &[Change] {
        self.git_line_changes
            .accepted
            .iter()
            .find(|f| {
                f.key.id == id
                    && f.key.version == version
                    && self.documents.iter().any(|d| {
                        d.id == id
                            && d.version == version
                            && d.path == f.key.path
                            && d.encoding == f.key.encoding
                    })
            })
            .and_then(|f| f.result.as_ref().ok())
            .map_or(&[], Vec::as_slice)
    }
    pub fn git_line_changes_error(&self, id: DocumentId, version: u64) -> Option<&str> {
        self.git_line_changes
            .accepted
            .iter()
            .find(|f| f.key.id == id && f.key.version == version)
            .and_then(|f| f.result.as_ref().err())
            .map(String::as_str)
    }
    /// The GUI ticks this even when SCM is hidden; it never switches the selected repo.
    pub fn tick_git_line_changes(&mut self) -> bool {
        let now = Instant::now();
        let keys: Vec<_> = self
            .documents
            .iter()
            .filter(|d| {
                !d.untitled
                    && !d.read_only
                    && (self.active == Some(d.id)
                        || self.document_views().iter().any(|v| v.document == d.id))
            })
            .take(2)
            .map(|d| Key {
                id: d.id,
                version: d.version,
                path: d.path.clone(),
                encoding: d.encoding,
            })
            .collect();
        let state = &mut self.git_line_changes;
        let mut changed = false;
        if keys != state.desired {
            state.desired = keys;
            state.changed = now;
            state.cancel.store(true, Ordering::SeqCst);
            state.accepted.retain(|f| state.desired.contains(&f.key));
            changed = true;
        }
        while let Ok(reply) = state.receiver.try_recv() {
            state.pending = false;
            if !state.cancel.load(Ordering::SeqCst) {
                state.accepted = reply
                    .files
                    .into_iter()
                    .filter(|f| state.desired.contains(&f.key))
                    .collect();
                state.refreshed = now;
                changed = true;
            }
        }
        if state.pending
            || state.desired.is_empty()
            || now.duration_since(state.changed) < Duration::from_millis(120)
            || (state.accepted.len() == state.desired.len()
                && now.duration_since(state.refreshed) < Duration::from_secs(2))
        {
            return changed;
        }
        state.cancel = Arc::new(AtomicBool::new(false));
        let files = state
            .desired
            .iter()
            .filter_map(|key| {
                self.documents
                    .iter()
                    .find(|d| d.id == key.id)
                    .map(|d| Input {
                        key: key.clone(),
                        text: d.snapshot(),
                    })
            })
            .collect();
        if state.sender.as_ref().is_some_and(|s| {
            s.try_send(Request {
                files,
                cancel: state.cancel.clone(),
            })
            .is_ok()
        }) {
            state.pending = true;
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, process::Command as Process};
    fn git(p: &std::path::Path, args: &[&str]) {
        let o = Process::new("git")
            .arg("-C")
            .arg(p)
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    }
    fn wait(s: &mut AppService, ready: impl Fn(&AppService) -> bool) {
        let end = Instant::now() + Duration::from_secs(15);
        loop {
            s.poll();
            s.tick_git_line_changes();
            if ready(s) {
                break;
            }
            assert!(Instant::now() < end, "{}", s.status);
            std::thread::sleep(Duration::from_millis(3));
        }
    }
    #[test]
    fn two_groups_use_own_index_codec_and_discard_obsolete_versions() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("service");
        fs::create_dir(&root).unwrap();
        git(&root, &["-c", "init.templateDir=", "init", "-q"]);
        fs::write(
            root.join(".editorconfig"),
            "root=true\n[*]\ncharset=latin1\n",
        )
        .unwrap();
        let a = root.join("a.php");
        let b = root.join("b.js");
        fs::write(&a, b"caf\xe9\n").unwrap();
        fs::write(&b, b"old\n").unwrap();
        git(&root, &["add", "."]);
        let mut s = AppService::new(d.path().join("profile/session.json"));
        s.dispatch(Command::OpenFile(a.clone())).unwrap();
        wait(&mut s, |s| s.active_document().is_some() && !s.is_busy());
        let first = s.active.unwrap();
        assert_eq!(
            s.active_document().unwrap().encoding(),
            FileEncoding::Latin1
        );
        s.dispatch(Command::OpenFile(b)).unwrap();
        wait(&mut s, |s| {
            s.active_document()
                .is_some_and(|d| d.path.ends_with("b.js"))
                && !s.is_busy()
        });
        let second = s.active.unwrap();
        // Use the public split command rather than inventing a second buffer.
        s.dispatch(Command::CreateView).unwrap();
        s.dispatch(Command::Activate { id: first }).unwrap();
        let v = s.documents.iter().find(|d| d.id == first).unwrap().version;
        s.dispatch(Command::Edit {
            id: first,
            expected_version: v,
            text: "café changé\n".into(),
        })
        .unwrap();
        let v = s.documents.iter().find(|d| d.id == second).unwrap().version;
        s.dispatch(Command::Edit {
            id: second,
            expected_version: v,
            text: "new\n".into(),
        })
        .unwrap();
        s.tick_git_line_changes();
        std::thread::sleep(Duration::from_millis(130));
        s.tick_git_line_changes();
        let v = s.documents.iter().find(|d| d.id == first).unwrap().version;
        s.dispatch(Command::Edit {
            id: first,
            expected_version: v,
            text: "café\n".into(),
        })
        .unwrap();
        s.tick_git_line_changes();
        wait(&mut s, |s| {
            !s.git_line_changes.pending
                && s.git_line_changes.accepted.len() == 2
                && s.git_line_changes.accepted.iter().any(|f| {
                    f.key.id == first
                        && f.key.version
                            == s.documents.iter().find(|d| d.id == first).unwrap().version
                })
        });
        let doc = s.documents.iter().find(|d| d.id == first).unwrap();
        assert!(s.git_line_changes(first, doc.version).is_empty());
        assert!(s.git_line_changes_error(first, doc.version).is_none());
        assert_eq!(fs::read(&a).unwrap(), b"caf\xe9\n");
        let doc = s.documents.iter().find(|d| d.id == second).unwrap();
        assert_eq!(s.git_line_changes(second, doc.version).len(), 1);
        s.git_line_changes.refreshed -= Duration::from_secs(3);
        s.tick_git_line_changes();
        assert!(s.git_line_changes_pending());
        let until = Instant::now() + Duration::from_secs(5);
        while s.git_line_changes_pending() {
            s.cancel_git_line_changes();
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(s.git_line_changes.accepted.is_empty());
    }
}
