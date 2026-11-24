//! Independent native SQL adapters. One connection, explicit transactions, bounded previews.
use crate::cell_edit::{base_type, validate_cell_edit};
use crate::remote::{
    CatalogPage, CatalogRequest, Cell, ConnectionConfig, DbObject, EditBatch, Engine, RemoteColumn,
    RemoteResult, TableRef,
};
use futures_util::TryStreamExt;
use sqlparser::{
    ast::Statement,
    dialect::{MySqlDialect, PostgreSqlDialect},
    parser::Parser,
};
mod sqlx {
    pub use sqlx_core::{
        Either, Error, column::Column, connection::Connection, executor::Executor, query::query,
        raw_sql::raw_sql, row::Row, statement::Statement, type_info::TypeInfo, types,
        value::ValueRef,
    };
    pub use sqlx_mysql::{self as mysql, MySqlConnection};
    pub use sqlx_postgres::{self as postgres, PgConnection};
}
use sqlx::{
    Column, Connection, Executor, Row, Statement as _, TypeInfo, ValueRef,
    mysql::{MySqlConnectOptions, MySqlRow, MySqlSslMode},
    postgres::{PgConnectOptions, PgRow, PgSslMode},
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const MAX_SQL: usize = 256 * 1024;
const MAX_CELL: usize = 64 * 1024;
const MAX_BYTES: usize = 4 * 1024 * 1024;
const MAX_COLUMNS: usize = 256;
const MAX_METADATA_BYTES: usize = 1024 * 1024;

enum ConnectionKind {
    Pg(sqlx::PgConnection),
    My(sqlx::MySqlConnection),
}

#[derive(Clone, Debug)]
pub(crate) struct PreparedSqlImport {
    pub table: TableRef,
    pub schema: Vec<RemoteColumn>,
    pub columns: Vec<String>,
    sql: String,
    values: Vec<Vec<ImportValue>>,
}
impl PreparedSqlImport {
    pub fn len(&self) -> usize {
        self.values.len()
    }
}
#[derive(Clone, Debug)]
enum ImportValue {
    Text(Option<String>),
    Binary(Option<Vec<u8>>),
}
fn is_binary_type(kind: &str) -> bool {
    let kind = kind.trim().to_ascii_lowercase();
    let kind = kind.strip_prefix("generated ").unwrap_or(&kind);
    let kind = kind.split('(').next().unwrap_or(kind).trim();
    matches!(
        kind,
        "bytea" | "blob" | "tinyblob" | "mediumblob" | "longblob" | "binary" | "varbinary"
    )
}
fn import_value(cell: &Cell, column: &RemoteColumn, pg: bool) -> Result<ImportValue, String> {
    if is_binary_type(&column.data_type) {
        return match cell {
            Cell::Null => Ok(ImportValue::Binary(None)),
            Cell::Binary(v) => Ok(ImportValue::Binary(Some(v.clone()))),
            Cell::Text(v) if v.starts_with("0x") => {
                let hex = &v[2..];
                if hex.len() % 2 != 0 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err("Binário hexadecimal inválido".into());
                }
                let bytes = (0..hex.len())
                    .step_by(2)
                    .map(|i| {
                        u8::from_str_radix(&hex[i..i + 2], 16)
                            .map_err(|_| "Binário hexadecimal inválido".to_owned())
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(ImportValue::Binary(Some(bytes)))
            }
            _ => Err("Coluna binária exige bytes ou texto0xhex".into()),
        };
    }
    // COLUMN_TYPE retains UNSIGNED and dimensions for the cell editor. Imports
    // must still classify numeric types before the server can coerce text.
    let kind = base_type(column)
        .trim_end_matches(" zerofill")
        .trim_end_matches(" unsigned")
        .to_owned();
    let value = bound_cell(cell, pg)?;
    if let Some(v) = &value {
        if matches!(
            kind.as_str(),
            "smallint"
                | "integer"
                | "bigint"
                | "numeric"
                | "real"
                | "double precision"
                | "int"
                | "tinyint"
                | "mediumint"
                | "decimal"
                | "float"
                | "double"
        ) {
            let raw: Box<serde_json::value::RawValue> = serde_json::from_str(v)
                .map_err(|_| format!("Valor não numérico para {}", column.name))?;
            if !raw
                .get()
                .starts_with(['-', '0', '1', '2', '3', '4', '5', '6', '7', '8', '9'])
            {
                return Err(format!("Valor não numérico para {}", column.name));
            }
        }
        if kind == "boolean" && !matches!(v.as_str(), "true" | "false") {
            return Err("Booleano PostgreSQL deve ser true/false".into());
        }
        if matches!(kind.as_str(), "json" | "jsonb") {
            serde_json::from_str::<Box<serde_json::value::RawValue>>(v)
                .map_err(|_| "JSON de destino inválido")?;
        }
    }
    Ok(ImportValue::Text(value))
}

pub struct SqlSession {
    connection: Option<ConnectionKind>,
    config: ConnectionConfig,
    cancel: Arc<AtomicBool>,
    transaction: bool,
    /// Native SQL may change schema, role, variables or temporary objects. Once
    /// writes were allowed, reconnecting cannot silently restore this context.
    explicit_reconnect: bool,
    interrupted: bool,
    backend_id: u64,
    mariadb: bool,
    password: zeroize::Zeroizing<String>,
}

impl SqlSession {
    async fn open_connection(
        config: &ConnectionConfig,
        password: &str,
    ) -> Result<ConnectionKind, String> {
        let deadline = Duration::from_secs(config.timeout_secs.clamp(1, 120));
        let connection = match config.engine {
            Engine::Postgres | Engine::AuroraPostgres => {
                let mut options = PgConnectOptions::new()
                    .host(&config.host)
                    .port(config.port)
                    .database(&config.database)
                    .username(&config.username)
                    .password(password)
                    .application_name("Codisseu")
                    .ssl_mode(if config.tls {
                        PgSslMode::VerifyFull
                    } else {
                        PgSslMode::Disable
                    })
                    .statement_cache_capacity(0);
                if let Some(pem) = crate::tls_roots::material(config)? {
                    options = options.ssl_root_cert_from_pem(pem.to_vec());
                }
                ConnectionKind::Pg(
                    tokio::time::timeout(deadline, sqlx::PgConnection::connect_with(&options))
                        .await
                        .map_err(|_| "Tempo limite ao conectar PostgreSQL".to_owned())?
                        .map_err(db_error)?,
                )
            }
            Engine::Mysql | Engine::AuroraMysql => {
                let mut options = MySqlConnectOptions::new()
                    .host(&config.host)
                    .port(config.port)
                    .database(&config.database)
                    .username(&config.username)
                    .ssl_mode(if config.tls {
                        MySqlSslMode::VerifyIdentity
                    } else {
                        MySqlSslMode::Disabled
                    })
                    .statement_cache_capacity(0);
                if !password.is_empty() {
                    options = options.password(password);
                }
                if let Some(pem) = crate::tls_roots::material(config)? {
                    options = options.ssl_ca_from_pem(pem.to_vec());
                }
                ConnectionKind::My(
                    tokio::time::timeout(deadline, sqlx::MySqlConnection::connect_with(&options))
                        .await
                        .map_err(|_| "Tempo limite ao conectar MySQL".to_owned())?
                        .map_err(db_error)?,
                )
            }
            _ => return Err("Motor incompatível com o adaptador SQL".into()),
        };
        Ok(connection)
    }

    pub async fn connect(config: &ConnectionConfig, password: &str) -> Result<Self, String> {
        let mut connection = Self::open_connection(config, password).await?;
        let mut mariadb = false;
        let backend_id = match &mut connection {
            ConnectionKind::Pg(c) => sqlx::query("SELECT pg_backend_pid()::bigint")
                .fetch_one(c)
                .await
                .map_err(db_error)?
                .get::<i64, _>(0) as u64,
            ConnectionKind::My(c) => {
                let row = sqlx::query("SELECT CONNECTION_ID(),VERSION()")
                    .fetch_one(c)
                    .await
                    .map_err(db_error)?;
                mariadb = row.get::<String, _>(1).contains("MariaDB");
                row.get::<u64, _>(0)
            }
        };
        let mut session = Self {
            interrupted: false,
            backend_id,
            mariadb,
            password: zeroize::Zeroizing::new(password.to_owned()),
            connection: Some(connection),
            config: config.clone(),
            cancel: Arc::new(AtomicBool::new(false)),
            transaction: false,
            explicit_reconnect: !config.read_only,
        };
        let millis = session.config.timeout_secs.clamp(1, 120) * 1000;
        let mysql_timeout = session.mysql_timeout(millis);
        match session.connection.as_mut().unwrap() {
            ConnectionKind::Pg(connection) => {
                for sql in [
                    format!("SET statement_timeout = '{millis}ms'"),
                    "SET lock_timeout = '1000ms'".into(),
                    format!(
                        "SET idle_in_transaction_session_timeout = '{}ms'",
                        session.config.idle_secs.clamp(5, 3600) * 1000
                    ),
                    format!(
                        "SET default_transaction_read_only = {}",
                        if session.config.read_only {
                            "on"
                        } else {
                            "off"
                        }
                    ),
                ] {
                    sqlx::query(&sql)
                        .execute(&mut *connection)
                        .await
                        .map_err(db_error)?;
                }
            }
            ConnectionKind::My(connection) => {
                for sql in [
                    mysql_timeout,
                    "SET SESSION sql_mode = CONCAT_WS(',',@@SESSION.sql_mode,'STRICT_ALL_TABLES')"
                        .into(),
                    "SET SESSION innodb_lock_wait_timeout = 1".into(),
                    "SET SESSION lock_wait_timeout = 1".into(),
                    format!(
                        "SET SESSION TRANSACTION READ {}",
                        if session.config.read_only {
                            "ONLY"
                        } else {
                            "WRITE"
                        }
                    ),
                    format!(
                        "SET SESSION wait_timeout = {}",
                        session.config.idle_secs.clamp(5, 3600)
                    ),
                ] {
                    sqlx::query(&sql)
                        .execute(&mut *connection)
                        .await
                        .map_err(db_error)?;
                }
            }
        }
        Ok(session)
    }

    pub fn set_cancel_flag(&mut self, cancel: Arc<AtomicBool>) {
        self.cancel = cancel;
    }
    pub fn is_connected(&self) -> bool {
        self.connection.is_some() && !self.interrupted
    }
    pub fn is_closed(&self) -> bool {
        self.connection.is_none() || self.interrupted
    }
    pub fn transaction_active(&self) -> bool {
        self.transaction
    }
    pub fn requires_explicit_reconnect(&self) -> bool {
        self.explicit_reconnect
    }
    fn postgres(&self) -> bool {
        matches!(
            self.config.engine,
            Engine::Postgres | Engine::AuroraPostgres
        )
    }
    fn limit(&self) -> usize {
        self.config.row_limit.clamp(1, 500)
    }
    fn mysql_timeout(&self, millis: u64) -> String {
        if self.mariadb {
            format!(
                "SET SESSION max_statement_time = {}.{:03}",
                millis / 1000,
                millis % 1000
            )
        } else {
            format!("SET SESSION max_execution_time = {millis}")
        }
    }

    async fn control(&mut self, sql: &str) -> Result<(), String> {
        if !self.is_connected() {
            return Err("Sessão SQL encerrada; reconecte".into());
        }
        let future = async {
            match self
                .connection
                .as_mut()
                .ok_or("Conexão SQL encerrada; reconecte")?
            {
                ConnectionKind::Pg(connection) => {
                    sqlx::raw_sql(sql)
                        .execute(connection)
                        .await
                        .map_err(db_error)?;
                }
                ConnectionKind::My(connection) => {
                    sqlx::raw_sql(sql)
                        .execute(connection)
                        .await
                        .map_err(db_error)?;
                }
            }
            Ok(())
        };
        let result = match tokio::time::timeout(Duration::from_secs(3), future).await {
            Ok(result) => result,
            Err(_) => Err("Controle SQL excedeu prazo; conexão fechada".into()),
        };
        if result.is_err() {
            self.interrupted = true;
            self.transaction = false;
        }
        result
    }

    async fn begin(&mut self) -> Result<(), String> {
        if !self.transaction {
            let sql = if self.config.read_only {
                "START TRANSACTION READ ONLY"
            } else {
                "START TRANSACTION"
            };
            self.control(sql).await?;
            self.transaction = true;
        }
        Ok(())
    }

    /// Reexecute a complete SELECT in a read-only transaction on this connection.
    pub async fn export_query(
        &mut self,
        request: &crate::query_export::QueryExportRequest,
        progress: crate::query_export::QueryExportProgress,
    ) -> Result<crate::query_export::QueryExportReport, String> {
        request.validate()?;
        let canonical = validate_query(&request.sql, self.postgres())?;
        if self.transaction {
            return Err("Faça Commit ou Rollback antes de exportar a consulta completa".into());
        }
        let sql = format!("SELECT * FROM ({canonical}) AS codisseu_export");
        let postgres = self.postgres();
        let cancel = self.cancel.clone();
        let timeout = Duration::from_secs(request.timeout_secs);
        let started = Instant::now();
        self.control("START TRANSACTION READ ONLY").await?;
        self.transaction = true;
        let wait_flag = self.cancel.clone();
        let future = async {
            if postgres {
                self.control(&format!(
                    "SET LOCAL statement_timeout = '{}ms'",
                    request.timeout_secs * 1000
                ))
                .await?;
            } else {
                self.control(&self.mysql_timeout(request.timeout_secs * 1000))
                    .await?;
            }
            let mut writer;
            match self
                .connection
                .as_mut()
                .ok_or("Conexão SQL encerrada; reconecte")?
            {
                ConnectionKind::Pg(connection) => {
                    let statement = connection.prepare(&sql).await.map_err(db_error)?;
                    let columns = statement
                        .columns()
                        .iter()
                        .map(|c| c.name().to_owned())
                        .collect::<Vec<_>>();
                    writer =
                        crate::query_export::CsvStreamWriter::new(request, &columns, progress)?;
                    let mut rows = sqlx::query(&sql).persistent(false).fetch(&mut *connection);
                    let mut count = 0u64;
                    while let Some(row) = rows.try_next().await.map_err(db_error)? {
                        if row.columns().iter().enumerate().any(|(i, _)| {
                            row.try_get_raw(i).is_ok_and(|v| {
                                !v.is_null() && v.as_bytes().is_ok_and(|b| b.len() > MAX_CELL)
                            })
                        }) {
                            return Err("Célula da exportação acima de 64 KiB".into());
                        }
                        writer.row(&pg_cells(&row)?)?;
                        count += 1;
                        if count.is_multiple_of(1024) {
                            tokio::task::yield_now().await;
                        }
                        if cancel.load(Ordering::SeqCst) || started.elapsed() >= timeout {
                            return Err("Exportação cancelada ou prazo excedido; sessão fechada; interrupção no servidor não confirmada".into());
                        }
                    }
                }
                ConnectionKind::My(connection) => {
                    let statement = connection.prepare(&sql).await.map_err(db_error)?;
                    let columns = statement
                        .columns()
                        .iter()
                        .map(|c| c.name().to_owned())
                        .collect::<Vec<_>>();
                    writer =
                        crate::query_export::CsvStreamWriter::new(request, &columns, progress)?;
                    let mut rows = sqlx::query(&sql).persistent(false).fetch(&mut *connection);
                    let mut count = 0u64;
                    while let Some(row) = rows.try_next().await.map_err(db_error)? {
                        if row.columns().iter().enumerate().any(|(i,_)|row.try_get_raw(i).is_ok_and(|v|!v.is_null() && <&[u8] as sqlx_core::decode::Decode<'_,sqlx_mysql::MySql>>::decode(v).is_ok_and(|b|b.len()>MAX_CELL))) { return Err("Célula da exportação acima de 64 KiB".into()); }
                        writer.row(&my_cells(&row)?)?;
                        count += 1;
                        if count.is_multiple_of(1024) {
                            tokio::task::yield_now().await;
                        }
                        if cancel.load(Ordering::SeqCst) || started.elapsed() >= timeout {
                            return Err("Exportação cancelada ou prazo excedido; sessão fechada; interrupção no servidor não confirmada".into());
                        }
                    }
                }
            }
            self.rollback().await?;
            if !postgres {
                self.control(&self.mysql_timeout(self.config.timeout_secs * 1000))
                    .await?;
            }
            Ok::<_, String>(writer)
        };
        let result = tokio::select! {
            result = tokio::time::timeout(timeout, future) => result.map_err(|_|"Prazo da exportação excedido; sessão fechada; interrupção no servidor não confirmada".to_owned()).and_then(|v|v),
            _ = wait_cancel(wait_flag) => Err("Exportação cancelada; sessão fechada; interrupção no servidor não confirmada".into()),
        };
        match result {
            Ok(writer) if !self.cancel.load(Ordering::SeqCst) && started.elapsed() < timeout => {
                writer.finish()
            }
            Ok(_) => {
                self.interrupted = true;
                self.transaction = false;
                Err("Exportação cancelada ou prazo excedido antes de publicar arquivo".into())
            }
            Err(error) => {
                self.interrupted = true;
                self.transaction = false;
                Err(error)
            }
        }
    }

    /// Applying this saved permission never commits pending user work.
    pub async fn set_read_only(&mut self, value: bool) -> Result<(), String> {
        if self.config.read_only == value {
            return Ok(());
        }
        if self.transaction {
            return Err("Faça Commit ou Rollback antes de alterar Read-only".into());
        }
        if !value {
            self.explicit_reconnect = true;
        }
        let sql = if self.postgres() {
            format!(
                "SET default_transaction_read_only = {}",
                if value { "on" } else { "off" }
            )
        } else {
            format!(
                "SET SESSION TRANSACTION READ {}",
                if value { "ONLY" } else { "WRITE" }
            )
        };
        self.control(&sql).await?;
        self.config.read_only = value;
        Ok(())
    }

    pub async fn execute(&mut self, text: &str) -> Result<RemoteResult, String> {
        if text.len() > MAX_SQL {
            return Err("Comando excede 256 KiB".into());
        }
        crate::sql_text::single_statement(
            text,
            if self.postgres() {
                crate::sql_text::SqlDialect::Postgres
            } else {
                crate::sql_text::SqlDialect::Mysql
            },
        )?;
        if validate_query(text, self.postgres()).is_ok() {
            return self
                .execute_page(&crate::query_page::QueryPageRequest::first(text.to_owned()))
                .await;
        }
        let kind = native_statement(text, self.postgres())?;
        if self.config.read_only && !readonly_inspection(text, self.postgres()) {
            return Err("Conexão Read-only: somente consultas e inspeção são permitidas. Desative Read-only e salve para executar escritas e comandos de sessão".into());
        }
        let temporary_read = self.config.read_only && !kind.diagnostics;
        if temporary_read {
            self.control("START TRANSACTION READ ONLY").await?;
            self.transaction = true;
        } else if !self.config.read_only && kind.manual_dml {
            self.begin().await?;
        }
        let savepoint = !temporary_read && self.transaction && kind.manual_dml;
        if savepoint {
            self.control("SAVEPOINT codisseu_console_query").await?;
        }
        // SHOW WARNINGS/ERRORS must see the previous server diagnostics: no
        // prepare, transaction setup or status probe may precede these commands.
        let mut result = self.query_mode(text, false).await;
        if kind.inspection
            && let Ok(result) = &mut result
        {
            // MySQL reports several SHOW/DESCRIBE fields with binary charset
            // even though their payload is textual metadata (Type, Key, ...).
            // Never apply this conversion to user table/query binary values.
            for cell in result.rows.iter_mut().flatten() {
                if let Cell::Binary(bytes) = cell
                    && let Ok(value) = String::from_utf8(bytes.clone())
                {
                    *cell = Cell::Text(value);
                }
            }
        }
        if self.is_connected() {
            if temporary_read {
                self.rollback().await?;
            }
            if savepoint {
                if result.is_err() {
                    self.control("ROLLBACK TO SAVEPOINT codisseu_console_query")
                        .await?;
                }
                self.control("RELEASE SAVEPOINT codisseu_console_query")
                    .await?;
            }
            if !kind.inspection && !kind.transaction_setting {
                self.sync_transaction().await?;
            }
        }
        result
    }

    /// SQLx does not expose server transaction status. Savepoint probes use
    /// server acknowledgements instead of guessing from DDL/SET text. They also
    /// detect implicit MySQL commits and PostgreSQL's aborted transactions.
    async fn sync_transaction(&mut self) -> Result<(), String> {
        let pg = self.postgres();
        let operation = async {
            match self.connection.as_mut().ok_or("Sessão SQL encerrada")? {
                ConnectionKind::Pg(c) => match sqlx::raw_sql("SAVEPOINT codisseu_state_probe")
                    .execute(&mut *c)
                    .await
                {
                    Ok(_) => {
                        sqlx::raw_sql("RELEASE SAVEPOINT codisseu_state_probe")
                            .execute(c)
                            .await
                            .map_err(db_error)?;
                        Ok(true)
                    }
                    Err(sqlx::Error::Database(e)) if e.code().as_deref() == Some("25P01") => {
                        Ok(false)
                    }
                    Err(sqlx::Error::Database(e)) if e.code().as_deref() == Some("25P02") => {
                        Ok(true)
                    }
                    Err(e) => Err(db_error(e)),
                },
                ConnectionKind::My(c) => {
                    sqlx::raw_sql("SAVEPOINT codisseu_state_probe")
                        .execute(&mut *c)
                        .await
                        .map_err(db_error)?;
                    let active = match sqlx::raw_sql("RELEASE SAVEPOINT codisseu_state_probe")
                        .execute(&mut *c)
                        .await
                    {
                        Ok(_) => true,
                        Err(sqlx::Error::Database(e))
                            if e.try_downcast_ref::<sqlx_mysql::MySqlDatabaseError>()
                                .is_some_and(|e| e.number() == 1305) =>
                        {
                            false
                        }
                        Err(e) => return Err(db_error(e)),
                    };
                    // autocommit=0 promises a manual session even before its
                    // first transactional table access; retain the close guard.
                    let automatic: i64 = sqlx::query("SELECT @@session.autocommit")
                        .fetch_one(c)
                        .await
                        .map_err(db_error)?
                        .try_get(0)
                        .map_err(db_error)?;
                    Ok(active || automatic == 0)
                }
            }
        };
        match tokio::time::timeout(Duration::from_secs(3), operation).await {
            Ok(Ok(active)) => {
                self.transaction = active;
                Ok(())
            }
            other => {
                self.interrupted = true;
                self.transaction = false;
                let detail = match other {
                    Ok(Err(e)) => e,
                    _ => "Prazo de sincronização excedido".into(),
                };
                Err(format!(
                    "Estado da transação {} não confirmado; sessão encerrada: {detail}",
                    if pg { "PostgreSQL" } else { "MySQL" }
                ))
            }
        }
    }

    pub async fn execute_page(
        &mut self,
        request: &crate::query_page::QueryPageRequest,
    ) -> Result<RemoteResult, String> {
        request.validate()?;
        let canonical = validate_query(&request.sql, self.postgres())?;
        let bounded = request.wrapped_sql(&canonical, self.limit());
        let temporary_read = self.config.read_only && !self.transaction;
        if temporary_read {
            // Read-only permission is enforced by the server as well as the
            // lexical guard. Writable standalone reads use server autocommit.
            self.control("START TRANSACTION READ ONLY").await?;
            self.transaction = true;
        }
        // A failed PostgreSQL statement aborts its transaction. A savepoint lets
        // a console typo recover without losing earlier uncommitted grid edits.
        let savepoint = !self.config.read_only && self.transaction;
        if savepoint {
            self.control("SAVEPOINT codisseu_console_query").await?;
        }
        let result = self.query(&bounded).await;
        if self.is_connected() {
            if self.config.read_only || temporary_read {
                // Standalone reads never keep snapshots/locks between queries.
                self.rollback().await?;
            } else if savepoint {
                if result.is_err() {
                    self.control("ROLLBACK TO SAVEPOINT codisseu_console_query")
                        .await?;
                }
                self.control("RELEASE SAVEPOINT codisseu_console_query")
                    .await?;
            }
        }
        result
    }

    /// Plan only, never ANALYZE. Server read-only mode also covers planner-time functions.
    pub async fn explain(&mut self, text: &str) -> Result<RemoteResult, String> {
        let canonical = validate_query(text, self.postgres())?;
        if self.transaction {
            return Err("Confirme ou reverta a transação antes de solicitar o plano".into());
        }
        self.control("START TRANSACTION READ ONLY").await?;
        self.transaction = true;
        let sql = if self.postgres() {
            format!("EXPLAIN (FORMAT TEXT) {canonical}")
        } else {
            format!("EXPLAIN {canonical}")
        };
        let result = self.query(&sql).await;
        if self.is_connected() {
            self.rollback().await?;
        }
        result
    }

    async fn query(&mut self, text: &str) -> Result<RemoteResult, String> {
        self.query_mode(text, true).await
    }

    async fn query_mode(&mut self, text: &str, prepared: bool) -> Result<RemoteResult, String> {
        if !self.is_connected() {
            return Err("Sessão SQL encerrada; reconecte".into());
        }
        let started = Instant::now();
        let limit = self.limit();
        let cancel = Arc::clone(&self.cancel);
        let deadline = Duration::from_secs(self.config.timeout_secs.clamp(1, 120) + 1);
        let mut recoverable_error = false;
        let mut map_query_error = |error| {
            recoverable_error = recoverable_query_error(&error);
            db_error(error)
        };
        let future = async {
            let mut result = RemoteResult {
                columns: Vec::new(),
                rows: Vec::new(),
                truncated: false,
                elapsed_ms: 0,
                affected_rows: 0,
            };
            let mut bytes = 0;
            match self
                .connection
                .as_mut()
                .ok_or("Conexão SQL encerrada; reconecte")?
            {
                ConnectionKind::Pg(connection) => {
                    if prepared {
                        let statement = connection
                            .prepare(text)
                            .await
                            .map_err(&mut map_query_error)?;
                        result.columns = statement
                            .columns()
                            .iter()
                            .map(|c| c.name().to_owned())
                            .collect();
                    }
                    if result.columns.len() > MAX_COLUMNS {
                        return Err("Resultado excede 256 colunas".into());
                    }
                    let mut stream = if prepared {
                        connection.fetch_many(sqlx::query(text).persistent(false))
                    } else {
                        sqlx::raw_sql(text).fetch_many(&mut *connection)
                    };
                    while let Some(item) = stream.try_next().await.map_err(&mut map_query_error)? {
                        let row = match item {
                            sqlx::Either::Left(done) => {
                                result.affected_rows =
                                    result.affected_rows.saturating_add(done.rows_affected());
                                continue;
                            }
                            sqlx::Either::Right(row) => row,
                        };
                        if result.columns.is_empty() {
                            result.columns =
                                row.columns().iter().map(|c| c.name().to_owned()).collect();
                            if result.columns.len() > MAX_COLUMNS {
                                return Err("Resultado excede 256 colunas".into());
                            }
                        }
                        if result.truncated || result.rows.len() == limit {
                            result.truncated = true;
                            continue;
                        }
                        if row.columns().iter().enumerate().any(|(i, _)| {
                            row.try_get_raw(i).is_ok_and(|raw| {
                                !raw.is_null() && raw.as_bytes().is_ok_and(|b| b.len() > MAX_CELL)
                            })
                        }) {
                            result.truncated = true;
                            continue;
                        }
                        let cells = pg_cells(&row)?;
                        if !accept_row(&cells, &mut bytes) {
                            result.truncated = true;
                            continue;
                        }
                        result.rows.push(cells);
                    }
                }
                ConnectionKind::My(connection) => {
                    if prepared {
                        let statement = connection
                            .prepare(text)
                            .await
                            .map_err(&mut map_query_error)?;
                        result.columns = statement
                            .columns()
                            .iter()
                            .map(|c| c.name().to_owned())
                            .collect();
                    }
                    if result.columns.len() > MAX_COLUMNS {
                        return Err("Resultado excede 256 colunas".into());
                    }
                    let mut stream = if prepared {
                        connection.fetch_many(sqlx::query(text).persistent(false))
                    } else {
                        sqlx::raw_sql(text).fetch_many(&mut *connection)
                    };
                    while let Some(item) = stream.try_next().await.map_err(&mut map_query_error)? {
                        let row = match item {
                            sqlx::Either::Left(done) => {
                                result.affected_rows =
                                    result.affected_rows.saturating_add(done.rows_affected());
                                continue;
                            }
                            sqlx::Either::Right(row) => row,
                        };
                        if result.columns.is_empty() {
                            result.columns =
                                row.columns().iter().map(|c| c.name().to_owned()).collect();
                            if result.columns.len() > MAX_COLUMNS {
                                return Err("Resultado excede 256 colunas".into());
                            }
                        }
                        if result.truncated || result.rows.len() == limit {
                            result.truncated = true;
                            continue;
                        }
                        if row.columns().iter().enumerate().any(|(i,_)| row.try_get_raw(i).is_ok_and(|raw| !raw.is_null() && <&[u8] as sqlx_core::decode::Decode<'_,sqlx_mysql::MySql>>::decode(raw).is_ok_and(|b|b.len()>MAX_CELL))) {result.truncated=true;continue;}
                        let cells = my_cells(&row)?;
                        if !accept_row(&cells, &mut bytes) {
                            result.truncated = true;
                            continue;
                        }
                        result.rows.push(cells);
                    }
                }
            }
            result.elapsed_ms = started.elapsed().as_millis();
            Ok(result)
        };
        let result = tokio::select! {
            result = tokio::time::timeout(deadline, future) => result.map_err(|_| "Prazo da consulta excedido; conexão fechada; o servidor também aplica timeout".to_owned()),
            _ = wait_cancel(cancel) => Err("Cancelamento solicitado; conexão fechada. Interrupção no servidor não confirmada; aguarde o timeout configurado".into()),
        };
        match result {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(error)) if recoverable_error => Err(error),
            Ok(Err(error)) | Err(error) => {
                self.interrupted = true;
                self.transaction = false;
                Err(error)
            }
        }
    }

    /// Lazy database names (MySQL) or schemas inside the current database (PG).
    /// Never enumerates application tables or reads table data.
    pub async fn namespaces(&mut self) -> Result<Vec<String>, String> {
        let sql = if self.postgres() {
            "SELECT schema_name FROM information_schema.schemata WHERE schema_name NOT IN ('pg_catalog','information_schema') AND schema_name NOT LIKE 'pg_toast%' AND schema_name NOT LIKE 'pg_temp_%' ORDER BY schema_name LIMIT 257"
        } else {
            "SELECT schema_name FROM information_schema.schemata ORDER BY schema_name LIMIT 257"
        };
        let result = self
            .metadata_query(sql, "Mais de 256 namespaces acessíveis")
            .await?;
        Ok(result.rows.iter().map(|row| text(&row[0])).collect())
    }

    /// Compatibility API: first bounded page. Page-aware callers use the cursor API.
    pub async fn inspect_namespace(&mut self, namespace: &str) -> Result<Vec<DbObject>, String> {
        self.inspect_namespace_page(CatalogRequest {
            namespace: namespace.into(),
            ..Default::default()
        })
        .await
        .map(|page| page.objects)
    }
    /// One explicit keyset page. Never scans application rows or allocates the whole catalog.
    pub async fn inspect_namespace_page(
        &mut self,
        request: CatalogRequest,
    ) -> Result<CatalogPage, String> {
        crate::remote::validate_catalog_request(&request)?;
        let pg = self.postgres();
        let sort = if pg {
            "table_name COLLATE \"C\""
        } else {
            "CAST(table_name AS BINARY)"
        };
        let mut predicates = vec![format!("table_schema={}", literal(&request.namespace, pg))];
        if !request.prefix.is_empty() {
            let prefix = literal(&request.prefix, pg);
            predicates.push(format!(
                "LOWER(LEFT(table_name,CHAR_LENGTH({prefix})))=LOWER({prefix})"
            ));
        }
        if let Some(after) = &request.after {
            predicates.push(format!("{sort}>{}", literal(after, pg)));
        }
        if let Some(before) = &request.before {
            predicates.push(format!("{sort}<{}", literal(before, pg)));
        }
        let descending = request.before.is_some();
        let sql = format!(
            "SELECT table_schema,table_name,table_type FROM information_schema.tables WHERE {} ORDER BY {sort} {} LIMIT 257",
            predicates.join(" AND "),
            if descending { "DESC" } else { "ASC" }
        );
        let result = self.metadata_rows(&sql).await?;
        let more = result.rows.len() > 256;
        let mut objects: Vec<DbObject> = result
            .rows
            .into_iter()
            .take(256)
            .map(|row| DbObject {
                namespace: text(&row[0]),
                name: text(&row[1]),
                kind: text(&row[2]),
                columns: vec![],
            })
            .collect();
        if descending {
            objects.reverse();
        }
        let previous = if (descending && more) || (!descending && request.after.is_some()) {
            objects.first().map(|o| o.name.clone())
        } else {
            None
        };
        let next = if (!descending && more) || descending {
            objects.last().map(|o| o.name.clone())
        } else {
            None
        };
        Ok(CatalogPage {
            request,
            objects,
            previous,
            next,
        })
    }

    /// Fetch columns and primary-key membership for exactly one qualified table.
    pub async fn columns_for_table(
        &mut self,
        table: &TableRef,
    ) -> Result<Vec<RemoteColumn>, String> {
        validate_metadata_name(&table.namespace)?;
        validate_metadata_name(&table.name)?;
        self.columns(table).await
    }

    async fn metadata_query(&mut self, sql: &str, overflow: &str) -> Result<RemoteResult, String> {
        let result = self.metadata_rows(sql).await?;
        if result.rows.len() > 256 {
            return Err(overflow.into());
        }
        Ok(result)
    }
    async fn metadata_rows(&mut self, sql: &str) -> Result<RemoteResult, String> {
        let old_limit = self.config.row_limit;
        self.config.row_limit = 257;
        let result = self.execute(sql).await;
        self.config.row_limit = old_limit;
        let result = result?;
        if result.truncated {
            return Err("Página de metadados excede limite de transferência".into());
        }
        let bytes: usize = result
            .rows
            .iter()
            .flatten()
            .map(|cell| match cell {
                Cell::Text(value) | Cell::Number(value) | Cell::Json(value) => value.len(),
                Cell::Binary(value) => value.len(),
                Cell::Null | Cell::Bool(_) => 1,
            })
            .sum();
        if bytes > MAX_METADATA_BYTES {
            return Err("Metadados excedem 1 MiB; restrinja o namespace/tabela".into());
        }
        Ok(result)
    }

    pub async fn inspect(&mut self) -> Result<Vec<DbObject>, String> {
        let sql = if self.postgres() {
            "SELECT table_schema, table_name, table_type FROM information_schema.tables WHERE table_schema NOT IN ('pg_catalog','information_schema') ORDER BY table_schema,table_name LIMIT 257".to_owned()
        } else {
            "SELECT table_schema, table_name, table_type FROM information_schema.tables WHERE table_schema = DATABASE() ORDER BY table_name LIMIT 257".to_owned()
        };
        let old_limit = self.config.row_limit;
        self.config.row_limit = 257;
        let result = self.execute(&sql).await;
        self.config.row_limit = old_limit;
        let result = result?;
        if result.truncated || result.rows.len() > 256 {
            return Err("Mais de 256 objetos: restrinja o database/schema da conexão".into());
        }
        Ok(result
            .rows
            .into_iter()
            .map(|row| DbObject {
                namespace: text(&row[0]),
                name: text(&row[1]),
                kind: text(&row[2]),
                columns: Vec::new(),
            })
            .collect())
    }

    async fn columns(&mut self, table: &TableRef) -> Result<Vec<RemoteColumn>, String> {
        // Names are values here; escaping is dialect-specific and metadata SQL stays one statement.
        let namespace = literal(&table.namespace, self.postgres());
        let name = literal(&table.name, self.postgres());
        let sql = if self.postgres() {
            format!(
                "SELECT c.column_name,CASE WHEN c.is_generated <> 'NEVER' OR c.is_identity='YES' THEN 'generated ' ELSE '' END || pg_catalog.format_type(a.atttypid,a.atttypmod),c.is_nullable,CASE WHEN EXISTS (SELECT 1 FROM information_schema.table_constraints tc JOIN information_schema.key_column_usage ku ON tc.constraint_name=ku.constraint_name AND tc.constraint_schema=ku.constraint_schema AND tc.table_name=ku.table_name WHERE tc.constraint_type='PRIMARY KEY' AND tc.table_schema=c.table_schema AND tc.table_name=c.table_name AND ku.column_name=c.column_name) THEN 'YES' ELSE 'NO' END FROM information_schema.columns c JOIN pg_catalog.pg_namespace n ON n.nspname=c.table_schema JOIN pg_catalog.pg_class t ON t.relnamespace=n.oid AND t.relname=c.table_name JOIN pg_catalog.pg_attribute a ON a.attrelid=t.oid AND a.attname=c.column_name WHERE c.table_schema={namespace} AND c.table_name={name} ORDER BY c.ordinal_position LIMIT 257"
            )
        } else {
            format!(
                "SELECT c.column_name,CONCAT(CASE WHEN c.generation_expression <> '' THEN 'generated ' ELSE '' END,c.column_type),c.is_nullable,CASE WHEN c.column_key='PRI' THEN 'YES' ELSE 'NO' END FROM information_schema.columns c WHERE c.table_schema={namespace} AND c.table_name={name} ORDER BY c.ordinal_position LIMIT 257"
            )
        };
        let result = self
            .metadata_query(&sql, "Tabela excede 256 colunas")
            .await?;
        if result.rows.is_empty() {
            return Err("Tabela não encontrada ou sem acesso".into());
        }
        Ok(result
            .rows
            .into_iter()
            .map(|r| RemoteColumn {
                name: text(&r[0]),
                data_type: text(&r[1]),
                nullable: text(&r[2]) == "YES",
                primary_key: text(&r[3]) == "YES",
            })
            .collect())
    }

    pub async fn load_table(
        &mut self,
        table: &TableRef,
    ) -> Result<(RemoteResult, Vec<RemoteColumn>), String> {
        self.load_table_page(table, 0).await
    }
    pub async fn load_table_page(
        &mut self,
        table: &TableRef,
        offset: u64,
    ) -> Result<(RemoteResult, Vec<RemoteColumn>), String> {
        if offset > 1_000_000 {
            return Err("Offset excede 1.000.000; use um filtro SELECT".into());
        }
        let columns = self.columns(table).await?;
        let order: Vec<_> = columns
            .iter()
            .filter(|c| c.primary_key)
            .map(|c| quote(&c.name, self.postgres()))
            .collect();
        if order.is_empty() && offset > 0 {
            return Err("Paginação exige chave primária para ordenar".into());
        }
        let suffix = if order.is_empty() {
            String::new()
        } else {
            format!(" ORDER BY {}", order.join(","))
        };
        let mut result = self
            .execute(&format!(
                "SELECT * FROM {}{} LIMIT {} OFFSET {offset}",
                table_name(table, self.postgres()),
                suffix,
                self.limit() + 1
            ))
            .await?;
        if !self.postgres() {
            normalize_mysql_text(&mut result, &columns)?;
        }
        Ok((result, columns))
    }

    /// Preserve the immutable compatibility API. The worker uses certify_result
    /// to normalize MySQL text whose wire metadata reports a binary collation.
    pub async fn editable_origin(
        &mut self,
        sql: &str,
        result: &RemoteResult,
    ) -> Result<Option<(TableRef, Vec<RemoteColumn>)>, String> {
        if result
            .rows
            .iter()
            .flatten()
            .any(|cell| matches!(cell, Cell::Binary(_)))
        {
            return Ok(None);
        }
        self.result_origin(sql, result).await
    }

    /// Certify and normalize a real SELECT result using authoritative table types.
    /// BLOB/unknown wire types retain their bytes and never gain write permission.
    pub async fn certify_result(
        &mut self,
        sql: &str,
        result: &mut RemoteResult,
    ) -> Result<Option<(TableRef, Vec<RemoteColumn>)>, String> {
        let Some(origin) = self.result_origin(sql, result).await? else {
            return Ok(None);
        };
        if !self.postgres() {
            normalize_mysql_text(result, &origin.1)?;
        }
        if result
            .rows
            .iter()
            .flatten()
            .any(|cell| matches!(cell, Cell::Binary(_)))
        {
            return Ok(None);
        }
        Ok(Some(origin))
    }

    /// Direct base-table projections must contain the complete actual PK.
    /// Projection order is preserved for row values and optimistic comparisons;
    /// calculated/renamed/duplicate columns and joins remain read-only.
    async fn result_origin(
        &mut self,
        sql: &str,
        result: &RemoteResult,
    ) -> Result<Option<(TableRef, Vec<RemoteColumn>)>, String> {
        if self.config.read_only {
            return Ok(None);
        }
        let Some(candidate) = editable_candidate(sql, self.postgres()) else {
            return Ok(None);
        };
        let table = if self.postgres() {
            // Resolve exactly what the successful query resolved, including search_path.
            // Traditional inheritance can duplicate parent keys; partitioned PKs are global.
            let relation = candidate
                .source
                .iter()
                .map(|part| quote(part, true))
                .collect::<Vec<_>>()
                .join(".");
            let lookup = format!(
                "SELECT n.nspname,c.relname FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE c.oid=pg_catalog.to_regclass({}) AND c.relkind IN ('r','p') AND (c.relkind='p' OR NOT EXISTS(SELECT 1 FROM pg_catalog.pg_inherits i WHERE i.inhparent=c.oid))",
                literal(&relation, true)
            );
            let resolved = self.execute(&lookup).await?;
            let Some(row) = resolved.rows.first() else {
                return Ok(None);
            };
            TableRef {
                namespace: text(&row[0]),
                name: text(&row[1]),
            }
        } else {
            let namespace = if candidate.source.len() == 2 {
                candidate.source[0].clone()
            } else {
                self.config.database.clone()
            };
            if namespace.is_empty() {
                return Ok(None);
            }
            let name = candidate.source.last().cloned().unwrap_or_default();
            let lookup = format!(
                "SELECT table_schema,table_name FROM information_schema.tables WHERE table_schema={} AND table_name={} AND table_type='BASE TABLE' AND engine='InnoDB'",
                literal(&namespace, false),
                literal(&name, false)
            );
            let resolved = self.execute(&lookup).await?;
            let Some(row) = resolved.rows.first() else {
                return Ok(None);
            };
            TableRef {
                namespace: text(&row[0]),
                name: text(&row[1]),
            }
        };
        let actual = self.columns(&table).await?;
        let names = match candidate.projection {
            ProjectionShape::All => actual.iter().map(|c| c.name.clone()).collect(),
            ProjectionShape::Columns(names) => names,
        };
        if names.is_empty()
            || names != result.columns
            || names
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != names.len()
            || result.rows.iter().any(|row| row.len() != names.len())
            || !actual.iter().any(|c| c.primary_key)
            || actual
                .iter()
                .filter(|c| c.primary_key)
                .any(|c| !names.contains(&c.name))
        {
            return Ok(None);
        }
        let Some(columns) = names
            .iter()
            .map(|name| actual.iter().find(|c| c.name == *name).cloned())
            .collect::<Option<Vec<_>>>()
        else {
            return Ok(None);
        };
        Ok(Some((table, columns)))
    }

    pub async fn apply(&mut self, batch: &EditBatch) -> Result<RemoteResult, String> {
        if self.config.read_only {
            return Err("Conexão somente leitura; habilite transação manual para editar".into());
        }
        if batch.edits.is_empty() || batch.edits.len() > 500 {
            return Err("Lote precisa conter entre 1 e 500 edições".into());
        }
        let actual = self.columns(&batch.table).await?;
        if batch.columns.is_empty()
            || batch
                .columns
                .iter()
                .map(|c| &c.name)
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != batch.columns.len()
            || batch.columns.iter().any(|c| !actual.contains(c))
            || actual
                .iter()
                .filter(|c| c.primary_key)
                .any(|c| !batch.columns.contains(c))
        {
            return Err("Schema mudou desde a leitura; recarregue a tabela".into());
        }
        if !actual.iter().any(|c| c.primary_key) {
            return Err("Edição exige chave primária real".into());
        }
        if !self.postgres() {
            let check = self.execute(&format!("SELECT ENGINE FROM information_schema.tables WHERE table_schema={} AND table_name={}",literal(&batch.table.namespace,false),literal(&batch.table.name,false))).await?;
            if check.rows.first().map(|r| text(&r[0]).to_uppercase()) != Some("INNODB".into()) {
                return Err("Edição MySQL exige tabela InnoDB transacional".into());
            }
        }
        // A normal base table only: views and traditional inheritance can make
        // an UPDATE touch a different/multiple row despite a plausible key.
        self.validate_import_table(&batch.table).await?;
        if self.postgres() {
            let check = self.execute(&format!("SELECT EXISTS(SELECT 1 FROM pg_catalog.pg_inherits i JOIN pg_catalog.pg_class c ON c.oid=i.inhparent JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname={} AND c.relname={} AND c.relkind <> 'p')",literal(&batch.table.namespace,true),literal(&batch.table.name,true))).await?;
            if check.rows.first().and_then(|r| r.first()) == Some(&Cell::Bool(true)) {
                return Err("Edição exige tabela sem herança tradicional".into());
            }
        }
        let statements = build_updates(batch, self.postgres())?;
        self.begin().await?;
        // Hold the relation's transaction-scoped metadata lock while checking
        // the schema again. Concurrent DDL cannot swap the target after validation.
        self.execute(&format!(
            "SELECT * FROM {} WHERE 1=0",
            table_name(&batch.table, self.postgres())
        ))
        .await?;
        if self.columns(&batch.table).await? != actual {
            return Err("Schema mudou durante o envio; recarregue a tabela".into());
        }
        self.validate_import_table(&batch.table).await?;
        self.control("SAVEPOINT codisseu_grid_batch").await?;
        let started = Instant::now();
        let mut affected = 0;
        for (sql, values) in statements {
            let cancel = Arc::clone(&self.cancel);
            let deadline = Duration::from_secs(self.config.timeout_secs.clamp(1, 120) + 1);
            let future = async {
                match self.connection.as_mut().ok_or("Conexão SQL encerrada")? {
                    ConnectionKind::Pg(connection) => {
                        let mut query = sqlx::query(&sql).persistent(false);
                        for value in values {
                            query = query.bind(value);
                        }
                        query
                            .execute(connection)
                            .await
                            .map(|r| r.rows_affected())
                            .map_err(db_error)
                    }
                    ConnectionKind::My(connection) => {
                        let mut query = sqlx::query(&sql).persistent(false);
                        for value in values {
                            query = query.bind(value);
                        }
                        query
                            .execute(connection)
                            .await
                            .map(|r| r.rows_affected())
                            .map_err(db_error)
                    }
                }
            };
            let outcome = tokio::select! {
                result = tokio::time::timeout(deadline,future) => result.map_err(|_|"Tempo de edição excedido; conexão fechada".to_owned()),
                _ = wait_cancel(cancel) => Err("Edição cancelada; conexão fechada; transação não confirmada".into()),
            };
            let count = match outcome {
                Err(error) => {
                    self.interrupted = true;
                    self.transaction = false;
                    return Err(error);
                }
                Ok(Err(error)) => {
                    self.control("ROLLBACK TO SAVEPOINT codisseu_grid_batch")
                        .await?;
                    self.control("RELEASE SAVEPOINT codisseu_grid_batch")
                        .await?;
                    return Err(error);
                }
                Ok(Ok(count)) => count,
            };
            if count != 1 {
                self.control("ROLLBACK TO SAVEPOINT codisseu_grid_batch")
                    .await?;
                self.control("RELEASE SAVEPOINT codisseu_grid_batch")
                    .await?;
                return Err(format!(
                    "Conflito: esperado 1 registro, recebido {count}; lote revertido"
                ));
            }
            affected += count;
        }
        self.control("RELEASE SAVEPOINT codisseu_grid_batch")
            .await?;
        Ok(RemoteResult {
            columns: Vec::new(),
            rows: Vec::new(),
            truncated: false,
            elapsed_ms: started.elapsed().as_millis(),
            affected_rows: affected,
        })
    }

    async fn validate_import_table(&mut self, table: &TableRef) -> Result<(), String> {
        if !self.postgres() {
            let check=self.execute(&format!("SELECT ENGINE FROM information_schema.tables WHERE table_schema={} AND table_name={}",literal(&table.namespace,false),literal(&table.name,false))).await?;
            if check.rows.first().map(|r| text(&r[0]).to_uppercase()) != Some("INNODB".into()) {
                return Err("Importação MySQL exige tabela InnoDB transacional".into());
            }
        } else {
            let check=self.execute(&format!("SELECT c.relkind::text FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname={} AND c.relname={}",literal(&table.namespace,true),literal(&table.name,true))).await?;
            if !check
                .rows
                .first()
                .is_some_and(|r| matches!(text(&r[0]).as_str(), "r" | "p"))
            {
                return Err("Importação exige tabela PostgreSQL real".into());
            }
        }
        Ok(())
    }
    pub(crate) async fn prepare_import(
        &mut self,
        request: &crate::transfer::ImportRequest,
    ) -> Result<PreparedSqlImport, String> {
        use crate::transfer::{ImportTarget, map_import};
        if self.config.read_only {
            return Err("Conexão somente leitura; reconecte em modo manual para importar".into());
        }
        let ImportTarget::SqlTable(table) = &request.target else {
            return Err("Escolha uma tabela SQL".into());
        };
        let data = map_import(request)?;
        let schema = self.columns(table).await?;
        if schema.is_empty() {
            return Err("Tabela inexistente ou sem colunas visíveis".into());
        }
        self.validate_import_table(table).await?;
        let mut columns = Vec::new();
        for name in &data.columns {
            columns.push(
                schema
                    .iter()
                    .find(|c| c.name == *name)
                    .cloned()
                    .ok_or_else(|| format!("Coluna {name} não existe na tabela"))?,
            );
        }
        if columns
            .iter()
            .any(|column| column.data_type.starts_with("generated "))
        {
            return Err("Coluna gerada/identidade não aceita importação explícita".into());
        }
        let pg = self.postgres();
        let mut placeholders = Vec::new();
        for (index, column) in columns.iter().enumerate() {
            placeholders.push(if is_binary_type(&column.data_type) {
                if pg {
                    format!("${}", index + 1)
                } else {
                    "?".into()
                }
            } else {
                parameter(index + 1, column, pg)?
            });
        }
        let values = data
            .rows
            .iter()
            .map(|row| {
                row.iter()
                    .zip(&columns)
                    .map(|(cell, column)| import_value(cell, column, pg))
                    .collect::<Result<Vec<_>, _>>()
            })
            .collect::<Result<Vec<_>, _>>()?;
        let sql = format!(
            "INSERT INTO {}.{} ({}) VALUES ({})",
            quote(&table.namespace, pg),
            quote(&table.name, pg),
            columns
                .iter()
                .map(|c| quote(&c.name, pg))
                .collect::<Vec<_>>()
                .join(","),
            placeholders.join(",")
        );
        Ok(PreparedSqlImport {
            table: table.clone(),
            schema,
            columns: data.columns,
            sql,
            values,
        })
    }
    pub(crate) async fn apply_import(
        &mut self,
        plan: &PreparedSqlImport,
    ) -> Result<crate::transfer::ImportReport, String> {
        if self.config.read_only {
            return Err("Conexão somente leitura".into());
        }
        self.begin().await?;
        // Open the real target inside the transaction before checking metadata.
        // PG AccessShare / MySQL MDL then prevent a concurrent DDL replacement
        // between the storage-engine/schema check and the bound INSERT batch.
        let pg = self.postgres();
        self.execute(&format!(
            "SELECT * FROM {}.{} WHERE 1=0",
            quote(&plan.table.namespace, pg),
            quote(&plan.table.name, pg)
        ))
        .await?;
        self.validate_import_table(&plan.table).await?;
        if self.columns(&plan.table).await? != plan.schema {
            return Err("Schema mudou após a prévia; prepare a importação novamente".into());
        }
        self.begin().await?;
        self.control("SAVEPOINT codisseu_import_batch").await?;
        for row in &plan.values {
            let result = match self.connection.as_mut().ok_or("Sessão SQL encerrada")? {
                ConnectionKind::Pg(connection) => {
                    let mut query = sqlx::query(&plan.sql).persistent(false);
                    for value in row {
                        query = match value {
                            ImportValue::Text(v) => query.bind(v.clone()),
                            ImportValue::Binary(v) => query.bind(v.clone()),
                        };
                    }
                    query
                        .execute(connection)
                        .await
                        .map(|r| r.rows_affected())
                        .map_err(db_error)
                }
                ConnectionKind::My(connection) => {
                    let mut query = sqlx::query(&plan.sql).persistent(false);
                    for value in row {
                        query = match value {
                            ImportValue::Text(v) => query.bind(v.clone()),
                            ImportValue::Binary(v) => query.bind(v.clone()),
                        };
                    }
                    query
                        .execute(connection)
                        .await
                        .map(|r| r.rows_affected())
                        .map_err(db_error)
                }
            };
            match result {
                Ok(1) => {}
                other => {
                    self.control("ROLLBACK TO SAVEPOINT codisseu_import_batch")
                        .await?;
                    self.control("RELEASE SAVEPOINT codisseu_import_batch")
                        .await?;
                    return Ok(crate::transfer::ImportReport {
                        attempted: plan.values.len(),
                        applied: 0,
                        failed: plan.values.len(),
                        unknown: 0,
                        transaction_pending: true,
                        atomic: true,
                        summary: format!(
                            "Lote revertido integralmente: {}",
                            match other {
                                Err(e) => e,
                                Ok(count) => format!("esperado1registro, recebido{count}"),
                            }
                        ),
                    });
                }
            }
        }
        self.control("RELEASE SAVEPOINT codisseu_import_batch")
            .await?;
        Ok(crate::transfer::ImportReport {
            attempted: plan.values.len(),
            applied: plan.values.len(),
            failed: 0,
            unknown: 0,
            transaction_pending: true,
            atomic: true,
            summary: format!(
                "{} linhas inseridas na transação; revise e faça Commit ou Rollback",
                plan.values.len()
            ),
        })
    }
    pub async fn commit(&mut self) -> Result<(), String> {
        if self.config.read_only {
            return Err("Conexão somente leitura".into());
        }
        if self.transaction {
            self.control("COMMIT").await?;
            self.transaction = false;
            self.sync_transaction().await?;
        }
        Ok(())
    }
    pub async fn rollback(&mut self) -> Result<(), String> {
        if self.transaction {
            self.control("ROLLBACK").await?;
            self.transaction = false;
            if !self.config.read_only {
                self.sync_transaction().await?;
            }
        }
        Ok(())
    }
    pub(crate) fn mark_interrupted(&mut self) {
        self.interrupted = true;
    }

    /// Stop our exact server session before releasing its socket. Keeping the
    /// original connection alive prevents the server ID being reused meanwhile.
    /// This short-lived control connection is only opened for uncertain work.
    async fn terminate_interrupted(&mut self) -> Result<(), String> {
        let cleanup = async {
            let mut control = Self::open_connection(&self.config, &self.password).await?;
            let result = match &mut control {
                ConnectionKind::Pg(c) => {
                    let row = sqlx::query("SELECT pg_terminate_backend($1)")
                        .bind(self.backend_id as i32)
                        .fetch_one(&mut *c)
                        .await
                        .map_err(db_error)?;
                    if row.get::<bool, _>(0) {
                        Ok(())
                    } else {
                        let remaining = sqlx::query(
                            "SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE pid = $1)",
                        )
                        .bind(self.backend_id as i32)
                        .fetch_one(c)
                        .await
                        .map_err(db_error)?;
                        if remaining.get::<bool, _>(0) {
                            Err("Servidor não confirmou encerramento da sessão".into())
                        } else {
                            Ok(())
                        }
                    }
                }
                ConnectionKind::My(c) => {
                    match sqlx::raw_sql(&format!("KILL CONNECTION {}", self.backend_id))
                        .execute(c)
                        .await
                    {
                        Ok(_) => Ok(()),
                        // Unknown thread means the original session already ended.
                        Err(sqlx::Error::Database(e)) if e.code().as_deref() == Some("1094") => {
                            Ok(())
                        }
                        Err(e) => Err(db_error(e)),
                    }
                }
            };
            match control {
                ConnectionKind::Pg(c) => {
                    let _ = c.close().await;
                }
                ConnectionKind::My(c) => {
                    let _ = c.close().await;
                }
            }
            result
        };
        tokio::time::timeout(Duration::from_secs(3), cleanup)
            .await
            .map_err(|_| "Prazo do encerramento remoto excedido".to_owned())?
    }

    pub async fn close(&mut self) -> Result<(), String> {
        let result = if self.interrupted {
            Ok(())
        } else {
            self.rollback().await
        };
        if self.interrupted && self.connection.is_some() {
            // Keep the original slot quarantined if remote termination cannot be
            // confirmed. The worker must not reconnect and accumulate sessions.
            self.terminate_interrupted().await?;
        }
        if let Some(connection) = self.connection.take() {
            // Never drain an interrupted result stream before closing it.
            let interrupted = self.interrupted;
            let future = async move {
                match connection {
                    ConnectionKind::Pg(c) => {
                        if interrupted {
                            c.close_hard().await
                        } else {
                            c.close().await
                        }
                    }
                    ConnectionKind::My(c) => {
                        if interrupted {
                            c.close_hard().await
                        } else {
                            c.close().await
                        }
                    }
                }
            };
            let _ = tokio::time::timeout(Duration::from_secs(2), future).await;
        }
        self.transaction = false;
        result
    }
}

fn normalize_mysql_text(result: &mut RemoteResult, columns: &[RemoteColumn]) -> Result<(), String> {
    // MySQL reports utf8*_bin textual columns with a BINARY flag; only real
    // catalog types can distinguish such text from BLOB/VARBINARY payloads.
    for row in &mut result.rows {
        for (value, column) in row.iter_mut().zip(columns) {
            if matches!(
                base_type(column).as_str(),
                "varchar"
                    | "char"
                    | "tinytext"
                    | "text"
                    | "mediumtext"
                    | "longtext"
                    | "enum"
                    | "set"
            ) && let Cell::Binary(bytes) = value
            {
                *value = Cell::Text(
                    String::from_utf8(bytes.clone())
                        .map_err(|_| "Texto MySQL contém UTF-8 inválido")?,
                );
            }
        }
    }
    Ok(())
}

enum ProjectionShape {
    All,
    Columns(Vec<String>),
}
struct EditableCandidate {
    source: Vec<String>,
    projection: ProjectionShape,
}
fn ident_name(ident: &sqlparser::ast::Ident, pg: bool) -> String {
    if pg && ident.quote_style.is_none() {
        ident.value.to_ascii_lowercase()
    } else {
        ident.value.clone()
    }
}
fn object_parts(name: &sqlparser::ast::ObjectName, pg: bool) -> Option<Vec<String>> {
    name.0
        .iter()
        .map(|part| part.as_ident().map(|ident| ident_name(ident, pg)))
        .collect()
}
fn plain_column(
    expr: &sqlparser::ast::Expr,
    qualifiers: &[Vec<String>],
    pg: bool,
) -> Option<String> {
    use sqlparser::ast::Expr;
    match expr {
        Expr::Identifier(name) => Some(ident_name(name, pg)),
        Expr::CompoundIdentifier(names) if names.len() > 1 => {
            let qualifier = names[..names.len() - 1]
                .iter()
                .map(|name| ident_name(name, pg))
                .collect::<Vec<_>>();
            qualifiers
                .contains(&qualifier)
                .then(|| ident_name(names.last().unwrap(), pg))
        }
        _ => None,
    }
}
fn plain_wildcard(options: &sqlparser::ast::WildcardAdditionalOptions) -> bool {
    options.opt_ilike.is_none()
        && options.opt_exclude.is_none()
        && options.opt_except.is_none()
        && options.opt_replace.is_none()
        && options.opt_rename.is_none()
}
fn editable_candidate(sql: &str, pg: bool) -> Option<EditableCandidate> {
    use sqlparser::ast::{
        GroupByExpr, SelectFlavor, SelectItem, SelectItemQualifiedWildcardKind, SetExpr,
        TableFactor,
    };
    validate_query(sql, pg).ok()?;
    let statements = if pg {
        Parser::parse_sql(&PostgreSqlDialect {}, sql)
    } else {
        Parser::parse_sql(&MySqlDialect {}, sql)
    }
    .ok()?;
    let Statement::Query(query) = statements.first()? else {
        return None;
    };
    if query.with.is_some()
        || !query.locks.is_empty()
        || query.for_clause.is_some()
        || query.settings.is_some()
        || query.format_clause.is_some()
        || !query.pipe_operators.is_empty()
    {
        return None;
    }
    let SetExpr::Select(select) = query.body.as_ref() else {
        return None;
    };
    if select.distinct.is_some()
        || select.top.is_some()
        || select.exclude.is_some()
        || select.into.is_some()
        || select.from.len() != 1
        || !select.lateral_views.is_empty()
        || select.prewhere.is_some()
        || !matches!(&select.group_by,GroupByExpr::Expressions(groups,modifiers) if groups.is_empty() && modifiers.is_empty())
        || !select.cluster_by.is_empty()
        || !select.distribute_by.is_empty()
        || !select.sort_by.is_empty()
        || select.having.is_some()
        || !select.named_window.is_empty()
        || select.qualify.is_some()
        || select.value_table_mode.is_some()
        || select.connect_by.is_some()
        || select.flavor != SelectFlavor::Standard
    {
        return None;
    }
    let from = &select.from[0];
    if !from.joins.is_empty() {
        return None;
    }
    let TableFactor::Table {
        name,
        alias,
        args,
        with_hints,
        version,
        with_ordinality,
        partitions,
        json_path,
        sample,
        index_hints,
    } = &from.relation
    else {
        return None;
    };
    if args.is_some()
        || !with_hints.is_empty()
        || version.is_some()
        || *with_ordinality
        || !partitions.is_empty()
        || json_path.is_some()
        || sample.is_some()
        || !index_hints.is_empty()
    {
        return None;
    }
    let source = object_parts(name, pg)?;
    if !(1..=2).contains(&source.len()) {
        return None;
    }
    let qualifiers = if let Some(alias) = alias {
        if !alias.columns.is_empty() {
            return None;
        }
        vec![vec![ident_name(&alias.name, pg)]]
    } else {
        vec![vec![source.last()?.clone()], source.clone()]
    };
    // A second SELECT would introduce a derived expression/subquery outside this contract.
    let tokens = if pg {
        sqlparser::tokenizer::Tokenizer::new(&PostgreSqlDialect {}, sql).tokenize()
    } else {
        sqlparser::tokenizer::Tokenizer::new(&MySqlDialect {}, sql).tokenize()
    }
    .ok()?;
    if tokens.iter().filter(|token|matches!(token,sqlparser::tokenizer::Token::Word(word) if word.quote_style.is_none() && word.value.eq_ignore_ascii_case("select"))).count()!=1{return None;}
    let projection = match select.projection.as_slice() {
        [SelectItem::Wildcard(options)] if plain_wildcard(options) => ProjectionShape::All,
        [
            SelectItem::QualifiedWildcard(
                SelectItemQualifiedWildcardKind::ObjectName(name),
                options,
            ),
        ] if plain_wildcard(options) => {
            if !qualifiers.contains(&object_parts(name, pg)?) {
                return None;
            }
            ProjectionShape::All
        }
        items => {
            let names = items
                .iter()
                .map(|item| match item {
                    SelectItem::UnnamedExpr(expr) => plain_column(expr, &qualifiers, pg),
                    SelectItem::ExprWithAlias { expr, alias } => {
                        let column = plain_column(expr, &qualifiers, pg)?;
                        (column == ident_name(alias, pg)).then_some(column)
                    }
                    _ => None,
                })
                .collect::<Option<Vec<_>>>()?;
            ProjectionShape::Columns(names)
        }
    };
    Some(EditableCandidate { source, projection })
}

async fn wait_cancel(cancel: Arc<AtomicBool>) {
    loop {
        if cancel.load(Ordering::SeqCst) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
fn validate_metadata_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > 1024 || name.contains('\0') {
        Err("Namespace/tabela vazio, inválido ou maior que 1024 bytes".into())
    } else {
        Ok(())
    }
}

fn recoverable_query_error(error: &sqlx::Error) -> bool {
    let sqlx::Error::Database(error) = error else {
        return false;
    };
    let Some(code) = error.code() else {
        return false;
    };
    // Only completed statement failures with well-defined transaction recovery.
    // Network errors, cancellation, deadlocks and lock/statement timeouts close.
    matches!(code.get(..2), Some("42" | "22" | "23" | "25" | "0A"))
        || matches!(code.as_ref(), "25006" | "25000" | "25P02")
}

fn db_error(error: sqlx::Error) -> String {
    // Server error messages can contain SQL values; the UI must not persist this message.
    match error {
        sqlx::Error::Tls(error) if error.to_string() == "server does not support TLS" => {
            "O servidor não suporta TLS. Para um banco local sem TLS, desmarque TLS no cadastro; em servidores remotos, verifique a configuração do servidor.".into()
        }
        sqlx::Error::Tls(_) => {
            "Falha na negociação TLS. Confira suporte TLS, CA, certificado e hostname.".into()
        }
        sqlx::Error::Io(error)
            if error.kind() == std::io::ErrorKind::InvalidData
                && error.to_string().starts_with("invalid peer certificate:") =>
        {
            "Certificado TLS não confiável ou inválido. Configure a CA PEM do servidor e verifique o hostname.".into()
        }
        sqlx::Error::Database(error) => {
            let number = error
                .try_downcast_ref::<sqlx_mysql::MySqlDatabaseError>()
                .map(|e| format!("/{}", e.number()))
                .unwrap_or_default();
            format!(
                "SQL [{}{number}]: {}",
                error.code().unwrap_or_default(),
                error.message()
            )
        }
        _ => "Falha de transporte ou protocolo SQL; verifique conexão, autenticação e TLS".into(),
    }
}
struct NativeStatement {
    manual_dml: bool,
    inspection: bool,
    diagnostics: bool,
    transaction_setting: bool,
}

fn native_statement(sql: &str, postgres: bool) -> Result<NativeStatement, String> {
    let tokens = if postgres {
        sqlparser::tokenizer::Tokenizer::new(&PostgreSqlDialect {}, sql).tokenize()
    } else {
        sqlparser::tokenizer::Tokenizer::new(&MySqlDialect {}, sql).tokenize()
    }
    .map_err(|_| "SQL não tokenizável")?;
    let first = tokens
        .iter()
        .find(|t| !matches!(t, sqlparser::tokenizer::Token::Whitespace(_)));
    let word = match first {
        Some(sqlparser::tokenizer::Token::Word(w)) if w.quote_style.is_none() => {
            w.value.to_ascii_uppercase()
        }
        _ => String::new(),
    };
    let words = tokens
        .iter()
        .filter_map(|t| match t {
            sqlparser::tokenizer::Token::Word(w) if w.quote_style.is_none() => {
                Some(w.value.to_ascii_uppercase())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let diagnostics = words.first().is_some_and(|w| w == "SHOW")
        && (words
            .get(1)
            .is_some_and(|w| matches!(w.as_str(), "WARNINGS" | "ERRORS"))
            || (words.get(1).is_some_and(|w| w == "COUNT")
                && words
                    .get(2)
                    .is_some_and(|w| matches!(w.as_str(), "WARNINGS" | "ERRORS"))));
    let transaction_setting = word == "SET"
        && (words.get(1).is_some_and(|w| w == "TRANSACTION")
            || (words
                .get(1)
                .is_some_and(|w| matches!(w.as_str(), "SESSION" | "LOCAL" | "GLOBAL"))
                && words.get(2).is_some_and(|w| w == "TRANSACTION")));
    Ok(NativeStatement {
        diagnostics,
        transaction_setting,
        manual_dml: matches!(
            word.as_str(),
            "SELECT" | "WITH" | "INSERT" | "UPDATE" | "DELETE" | "MERGE" | "REPLACE"
        ),
        inspection: matches!(word.as_str(), "SHOW" | "DESCRIBE" | "DESC" | "EXPLAIN"),
    })
}

fn readonly_inspection(sql: &str, postgres: bool) -> bool {
    if first_native_word(sql, postgres).is_some_and(|w| matches!(w.as_str(), "SELECT" | "WITH")) {
        return validate_read_words(sql, postgres).is_ok();
    }
    if !postgres {
        use sqlparser::tokenizer::Token;
        if let Ok(tokens) = sqlparser::tokenizer::Tokenizer::new(&MySqlDialect {}, sql).tokenize() {
            let tokens: Vec<_> = tokens
                .into_iter()
                .filter(|t| !matches!(t, Token::Whitespace(_)))
                .collect();
            let word = |index: usize, expected: &str| matches!(tokens.get(index), Some(Token::Word(w)) if w.quote_style.is_none() && w.value.eq_ignore_ascii_case(expected));
            if word(0, "EXPLAIN")
                && word(1, "FORMAT")
                && tokens.get(2) == Some(&Token::Eq)
                && (word(3, "JSON") || word(3, "TREE") || word(3, "TRADITIONAL"))
            {
                let query = tokens[4..]
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(" ");
                return validate_query(&query, false).is_ok();
            }
        }
    }
    let Ok(statements) = (if postgres {
        Parser::parse_sql(&PostgreSqlDialect {}, sql)
    } else {
        Parser::parse_sql(&MySqlDialect {}, sql)
    }) else {
        // MySQL SHOW INDEX/GRANTS/ENGINE and DESCRIBE with a column filter
        // exceed sqlparser's grammar. Their native syntax is server validated.
        return !postgres
            && native_statement(sql, false).is_ok_and(|s| s.inspection)
            && first_native_word(sql, false)
                .is_some_and(|w| matches!(w.as_str(), "SHOW" | "DESC" | "DESCRIBE"))
            && validate_read_words(sql, false).is_ok();
    };
    if statements.len() != 1 || validate_read_words(sql, postgres).is_err() {
        return false;
    }
    match &statements[0] {
        Statement::Explain {
            analyze,
            options,
            statement,
            ..
        } => {
            !analyze
                && options.as_ref().is_none_or(|opts| {
                    !opts
                        .iter()
                        .any(|o| o.name.value.eq_ignore_ascii_case("analyze"))
                })
                && validate_query(&statement.to_string(), postgres).is_ok()
        }
        Statement::ExplainTable { .. } => !postgres,
        _ => first_native_word(sql, postgres).is_some_and(|w| w == "SHOW"),
    }
}

/// Only statements accepted by the SELECT wrapper expose SQL paging controls.
pub fn supports_query_pages(sql: &str, engine: Engine) -> bool {
    engine.is_sql()
        && validate_query(
            sql,
            matches!(engine, Engine::Postgres | Engine::AuroraPostgres),
        )
        .is_ok()
}

fn first_native_word(sql: &str, postgres: bool) -> Option<String> {
    let tokens = if postgres {
        sqlparser::tokenizer::Tokenizer::new(&PostgreSqlDialect {}, sql).tokenize()
    } else {
        sqlparser::tokenizer::Tokenizer::new(&MySqlDialect {}, sql).tokenize()
    }
    .ok()?;
    match tokens
        .into_iter()
        .find(|t| !matches!(t, sqlparser::tokenizer::Token::Whitespace(_)))?
    {
        sqlparser::tokenizer::Token::Word(w) if w.quote_style.is_none() => {
            Some(w.value.to_ascii_uppercase())
        }
        _ => None,
    }
}

fn validate_query(sql: &str, postgres: bool) -> Result<String, String> {
    if sql.is_empty() || sql.len() > MAX_SQL {
        return Err("Informe uma única consulta SQL de até 256 KiB".into());
    }
    let statements = if postgres {
        Parser::parse_sql(&PostgreSqlDialect {}, sql)
    } else {
        Parser::parse_sql(&MySqlDialect {}, sql)
    }
    .map_err(|_| "SQL inválido ou não suportado pelo parser".to_owned())?;
    if statements.len() != 1 || !matches!(statements.first(), Some(Statement::Query(_))) {
        return Err("Paginação e exportação completa exigem uma única consulta SELECT/WITH".into());
    }
    validate_read_words(sql, postgres)?;
    Ok(statements[0].to_string())
}

fn validate_read_words(sql: &str, postgres: bool) -> Result<(), String> {
    // Inspect lexical words, never string literals/comments. The server read-only
    // transaction remains the enforcement boundary; this also blocks control helpers.
    let tokens = if postgres {
        sqlparser::tokenizer::Tokenizer::new(&PostgreSqlDialect {}, sql).tokenize()
    } else {
        sqlparser::tokenizer::Tokenizer::new(&MySqlDialect {}, sql).tokenize()
    }
    .map_err(|_| "SQL não tokenizável")?;
    for token in tokens {
        if let sqlparser::tokenizer::Token::Word(word) = token {
            let name = word.value.to_ascii_lowercase();
            if (word.quote_style.is_none()
                && matches!(
                    name.as_str(),
                    "into" | "update" | "insert" | "delete" | "merge" | "share"
                ))
                || matches!(
                    name.as_str(),
                    "set_config"
                        | "pg_terminate_backend"
                        | "pg_cancel_backend"
                        | "get_lock"
                        | "release_lock"
                        | "load_file"
                        | "sleep"
                        | "benchmark"
                        | "pg_sleep"
                )
                || name.starts_with("pg_advisory_")
                || name.starts_with("pg_try_advisory_")
            {
                return Err(
                    "Consulta contém controle de sessão, espera, escrita ou lock não permitido"
                        .into(),
                );
            }
        }
    }
    Ok(())
}
fn quote(name: &str, pg: bool) -> String {
    if pg {
        format!("\"{}\"", name.replace('"', "\"\""))
    } else {
        format!("`{}`", name.replace('`', "``"))
    }
}
fn literal(value: &str, pg: bool) -> String {
    if pg {
        format!("E'{}'", value.replace('\\', "\\\\").replace('\'', "''"))
    } else {
        // Hex text is independent of NO_BACKSLASH_ESCAPES and ANSI_QUOTES.
        let hex: String = value
            .as_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        format!("CONVERT(X'{hex}' USING utf8mb4)")
    }
}
fn table_name(table: &TableRef, pg: bool) -> String {
    format!("{}.{}", quote(&table.namespace, pg), quote(&table.name, pg))
}
fn text(cell: &Cell) -> String {
    match cell {
        Cell::Text(s) | Cell::Number(s) | Cell::Json(s) => s.clone(),
        Cell::Bool(b) => b.to_string(),
        // System metadata names are UTF-8, including MySQL binary collations.
        Cell::Binary(bytes) => String::from_utf8(bytes.clone()).unwrap_or_default(),
        _ => String::new(),
    }
}
fn cell_bytes(cell: &Cell) -> usize {
    match cell {
        Cell::Null => 0,
        Cell::Bool(_) => 1,
        Cell::Text(s) | Cell::Number(s) | Cell::Json(s) => s.len(),
        Cell::Binary(b) => b.len(),
    }
}
fn accept_row(cells: &[Cell], bytes: &mut usize) -> bool {
    if cells.iter().any(|c| cell_bytes(c) > MAX_CELL) {
        return false;
    }
    let added = cells
        .iter()
        .map(|c| cell_bytes(c) + std::mem::size_of::<Cell>())
        .sum::<usize>();
    if bytes.saturating_add(added) > MAX_BYTES {
        return false;
    }
    *bytes += added;
    true
}

fn pg_cells(row: &PgRow) -> Result<Vec<Cell>, String> {
    row.columns()
        .iter()
        .enumerate()
        .map(|(i, col)| {
            let raw = row.try_get_raw(i).map_err(db_error)?;
            if raw.is_null() {
                return Ok(Cell::Null);
            }
            let kind = col.type_info().name();
            let cell = match kind {
                "BOOL" => Cell::Bool(row.try_get(i).map_err(db_error)?),
                "INT2" => Cell::Number(row.try_get::<i16, _>(i).map_err(db_error)?.to_string()),
                "INT4" => Cell::Number(row.try_get::<i32, _>(i).map_err(db_error)?.to_string()),
                "INT8" => Cell::Number(row.try_get::<i64, _>(i).map_err(db_error)?.to_string()),
                "FLOAT4" => Cell::Number(row.try_get::<f32, _>(i).map_err(db_error)?.to_string()),
                "FLOAT8" => Cell::Number(row.try_get::<f64, _>(i).map_err(db_error)?.to_string()),
                "NUMERIC" => {
                    let number = row
                        .try_get::<sqlx::types::BigDecimal, _>(i)
                        .map_err(db_error)?;
                    let wire = raw.as_bytes().map_err(|_| "Decimal PostgreSQL inválido")?;
                    let scale = wire.get(6..8).map(|s| u16::from_be_bytes([s[0], s[1]]));
                    Cell::Number(if let Some(scale) = scale {
                        number.with_scale(i64::from(scale)).to_string()
                    } else {
                        number.to_string()
                    })
                }
                "DATE" => Cell::Text(
                    row.try_get::<sqlx::types::chrono::NaiveDate, _>(i)
                        .map_err(db_error)?
                        .to_string(),
                ),
                "TIME" => Cell::Text(
                    row.try_get::<sqlx::types::chrono::NaiveTime, _>(i)
                        .map_err(db_error)?
                        .to_string(),
                ),
                "TIMESTAMP" => Cell::Text(
                    row.try_get::<sqlx::types::chrono::NaiveDateTime, _>(i)
                        .map_err(db_error)?
                        .to_string(),
                ),
                "TIMESTAMPTZ" => Cell::Text(
                    row.try_get::<sqlx::types::chrono::DateTime<sqlx::types::chrono::Utc>, _>(i)
                        .map_err(db_error)?
                        .to_rfc3339(),
                ),
                "JSON" | "JSONB" => Cell::Json(
                    row.try_get::<sqlx::types::Json<Box<serde_json::value::RawValue>>, _>(i)
                        .map_err(db_error)?
                        .0
                        .get()
                        .to_owned(),
                ),
                "UUID" => Cell::Text(
                    row.try_get::<sqlx::types::Uuid, _>(i)
                        .map_err(db_error)?
                        .to_string(),
                ),
                "TEXT" | "VARCHAR" | "BPCHAR" | "NAME" => {
                    Cell::Text(row.try_get(i).map_err(db_error)?)
                }
                "BYTEA" => Cell::Binary(row.try_get(i).map_err(db_error)?),
                // Preserve unsupported typed wire data instead of guessing a lossy conversion.
                _ => Cell::Binary(
                    raw.as_bytes()
                        .map_err(|_| "Valor PostgreSQL não decodificável")?
                        .to_vec(),
                ),
            };
            Ok(cell)
        })
        .collect()
}
fn my_cells(row: &MySqlRow) -> Result<Vec<Cell>, String> {
    row.columns()
        .iter()
        .enumerate()
        .map(|(i, col)| {
            let raw = row.try_get_raw(i).map_err(db_error)?;
            if raw.is_null() {
                return Ok(Cell::Null);
            }
            let kind = col.type_info().name();
            let cell = match kind {
                "BOOLEAN" => Cell::Bool(row.try_get(i).map_err(db_error)?),
                "TINYINT" | "SMALLINT" | "INT" | "MEDIUMINT" | "BIGINT" => {
                    Cell::Number(row.try_get::<i64, _>(i).map_err(db_error)?.to_string())
                }
                "TINYINT UNSIGNED" | "SMALLINT UNSIGNED" | "INT UNSIGNED"
                | "MEDIUMINT UNSIGNED" | "BIGINT UNSIGNED" => {
                    Cell::Number(row.try_get::<u64, _>(i).map_err(db_error)?.to_string())
                }
                "FLOAT" => Cell::Number(row.try_get::<f32, _>(i).map_err(db_error)?.to_string()),
                "DOUBLE" => Cell::Number(row.try_get::<f64, _>(i).map_err(db_error)?.to_string()),
                "DECIMAL" => Cell::Number(
                    row.try_get::<sqlx::types::BigDecimal, _>(i)
                        .map_err(db_error)?
                        .to_string(),
                ),
                "DATE" => Cell::Text(
                    row.try_get::<sqlx::types::chrono::NaiveDate, _>(i)
                        .map_err(db_error)?
                        .to_string(),
                ),
                "TIME" => Cell::Text(
                    row.try_get::<sqlx_mysql::types::MySqlTime, _>(i)
                        .map_err(db_error)?
                        .to_string(),
                ),
                "DATETIME" | "TIMESTAMP" => Cell::Text(
                    row.try_get::<sqlx::types::chrono::NaiveDateTime, _>(i)
                        .map_err(db_error)?
                        .to_string(),
                ),
                "JSON" => Cell::Json(
                    row.try_get::<sqlx::types::Json<Box<serde_json::value::RawValue>>, _>(i)
                        .map_err(db_error)?
                        .0
                        .get()
                        .to_owned(),
                ),
                _ => match row.try_get::<String, _>(i) {
                    Ok(s) => Cell::Text(s),
                    Err(_) => Cell::Binary(
                        <&[u8] as sqlx_core::decode::Decode<'_, sqlx_mysql::MySql>>::decode(raw)
                            .map_err(|_| "Valor MySQL não decodificável")?
                            .to_vec(),
                    ),
                },
            };
            Ok(cell)
        })
        .collect()
}

type BoundUpdate = (String, Vec<Option<String>>);
type GroupedRow = (Vec<Cell>, Vec<(usize, Cell)>);
fn build_updates(batch: &EditBatch, pg: bool) -> Result<Vec<BoundUpdate>, String> {
    if batch.edits.len() > 500 {
        return Err("Lote excede 500 edições".into());
    }
    let mut bytes = 0usize;
    for edit in &batch.edits {
        for value in edit.row.iter().chain(std::iter::once(&edit.value)) {
            let size = cell_bytes(value);
            if size > MAX_CELL {
                return Err("Célula excede 64 KiB".into());
            }
            bytes = bytes
                .checked_add(size + std::mem::size_of::<Cell>())
                .ok_or("Lote excede 4 MiB")?;
            if bytes > MAX_BYTES {
                return Err("Lote excede 4 MiB; envie menos células por vez".into());
            }
        }
    }
    let columns = &batch.columns;
    if columns.is_empty() || columns.len() > MAX_COLUMNS || !columns.iter().any(|c| c.primary_key) {
        return Err("Edição exige schema limitado e chave primária".into());
    }
    let mut rows: Vec<GroupedRow> = Vec::new();
    for edit in &batch.edits {
        if edit.row.len() != columns.len() {
            return Err("Linha original não corresponde ao schema".into());
        }
        let index = columns
            .iter()
            .position(|c| c.name == edit.column)
            .ok_or("Coluna de edição não existe")?;
        if columns[index].primary_key {
            return Err("Alteração de chave primária não suportada nesta etapa".into());
        }
        if columns
            .iter()
            .zip(&edit.row)
            .any(|(c, v)| c.primary_key && *v == Cell::Null)
        {
            return Err("Chave primária nula não pode identificar uma linha".into());
        }
        let value = validate_cell_edit(&columns[index], &edit.value)?;
        if value == edit.row[index] {
            continue;
        }
        if let Some((_, changes)) = rows.iter_mut().find(|(row, _)| *row == edit.row) {
            if changes.iter().any(|(i, _)| *i == index) {
                return Err("Coluna repetida no lote".into());
            }
            changes.push((index, value.clone()));
        } else {
            if rows.iter().any(|(row, _)| {
                columns
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| c.primary_key)
                    .all(|(i, _)| row[i] == edit.row[i])
            }) {
                return Err("Mesma chave primária com originais divergentes no lote".into());
            }
            rows.push((edit.row.clone(), vec![(index, value)]));
        }
    }
    let mut updates = Vec::new();
    for (row, changes) in rows {
        let mut values = Vec::new();
        let mut sets = Vec::new();
        for (index, value) in changes {
            let value = bound_cell(&value, pg)?;
            values.push(value);
            let bind = parameter(values.len(), &columns[index], pg)?;
            sets.push(format!("{}={bind}", quote(&columns[index].name, pg)));
        }
        // PK first lets both engines use a unique lookup before original-value comparisons.
        let mut indices: Vec<usize> = (0..columns.len()).collect();
        indices.sort_by_key(|&i| !columns[i].primary_key);
        let mut predicates = Vec::new();
        for i in indices {
            values.push(bound_cell(&row[i], pg)?);
            let bind = parameter(values.len(), &columns[i], pg)?;
            let name = quote(&columns[i].name, pg);
            predicates.push(if pg {
                if columns[i].primary_key {
                    format!("{name}={bind}")
                } else if base_type(&columns[i]) == "json" {
                    format!(
                        "to_jsonb({name}) IS NOT DISTINCT FROM CAST(${0} AS jsonb)",
                        values.len()
                    )
                } else {
                    format!("{name} IS NOT DISTINCT FROM {bind}")
                }
            } else if columns[i].primary_key {
                format!("{name} <=> {bind}")
            } else if base_type(&columns[i]) == "json" {
                format!("{name} <=> CAST({bind} AS JSON)")
            } else if matches!(
                crate::cell_edit::editor_kind(&RemoteColumn {
                    data_type: columns[i]
                        .data_type
                        .strip_prefix("generated ")
                        .unwrap_or(&columns[i].data_type)
                        .to_owned(),
                    ..columns[i].clone()
                }),
                crate::cell_edit::CellEditorKind::Text
            ) {
                format!("CAST({name} AS BINARY) <=> CAST({bind} AS BINARY)")
            } else {
                // Bound textual numbers/dates must be compared with the native
                // type: binary comparison would reject equivalent representations
                // such as DECIMAL 1.00 vs 1 or DATETIME(6)'s trailing zeroes.
                format!("{name} <=> {bind}")
            });
        }
        updates.push((
            format!(
                "UPDATE {} SET {} WHERE {}",
                table_name(&batch.table, pg),
                sets.join(","),
                predicates.join(" AND ")
            ),
            values,
        ));
    }
    Ok(updates)
}
fn bound_cell(cell: &Cell, pg: bool) -> Result<Option<String>, String> {
    if cell_bytes(cell) > MAX_CELL {
        return Err("Célula excede 64 KiB".into());
    }
    Ok(match cell {
        Cell::Null => None,
        Cell::Text(s) | Cell::Number(s) | Cell::Json(s) => Some(s.clone()),
        Cell::Bool(b) => Some(if pg {
            b.to_string()
        } else {
            u8::from(*b).to_string()
        }),
        Cell::Binary(_) => {
            return Err(
                "Edição com dados binários/tipos não suportados permanece somente leitura".into(),
            );
        }
    })
}
fn parameter(index: usize, column: &RemoteColumn, pg: bool) -> Result<String, String> {
    if !pg {
        let base = base_type(column);
        let unsigned = base.ends_with(" unsigned");
        let kind = base.strip_suffix(" unsigned").unwrap_or(&base);
        let cast = match kind {
            "tinyint" | "smallint" | "mediumint" | "int" | "integer" | "bigint" | "boolean" => {
                Some(if unsigned {
                    "UNSIGNED".to_owned()
                } else {
                    "SIGNED".to_owned()
                })
            }
            "decimal" | "numeric" => {
                let dimensions = column
                    .data_type
                    .split_once('(')
                    .and_then(|(_, rest)| rest.split_once(')'))
                    .map(|(inside, _)| {
                        inside
                            .split(',')
                            .map(|v| v.trim().parse::<u16>())
                            .collect::<Result<Vec<_>, _>>()
                    })
                    .transpose()
                    .map_err(|_| "Precisão decimal de metadados inválida")?;
                let precision = dimensions
                    .as_ref()
                    .and_then(|v| v.first())
                    .copied()
                    .unwrap_or(65);
                let scale = dimensions
                    .as_ref()
                    .and_then(|v| v.get(1))
                    .copied()
                    .unwrap_or(30);
                if precision == 0 || precision > 65 || scale > precision {
                    return Err("Precisão decimal de metadados inválida".into());
                }
                Some(format!("DECIMAL({precision},{scale})"))
            }
            "float" => Some("FLOAT".to_owned()),
            "double" | "real" | "double precision" => Some("DOUBLE".to_owned()),
            _ => None,
        };
        return Ok(cast.map_or_else(
            || "?".into(),
            |cast| format!("CAST(CAST(? AS CHAR) AS {cast})"),
        ));
    }
    let kind = match base_type(column).as_str() {
        "smallint" => "smallint",
        "integer" => "integer",
        "bigint" => "bigint",
        "numeric" => "numeric",
        "real" => "real",
        "double precision" => "double precision",
        "character varying" => "varchar",
        "character" => "bpchar",
        "text" => "text",
        "boolean" => "boolean",
        "date" => "date",
        "timestamp without time zone" => "timestamp",
        "timestamp with time zone" => "timestamptz",
        "time without time zone" | "time" => "time",
        "uuid" => "uuid",
        "json" => "json",
        "jsonb" => "jsonb",
        _ => {
            return Err(format!(
                "Tipo {} sem edição segura nesta etapa",
                column.data_type
            ));
        }
    };
    // Parameters are always bound text; casts preserve exact decimal/int64/date values.
    Ok(format!("CAST(${index} AS {kind})"))
}

pub fn preview(batch: &EditBatch, engine: Engine) -> Result<String, String> {
    let pg = matches!(engine, Engine::Postgres | Engine::AuroraPostgres);
    let statements = build_updates(batch, pg)?;
    let mut output = String::new();
    for (sql, values) in statements {
        output.push_str(
            "-- Prévia para copiar; envio pelo Codisseu usa parâmetros.
",
        );
        output.push_str(&render_sql(&sql, &values, pg)?);
        output.push_str(
            ";
-- Forma parametrizada:
-- ",
        );
        output.push_str(&sql);
        output.push_str(";\n-- Parâmetros (não são SQL executável):\n");
        for (i, value) in values.iter().enumerate() {
            output.push_str(&format!(
                "-- {} = {}\n",
                i + 1,
                match value {
                    None => "NULL".into(),
                    Some(value) => serde_json::to_string(value).unwrap_or_default(),
                }
            ));
        }
    }
    Ok(output)
}

fn render_sql(sql: &str, values: &[Option<String>], pg: bool) -> Result<String, String> {
    let tokens = if pg {
        sqlparser::tokenizer::Tokenizer::new(&PostgreSqlDialect {}, sql).tokenize()
    } else {
        sqlparser::tokenizer::Tokenizer::new(&MySqlDialect {}, sql).tokenize()
    }
    .map_err(|_| "Falha ao produzir prévia SQL")?;
    let mut question = 0;
    let mut rendered = String::new();
    for token in tokens {
        if let sqlparser::tokenizer::Token::Placeholder(name) = token {
            let i = if name == "?" {
                let i = question;
                question += 1;
                i
            } else {
                name.trim_start_matches('$')
                    .parse::<usize>()
                    .map_err(|_| "Parâmetro SQL inválido")?
                    .checked_sub(1)
                    .ok_or("Parâmetro SQL inválido")?
            };
            let value = values.get(i).ok_or("Parâmetro SQL ausente")?;
            rendered.push_str(
                &value
                    .as_ref()
                    .map_or_else(|| "NULL".into(), |value| literal(value, pg)),
            );
        } else if let sqlparser::tokenizer::Token::Word(word) = &token {
            if word.quote_style.is_some() {
                rendered.push_str(&quote(&word.value, pg));
            } else {
                rendered.push_str(&token.to_string());
            }
        } else {
            rendered.push_str(&token.to_string());
        }
    }
    Ok(rendered)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn formatted_unsigned_metadata_retains_import_numeric_validation() {
        for kind in [
            "bigint unsigned",
            "int unsigned",
            "tinyint(1) unsigned",
            "decimal(30,10) unsigned",
            "numeric(20,4)",
            "double unsigned",
            "bigint unsigned zerofill",
        ] {
            let column = RemoteColumn {
                name: "amount".into(),
                data_type: kind.into(),
                primary_key: false,
                nullable: true,
            };
            for invalid in ["not a number", "true", "[]", "{\"amount\":1}", "\"42\""] {
                assert!(
                    import_value(&Cell::Text(invalid.into()), &column, false).is_err(),
                    "{kind}: {invalid}"
                );
            }
            assert!(
                import_value(&Cell::Number("42".into()), &column, false).is_ok(),
                "{kind}"
            );
            assert!(import_value(&Cell::Null, &column, false).is_ok(), "{kind}");
        }
    }
    #[test]
    fn rejects_scripts_control_locks_and_data_modifying_ctes() {
        for sql in [
            "SELECT 1; SELECT 2",
            "COMMIT",
            "SET transaction_read_only=off",
            "SELECT * INTO moved FROM users",
            "SELECT * FROM users FOR UPDATE",
            "WITH x AS (DELETE FROM users RETURNING *) SELECT * FROM x",
            "SELECT pg_catalog.set_config('transaction_read_only','off',true)",
            "SELECT pg_catalog.\"set_config\"('default_transaction_read_only','off',false)",
        ] {
            assert!(validate_query(sql, true).is_err(), "{sql}");
        }
        assert!(validate_query("SELECT '; UPDATE DELETE INTO' AS name /* ; */", true).is_ok());
        assert!(validate_query("WITH x AS (SELECT 1) SELECT * FROM x", false).is_ok());
    }
    #[test]
    fn console_inspection_does_not_require_a_pageable_select() {
        for sql in [
            "describe console_transport.transports;",
            "DESC `db`.`update`",
            "SHOW CREATE TABLE t",
            "SHOW FULL COLUMNS FROM t",
            "SHOW INDEX FROM t",
            "SHOW GRANTS FOR CURRENT_USER",
            "SHOW WARNINGS",
            "EXPLAIN SELECT * FROM t",
        ] {
            assert!(readonly_inspection(sql, false), "{sql}");
            assert!(validate_query(sql, false).is_err(), "{sql}");
        }
        for sql in [
            "SHOW search_path",
            "SHOW ALL",
            "EXPLAIN (FORMAT JSON) SELECT 1",
        ] {
            assert!(readonly_inspection(sql, true), "{sql}");
        }
        for pg in [false, true] {
            for sql in [
                "UPDATE t SET n=1",
                "CREATE TABLE t(id int)",
                "SET transaction_read_only=off",
                "EXPLAIN ANALYZE DELETE FROM t",
                "EXPLAIN (ANALYZE true) SELECT 1",
                "SELECT set_config('transaction_read_only','off',true)",
                "COMMIT",
                "CALL f()",
            ] {
                assert!(!readonly_inspection(sql, pg), "{sql}");
            }
        }
    }
    #[test]
    fn import_binary_validation_rejects_unicode_without_panicking() {
        let column = RemoteColumn {
            name: "bytes".into(),
            data_type: "bytea".into(),
            primary_key: false,
            nullable: true,
        };
        for text in ["0xaéa", "0x0g", "0x0"] {
            assert!(import_value(&Cell::Text(text.into()), &column, true).is_err());
        }
        assert!(
            matches!(import_value(&Cell::Text("0x00ff".into()),&column,true).unwrap(),ImportValue::Binary(Some(v)) if v==vec![0,255])
        );
    }
    #[test]
    fn metadata_literals_are_independent_of_server_backslash_modes() {
        let hostile = r"a\'b";
        assert_eq!(literal(hostile, true), r"E'a\\''b'");
        assert_eq!(
            literal(hostile, false),
            "CONVERT(X'615c2762' USING utf8mb4)"
        );
    }
    #[test]
    fn editable_shape_rejects_ambiguous_row_projections() {
        for sql in [
            "SELECT * FROM users",
            "SELECT u.* FROM users u WHERE u.id=1 ORDER BY u.id LIMIT 10",
            "SELECT u.id AS id,u.name FROM public.users u",
        ] {
            assert!(editable_candidate(sql, true).is_some(), "{sql}");
        }
        for sql in [
            "SELECT a.* FROM users a JOIN users b ON a.id=b.id",
            "SELECT COUNT(*) FROM users",
            "SELECT id+0 AS id,name FROM users",
            "SELECT name AS id,id AS name FROM users",
            "SELECT DISTINCT * FROM users",
            "WITH x AS(SELECT * FROM users) SELECT * FROM x",
            "SELECT * FROM users UNION ALL SELECT * FROM users",
            "SELECT * FROM users WHERE id IN(SELECT id FROM users)",
            "SELECT u.* FROM users u(id,name)",
        ] {
            assert!(editable_candidate(sql, true).is_none(), "{sql}");
        }
        let column = sqlparser::ast::Ident::new("ÁBC");
        assert_eq!(ident_name(&column, true), "Ábc");
    }
    #[test]
    fn bounded_payload_omits_entire_row() {
        let mut bytes = 0;
        assert!(!accept_row(
            &[Cell::Text("a".repeat(MAX_CELL + 1))],
            &mut bytes
        ));
        assert_eq!(bytes, 0);
        let mut bytes = MAX_BYTES - 1;
        assert!(!accept_row(&[Cell::Null], &mut bytes));
    }
    #[test]
    fn parameterized_preview_quotes_identifiers_without_interpolating_values() {
        use crate::remote::CellEdit;
        let batch = EditBatch {
            table: TableRef {
                namespace: "public".into(),
                name: "a\"b".into(),
            },
            columns: vec![
                RemoteColumn {
                    name: "id".into(),
                    data_type: "bigint".into(),
                    primary_key: true,
                    nullable: false,
                },
                RemoteColumn {
                    name: "name".into(),
                    data_type: "text".into(),
                    primary_key: false,
                    nullable: true,
                },
            ],
            edits: vec![CellEdit {
                row: vec![Cell::Number("1".into()), Cell::Null],
                column: "name".into(),
                value: Cell::Text("'; DROP TABLE a; --".into()),
            }],
        };
        let statements = build_updates(&batch, true).unwrap();
        assert_eq!(statements.len(), 1);
        assert!(statements[0].0.contains("\"a\"\"b\""));
        assert!(!statements[0].0.contains("DROP"));
        assert!(statements[0].0.contains("IS NOT DISTINCT FROM"));
        assert_eq!(statements[0].1[2], None);
        let rendered = preview(&batch, Engine::Postgres).unwrap();
        let parsed = Parser::parse_sql(&PostgreSqlDialect {}, &rendered)
            .unwrap_or_else(|error| panic!("{error}: {rendered}"));
        assert_eq!(
            parsed.len(),
            1,
            "a quoted malicious value must remain a single UPDATE"
        );
        assert!(matches!(parsed[0], Statement::Update { .. }));
        let rendered = preview(&batch, Engine::Mysql).unwrap();
        let parsed = Parser::parse_sql(&MySqlDialect {}, &rendered)
            .unwrap_or_else(|error| panic!("{error}: {rendered}"));
        assert_eq!(parsed.len(), 1);
        assert!(matches!(parsed[0], Statement::Update { .. }));
    }
}
