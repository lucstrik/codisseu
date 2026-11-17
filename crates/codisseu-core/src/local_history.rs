//! Private profile snapshots. Filesystem and SQLite work never run on the GUI thread.
use crate::*;
use rusqlite::{Connection, OptionalExtension, params};
use std::{
    fs,
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const MAX_CONTENT: usize = codisseu_editor::MAX_EDITOR_DOCUMENT_BYTES;
const MAX_TOTAL: i64 = 64 * 1024 * 1024;
const MAX_ROWS: i64 = 2048;
#[derive(Clone, Debug)]
pub struct Revision {
    pub id: u64,
    pub date: String,
    pub label: Option<String>,
    pub reason: String,
    pub bytes: usize,
}
#[derive(Clone, Debug)]
pub struct Preview {
    pub id: u64,
    pub text: Arc<str>,
}
#[derive(Default)]
pub struct View {
    pub path: PathBuf,
    pub revisions: Vec<Revision>,
    pub preview: Option<Preview>,
}
enum Content {
    Text(Arc<str>),
    Snapshot(TextSnapshot),
}
enum Request {
    Record {
        path: PathBuf,
        before: Option<Arc<str>>,
        after: Content,
        label: Option<String>,
        reason: &'static str,
    },
    List {
        path: PathBuf,
        token: u64,
    },
    Read {
        path: PathBuf,
        id: u64,
        token: u64,
    },
}
enum ResultData {
    Recorded(PathBuf),
    Listed {
        path: PathBuf,
        token: u64,
        items: Vec<Revision>,
    },
    Read {
        path: PathBuf,
        token: u64,
        id: u64,
        text: String,
    },
}
pub struct Service {
    sender: Option<SyncSender<Request>>,
    receiver: Receiver<(Option<u64>, Result<ResultData, String>)>,
    pending: usize,
    token: u64,
    pub view: Option<View>,
    pub error: Option<String>,
    pub(super) warning: Option<String>,
}
impl Service {
    pub fn new(directory: PathBuf) -> Self {
        let (sender, input) = mpsc::sync_channel(32);
        let (output, receiver) = mpsc::sync_channel(32);
        let result = std::thread::Builder::new()
            .name("codisseu-local-history".into())
            .spawn(move || {
                while let Ok(request) = input.recv() {
                    let token = match &request {
                        Request::Record { .. } => None,
                        Request::List { token, .. } | Request::Read { token, .. } => Some(*token),
                    };
                    let result =
                        process(&directory, request).map_err(|e| format!("Local History: {e}"));
                    // Normal application exit waits for is_busy(). An abandoned
                    // service stops after its in-flight operation, without a GUI join.
                    if output.send((token, result)).is_err() {
                        break;
                    }
                }
            });
        let error = result
            .err()
            .map(|e| format!("Local History: worker indisponível: {e}"));
        Self {
            sender: error.is_none().then_some(sender),
            receiver,
            pending: 0,
            token: 0,
            view: None,
            error,
            warning: None,
        }
    }
    pub fn is_busy(&self) -> bool {
        self.pending > 0
    }
    fn send(&mut self, request: Request) -> Result<(), String> {
        let recording = matches!(&request, Request::Record { .. });
        let result = self
            .sender
            .as_ref()
            .ok_or("Worker de histórico indisponível".to_string())
            .and_then(|s| {
                s.try_send(request).map_err(|_| {
                    "Fila de histórico cheia; esta revisão não foi registrada".to_string()
                })
            });
        if let Err(error) = &result {
            self.error = Some(error.clone());
            if recording {
                self.warning = Some(error.clone());
            }
        } else {
            self.pending += 1;
        }
        result
    }
    pub(super) fn record(
        &mut self,
        path: PathBuf,
        before: Option<Arc<str>>,
        text: Arc<str>,
        reason: &'static str,
    ) {
        let _ = self.send(Request::Record {
            path,
            before,
            after: Content::Text(text),
            label: None,
            reason,
        });
    }
    fn label(
        &mut self,
        path: PathBuf,
        snapshot: TextSnapshot,
        label: String,
    ) -> Result<(), String> {
        if label.trim().is_empty()
            || label.chars().count() > 128
            || label.contains(['\0', '\n', '\r'])
        {
            return Err("Rótulo deve ter1..128 caracteres em uma linha".into());
        }
        self.send(Request::Record {
            path,
            before: None,
            after: Content::Snapshot(snapshot),
            label: Some(label.trim().into()),
            reason: "Label",
        })
    }
    pub fn close(&mut self) {
        self.token += 1;
        self.view = None;
    }
    fn list(&mut self, path: PathBuf) -> Result<(), String> {
        self.token += 1;
        self.error = None;
        self.send(Request::List {
            path: path.clone(),
            token: self.token,
        })?;
        self.view = Some(View {
            path,
            ..Default::default()
        });
        Ok(())
    }
    fn read(&mut self, path: PathBuf, id: u64) -> Result<(), String> {
        self.token += 1;
        self.error = None;
        self.send(Request::Read {
            path,
            id,
            token: self.token,
        })?;
        if let Some(view) = &mut self.view {
            view.preview = None;
        }
        Ok(())
    }
    pub(super) fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok((request_token, result)) = self.receiver.try_recv() {
            self.pending = self.pending.saturating_sub(1);
            changed = true;
            match result {
                Err(error) if request_token.is_none_or(|token| token == self.token) => {
                    if request_token.is_none() {
                        self.warning = Some(error.clone());
                    }
                    self.error = Some(error);
                }
                Err(_) => {}
                Ok(ResultData::Recorded(path)) => {
                    if self.view.as_ref().is_some_and(|v| v.path == path) {
                        let _ = self.list(path);
                    }
                }
                Ok(ResultData::Listed { path, token, items }) if token == self.token => {
                    if let Some(view) = &mut self.view
                        && view.path == path
                    {
                        let first = items.first().map(|r| r.id);
                        view.revisions = items;
                        if let Some(id) = first {
                            let _ = self.read(path, id);
                        }
                    }
                }
                Ok(ResultData::Read {
                    path,
                    token,
                    id,
                    text,
                }) if token == self.token => {
                    if let Some(view) = &mut self.view
                        && view.path == path
                    {
                        view.preview = Some(Preview {
                            id,
                            text: Arc::from(text),
                        });
                    }
                }
                _ => {}
            }
        }
        changed
    }
}
fn open(directory: &Path) -> Result<Connection, String> {
    if !directory.exists() {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(directory).map_err(|e| e.to_string())?;
    }
    if fs::symlink_metadata(directory)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_symlink()
    {
        return Err("Pasta de histórico é um symlink".into());
    }
    let path = directory.join("history.sqlite3");
    match fs::symlink_metadata(&path) {
        Ok(meta) if !meta.is_file() || meta.file_type().is_symlink() => {
            return Err("Store de histórico não é um arquivo regular".into());
        }
        Ok(meta) if meta.len() > 128 * 1024 * 1024 => {
            return Err("Store excede128MiB; preservado".into());
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(&path) {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e.to_string()),
            }
        }
        Err(e) => return Err(e.to_string()),
    }
    let mut db = Connection::open(&path).map_err(|e| e.to_string())?;
    db.busy_timeout(Duration::from_millis(500))
        .map_err(|e| e.to_string())?;
    db.execute_batch("PRAGMA trusted_schema=OFF;")
        .map_err(|e| e.to_string())?;
    let tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let version: i64 = tx
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    if version == 0 {
        let tables: i64 = tx
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if tables != 0 {
            return Err("Schema desconhecido; store preservado".into());
        }
        tx.execute_batch("CREATE TABLE revisions(id INTEGER PRIMARY KEY AUTOINCREMENT,path TEXT NOT NULL,created INTEGER NOT NULL,label TEXT,reason TEXT NOT NULL,content BLOB NOT NULL); CREATE INDEX revisions_path ON revisions(path,id); PRAGMA user_version=1;").map_err(|e|e.to_string())?;
    } else if version != 1 {
        return Err("Versão de histórico desconhecida; store preservado".into());
    }
    tx.commit().map_err(|e| e.to_string())?;
    let objects:i64=db.query_row("SELECT count(*) FROM sqlite_master WHERE name NOT IN ('revisions','revisions_path','sqlite_sequence')",[],|r|r.get(0)).map_err(|e|e.to_string())?;
    let mut info = db
        .prepare("SELECT name,type FROM pragma_table_info('revisions') ORDER BY cid")
        .map_err(|e| e.to_string())?;
    let actual = info
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let expected = [
        ("id", "INTEGER"),
        ("path", "TEXT"),
        ("created", "INTEGER"),
        ("label", "TEXT"),
        ("reason", "TEXT"),
        ("content", "BLOB"),
    ];
    if objects != 0
        || actual
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect::<Vec<_>>()
            != expected
    {
        return Err("Schema de histórico desconhecido; store preservado".into());
    }
    drop(info);
    let size: i64 = db
        .query_row("PRAGMA page_size", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    db.execute_batch(&format!(
        "PRAGMA secure_delete=ON; PRAGMA max_page_count={};",
        128 * 1024 * 1024 / size.max(1)
    ))
    .map_err(|e| e.to_string())?;
    Ok(db)
}
fn path_text(path: &Path) -> Result<&str, String> {
    let s = path.to_str().ok_or("Histórico requer caminho UTF-8")?;
    if s.len() > 4096 {
        return Err("Caminho longo para histórico".into());
    }
    Ok(s)
}
fn record(
    tx: &rusqlite::Transaction<'_>,
    path: &str,
    text: &str,
    label: Option<&str>,
    reason: &str,
) -> rusqlite::Result<()> {
    if text.len() > MAX_CONTENT {
        return Err(rusqlite::Error::InvalidParameterName(
            "Snapshot excede16MiB".into(),
        ));
    }
    if label.is_none() {
        let last: Option<Vec<u8>> = tx
            .query_row(
                "SELECT content FROM revisions WHERE path=?1 ORDER BY id DESC LIMIT 1",
                [path],
                |r| r.get(0),
            )
            .optional()?;
        if last.as_deref() == Some(text.as_bytes()) {
            return Ok(());
        }
    }
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64;
    tx.execute(
        "INSERT INTO revisions(path,created,label,reason,content) VALUES(?1,?2,?3,?4,?5)",
        params![path, ms, label, reason, text.as_bytes()],
    )?;
    Ok(())
}
fn process(directory: &Path, request: Request) -> Result<ResultData, String> {
    let mut db = open(directory)?;
    match request {
        Request::Record {
            path,
            before,
            after,
            label,
            reason,
        } => {
            let name = path_text(&path)?;
            let text = match after {
                Content::Text(s) => s,
                Content::Snapshot(s) => Arc::from(s.to_string()),
            };
            let tx = db
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(|e| e.to_string())?;
            if let Some(before) = before {
                record(&tx, name, &before, None, "Before Save").map_err(|e| e.to_string())?;
            }
            record(&tx, name, &text, label.as_deref(), reason).map_err(|e| e.to_string())?;
            tx.execute("DELETE FROM revisions WHERE path=?1 AND id NOT IN(SELECT id FROM revisions WHERE path=?1 ORDER BY id DESC LIMIT 32)",[name]).map_err(|e|e.to_string())?;
            tx.execute("DELETE FROM revisions WHERE id NOT IN(SELECT id FROM revisions ORDER BY id DESC LIMIT ?1)",[MAX_ROWS]).map_err(|e|e.to_string())?;
            tx.execute("DELETE FROM revisions WHERE id IN(SELECT id FROM(SELECT id,SUM(length(content)) OVER(ORDER BY id DESC) AS bytes FROM revisions) WHERE bytes>?1)",[MAX_TOTAL]).map_err(|e|e.to_string())?;
            tx.commit().map_err(|e| e.to_string())?;
            Ok(ResultData::Recorded(path))
        }
        Request::List { path, token } => {
            let mut statement=db.prepare("SELECT id,strftime('%Y-%m-%d %H:%M:%S',created/1000.0,'unixepoch','localtime'),label,reason,length(content) FROM revisions WHERE path=?1 ORDER BY id DESC LIMIT 32").map_err(|e|e.to_string())?;
            let rows = statement
                .query_map([path_text(&path)?], |r| {
                    Ok(Revision {
                        id: r.get::<_, i64>(0)? as u64,
                        date: r.get(1)?,
                        label: r.get(2)?,
                        reason: r.get(3)?,
                        bytes: r.get::<_, i64>(4)? as usize,
                    })
                })
                .map_err(|e| e.to_string())?;
            let items = rows
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            Ok(ResultData::Listed { path, token, items })
        }
        Request::Read { path, id, token } => {
            let bytes: Vec<u8> = db
                .query_row(
                    "SELECT content FROM revisions WHERE path=?1 AND id=?2 AND length(content)<=?3",
                    params![
                        path_text(&path)?,
                        i64::try_from(id).map_err(|_| "Id de revisão inválido")?,
                        MAX_CONTENT as i64
                    ],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            let text = String::from_utf8(bytes).map_err(|e| e.to_string())?;
            Ok(ResultData::Read {
                path,
                token,
                id,
                text,
            })
        }
    }
}

impl AppService {
    pub(super) fn show_local_history(&mut self, id: DocumentId) -> Result<(), String> {
        let doc = self
            .documents
            .iter()
            .find(|d| d.id == id && !d.untitled)
            .ok_or("Abra um arquivo salvo para consultar o histórico")?;
        self.local_history.list(doc.path.clone())
    }
    pub(super) fn select_local_revision(&mut self, id: u64) -> Result<(), String> {
        let view = self
            .local_history
            .view
            .as_ref()
            .filter(|v| v.revisions.iter().any(|r| r.id == id))
            .ok_or("Revisão não pertence ao histórico aberto")?;
        self.local_history.read(view.path.clone(), id)
    }
    pub(super) fn label_local_history(
        &mut self,
        id: DocumentId,
        version: u64,
        label: String,
    ) -> Result<(), String> {
        let doc = self
            .documents
            .iter()
            .find(|d| d.id == id && !d.untitled && d.version == version)
            .ok_or("Contexto do checkpoint mudou; tente novamente")?;
        self.local_history
            .label(doc.path.clone(), doc.snapshot(), label)
    }
    pub(super) fn restore_local_revision(
        &mut self,
        id: DocumentId,
        version: u64,
        revision: u64,
    ) -> Result<(), String> {
        if self.pending.values().any(|p| *p == Pending::Save(id)) {
            return Err("Aguarde o Save antes de restaurar".into());
        }
        let doc = self
            .documents
            .iter()
            .find(|d| d.id == id && d.version == version && !d.untitled)
            .ok_or("Documento ou versão mudou; restauração cancelada")?;
        let preview = self
            .local_history
            .view
            .as_ref()
            .filter(|v| v.path == doc.path)
            .and_then(|v| v.preview.as_ref())
            .filter(|p| p.id == revision)
            .ok_or("Selecione e carregue uma revisão deste arquivo")?;
        let text = preview.text.to_string();
        self.execute(Command::Edit {
            id,
            expected_version: version,
            text,
        })?;
        self.status = "Revisão restaurada no editor; Ctrl+Z desfaz, Ctrl+S salva".into();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn global_byte_row_limits_and_cross_path_revision_reads_are_enforced() {
        let dir = tempfile::tempdir().unwrap();
        let text: Arc<str> = Arc::from("x".repeat(MAX_CONTENT));
        for i in 0..5 {
            process(
                dir.path(),
                Request::Record {
                    path: PathBuf::from(format!("/owned/{i}")),
                    before: None,
                    after: Content::Text(text.clone()),
                    label: None,
                    reason: "Opened",
                },
            )
            .unwrap();
        }
        let mut db = open(dir.path()).unwrap();
        let bytes: i64 = db
            .query_row("SELECT sum(length(content)) FROM revisions", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(bytes, MAX_TOTAL);
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM revisions WHERE path='/owned/0'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        db.execute("DELETE FROM revisions", []).unwrap();
        let tx = db.transaction().unwrap();
        for i in 0..2049 {
            record(&tx, &format!("/owned/{i}"), "value", None, "Opened").unwrap();
        }
        tx.commit().unwrap();
        drop(db);
        process(
            dir.path(),
            Request::Record {
                path: PathBuf::from("/owned/last"),
                before: None,
                after: Content::Text(Arc::from("final")),
                label: None,
                reason: "Saved",
            },
        )
        .unwrap();
        let db = open(dir.path()).unwrap();
        assert_eq!(
            db.query_row("SELECT count(*) FROM revisions", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            MAX_ROWS
        );
        let id: i64 = db
            .query_row(
                "SELECT id FROM revisions WHERE path='/owned/last'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        drop(db);
        assert!(
            process(
                dir.path(),
                Request::Read {
                    path: PathBuf::from("/owned/other"),
                    id: id as u64,
                    token: 1
                }
            )
            .is_err()
        );
        assert!(
            process(
                dir.path(),
                Request::Read {
                    path: PathBuf::from("/owned/last"),
                    id: u64::MAX,
                    token: 1
                }
            )
            .is_err()
        );
    }
    #[test]
    fn unknown_schema_zero_and_symlink_storage_remain_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.sqlite3");
        let db = Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE sentinel(x); INSERT INTO sentinel VALUES('preserve');")
            .unwrap();
        drop(db);
        let before = fs::read(&path).unwrap();
        assert!(open(dir.path()).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            fs::rename(&path, dir.path().join("sentinel")).unwrap();
            symlink(dir.path().join("sentinel"), &path).unwrap();
            assert!(open(dir.path()).is_err());
            assert_eq!(fs::read(&path).unwrap(), before);
            let link = dir.path().join("linked-dir");
            symlink(dir.path(), &link).unwrap();
            assert!(open(&link).is_err());
        }
    }
    #[test]
    fn bounded_queue_reports_lost_record_and_old_errors_cannot_repopulate_current_view() {
        let (sender, _input) = mpsc::sync_channel(1);
        let (output, receiver) = mpsc::sync_channel(2);
        let mut service = Service {
            sender: Some(sender),
            receiver,
            pending: 0,
            token: 2,
            view: Some(View {
                path: PathBuf::from("new"),
                ..Default::default()
            }),
            error: None,
            warning: None,
        };
        service.record(PathBuf::from("new"), None, Arc::from("one"), "Opened");
        service.record(PathBuf::from("new"), None, Arc::from("two"), "Saved");
        assert!(service.warning.is_some());
        service.error = None;
        output.send((Some(1), Err("obsolete".into()))).unwrap();
        service.poll();
        assert!(service.error.is_none());
        assert_eq!(service.view.as_ref().unwrap().path, PathBuf::from("new"));
    }
}
