//! Codisseu application commands backed by one SQLite connection per worker.
//! Architecture and source decision: docs/contracts/database.md and
//! docs/db-reuse-provenance.md. No GUI dependency or automatic connection.
mod console_files;
mod mongo_options;
mod mongo_shell;
pub mod mongo_text;
pub mod query_export;
pub mod query_page;
pub mod remote;
pub mod remote_credentials;
mod remote_nosql;
mod remote_scope;
pub mod remote_sql;
pub mod sql_completion;
pub mod sql_formatting;
pub mod sql_text;
mod sqlite;
mod tls_roots;
pub mod transfer;
pub mod transfer_formats_extra;
pub mod transfer_nosql;
pub mod transfer_xlsx;
mod value;

use rusqlite::InterruptHandle;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError},
    },
};
pub use value::{Value, display_value, value_type_name};

pub const QUERY_LIMIT: usize = 500;
pub const CELL_BYTE_LIMIT: usize = 64 * 1024;
pub const RESULT_BYTE_LIMIT: usize = 4 * 1024 * 1024;
pub const SQL_BYTE_LIMIT: usize = 256 * 1024;
const QUEUE_SIZE: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    pub primary_key: bool,
    pub nullable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableInfo {
    pub name: String,
    pub columns: Vec<ColumnInfo>,
}

#[derive(Clone, Debug)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    pub truncated: bool,
    pub affected_rows: usize,
    pub elapsed_ms: u128,
    pub limit_reason: Option<String>,
}

#[derive(Debug)]
pub enum DbCommand {
    ExportQuery(query_export::QueryExportRequest),
    PreviewFileImport {
        path: PathBuf,
        request: transfer::ImportRequest,
    },
    ImportFileBatch {
        path: PathBuf,
        preview_id: u64,
    },
    Connect(PathBuf),
    Disconnect,
    RefreshSchema,
    Execute {
        sql: String,
    },
    ExecutePage(query_page::QueryPageRequest),
    Cancel,
}

struct Request {
    id: u64,
    generation: u64,
    command: DbCommand,
}
enum Outcome {
    Export(query_export::QueryExportReport),
    ImportPreview(transfer::ImportPreview),
    ImportReport(transfer::ImportReport),
    ImportClosed {
        report: Option<transfer::ImportReport>,
        error: String,
    },
    Connected {
        path: PathBuf,
        schema: Vec<TableInfo>,
    },
    Disconnected,
    Schema(Vec<TableInfo>),
    Query(QueryResult),
}
struct Response {
    id: u64,
    generation: u64,
    result: Result<Outcome, String>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pending {
    Export,
    Connection,
    Transfer,
    Schema,
    Query,
}

pub struct DbService {
    pub export_report: Option<query_export::QueryExportReport>,
    pub export_progress: query_export::QueryExportProgress,
    pub import_preview: Option<transfer::ImportPreview>,
    pub import_report: Option<transfer::ImportReport>,
    pub transfer_revision: u64,
    pub config: Option<PathBuf>,
    pub connected: bool,
    pub schema: Vec<TableInfo>,
    pub result: Option<QueryResult>,
    pub last_error: Option<String>,
    pub status: String,
    sender: Option<SyncSender<Request>>,
    receiver: Receiver<Response>,
    interrupt: Arc<Mutex<Option<InterruptHandle>>>,
    generation: Arc<AtomicU64>,
    running: Arc<AtomicBool>,
    stopped: Arc<AtomicBool>,
    pending: BTreeMap<u64, Pending>,
    next_id: u64,
    latest_query: u64,
    latest_schema: u64,
}

impl Default for DbService {
    fn default() -> Self {
        Self::new()
    }
}

impl DbService {
    /// No database is opened until an explicit Connect command.
    pub fn new() -> Self {
        let (sender, commands) = mpsc::sync_channel(QUEUE_SIZE);
        let (responses, receiver) = mpsc::sync_channel(QUEUE_SIZE);
        let interrupt = Arc::new(Mutex::new(None));
        let generation = Arc::new(AtomicU64::new(0));
        let running = Arc::new(AtomicBool::new(false));
        let stopped = Arc::new(AtomicBool::new(false));
        let export_progress = query_export::QueryExportProgress::default();
        let worker_progress = export_progress.clone();
        let worker_interrupt = Arc::clone(&interrupt);
        let worker_generation = Arc::clone(&generation);
        let worker_running = Arc::clone(&running);
        let worker_stopped = Arc::clone(&stopped);
        std::thread::Builder::new()
            .name("codisseu-sqlite".into())
            .spawn(move || {
                let mut backend = Backend {
                    export_progress: worker_progress,
                    prepared_import: None,
                    preview_sequence: 0,
                    session: None,
                    interrupt: worker_interrupt,
                    epoch: worker_generation,
                    running: worker_running,
                };
                while let Ok(request) = commands.recv() {
                    backend.running.store(true, Ordering::SeqCst);
                    let response = backend.handle(request);
                    backend.running.store(false, Ordering::SeqCst);
                    if responses.send(response).is_err() {
                        break;
                    }
                }
                if let Ok(mut handle) = backend.interrupt.lock() {
                    *handle = None;
                }
                drop(backend);
                worker_stopped.store(true, Ordering::SeqCst);
            })
            .expect("Não foi possível iniciar o worker SQLite");
        Self {
            export_report: None,
            export_progress,
            import_preview: None,
            import_report: None,
            transfer_revision: 0,
            config: None,
            connected: false,
            schema: Vec::new(),
            result: None,
            last_error: None,
            status: "SQLite desconectado · somente leitura".into(),
            sender: Some(sender),
            receiver,
            interrupt,
            generation,
            running,
            stopped,
            pending: BTreeMap::new(),
            next_id: 1,
            latest_query: 0,
            latest_schema: 0,
        }
    }

    pub fn is_busy(&self) -> bool {
        !self.pending.is_empty()
    }
    /// Identity of the currently published query result, suitable for presentation caches.
    /// Enqueueing a query or clearing the result returns `None` until a result is published.
    pub fn result_revision(&self) -> Option<u64> {
        self.result.as_ref().map(|_| self.latest_query)
    }
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
    pub fn worker_stopped(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }

    pub fn dispatch(&mut self, command: DbCommand) -> Result<(), String> {
        let result = self.submit(command);
        if let Err(error) = &result {
            self.last_error = Some(error.clone());
            self.status = error.clone();
        }
        result
    }

    fn submit(&mut self, command: DbCommand) -> Result<(), String> {
        if matches!(command, DbCommand::Cancel) {
            self.import_preview = None;
            self.cancel();
            self.status = if self.is_busy() {
                "Cancelamento solicitado ao SQLite; aguardando confirmação"
            } else {
                "Nenhuma operação SQLite em andamento"
            }
            .into();
            return Ok(());
        }
        if self.pending.values().any(|p| *p == Pending::Export) {
            return Err("Aguarde ou cancele a exportação SQLite".into());
        }
        let kind = match &command {
            DbCommand::ExportQuery(request) => {
                request.validate()?;
                if self.is_busy() {
                    return Err("Aguarde as operações SQLite antes de exportar".into());
                }
                Pending::Export
            }
            DbCommand::Connect(_) | DbCommand::Disconnect => {
                if self.is_busy() {
                    return Err("Aguarde ou cancele a operação antes de trocar a conexão".into());
                }
                Pending::Connection
            }
            DbCommand::PreviewFileImport { path, .. } | DbCommand::ImportFileBatch { path, .. } => {
                if self.is_busy() {
                    return Err("Aguarde as operações SQLite antes de importar".into());
                }
                if self.config.as_ref() != Some(path) {
                    return Err("Arquivo SQLite mudou; abra novamente a importação".into());
                }
                Pending::Transfer
            }
            DbCommand::RefreshSchema => Pending::Schema,
            DbCommand::Execute { sql } => {
                if sql.len() > SQL_BYTE_LIMIT {
                    return Err("SQL excede 256 KiB".into());
                }
                if sql.trim().is_empty() {
                    return Err("Selecione uma instrução SQL ou abra um documento SQL".into());
                }
                Pending::Query
            }
            DbCommand::ExecutePage(request) => {
                request.validate()?;
                Pending::Query
            }
            DbCommand::Cancel => unreachable!(),
        };
        if kind != Pending::Connection && !self.connected {
            return Err("Conecte um arquivo SQLite existente primeiro".into());
        }
        if kind != Pending::Connection && self.pending.values().any(|p| *p == Pending::Connection) {
            return Err("Aguarde a mudança de conexão SQLite".into());
        }
        if self.pending.len() >= QUEUE_SIZE {
            return Err("Fila SQLite cheia; aguarde ou cancele".into());
        }
        if kind == Pending::Export {
            self.export_report = None;
            self.export_progress.reset();
        }
        if matches!(command, DbCommand::PreviewFileImport { .. }) {
            self.import_preview = None;
            self.import_report = None;
        }
        let id = self.next_id;
        self.sender
            .as_ref()
            .ok_or("Worker SQLite encerrado")?
            .try_send(Request {
                id,
                generation: self.generation.load(Ordering::SeqCst),
                command,
            })
            .map_err(|_| "Worker SQLite indisponível ou fila cheia".to_owned())?;
        self.pending.insert(id, kind);
        self.next_id += 1;
        if kind == Pending::Query {
            self.latest_query = id;
            self.result = None;
        }
        if kind == Pending::Schema {
            self.latest_schema = id;
        }
        self.last_error = None;
        self.status = "SQLite: operação em andamento · somente leitura".into();
        Ok(())
    }

    fn cancel(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        if let Ok(guard) = self.interrupt.lock()
            && let Some(handle) = guard.as_ref()
        {
            handle.interrupt();
        }
    }

    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        loop {
            let reply = match self.receiver.try_recv() {
                Ok(reply) => reply,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if self.is_busy() {
                        self.pending.clear();
                        self.last_error = Some("Worker SQLite encerrou antes de responder".into());
                        self.status = self.last_error.clone().unwrap_or_default();
                        changed = true;
                    }
                    break;
                }
            };
            changed = true;
            let kind = self.pending.remove(&reply.id);
            if (kind == Some(Pending::Query) && reply.id != self.latest_query)
                || (kind == Some(Pending::Schema) && reply.id != self.latest_schema)
            {
                continue;
            }
            let cancelled = reply.generation != self.generation.load(Ordering::SeqCst);
            if cancelled
                && kind != Some(Pending::Connection)
                && !(kind == Some(Pending::Export)
                    && matches!(&reply.result, Ok(Outcome::Export(_))))
                && !(kind == Some(Pending::Transfer)
                    && matches!(
                        &reply.result,
                        Ok(Outcome::ImportReport(_) | Outcome::ImportClosed { .. })
                    ))
            {
                if kind != Some(Pending::Export) {
                    self.result = None;
                }
                match reply.result {
                    Err(error) => {
                        self.status = error.clone();
                        self.last_error = Some(error);
                    }
                    Ok(_) => {
                        self.status =
                            "Resultado descartado após cancelamento; operação já havia terminado"
                                .into();
                    }
                }
                continue;
            }
            match reply.result {
                Ok(Outcome::Export(report)) => {
                    self.status = format!(
                        "CSV salvo: {} linhas · {} bytes · {} ms",
                        report.rows, report.bytes, report.elapsed_ms
                    );
                    self.export_report = Some(report);
                }
                Ok(Outcome::ImportClosed { report, error }) => {
                    self.connected = false;
                    self.config = None;
                    self.schema.clear();
                    self.result = None;
                    self.import_report = report;
                    self.import_preview = None;
                    self.transfer_revision += 1;
                    self.status = error.clone();
                    self.last_error = Some(error);
                }
                Ok(Outcome::ImportPreview(preview)) => {
                    self.import_preview = Some(preview);
                    self.import_report = None;
                    self.transfer_revision += 1;
                    self.status = "Prévia SQLite; nenhuma linha gravada".into();
                }
                Ok(Outcome::ImportReport(report)) => {
                    self.status = report.summary.clone();
                    self.import_report = Some(report);
                    self.import_preview = None;
                    self.transfer_revision += 1;
                }
                Err(error) => {
                    self.status = error.clone();
                    self.last_error = Some(error);
                }
                Ok(Outcome::Connected { path, schema }) => {
                    self.config = Some(path);
                    self.connected = true;
                    self.schema = schema;
                    self.result = None;
                    self.status = "SQLite conectado · somente leitura".into();
                }
                Ok(Outcome::Disconnected) => {
                    self.connected = false;
                    self.config = None;
                    self.schema.clear();
                    self.result = None;
                    self.status = "SQLite desconectado".into();
                }
                Ok(Outcome::Schema(schema)) => {
                    self.schema = schema;
                    self.status = "Schema SQLite atualizado".into();
                }
                Ok(Outcome::Query(result)) => {
                    self.status = if let Some(reason) = &result.limit_reason {
                        format!(
                            "{} linhas · resultado limitado: {reason} · {} ms",
                            result.rows.len(),
                            result.elapsed_ms
                        )
                    } else {
                        format!(
                            "{} linhas · {} ms · somente leitura",
                            result.rows.len(),
                            result.elapsed_ms
                        )
                    };
                    self.result = Some(result);
                }
            }
        }
        changed
    }
}

impl Drop for DbService {
    fn drop(&mut self) {
        self.cancel();
        // Disconnect the sender. The interrupted worker then drops its only
        // connection. No blocking join or filesystem call on the GUI thread.
        self.sender.take();
    }
}

struct Backend {
    export_progress: query_export::QueryExportProgress,
    prepared_import: Option<(u64, u64, sqlite::PreparedFileImport)>,
    preview_sequence: u64,
    session: Option<sqlite::Session>,
    interrupt: Arc<Mutex<Option<InterruptHandle>>>,
    epoch: Arc<AtomicU64>,
    running: Arc<AtomicBool>,
}

impl Backend {
    fn handle(&mut self, request: Request) -> Response {
        let result = if self.epoch.load(Ordering::SeqCst) != request.generation {
            Err("Operação SQLite cancelada antes de iniciar".into())
        } else {
            self.apply(request.command, request.generation)
        };
        Response {
            id: request.id,
            generation: request.generation,
            result,
        }
    }

    fn apply(&mut self, command: DbCommand, generation: u64) -> Result<Outcome, String> {
        if let DbCommand::Connect(path) = command {
            self.prepared_import = None;
            let candidate = sqlite::Session::open(&path, Arc::clone(&self.epoch), generation)?;
            let schema = candidate.inspect()?;
            if generation != self.epoch.load(Ordering::SeqCst) {
                return Err("Abertura SQLite cancelada; conexão anterior preservada".into());
            }
            let path = candidate.path.clone();
            *self
                .interrupt
                .lock()
                .map_err(|_| "Controle SQLite indisponível")? = Some(candidate.interrupt_handle());
            self.session = Some(candidate);
            return Ok(Outcome::Connected { path, schema });
        }
        if matches!(command, DbCommand::Disconnect) {
            self.prepared_import = None;
            self.interrupt
                .lock()
                .map_err(|_| "Controle SQLite indisponível")?
                .take();
            self.session.take();
            return Ok(Outcome::Disconnected);
        }
        if matches!(command, DbCommand::PreviewFileImport { .. }) {
            self.prepared_import = None;
        }
        if let DbCommand::ImportFileBatch { path, preview_id } = &command {
            let (token, preview_generation, plan) = self
                .prepared_import
                .take()
                .ok_or("Prepare uma prévia SQLite primeiro")?;
            if preview_generation != generation
                || token != *preview_id
                || plan.path != *path
                || self.session.as_ref().is_none_or(|s| s.path != *path)
            {
                return Err("Prévia SQLite obsoleta; prepare novamente".into());
            }
            let current =
                self.session
                    .as_ref()
                    .unwrap()
                    .prepare_import(&transfer::ImportRequest {
                        allow_partial: true,
                        target: transfer::ImportTarget::SqlTable(remote::TableRef {
                            namespace: "main".into(),
                            name: plan.table.clone(),
                        }),
                        data: plan.data.clone(),
                        mapping: plan
                            .data
                            .columns
                            .iter()
                            .enumerate()
                            .map(|(i, c)| transfer::ImportMapping {
                                source_column: i,
                                target_column: c.clone(),
                            })
                            .collect(),
                    })?;
            if current.schema != plan.schema {
                return Err("Schema SQLite mudou após a prévia".into());
            }
            self.interrupt
                .lock()
                .map_err(|_| "Controle SQLite indisponível")?
                .take();
            self.session.take();
            let report =
                sqlite::import_file(&plan, self.epoch.clone(), generation, &self.interrupt);
            self.interrupt
                .lock()
                .map_err(|_| "Controle SQLite indisponível")?
                .take();
            let restored = sqlite::Session::open(
                &plan.path,
                self.epoch.clone(),
                self.epoch.load(Ordering::SeqCst),
            );
            if let Ok(session) = restored {
                *self
                    .interrupt
                    .lock()
                    .map_err(|_| "Controle SQLite indisponível")? =
                    Some(session.interrupt_handle());
                self.session = Some(session);
            } else {
                let error = format!(
                    "{}; falha ao restaurar leitura SQLite; reconecte",
                    report
                        .as_ref()
                        .map(|r| r.summary.as_str())
                        .unwrap_or("Importação falhou")
                );
                return Ok(Outcome::ImportClosed {
                    report: report.ok(),
                    error,
                });
            }
            return report.map(Outcome::ImportReport);
        }
        let session = self.session.as_ref().ok_or("SQLite está desconectado")?;
        session.begin_request(Arc::clone(&self.epoch), generation)?;
        match command {
            DbCommand::ExportQuery(request) => session
                .export_query(
                    &request,
                    self.export_progress.clone(),
                    self.epoch.clone(),
                    generation,
                )
                .map(Outcome::Export),
            DbCommand::PreviewFileImport { path, request } => {
                if path != session.path {
                    return Err("Arquivo SQLite mudou".into());
                }
                let plan = session.prepare_import(&request)?;
                self.preview_sequence += 1;
                let preview = transfer::ImportPreview {
                    preview_id: self.preview_sequence,
                    rows: plan.data.rows.len(),
                    columns: plan.data.columns.clone(),
                    description: format!(
                        "INSERT em {}; arquivo aberto temporariamente para escrita somente neste lote",
                        plan.table
                    ),
                    warnings: if plan.partial {
                        vec![format!("Origem truncada: somente{}linhas carregadas",plan.data.rows.len()),"SQLite faz Commit atômico imediato ao aplicar; nenhuma transação manual permanece aberta".into()]
                    } else {
                        vec!["SQLite faz Commit atômico imediato ao aplicar; nenhuma transação manual permanece aberta".into()]
                    },
                };
                self.prepared_import = Some((self.preview_sequence, generation, plan));
                Ok(Outcome::ImportPreview(preview))
            }
            DbCommand::Execute { sql } => {
                self.prepared_import = None;
                session.select(&sql).map(Outcome::Query)
            }
            DbCommand::ExecutePage(request) => {
                self.prepared_import = None;
                session.select_page(&request).map(Outcome::Query)
            }
            DbCommand::RefreshSchema => session.inspect().map(Outcome::Schema),
            _ => Err("Pedido inválido ao worker SQLite".into()),
        }
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn dropping_service_closes_the_worker() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("shutdown.sqlite");
        drop(rusqlite::Connection::open(&path).unwrap());
        let mut db = DbService::new();
        db.dispatch(DbCommand::Connect(path.clone())).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while db.is_busy() {
            db.poll();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(db.connected, "{:?}", db.last_error);
        db.dispatch(DbCommand::Execute { sql: "WITH RECURSIVE endless(x) AS (SELECT 0 UNION ALL SELECT x+1 FROM endless) SELECT sum(x) FROM endless".into() }).unwrap();
        while !db.is_running() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        std::thread::sleep(Duration::from_millis(20));
        let stopped = Arc::clone(&db.stopped);
        drop(db);
        let deadline = Instant::now() + Duration::from_secs(2);
        while !stopped.load(Ordering::SeqCst) {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        let probe = rusqlite::Connection::open(&path).unwrap();
        probe.busy_timeout(Duration::from_millis(50)).unwrap();
        probe.execute_batch("BEGIN EXCLUSIVE; ROLLBACK;").unwrap();
    }
}
