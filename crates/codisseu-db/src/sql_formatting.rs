//! Offline whitespace formatting with a conservative lexical equivalence guard.
use sqlparser::{
    dialect::{Dialect, GenericDialect, MySqlDialect, PostgreSqlDialect},
    tokenizer::{Token, Tokenizer, Whitespace},
};
pub const MAX_BYTES: usize = 256 * 1024;

fn tokens(sql: &str, dialect: &dyn Dialect) -> Result<Vec<Token>, String> {
    let mut tokens = Tokenizer::new(dialect, sql)
        .with_unescape(false)
        .tokenize()
        .map_err(|e| e.to_string())?;
    tokens.retain(|t| {
        !matches!(
            t,
            Token::Whitespace(Whitespace::Space | Whitespace::Tab | Whitespace::Newline)
        )
    });
    for token in &mut tokens {
        if let Token::Whitespace(Whitespace::SingleLineComment { comment, .. }) = token {
            *comment = comment.trim_end_matches(['\r', '\n']).into();
        }
    }
    Ok(tokens)
}

pub fn format(sql: &str) -> Result<String, String> {
    format_with_eol(sql, sql.contains("\r\n"))
}

pub fn format_with_eol(sql: &str, crlf: bool) -> Result<String, String> {
    if sql.len() > MAX_BYTES {
        return Err("Formatação SQL limitada a256KiB".into());
    }
    if sql.contains('\0') {
        return Err("SQL contém NUL".into());
    }
    let source_tokens = tokens(sql, &GenericDialect {})
        .map_err(|_| "SQL incompleto ou não suportado pelo formatador")?;
    if source_tokens.len() > 32768 {
        return Err("SQL excede32768 tokens".into());
    }
    let mut previous_literal = false;
    for token in &source_tokens {
        if matches!(token, Token::Whitespace(_)) {
            continue;
        }
        let literal = matches!(
            token,
            Token::SingleQuotedString(_) | Token::EscapedStringLiteral(_)
        );
        if literal && previous_literal {
            return Err(
                "Literais adjacentes exigem preservar quebras SQL; formatação recusada".into(),
            );
        }
        previous_literal = literal;
    }
    let mut depth = 0usize;
    for token in &source_tokens {
        match token {
            Token::LParen => {
                depth += 1;
                if depth > 64 {
                    return Err("SQL excede64 níveis".into());
                }
            }
            Token::RParen => {
                depth = depth.checked_sub(1).ok_or("Parênteses SQL incompletos")?;
            }
            _ => {}
        }
    }
    if depth != 0 {
        return Err("Parênteses SQL incompletos".into());
    }
    let options = sqlformat::FormatOptions {
        // Two line breaks leave one empty line between statements. The default
        // collapses even explicitly separated queries onto consecutive lines.
        lines_between_queries: 2,
        dialect: if source_tokens
            .iter()
            .any(|t| matches!(t, Token::DollarQuotedString(_)))
        {
            sqlformat::Dialect::PostgreSql
        } else {
            sqlformat::Dialect::Generic
        },
        ..Default::default()
    };
    // Protect opaque PostgreSQL bodies that the upstream formatter does not
    // understand. Collision-free markers are restored before equivalence checks.
    let mut protected = sql.to_owned();
    let mut literals = Vec::new();
    for token in &source_tokens {
        if matches!(token, Token::DollarQuotedString(_)) {
            let raw = token.to_string();
            if literals.iter().any(|(_, previous)| previous == &raw) {
                continue;
            }
            if literals.len() >= 128 {
                return Err("SQL excede128 literais dollar-quoted".into());
            }
            let marker = format!("codisseu_opaque_sql_{}_end_", literals.len());
            if sql.contains(&marker) {
                return Err("SQL contém marcador reservado do formatador".into());
            }
            protected = protected.replace(&raw, &marker);
            literals.push((marker, raw));
        }
    }
    let mut formatted = sqlformat::format(&protected, &sqlformat::QueryParams::None, &options);
    for (marker, raw) in literals {
        formatted = formatted.replace(&marker, &raw);
    }
    if formatted.len() > 1024 * 1024 {
        return Err("Formatação SQL excederia1MiB".into());
    }
    // Only normalize formatter-introduced newlines; quoted/comment contents must
    // remain byte-equivalent under the token guard below.
    let mut formatted = if crlf {
        formatted.replace("\r\n", "\n").replace('\n', "\r\n")
    } else {
        formatted
    };
    if sql.ends_with('\n') && !formatted.ends_with('\n') {
        formatted.push_str(if crlf { "\r\n" } else { "\n" });
    } else if !sql.ends_with('\n') {
        formatted.truncate(formatted.trim_end_matches(['\r', '\n']).len());
    }
    for dialect in [
        &GenericDialect {} as &dyn Dialect,
        &PostgreSqlDialect {},
        &MySqlDialect {},
    ] {
        let before =
            tokens(sql, dialect).map_err(|_| "SQL incompleto ou não suportado pelo formatador")?;
        let after =
            tokens(&formatted, dialect).map_err(|_| "Formatador produziu SQL não suportado")?;
        if before != after {
            return Err("Formatação SQL recusada: alteraria tokens ou comentários".into());
        }
    }
    Ok(formatted)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn multiple_queries_have_one_blank_line_and_no_extra_trailing_lines() {
        for separator in ["", " ", "\n", "\n\n\n"] {
            for eol in ["\n", "\r\n"] {
                for final_eol in ["", eol] {
                    let source =
                        format!("select 1;{separator}select 2;{separator}select 3;{final_eol}");
                    let expected = format!(
                        "select{eol}  1;{eol}{eol}select{eol}  2;{eol}{eol}select{eol}  3;{final_eol}"
                    );
                    let output = format_with_eol(&source, eol == "\r\n").unwrap();
                    assert_eq!(output, expected);
                    assert_eq!(format_with_eol(&output, eol == "\r\n").unwrap(), output);
                }
            }
        }
    }

    #[test]
    fn query_spacing_does_not_split_quoted_semicolons_or_comments() {
        for sql in [
            "select 'ação 😀; select 9', \"semi;colon\";select 2;",
            "select $$begin;\nreturn 'ação 😀';\nend$$;select 2;",
            "select 1;/* keep ; select 9 */ select 2;",
            "select 1;-- keep ; select 9\nselect 2;",
            "-- first\nselect 1;\n-- second\nselect 2;",
        ] {
            let output = format(sql).unwrap();
            assert!(output.contains(";\n\n"), "{output}");
            assert_eq!(
                tokens(sql, &GenericDialect {}).unwrap(),
                tokens(&output, &GenericDialect {}).unwrap()
            );
            assert_eq!(format(&output).unwrap(), output);
        }
    }

    #[test]
    fn queries_ctes_comments_literals_and_idempotence() {
        for sql in [
            "select a,b from users where id=1 order by a; select 2;",
            "with x as (select 1 as id) select x.id,u.name from x join users u on u.id=x.id where u.name='ação 😀; from';",
            "-- comment\nselect 'a  b', :name, ? from t; /* keep */",
            "select $$a ; b$$, \"MixedCase\" from t;",
            "select $body$begin\nreturn 'ação 😀';\nend$body$;",
        ] {
            let output = format(sql).unwrap();
            assert!(output.contains('\n'), "{output}");
            assert_eq!(format(&output).unwrap(), output);
        }
    }
    #[test]
    fn formatted_query_preserves_real_sqlite_results() {
        let sql = "with x(v) as (values ('ação 😀'),('b')) select v || ';from' from x where length(v)>0 order by v;";
        let formatted = format(sql).unwrap();
        let connection = rusqlite::Connection::open_in_memory().unwrap();
        let run = |query: &str| {
            connection
                .prepare(query)
                .unwrap()
                .query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        assert_eq!(run(sql), run(&formatted));
    }
    #[test]
    fn crlf_final_newline_invalid_and_limits() {
        let output = format("select a,b from t;\r\n").unwrap();
        assert!(output.ends_with("\r\n"));
        assert!(!output.replace("\r\n", "").contains('\n'));
        assert!(format("select 'unfinished").is_err());
        assert!(format("select 'a'\n'b';").is_err());
        assert!(format(&" ".repeat(MAX_BYTES + 1)).is_err());
        assert!(format(&format!("select {}1{};", "(".repeat(65), ")".repeat(65))).is_err());
    }
}
