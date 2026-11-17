//! Bounded lexical SQL completion context. Catalog entries always come from a live session.
use crate::remote::{DbObject, RemoteColumn};
use crate::sql_text::{SqlDialect, statement_range};
use std::{collections::BTreeMap, ops::Range};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Relation {
    pub namespace: String,
    pub name: String,
    pub alias: String,
}
#[derive(Clone, Debug)]
pub struct Context {
    pub range: Range<usize>,
    pub prefix: String,
    pub qualifiers: Vec<String>,
    pub relation_position: bool,
    pub relations: Vec<Relation>,
    pub quote: Option<char>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Namespace,
    Table,
    Column,
    Keyword,
}
#[derive(Clone, Debug)]
pub struct Suggestion {
    pub label: String,
    pub insert: String,
    pub detail: String,
    pub kind: Kind,
}
#[derive(Clone)]
struct Token {
    value: String,
    range: Range<usize>,
    quote: Option<char>,
    opaque: bool,
}
fn ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}
fn tokens(chars: &[char], dialect: SqlDialect) -> Vec<Token> {
    let mysql = dialect == SqlDialect::Mysql;
    let mut out = vec![];
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        let line_comment = chars[i] == '#' && mysql
            || chars[i] == '-'
                && chars.get(i + 1) == Some(&'-')
                && (!mysql || chars.get(i + 2).is_none_or(|c| *c <= ' '));
        if line_comment {
            while i < chars.len() && !matches!(chars[i], '\n' | '\r') {
                i += 1;
            }
            out.push(Token {
                value: String::new(),
                range: start..i,
                quote: None,
                opaque: true,
            });
            continue;
        }
        if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            let mut depth = 1;
            while i < chars.len() && depth > 0 {
                if dialect == SqlDialect::Postgres
                    && chars[i] == '/'
                    && chars.get(i + 1) == Some(&'*')
                {
                    depth += 1;
                    i += 2;
                } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            out.push(Token {
                value: String::new(),
                range: start..i,
                quote: None,
                opaque: true,
            });
            continue;
        }
        if chars[i] == '$' && dialect == SqlDialect::Postgres {
            let mut e = i + 1;
            while e < chars.len() && (chars[e].is_alphanumeric() || chars[e] == '_') {
                e += 1;
            }
            if chars.get(e) == Some(&'$') {
                let delimiter = &chars[i..=e];
                i = e + 1;
                while i < chars.len() && !chars[i..].starts_with(delimiter) {
                    i += 1;
                }
                i = (i + delimiter.len()).min(chars.len());
                out.push(Token {
                    value: String::new(),
                    range: start..i,
                    quote: None,
                    opaque: true,
                });
                continue;
            }
        }
        let q = chars[i];
        if matches!(q, '\'' | '"') || (q == '`' && dialect != SqlDialect::Postgres) {
            i += 1;
            let mut value = String::new();
            while i < chars.len() {
                if chars[i] == q {
                    if chars.get(i + 1) == Some(&q) {
                        value.push(q);
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                if q == '\'' && chars[i] == '\\' {
                    i = (i + 2).min(chars.len());
                    continue;
                }
                value.push(chars[i]);
                i += 1;
            }
            out.push(Token {
                value,
                range: start..i,
                quote: Some(q),
                // MySQL double quotes are strings in the default server mode.
                // Do not offer identifiers inside them without ANSI_QUOTES metadata.
                opaque: q == '\'' || (mysql && q == '"'),
            });
            continue;
        }
        if ident(q) {
            i += 1;
            while i < chars.len() && ident(chars[i]) {
                i += 1;
            }
        } else {
            i += 1;
        }
        out.push(Token {
            value: chars[start..i].iter().collect(),
            range: start..i,
            quote: None,
            opaque: false,
        });
    }
    out
}
fn keyword(t: &Token, value: &str) -> bool {
    t.quote.is_none() && t.value.eq_ignore_ascii_case(value)
}
fn word(t: &Token) -> bool {
    !t.opaque && (t.quote.is_some() || t.value.chars().next().is_some_and(ident))
}

pub fn context(text: &str, cursor: usize, dialect: SqlDialect) -> Option<Context> {
    if text.len() > 256 * 1024 {
        return None;
    }
    let chars: Vec<char> = text.chars().collect();
    if cursor > chars.len() {
        return None;
    }
    let span = statement_range(text, cursor, dialect).ok()?;
    if cursor < span.start {
        return None;
    }
    let ts = tokens(&chars[span.clone()], dialect);
    let cursor = cursor - span.start;
    if ts
        .iter()
        .any(|t| t.opaque && t.range.start <= cursor && cursor <= t.range.end)
    {
        return None;
    }
    let current = ts
        .iter()
        .position(|t| word(t) && t.range.start <= cursor && cursor <= t.range.end);
    let (range, prefix, quote, before) = if let Some(index) = current {
        let t = &ts[index];
        let prefix = if let Some(quote) = t.quote {
            if cursor == t.range.start {
                return None;
            }
            let mut prefix = String::new();
            let mut i = t.range.start + 1;
            while i < cursor {
                let c = chars[span.start + i];
                if c == quote {
                    if i + 1 < cursor && chars[span.start + i + 1] == quote {
                        prefix.push(quote);
                        i += 2;
                    } else {
                        break;
                    }
                } else {
                    prefix.push(c);
                    i += 1;
                }
            }
            prefix
        } else {
            chars[span.start + t.range.start..span.start + cursor]
                .iter()
                .collect()
        };
        (t.range.clone(), prefix, t.quote, index)
    } else {
        (
            cursor..cursor,
            String::new(),
            None,
            ts.iter().take_while(|t| t.range.end <= cursor).count(),
        )
    };
    let mut qualifiers = vec![];
    let mut p = before;
    while p >= 2 && ts[p - 1].value == "." && word(&ts[p - 2]) {
        qualifiers.insert(0, ts[p - 2].value.clone());
        p -= 2;
    }
    let previous = ts[..p].iter().rev().find(|t| !t.opaque);
    let relation_position = previous.is_some_and(|t| {
        ["FROM", "JOIN", "UPDATE", "INTO"]
            .iter()
            .any(|k| keyword(t, k))
    });
    let mut relations = vec![];
    for n in 0..ts.len() {
        if !["FROM", "JOIN"].iter().any(|k| keyword(&ts[n], k)) {
            continue;
        }
        let mut j = n + 1;
        let Some(t) = ts.get(j).filter(|t| word(t)) else {
            continue;
        };
        let mut name = t.value.clone();
        let mut namespace = String::new();
        j += 1;
        if ts.get(j).is_some_and(|t| t.value == ".") && ts.get(j + 1).is_some_and(word) {
            namespace = name;
            name = ts[j + 1].value.clone();
            j += 2;
        }
        if ts.get(j).is_some_and(|t| keyword(t, "AS")) {
            j += 1;
        }
        let alias = ts
            .get(j)
            .filter(|t| {
                word(t)
                    && (t.quote.is_some()
                        || (![' ', '.'].iter().any(|c| t.value.contains(*c))
                            && !is_keyword(&t.value)))
            })
            .map_or_else(|| name.clone(), |t| t.value.clone());
        relations.push(Relation {
            namespace,
            name,
            alias,
        });
        if relations.len() >= 16 {
            break;
        }
    }
    Some(Context {
        range: span.start + range.start..span.start + range.end,
        prefix,
        qualifiers,
        relation_position,
        relations,
        quote,
    })
}
fn is_keyword(s: &str) -> bool {
    [
        "SELECT",
        "FROM",
        "WHERE",
        "JOIN",
        "LEFT",
        "RIGHT",
        "INNER",
        "OUTER",
        "ON",
        "AS",
        "ORDER",
        "GROUP",
        "BY",
        "LIMIT",
        "OFFSET",
        "HAVING",
        "UNION",
        "SET",
        "FULL",
        "CROSS",
        "NATURAL",
        "USING",
        "RETURNING",
        "FETCH",
        "FOR",
        "LOCK",
        "WITH",
        "WINDOW",
        "EXCEPT",
        "INTERSECT",
        "QUALIFY",
    ]
    .iter()
    .any(|k| s.eq_ignore_ascii_case(k))
}
pub fn quote_identifier(name: &str, dialect: SqlDialect) -> String {
    // Quote every catalog identifier: preserve PostgreSQL case and avoid an
    // incomplete hand-maintained reserved-word list. Keywords use another path.
    let q = if matches!(dialect, SqlDialect::Mysql) {
        '`'
    } else {
        '"'
    };
    format!("{q}{}{q}", name.replace(q, &format!("{q}{q}")))
}
pub fn relation_namespace(relation: &Relation, default: &str) -> String {
    if relation.namespace.is_empty() {
        default.into()
    } else {
        relation.namespace.clone()
    }
}
pub fn suggestions(
    ctx: &Context,
    dialect: SqlDialect,
    default: &str,
    namespaces: &[String],
    objects: &BTreeMap<String, Vec<DbObject>>,
    columns: &BTreeMap<(String, String), Vec<RemoteColumn>>,
) -> Vec<Suggestion> {
    let prefix = ctx.prefix.to_lowercase();
    let mut out = vec![];
    let mut qualified_columns = vec![];
    let mut push = |name: &str, detail: String, kind: Kind| {
        if name.to_lowercase().starts_with(&prefix) {
            out.push(Suggestion {
                label: name.into(),
                insert: if kind == Kind::Keyword {
                    name.into()
                } else {
                    quote_identifier(name, dialect)
                },
                detail,
                kind,
            });
        }
    };
    if ctx.qualifiers.is_empty() {
        if ctx.relation_position {
            for ns in namespaces {
                push(ns, "schema".into(), Kind::Namespace);
            }
            if let Some(tables) = objects.get(default) {
                for table in tables {
                    push(&table.name, table.namespace.clone(), Kind::Table);
                }
            }
        } else {
            // A bare column is safe only when its name is unique among all
            // referenced relations. While metadata is still arriving, qualify
            // known columns rather than temporarily offer ambiguous SQL.
            let canonical = |name: &str| {
                if dialect == SqlDialect::Mysql {
                    name.to_lowercase()
                } else {
                    name.to_owned()
                }
            };
            let mut names = BTreeMap::<String, usize>::new();
            let mut aliases = BTreeMap::<String, usize>::new();
            let mut complete = true;
            for relation in &ctx.relations {
                *aliases.entry(canonical(&relation.alias)).or_default() += 1;
                let ns = relation_namespace(relation, default);
                if let Some(cols) = columns.get(&(ns, relation.name.clone())) {
                    for col in cols {
                        *names.entry(canonical(&col.name)).or_default() += 1;
                    }
                } else {
                    complete = false;
                }
            }
            for relation in &ctx.relations {
                let ns = relation_namespace(relation, default);
                if let Some(cols) = columns.get(&(ns, relation.name.clone())) {
                    for col in cols {
                        if names.get(&canonical(&col.name)).copied().unwrap_or(0) > 1
                            || (!complete && ctx.relations.len() > 1)
                        {
                            if aliases.get(&canonical(&relation.alias)) == Some(&1)
                                && col.name.to_lowercase().starts_with(&prefix)
                            {
                                qualified_columns.push(Suggestion {
                                    label: format!("{}.{}", relation.alias, col.name),
                                    insert: format!(
                                        "{}.{}",
                                        quote_identifier(&relation.alias, dialect),
                                        quote_identifier(&col.name, dialect)
                                    ),
                                    detail: format!("{} · {}", col.data_type, relation.alias),
                                    kind: Kind::Column,
                                });
                            }
                        } else {
                            push(
                                &col.name,
                                format!("{} · {}", col.data_type, relation.alias),
                                Kind::Column,
                            );
                        }
                    }
                }
            }
            for k in [
                "SELECT", "FROM", "WHERE", "JOIN", "ON", "ORDER BY", "GROUP BY", "LIMIT", "AS",
                "AND", "OR", "NULL", "DISTINCT",
            ] {
                push(k, "SQL".into(), Kind::Keyword);
            }
            for ns in namespaces {
                push(ns, "schema".into(), Kind::Namespace);
            }
        }
    } else {
        let q = ctx.qualifiers.last().unwrap();
        let relation = ctx
            .relations
            .iter()
            .find(|r| r.alias.eq_ignore_ascii_case(q) || r.name.eq_ignore_ascii_case(q));
        if ctx.qualifiers.len() > 1 && !ctx.relation_position {
            let ns = &ctx.qualifiers[ctx.qualifiers.len() - 2];
            if let Some(cols) = columns.get(&(ns.clone(), q.clone())) {
                for c in cols {
                    push(&c.name, c.data_type.clone(), Kind::Column);
                }
            }
        } else if !ctx.relation_position
            && let Some(r) = relation
        {
            let ns = relation_namespace(r, default);
            if let Some(cols) = columns.get(&(ns, r.name.clone())) {
                for c in cols {
                    push(&c.name, c.data_type.clone(), Kind::Column);
                }
            }
        } else if let Some(tables) = objects.get(q) {
            for t in tables {
                push(&t.name, t.namespace.clone(), Kind::Table);
            }
        }
    }
    out.extend(qualified_columns);
    out.sort_by(|a, b| a.label.cmp(&b.label).then(a.detail.cmp(&b.detail)));
    out.dedup_by(|a, b| a.label == b.label && a.kind == b.kind);
    out.truncate(256);
    out
}
