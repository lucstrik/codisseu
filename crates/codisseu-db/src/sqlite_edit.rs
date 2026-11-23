//! Controlled SQLite grid writes. User SQL continues through the read-only authorizer.
use crate::remote::{Cell, EditBatch, RemoteColumn, TableRef};
use crate::sqlite::{Session, explain, permit_read, sqlite_import_value};
use crate::transfer::{SqlFlavor, quote_identifier};
use crate::{ColumnInfo, QueryResult, Value};
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use sqlparser::{ast::*, dialect::SQLiteDialect, parser::Parser};

pub(crate) type Origin = (TableRef, Vec<RemoteColumn>);
type RowChanges = (Vec<Cell>, Vec<(usize, Value)>);

pub(crate) fn cell(value: &Value) -> Cell {
    match value {
        Value::Null => Cell::Null,
        Value::Integer(v) => Cell::Number(v.to_string()),
        Value::Real(v) => Cell::Number(v.to_string()),
        Value::Text(v) => Cell::Text(v.clone()),
        Value::Blob(v) => Cell::Binary(v.clone()),
    }
}

pub(crate) fn validate_batch_budget(batch: &EditBatch) -> Result<(), String> {
    if batch.edits.is_empty()
        || batch.edits.len() > 500
        || batch.columns.is_empty()
        || batch.columns.len() > 256
    {
        return Err("Lote SQLite excede limites de células/colunas".into());
    }
    let size = |cell: &Cell| match cell {
        Cell::Text(v) | Cell::Json(v) | Cell::Number(v) => v.len(),
        Cell::Binary(v) => v.len(),
        _ => 8,
    };
    let mut bytes = 0usize;
    for edit in &batch.edits {
        if edit.row.len() != batch.columns.len() || size(&edit.value) > crate::CELL_BYTE_LIMIT {
            return Err("Célula ou linha SQLite excede limites".into());
        }
        bytes = bytes
            .saturating_add(size(&edit.value))
            .saturating_add(edit.row.iter().map(size).sum::<usize>());
        if bytes > crate::RESULT_BYTE_LIMIT {
            return Err("Lote SQLite excede4MiB".into());
        }
    }
    Ok(())
}

fn quote(name: &str) -> String {
    quote_identifier(name, SqlFlavor::Sqlite)
}

fn simple_table(sql: &str) -> Option<String> {
    let statements = Parser::parse_sql(&SQLiteDialect {}, sql).ok()?;
    let [Statement::Query(query)] = statements.as_slice() else {
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
        || !matches!(&select.group_by, GroupByExpr::Expressions(e,m) if e.is_empty() && m.is_empty())
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
    let parts = name
        .0
        .iter()
        .map(|p| p.as_ident().map(|v| v.value.clone()))
        .collect::<Option<Vec<_>>>()?;
    if parts.is_empty()
        || parts.len() > 2
        || (parts.len() == 2 && !parts[0].eq_ignore_ascii_case("main"))
    {
        return None;
    }
    let table = parts.last()?.clone();
    let qualifiers = if let Some(alias) = alias {
        if !alias.columns.is_empty() {
            return None;
        }
        vec![vec![alias.name.value.clone()]]
    } else {
        vec![vec![table.clone()], vec!["main".into(), table.clone()]]
    };
    let qualified = |names: &[Ident]| -> bool {
        qualifiers.iter().any(|q| {
            q.len() == names.len()
                && q.iter()
                    .zip(names)
                    .all(|(a, b)| a.eq_ignore_ascii_case(&b.value))
        })
    };
    for item in &select.projection {
        match item {
            SelectItem::UnnamedExpr(Expr::Identifier(_)) => {}
            SelectItem::UnnamedExpr(Expr::CompoundIdentifier(ids))
                if ids.len() > 1 && qualified(&ids[..ids.len() - 1]) => {}
            SelectItem::Wildcard(options) if options.to_string().is_empty() => {}
            SelectItem::QualifiedWildcard(
                SelectItemQualifiedWildcardKind::ObjectName(name),
                options,
            ) if options.to_string().is_empty()
                && qualified(
                    &name
                        .0
                        .iter()
                        .map(|p| p.as_ident().cloned())
                        .collect::<Option<Vec<_>>>()?,
                ) => {}
            _ => return None,
        }
    }
    // A nested SELECT could duplicate/synthesize rows despite the plain projection.
    let tokens = sqlparser::tokenizer::Tokenizer::new(&SQLiteDialect {}, sql)
        .tokenize()
        .ok()?;
    if tokens.iter().filter(|t| matches!(t,sqlparser::tokenizer::Token::Word(w) if w.quote_style.is_none() && w.value.eq_ignore_ascii_case("select"))).count()!=1 { return None; }
    Some(table)
}

impl Session {
    pub(crate) fn grid_schema(&self, table: &TableRef) -> Result<String, String> {
        self.database
            .query_row(
                "SELECT sql FROM main.sqlite_schema WHERE type='table' AND name=?1",
                [&table.name],
                |r| r.get(0),
            )
            .map_err(explain)
    }

    fn edit_columns(&self, table: &TableRef) -> Result<Vec<RemoteColumn>, String> {
        if table.namespace != "main" {
            return Err("Edição SQLite exige o arquivo main".into());
        }
        let (kind, create): (String, String) = self
            .database
            .query_row(
                "SELECT type,coalesce(sql,'') FROM main.sqlite_schema WHERE name=?1 COLLATE NOCASE",
                [&table.name],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(explain)?;
        if kind != "table" || create.to_ascii_uppercase().contains("CREATE VIRTUAL TABLE") {
            return Err("Edição exige tabela SQLite real".into());
        }
        let table_kind: String = self
            .database
            .query_row(
                "SELECT type FROM pragma_table_list WHERE schema='main' AND name=?1",
                [&table.name],
                |r| r.get(0),
            )
            .map_err(explain)?;
        if table_kind != "table" {
            return Err("Tabelas virtuais ou shadow são somente leitura".into());
        }
        let mut statement = self.database.prepare("SELECT name,type,\"notnull\",pk,hidden FROM pragma_table_xinfo(?1) WHERE hidden<>1 ORDER BY cid LIMIT 257").map_err(explain)?;
        let mut cursor = statement.query([&table.name]).map_err(explain)?;
        let has_primary_index: bool = self
            .database
            .query_row(
                "SELECT count(*)>0 FROM pragma_index_list(?1) WHERE origin='pk'",
                [&table.name],
                |r| r.get(0),
            )
            .map_err(explain)?;
        let mut columns = Vec::new();
        while let Some(row) = cursor.next().map_err(explain)? {
            if columns.len() == 256 {
                return Err("Tabela excede256colunas".into());
            }
            let name: String = row.get(0).map_err(explain)?;
            let mut data_type: String = row.get(1).map_err(explain)?;
            let not_null: bool = row.get(2).map_err(explain)?;
            let pk: i64 = row.get(3).map_err(explain)?;
            let hidden: i64 = row.get(4).map_err(explain)?;
            let rowid = pk == 1 && !has_primary_index && data_type.eq_ignore_ascii_case("INTEGER");
            if hidden != 0 {
                data_type = format!("generated {data_type}");
            }
            // SQLite INT and BIGINT share a signed64bit storage class.
            let affinity = data_type.to_ascii_uppercase();
            if hidden == 0 && affinity.contains("INT") {
                data_type = "bigint".into();
            } else if hidden == 0
                && (affinity.contains("REAL")
                    || affinity.contains("FLOA")
                    || affinity.contains("DOUB"))
            {
                data_type = "double precision".into();
            } else if hidden == 0 && affinity.contains("CLOB") {
                data_type = "text".into();
            }
            columns.push(RemoteColumn {
                name,
                data_type,
                primary_key: pk > 0,
                nullable: !not_null && !rowid,
            });
        }
        Ok(columns)
    }

    pub(crate) fn editable_origin(&self, sql: &str, result: &QueryResult) -> Option<Origin> {
        if !self.writable {
            return None;
        }
        let requested = simple_table(sql)?;
        let canonical: String = self
            .database
            .query_row(
                "SELECT name FROM main.sqlite_schema WHERE name=?1 COLLATE NOCASE",
                [requested],
                |r| r.get(0),
            )
            .ok()?;
        let table = TableRef {
            namespace: "main".into(),
            name: canonical,
        };
        let all = self.edit_columns(&table).ok()?;
        let mut columns = Vec::new();
        for name in &result.columns {
            if columns
                .iter()
                .any(|c: &RemoteColumn| c.name.eq_ignore_ascii_case(name))
            {
                return None;
            }
            columns.push(
                all.iter()
                    .find(|c| c.name.eq_ignore_ascii_case(name))?
                    .clone(),
            );
        }
        if !all.iter().any(|c| c.primary_key)
            || all
                .iter()
                .filter(|c| c.primary_key)
                .any(|pk| !columns.iter().any(|c| c.name == pk.name))
        {
            return None;
        }
        // Nullable composite keys can contain duplicate NULLs in SQLite. Never edit them.
        if result.rows.iter().any(|r| {
            r.len() != columns.len()
                || r.iter()
                    .zip(&columns)
                    .any(|(v, c)| c.primary_key && matches!(v, Value::Null))
        }) {
            return None;
        }
        Some((table, columns))
    }

    pub(crate) fn transaction(&self) -> bool {
        !self.database.is_autocommit()
    }

    fn editing_scope<T>(
        &self,
        table: Option<&str>,
        f: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let table = table.map(str::to_owned);
        self.database
            .authorizer(Some(move |ctx: AuthContext<'_>| match ctx.action {
                AuthAction::Transaction { .. } | AuthAction::Savepoint { .. } => {
                    Authorization::Allow
                }
                AuthAction::Update { table_name, .. }
                    if table.as_deref() == Some(table_name)
                        && ctx.database_name == Some("main")
                        && ctx.accessor.is_none() =>
                {
                    Authorization::Allow
                }
                _ => permit_read(ctx),
            }))
            .map_err(explain)?;
        let outcome = f();
        self.database
            .authorizer(Some(permit_read))
            .map_err(explain)?;
        outcome
    }

    pub(crate) fn commit_grid(&self) -> Result<(), String> {
        if !self.writable {
            return Err("SQLite somente leitura".into());
        }
        if self.transaction() {
            self.editing_scope(None, || {
                self.database.execute_batch("COMMIT").map_err(explain)
            })?;
        }
        Ok(())
    }

    pub(crate) fn rollback_grid(&self) -> Result<(), String> {
        // Cleanup must remain possible after cancellation of a prior operation.
        self.database
            .progress_handler(0, None::<fn() -> bool>)
            .map_err(explain)?;
        if self.transaction() {
            self.editing_scope(None, || {
                self.database.execute_batch("ROLLBACK").map_err(explain)
            })?;
        }
        Ok(())
    }

    pub(crate) fn apply_grid(
        &self,
        batch: &EditBatch,
        origin: &Origin,
        result: &QueryResult,
        expected_schema: &str,
    ) -> Result<usize, String> {
        validate_batch_budget(batch)?;
        if !self.writable {
            return Err("SQLite somente leitura; habilite edição explicitamente".into());
        }
        if batch.table != origin.0
            || batch.columns != origin.1
            || batch.edits.is_empty()
            || batch.edits.len() > 500
        {
            return Err("Lote SQLite obsoleto ou inválido".into());
        }
        let all = self.edit_columns(&batch.table)?;
        if batch.columns.iter().any(|c| !all.contains(c))
            || all
                .iter()
                .filter(|c| c.primary_key)
                .any(|pk| !batch.columns.contains(pk))
        {
            return Err("Schema SQLite mudou; execute novamente a consulta".into());
        }
        // Group every changed cell from a row in one UPDATE; optimistic guards use the original row once.
        let mut groups: Vec<RowChanges> = Vec::new();
        for edit in &batch.edits {
            if edit.row.len() != batch.columns.len()
                || !result
                    .rows
                    .iter()
                    .any(|r| r.iter().map(cell).collect::<Vec<_>>() == edit.row)
            {
                return Err("Linha original não pertence ao resultado SQLite atual".into());
            }
            if edit
                .row
                .iter()
                .zip(&batch.columns)
                .any(|(v, c)| c.primary_key && matches!(v, Cell::Null))
            {
                return Err("Chave SQLite contém NULL".into());
            }
            let index = batch
                .columns
                .iter()
                .position(|c| c.name == edit.column)
                .ok_or("Coluna SQLite não encontrada")?;
            let column = &batch.columns[index];
            if column.primary_key || column.data_type.starts_with("generated ") {
                return Err("Chave primária ou coluna gerada não editável".into());
            }
            let validated = crate::cell_edit::validate_cell_edit(column, &edit.value)?;
            let value = sqlite_import_value(
                &validated,
                &ColumnInfo {
                    name: column.name.clone(),
                    data_type: column.data_type.clone(),
                    primary_key: column.primary_key,
                    nullable: column.nullable,
                },
            )?;
            if let Some(group) = groups.iter_mut().find(|(r, _)| r == &edit.row) {
                if group.1.iter().any(|(i, _)| *i == index) {
                    return Err("Célula repetida no lote SQLite".into());
                }
                group.1.push((index, value));
            } else {
                groups.push((edit.row.clone(), vec![(index, value)]));
            }
        }
        let started = !self.transaction();
        let outcome = self.editing_scope(Some(&batch.table.name), || {
            if started {
                self.database
                    .execute_batch("BEGIN IMMEDIATE")
                    .map_err(explain)?;
            }
            self.database
                .execute_batch("SAVEPOINT codisseu_grid_batch")
                .map_err(explain)?;
            // The immediate transaction locks writers and schema before metadata is checked again.
            let current = self.edit_columns(&batch.table)?;
            if current != all || self.grid_schema(&batch.table)? != expected_schema {
                return Err("Schema SQLite mudou antes da edição".into());
            }
            for (original, changes) in &groups {
                let mut values = changes.iter().map(|(_, v)| v.clone()).collect::<Vec<_>>();
                let assignments = changes
                    .iter()
                    .enumerate()
                    .map(|(n, (index, _))| {
                        format!("{}=?{}", quote(&batch.columns[*index].name), n + 1)
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                let mut predicates = Vec::new();
                for (value, column) in original.iter().zip(&batch.columns) {
                    values.push(match value {
                        Cell::Null => Value::Null,
                        Cell::Number(v) => {
                            if let Ok(i) = v.parse::<i64>() {
                                Value::Integer(i)
                            } else {
                                Value::Real(
                                    v.parse().map_err(|_| "Número original SQLite inválido")?,
                                )
                            }
                        }
                        Cell::Bool(v) => Value::Integer(i64::from(*v)),
                        Cell::Text(v) | Cell::Json(v) => Value::Text(v.clone()),
                        Cell::Binary(v) => Value::Blob(v.clone()),
                    });
                    predicates.push(format!(
                        "{} IS ?{} COLLATE BINARY",
                        quote(&column.name),
                        values.len()
                    ));
                }
                let sql = format!(
                    "UPDATE main.{} SET {assignments} WHERE {}",
                    quote(&batch.table.name),
                    predicates.join(" AND ")
                );
                let count = self
                    .database
                    .execute(&sql, rusqlite::params_from_iter(values))
                    .map_err(explain)?;
                if count != 1 {
                    return Err(format!(
                        "Conflito SQLite: esperado1registro, recebido{count}; lote revertido"
                    ));
                }
            }
            self.database
                .execute_batch("RELEASE SAVEPOINT codisseu_grid_batch")
                .map_err(explain)?;
            Ok(groups.len())
        });
        if outcome.is_err() {
            let whole_transaction_reverted = !started && !self.transaction();
            self.database
                .progress_handler(0, None::<fn() -> bool>)
                .map_err(explain)?;
            self.editing_scope(None,||{
                // A statement interrupt can already have rolled back the transaction.
                if self.transaction() {
                    if started { self.database.execute_batch("ROLLBACK").map_err(explain)?; }
                    else { self.database.execute_batch("ROLLBACK TO SAVEPOINT codisseu_grid_batch; RELEASE SAVEPOINT codisseu_grid_batch").map_err(explain)?; }
                }
                Ok(())
            })?;
            if whole_transaction_reverted {
                return outcome.map_err(|error| {
                    format!(
                        "{error}; SQLite reverteu toda a transação, inclusive os envios anteriores"
                    )
                });
            }
        }
        outcome
    }
}
