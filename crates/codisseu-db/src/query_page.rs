//! Bounded views of a single query. Paging never counts or collects the complete result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColumnSort {
    /// Zero-based output column ordinal; never interpolates a column name as SQL.
    pub column: usize,
    pub descending: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryPageRequest {
    pub sql: String,
    pub offset: u64,
    pub sort: Option<ColumnSort>,
}

#[derive(Clone, Debug)]
pub struct BoundQueryPage {
    pub datasource_id: String,
    /// Explicit connection identity. Idle suspension does not change this identity.
    pub connection_generation: u64,
    pub payload: QueryPageRequest,
}

pub const MAX_PAGE_OFFSET: u64 = 1_000_000;

pub fn is_select(sql: &str) -> bool {
    let dialect = sqlparser::dialect::SQLiteDialect {};
    let read_words = sqlparser::tokenizer::Tokenizer::new(&dialect, sql).tokenize().is_ok_and(|tokens| {
        !tokens.iter().any(|t| matches!(t, sqlparser::tokenizer::Token::Word(w) if w.quote_style.is_none() && matches!(w.value.to_ascii_uppercase().as_str(), "INSERT" | "UPDATE" | "DELETE" | "MERGE" | "INTO")))
    });
    read_words
        && sqlparser::parser::Parser::parse_sql(&dialect, sql).is_ok_and(|statements| {
            statements.len() == 1 && matches!(statements[0], sqlparser::ast::Statement::Query(_))
        })
}

impl QueryPageRequest {
    pub fn first(sql: String) -> Self {
        Self {
            sql,
            offset: 0,
            sort: None,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.sql.trim().is_empty() || self.sql.len() > crate::SQL_BYTE_LIMIT {
            return Err("Consulta vazia ou excede 256 KiB".into());
        }
        if self.offset > MAX_PAGE_OFFSET || self.sort.is_some_and(|s| s.column >= 256) {
            return Err("Página ou coluna fora dos limites".into());
        }
        Ok(())
    }

    /// `canonical` has already passed the engine's single read-only query validation.
    pub fn wrapped_sql(&self, canonical: &str, page_size: usize) -> String {
        let order = self.sort.map_or_else(String::new, |sort| {
            format!(
                " ORDER BY {} {}",
                sort.column + 1,
                if sort.descending { "DESC" } else { "ASC" }
            )
        });
        format!(
            "SELECT * FROM (\n{canonical}\n) AS codisseu_result{order} LIMIT {} OFFSET {}",
            page_size.clamp(1, 500) + 1,
            self.offset
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn returning_statements_never_expose_select_paging() {
        assert!(!is_select(
            "WITH x AS (SELECT 1) INSERT INTO t SELECT * FROM x RETURNING id"
        ));
        assert!(is_select("WITH x AS (SELECT 'UPDATE') SELECT * FROM x"));
    }
    #[test]
    fn page_sql_preserves_inner_limit_and_uses_only_numeric_ordinals() {
        let request = QueryPageRequest {
            sql: "SELECT a FROM t LIMIT 15".into(),
            offset: 10,
            sort: Some(ColumnSort {
                column: 0,
                descending: true,
            }),
        };
        assert_eq!(
            request.wrapped_sql(&request.sql, 5),
            "SELECT * FROM (\nSELECT a FROM t LIMIT 15\n) AS codisseu_result ORDER BY 1 DESC LIMIT 6 OFFSET 10"
        );
        assert!(request.validate().is_ok());
        assert!(
            QueryPageRequest {
                offset: MAX_PAGE_OFFSET + 1,
                ..request.clone()
            }
            .validate()
            .is_err()
        );
        assert!(
            QueryPageRequest {
                sort: Some(ColumnSort {
                    column: 256,
                    descending: false
                }),
                ..request
            }
            .validate()
            .is_err()
        );
    }
}
