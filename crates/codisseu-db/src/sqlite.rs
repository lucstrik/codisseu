//! Codisseu's read-only adapter over public rusqlite/SQLite APIs.
use crate::{
    CELL_BYTE_LIMIT, ColumnInfo, QUERY_LIMIT, QueryResult, RESULT_BYTE_LIMIT, SQL_BYTE_LIMIT,
    TableInfo, Value,
};
use rusqlite::{
    Connection, InterruptHandle, OpenFlags,
    hooks::{AuthAction, AuthContext, Authorization},
    limits::Limit,
    types::ValueRef,
};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

pub(crate) struct Session {
    pub path: PathBuf,
    database: Connection,
}

pub(crate) fn explain(error: rusqlite::Error) -> String {
    if error.sqlite_error_code() == Some(rusqlite::ErrorCode::OperationInterrupted) {
        "SQLite interrompido: cancelamento confirmado pelo backend ou orçamento de 10 segundos esgotado".into()
    } else {
        format!("SQLite: {error}")
    }
}

impl Session {
    pub fn open(path: &Path, epoch: Arc<AtomicU64>, expected: u64) -> Result<Self, String> {
        let canonical =
            std::fs::canonicalize(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let metadata = std::fs::metadata(&canonical).map_err(|e| e.to_string())?;
        if !metadata.is_file() {
            return Err("Escolha um arquivo SQLite existente e regular".into());
        }
        let database = Connection::open_with_flags(&canonical, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(explain)?;
        database
            .busy_timeout(Duration::from_millis(500))
            .map_err(explain)?;
        for (setting, value) in [
            ("query_only", 1),
            ("trusted_schema", 0),
            ("cache_size", -2048),
        ] {
            database
                .pragma_update(None, setting, value)
                .map_err(explain)?;
        }
        for (limit, value) in [
            (Limit::SQLITE_LIMIT_ATTACHED, 0),
            (Limit::SQLITE_LIMIT_COLUMN, 256),
            (Limit::SQLITE_LIMIT_LENGTH, 1024 * 1024),
            (Limit::SQLITE_LIMIT_SQL_LENGTH, SQL_BYTE_LIMIT as i32),
            (Limit::SQLITE_LIMIT_COMPOUND_SELECT, 50),
            (Limit::SQLITE_LIMIT_EXPR_DEPTH, 100),
        ] {
            database.set_limit(limit, value).map_err(explain)?;
        }
        database.authorizer(Some(permit_read)).map_err(explain)?;
        let result = Self {
            path: canonical,
            database,
        };
        result.begin_request(epoch, expected)?;
        Ok(result)
    }

    pub fn interrupt_handle(&self) -> InterruptHandle {
        self.database.get_interrupt_handle()
    }

    pub fn begin_request(&self, epoch: Arc<AtomicU64>, expected: u64) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(10);
        self.database
            .progress_handler(
                1000,
                Some(move || epoch.load(Ordering::SeqCst) != expected || Instant::now() > deadline),
            )
            .map_err(explain)
    }

    pub fn inspect(&self) -> Result<Vec<TableInfo>, String> {
        let mut names = self.database.prepare(
            "SELECT name FROM main.sqlite_schema WHERE (type='table' OR type='view') AND substr(name,1,7)<>'sqlite_' ORDER BY name LIMIT 257"
        ).map_err(explain)?;
        let mut cursor = names.query([]).map_err(explain)?;
        let mut catalog = Vec::new();
        let mut metadata_bytes = 0;
        let mut total_columns = 0;
        while let Some(record) = cursor.next().map_err(explain)? {
            if catalog.len() == 256 {
                return Err("Schema excede 256 tabelas/views na M2".into());
            }
            let name: String = record.get(0).map_err(explain)?;
            let mut detail = self.database.prepare(
                "SELECT name,type,\"notnull\",pk FROM pragma_table_xinfo(?1) WHERE hidden<>1 ORDER BY cid LIMIT 257"
            ).map_err(explain)?;
            let mut fields = detail.query([&name]).map_err(explain)?;
            let has_primary_index: bool = self
                .database
                .query_row(
                    "SELECT count(*)>0 FROM pragma_index_list(?1) WHERE origin='pk'",
                    [&name],
                    |r| r.get(0),
                )
                .map_err(explain)?;
            let mut columns = Vec::new();
            metadata_bytes += name.len();
            while let Some(field) = fields.next().map_err(explain)? {
                let column_name: String = field.get(0).map_err(explain)?;
                let data_type: String = field.get(1).map_err(explain)?;
                let explicit_not_null: bool = field.get(2).map_err(explain)?;
                let pk_order: i64 = field.get(3).map_err(explain)?;
                let rowid = pk_order == 1
                    && !has_primary_index
                    && data_type.eq_ignore_ascii_case("INTEGER");
                metadata_bytes +=
                    column_name.len() + data_type.len() + std::mem::size_of::<ColumnInfo>();
                total_columns += 1;
                if columns.len() >= 256 || total_columns > 8192 || metadata_bytes > 1024 * 1024 {
                    return Err("Schema excede orçamento M2 de colunas/metadados".into());
                }
                columns.push(ColumnInfo {
                    name: column_name,
                    data_type,
                    primary_key: pk_order > 0,
                    nullable: !explicit_not_null && !rowid,
                });
            }
            catalog.push(TableInfo { name, columns });
        }
        Ok(catalog)
    }

    pub fn export_query(
        &self,
        request: &crate::query_export::QueryExportRequest,
        progress: crate::query_export::QueryExportProgress,
        epoch: Arc<AtomicU64>,
        expected: u64,
    ) -> Result<crate::query_export::QueryExportReport, String> {
        request.validate()?;
        let deadline = Instant::now() + Duration::from_secs(request.timeout_secs);
        let check = || {
            if epoch.load(Ordering::SeqCst) != expected {
                Err("Exportação SQLite cancelada; arquivo temporário removido".to_owned())
            } else if Instant::now() >= deadline {
                Err("Prazo da exportação SQLite excedido; arquivo temporário removido".to_owned())
            } else {
                Ok(())
            }
        };
        let export_error = |error| check().err().unwrap_or_else(|| explain(error));
        let handler_epoch = epoch.clone();
        self.database
            .progress_handler(
                1000,
                Some(move || {
                    handler_epoch.load(Ordering::SeqCst) != expected || Instant::now() >= deadline
                }),
            )
            .map_err(export_error)?;
        check()?;
        let mut statement = self.database.prepare(&request.sql).map_err(export_error)?;
        if !statement.readonly() || statement.column_count() == 0 {
            return Err("Exportação aceita uma única consulta somente leitura".into());
        }
        let columns = statement
            .column_names()
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let mut writer = crate::query_export::CsvStreamWriter::new(request, &columns, progress)?;
        let mut cursor = statement.query([]).map_err(export_error)?;
        while let Some(row) = cursor.next().map_err(export_error)? {
            check()?;
            let mut cells = Vec::with_capacity(columns.len());
            for i in 0..columns.len() {
                let raw = row.get_ref(i).map_err(export_error)?;
                if matches!(raw, ValueRef::Text(b)|ValueRef::Blob(b) if b.len()>CELL_BYTE_LIMIT) {
                    return Err(
                        "Célula da exportação acima de 64 KiB; arquivo temporário removido".into(),
                    );
                }
                cells.push(match row.get::<_, Value>(i).map_err(export_error)? {
                    Value::Null => crate::remote::Cell::Null,
                    Value::Integer(v) => crate::remote::Cell::Number(v.to_string()),
                    Value::Real(v) => crate::remote::Cell::Number(v.to_string()),
                    Value::Text(v) => crate::remote::Cell::Text(v),
                    Value::Blob(v) => crate::remote::Cell::Binary(v),
                });
            }
            writer.row(&cells)?;
        }
        drop(cursor);
        check()?;
        writer.finish()
    }

    pub fn select_page(
        &self,
        request: &crate::query_page::QueryPageRequest,
    ) -> Result<QueryResult, String> {
        request.validate()?;
        // Validate the exact original with SQLite's own parser and authorizer
        // before wrapping. LIMIT/OFFSET already in that query keep their scope.
        let prepared = self.database.prepare(&request.sql).map_err(explain)?;
        if !prepared.readonly() || prepared.column_count() == 0 {
            return Err("Paginação aceita uma única consulta somente leitura".into());
        }
        if request
            .sort
            .is_some_and(|sort| sort.column >= prepared.column_count())
        {
            return Err("Coluna de ordenação fora do resultado da consulta".into());
        }
        // SQLite disambiguates repeated output labels in a derived table.
        // Present the original labels while ordering strictly by ordinal.
        let columns = prepared
            .column_names()
            .into_iter()
            .map(str::to_owned)
            .collect();
        drop(prepared);
        let range =
            crate::sql_text::statement_range(&request.sql, 0, crate::sql_text::SqlDialect::Sqlite)?;
        let canonical = request
            .sql
            .chars()
            .skip(range.start)
            .take(range.len())
            .collect::<String>();
        let canonical = canonical.trim_end();
        let canonical = canonical.strip_suffix(';').unwrap_or(canonical);
        let mut result = self.select(&request.wrapped_sql(canonical, QUERY_LIMIT))?;
        result.columns = columns;
        Ok(result)
    }

    pub fn select(&self, sql: &str) -> Result<QueryResult, String> {
        let timer = Instant::now();
        // rusqlite 0.40 validates trailing statements with the SQLite parser.
        let mut prepared = self.database.prepare(sql).map_err(explain)?;
        let width = prepared.column_count();
        if width == 0 || !prepared.readonly() {
            return Err("Esta conexão M2 aceita uma consulta somente leitura com resultado".into());
        }
        let mut output = QueryResult {
            columns: Vec::with_capacity(width),
            rows: Vec::new(),
            truncated: false,
            affected_rows: 0,
            elapsed_ms: 0,
            limit_reason: None,
        };
        for index in 0..width {
            output
                .columns
                .push(prepared.column_name(index).map_err(explain)?.to_owned());
        }
        let mut budget = Budget {
            used: output
                .columns
                .iter()
                .map(|s| s.len() + std::mem::size_of::<String>())
                .sum(),
        };
        let mut records = prepared.query([]).map_err(explain)?;
        while let Some(record) = records.next().map_err(explain)? {
            if output.rows.len() == QUERY_LIMIT {
                output.limit_reason = Some("500 linhas".into());
                break;
            }
            if let Err(reason) = budget.reserve(record, width) {
                output.limit_reason = Some(reason);
                break;
            }
            let mut data = Vec::with_capacity(width);
            for index in 0..width {
                // Public FromSql for Value reports invalid UTF-8 instead of
                // replacing bytes; no local clone of a database value model.
                data.push(record.get::<_, Value>(index).map_err(explain)?);
            }
            output.rows.push(data);
        }
        output.truncated = output.limit_reason.is_some();
        output.elapsed_ms = timer.elapsed().as_millis();
        Ok(output)
    }
}

struct Budget {
    used: usize,
}
impl Budget {
    fn reserve(&mut self, row: &rusqlite::Row<'_>, width: usize) -> Result<(), String> {
        let mut extra = std::mem::size_of::<Vec<Value>>() + width * std::mem::size_of::<Value>();
        for position in 0..width {
            let size = match row.get_ref(position).map_err(explain)? {
                ValueRef::Text(data) | ValueRef::Blob(data) => data.len(),
                ValueRef::Integer(_) | ValueRef::Real(_) => 8,
                ValueRef::Null => 0,
            };
            if size > CELL_BYTE_LIMIT {
                return Err("célula acima de 64 KiB; linha omitida".into());
            }
            extra += size;
        }
        if self.used + extra > RESULT_BYTE_LIMIT {
            return Err("resultado acima de 4 MiB; linha omitida".into());
        }
        self.used += extra;
        Ok(())
    }
}

fn permit_read(context: AuthContext<'_>) -> Authorization {
    let allowed = match context.action {
        AuthAction::Select | AuthAction::Recursive | AuthAction::Read { .. } => true,
        AuthAction::Pragma {
            pragma_name,
            pragma_value,
        } => {
            ["table_xinfo", "table_info", "index_list"]
                .iter()
                .any(|name| pragma_name.eq_ignore_ascii_case(name))
                || (pragma_name.eq_ignore_ascii_case("data_version") && pragma_value.is_none())
        }
        AuthAction::Function { function_name } => {
            !["load_extension", "writefile", "readfile", "eval"]
                .iter()
                .any(|name| function_name.eq_ignore_ascii_case(name))
        }
        _ => false,
    };
    if allowed {
        Authorization::Allow
    } else {
        Authorization::Deny
    }
}

#[derive(Clone, Debug)]
pub(crate) struct PreparedFileImport {
    pub path: PathBuf,
    pub partial: bool,
    pub table: String,
    pub schema: Vec<ColumnInfo>,
    create_sql: String,
    pub data: crate::transfer::TransferData,
}
impl Session {
    pub fn prepare_import(
        &self,
        request: &crate::transfer::ImportRequest,
    ) -> Result<PreparedFileImport, String> {
        let crate::transfer::ImportTarget::SqlTable(table) = &request.target else {
            return Err("Escolha tabela SQLite".into());
        };
        if !table.namespace.is_empty() && table.namespace != "main" {
            return Err("Importação SQLite disponível somente no arquivo main".into());
        }
        let schema = self
            .inspect()?
            .into_iter()
            .find(|t| t.name == table.name)
            .ok_or("Tabela SQLite não encontrada")?
            .columns;
        let real: bool = self
            .database
            .query_row(
                "SELECT type='table' FROM main.sqlite_schema WHERE name=?1",
                [&table.name],
                |r| r.get(0),
            )
            .map_err(explain)?;
        if !real {
            return Err("Importação exige tabela SQLite real".into());
        }
        let data = crate::transfer::map_import(request)?;
        for name in &data.columns {
            if !schema.iter().any(|c| c.name == *name) {
                return Err(format!("Coluna inexistente: {name}"));
            }
        }
        for row in &data.rows {
            for (cell, name) in row.iter().zip(&data.columns) {
                let column = schema
                    .iter()
                    .find(|c| c.name == *name)
                    .ok_or("Coluna SQLite mudou")?;
                sqlite_import_value(cell, column)?;
            }
        }
        let create_sql: String = self
            .database
            .query_row(
                "SELECT sql FROM main.sqlite_schema WHERE name=?1",
                [&table.name],
                |r| r.get(0),
            )
            .map_err(explain)?;
        Ok(PreparedFileImport {
            create_sql,
            partial: request.data.source_truncated,
            path: self.path.clone(),
            table: table.name.clone(),
            schema,
            data,
        })
    }
}
pub(crate) fn import_file(
    plan: &PreparedFileImport,
    epoch: Arc<AtomicU64>,
    generation: u64,
    interrupt: &std::sync::Mutex<Option<InterruptHandle>>,
) -> Result<crate::transfer::ImportReport, String> {
    use crate::transfer::{SqlFlavor, quote_identifier};
    if std::fs::canonicalize(&plan.path).map_err(|e| e.to_string())? != plan.path {
        return Err("Caminho SQLite mudou desde a prévia".into());
    }
    let mut database = Connection::open_with_flags(&plan.path, OpenFlags::SQLITE_OPEN_READ_WRITE)
        .map_err(explain)?;
    database
        .busy_timeout(Duration::from_millis(500))
        .map_err(explain)?;
    database
        .pragma_update(None, "trusted_schema", 0)
        .map_err(explain)?;
    database
        .pragma_update(None, "foreign_keys", 1)
        .map_err(explain)?;
    database
        .set_limit(Limit::SQLITE_LIMIT_ATTACHED, 0)
        .map_err(explain)?;
    database
        .set_limit(Limit::SQLITE_LIMIT_LENGTH, 1024 * 1024)
        .map_err(explain)?;
    let deadline = Instant::now() + Duration::from_secs(10);
    let cancel = epoch.clone();
    database
        .progress_handler(
            500,
            Some(move || cancel.load(Ordering::SeqCst) != generation || Instant::now() > deadline),
        )
        .map_err(explain)?;
    *interrupt
        .lock()
        .map_err(|_| "Controle SQLite indisponível")? = Some(database.get_interrupt_handle());
    let table = plan.table.clone();
    database
        .authorizer(Some(move |ctx: AuthContext<'_>| match ctx.action {
            AuthAction::Insert { table_name } if table_name == table => Authorization::Allow,
            AuthAction::Select
            | AuthAction::Read { .. }
            | AuthAction::Transaction { .. }
            | AuthAction::Savepoint { .. } => Authorization::Allow,
            _ => Authorization::Deny,
        }))
        .map_err(explain)?;
    let columns = plan
        .data
        .columns
        .iter()
        .map(|c| quote_identifier(c, SqlFlavor::Sqlite))
        .collect::<Vec<_>>()
        .join(",");
    let placeholders = (1..=plan.data.columns.len())
        .map(|n| format!("?{n}"))
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "INSERT INTO {} ({columns}) VALUES ({placeholders})",
        quote_identifier(&plan.table, SqlFlavor::Sqlite)
    );
    let transaction = database
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(explain)?;
    let actual_create: String = transaction
        .query_row(
            "SELECT sql FROM main.sqlite_schema WHERE name=?1",
            [&plan.table],
            |r| r.get(0),
        )
        .map_err(explain)?;
    if actual_create != plan.create_sql {
        return Err("Schema SQLite mudou antes de abrir a transação de importação".into());
    }
    let outcome = (|| -> Result<(), String> {
        let mut statement = transaction.prepare(&sql).map_err(explain)?;
        for row in &plan.data.rows {
            if epoch.load(Ordering::SeqCst) != generation {
                return Err("Importação SQLite cancelada; lote revertido".into());
            }
            let values = row
                .iter()
                .zip(&plan.data.columns)
                .map(|(cell, name)| {
                    let column = plan
                        .schema
                        .iter()
                        .find(|c| c.name == *name)
                        .ok_or("Coluna SQLite mudou")?;
                    sqlite_import_value(cell, column)
                })
                .collect::<Result<Vec<_>, String>>()?;
            if statement
                .execute(rusqlite::params_from_iter(values))
                .map_err(explain)?
                != 1
            {
                return Err("INSERT SQLite não afetou exatamente1registro".into());
            }
        }
        Ok(())
    })();
    match outcome {
        Ok(()) => {
            if epoch.load(Ordering::SeqCst) != generation {
                return Err("Importação SQLite cancelada antes do Commit; lote revertido".into());
            }
            transaction.commit().map_err(explain)?;
            Ok(crate::transfer::ImportReport {
                attempted: plan.data.rows.len(),
                applied: plan.data.rows.len(),
                failed: 0,
                unknown: 0,
                transaction_pending: false,
                atomic: true,
                summary: format!(
                    "{} linhas importadas; Commit SQLite atômico concluído",
                    plan.data.rows.len()
                ),
            })
        }
        Err(error) => {
            drop(transaction);
            Ok(crate::transfer::ImportReport {
                attempted: plan.data.rows.len(),
                applied: 0,
                failed: plan.data.rows.len(),
                unknown: 0,
                transaction_pending: false,
                atomic: true,
                summary: format!("Lote SQLite revertido integralmente: {error}"),
            })
        }
    }
}

fn sqlite_import_value(
    cell: &crate::remote::Cell,
    column: &ColumnInfo,
) -> Result<rusqlite::types::Value, String> {
    use crate::remote::Cell;
    use rusqlite::types::Value;
    let kind = column.data_type.to_ascii_uppercase();
    let text_affinity = kind.contains("CHAR") || kind.contains("CLOB") || kind.contains("TEXT");
    let blob_affinity = kind.is_empty() || kind.contains("BLOB");
    let numeric = |value: &str| -> Result<Value, String> {
        if let Ok(integer) = value.parse::<i64>() {
            return Ok(Value::Integer(integer));
        }
        if kind.contains("INT") || !value.contains(['.', 'e', 'E']) {
            return Err(format!(
                "Número excede inteiro64bits em{}; use coluna TEXT para precisão arbitrária",
                column.name
            ));
        }
        let digits = value
            .split(['e', 'E'])
            .next()
            .unwrap_or("")
            .chars()
            .filter(char::is_ascii_digit)
            .collect::<String>();
        if digits.trim_start_matches('0').trim_end_matches('0').len() > 15 {
            return Err(format!(
                "Número de{} excede15algarismos seguros para REAL SQLite; use coluna TEXT",
                column.name
            ));
        }
        let real = value
            .parse::<f64>()
            .map_err(|_| format!("Número SQLite inválido em{}", column.name))?;
        if !real.is_finite() {
            return Err("Número SQLite não finito".into());
        }
        Ok(Value::Real(real))
    };
    Ok(match cell {
        Cell::Null => Value::Null,
        Cell::Bool(value) => Value::Integer(i64::from(*value)),
        Cell::Binary(value) => Value::Blob(value.clone()),
        Cell::Number(value) if text_affinity => Value::Text(value.clone()),
        Cell::Number(value) => numeric(value)?,
        Cell::Text(value)
            if !text_affinity
                && !blob_affinity
                && (kind.contains("INT")
                    || kind.contains("REAL")
                    || kind.contains("FLOA")
                    || kind.contains("DOUB")) =>
        {
            numeric(value)?
        }
        Cell::Text(value) if !text_affinity && !blob_affinity && value.parse::<f64>().is_ok() => {
            numeric(value)?
        }
        Cell::Text(value) | Cell::Json(value) => Value::Text(value.clone()),
    })
}
