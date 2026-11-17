//! A literal-only Mongo read grammar; it does not evaluate JavaScript.
use mongodb::bson::{Bson, Document};
use serde::Deserialize;

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub(crate) struct MongoRead {
    pub collection: String,
    #[serde(default)]
    pub database: Option<String>,
    #[serde(default)]
    pub filter: Document,
    #[serde(default)]
    pub projection: Option<Document>,
    #[serde(default)]
    pub sort: Option<Document>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub pipeline: Option<Vec<Document>>,
}

struct Reader<'a> {
    text: &'a str,
    at: usize,
}
impl Reader<'_> {
    fn error(&self, message: &str) -> String {
        let line = self.text[..self.at].bytes().filter(|b| *b == b'\n').count() + 1;
        format!("MongoDB · linha {line}: {message}")
    }
    fn skip(&mut self) -> Result<(), String> {
        loop {
            while self
                .text
                .as_bytes()
                .get(self.at)
                .is_some_and(u8::is_ascii_whitespace)
            {
                self.at += 1;
            }
            let rest = &self.text[self.at..];
            if rest.starts_with("//") {
                self.at += rest.find('\n').unwrap_or(rest.len());
            } else if rest.starts_with("/*") {
                self.at += rest
                    .find("*/")
                    .ok_or_else(|| self.error("comentário incompleto"))?
                    + 2;
            } else {
                return Ok(());
            }
        }
    }
    fn take(&mut self, c: u8) -> Result<bool, String> {
        self.skip()?;
        if self.text.as_bytes().get(self.at) == Some(&c) {
            self.at += 1;
            Ok(true)
        } else {
            Ok(false)
        }
    }
    fn expect(&mut self, c: u8) -> Result<(), String> {
        if self.take(c)? {
            Ok(())
        } else {
            Err(self.error(&format!("esperado '{}'", c as char)))
        }
    }
    fn ident(&mut self) -> Result<String, String> {
        self.skip()?;
        let start = self.at;
        for ch in self.text[start..].chars() {
            if ch == '_'
                || ch == '$'
                || ch.is_alphabetic()
                || (self.at > start && ch.is_ascii_digit())
            {
                self.at += ch.len_utf8();
            } else {
                break;
            }
        }
        if self.at == start {
            return Err(self.error("esperado identificador"));
        }
        Ok(self.text[start..self.at].into())
    }
    fn string(&mut self) -> Result<String, String> {
        self.skip()?;
        let quote = *self
            .text
            .as_bytes()
            .get(self.at)
            .ok_or_else(|| self.error("esperada string"))?;
        if !matches!(quote, b'\'' | b'"') {
            return Err(self.error("esperada string literal entre aspas"));
        }
        self.at += 1;
        let mut json = String::from("\"");
        loop {
            let ch = self.text[self.at..]
                .chars()
                .next()
                .ok_or_else(|| self.error("string incompleta"))?;
            self.at += ch.len_utf8();
            if ch as u32 == quote as u32 {
                break;
            }
            if ch == '\\' {
                let next = self.text[self.at..]
                    .chars()
                    .next()
                    .ok_or_else(|| self.error("escape incompleto"))?;
                self.at += next.len_utf8();
                if quote == b'\'' && next == '\'' {
                    json.push('\'');
                } else {
                    json.push('\\');
                    json.push(next);
                }
            } else if ch == '"' {
                json.push_str("\\\"");
            } else {
                json.push(ch);
            }
        }
        json.push('"');
        serde_json::from_str(&json).map_err(|e| self.error(&format!("string inválida: {e}")))
    }
    fn value(&mut self, depth: usize) -> Result<Bson, String> {
        if depth > 128 {
            return Err(self.error("literal excede 128 níveis"));
        }
        self.skip()?;
        match self.text.as_bytes().get(self.at).copied() {
            Some(b'{' ) => {
                self.at += 1;
                let mut doc = Document::new();
                while !self.take(b'}')? {
                    self.skip()?;
                    let key = if matches!(self.text.as_bytes().get(self.at), Some(b'\'' | b'"')) { self.string()? } else { self.ident()? };
                    self.expect(b':')?;
                    let value = self.value(depth + 1)?;
                    if doc.insert(key.clone(), value).is_some() { return Err(self.error(&format!("campo duplicado: {key}"))); }
                    if self.take(b'}')? { break; }
                    self.expect(b',')?;
                }
                Ok(Bson::Document(doc))
            }
            Some(b'[') => {
                self.at += 1;
                let mut values = Vec::new();
                while !self.take(b']')? {
                    values.push(self.value(depth + 1)?);
                    if self.take(b']')? { break; }
                    self.expect(b',')?;
                }
                Ok(Bson::Array(values))
            }
            Some(b'\'' | b'"') => Ok(Bson::String(self.string()?)),
            Some(b'-' | b'0'..=b'9') => {
                let start = self.at;
                while self.text.as_bytes().get(self.at).is_some_and(|b| b.is_ascii_digit() || matches!(b, b'-' | b'+' | b'.' | b'e' | b'E')) { self.at += 1; }
                let raw = &self.text[start..self.at];
                let number: serde_json::Number = serde_json::from_str(raw).map_err(|e| self.error(&format!("número inválido: {e}")))?;
                if !raw.contains(['.', 'e', 'E']) {
                    let n = raw.parse::<i64>().map_err(|_| self.error("inteiro fora de int64; use Extended JSON no console JSON"))?;
                    Ok(i32::try_from(n).map(Bson::Int32).unwrap_or(Bson::Int64(n)))
                } else { Ok(Bson::Double(number.as_f64().ok_or_else(|| self.error("número fora do intervalo"))?)) }
            }
            _ => match self.ident()?.as_str() {
                "true" => Ok(Bson::Boolean(true)), "false" => Ok(Bson::Boolean(false)), "null" => Ok(Bson::Null),
                _ => Err(self.error("somente literais; variáveis, funções e construtores JavaScript não são suportados")),
            },
        }
    }
    fn document(&mut self) -> Result<Document, String> {
        match self.value(0)? {
            Bson::Document(d) => Ok(d),
            _ => Err(self.error("esperado objeto")),
        }
    }
    fn string_call(&mut self) -> Result<String, String> {
        self.expect(b'(')?;
        let s = self.string()?;
        self.expect(b')')?;
        Ok(s)
    }
}

pub(crate) fn parse(text: &str) -> Result<MongoRead, String> {
    if text.len() > crate::SQL_BYTE_LIMIT {
        return Err("Consulta Mongo excede 256 KiB".into());
    }
    let mut p = Reader { text, at: 0 };
    if p.ident()? != "db" {
        return Err(p.error("use db.getCollection(...).find(...) ou .aggregate([...])"));
    }
    p.expect(b'.')?;
    let mut member = p.ident()?;
    let database = if member == "getSiblingDB" {
        let db = p.string_call()?;
        p.expect(b'.')?;
        member = p.ident()?;
        Some(db)
    } else {
        None
    };
    let collection = if member == "getCollection" {
        p.string_call()?
    } else {
        member
    };
    p.expect(b'.')?;
    let method = p.ident()?;
    p.expect(b'(')?;
    let mut request = MongoRead {
        database,
        collection,
        filter: Document::new(),
        projection: None,
        sort: None,
        limit: None,
        pipeline: None,
    };
    match method.as_str() {
        "find" => {
            if !p.take(b')')? {
                request.filter = p.document()?;
                if p.take(b',')? {
                    request.projection = Some(p.document()?);
                }
                p.expect(b')')?;
            }
        }
        "aggregate" => {
            let Bson::Array(stages) = p.value(0)? else {
                return Err(p.error("aggregate exige um array de estágios"));
            };
            request.pipeline = Some(
                stages
                    .into_iter()
                    .map(|s| match s {
                        Bson::Document(d) => Ok(d),
                        _ => Err(p.error("cada estágio deve ser objeto")),
                    })
                    .collect::<Result<_, _>>()?,
            );
            p.expect(b')')?;
        }
        _ => {
            return Err(p.error(&format!(
                "método {method} não suportado; use find ou aggregate de leitura"
            )));
        }
    }
    while p.take(b'.')? {
        let method = p.ident()?;
        p.expect(b'(')?;
        match method.as_str() {
            "sort" if request.pipeline.is_none() && request.sort.is_none() => {
                request.sort = Some(p.document()?)
            }
            "limit" if request.pipeline.is_none() && request.limit.is_none() => {
                let n = match p.value(0)? {
                    Bson::Int32(n) => i64::from(n),
                    Bson::Int64(n) => n,
                    _ => return Err(p.error("limit exige inteiro positivo")),
                };
                request.limit =
                    Some(usize::try_from(n).map_err(|_| p.error("limit exige inteiro positivo"))?);
            }
            _ => return Err(p.error(&format!("encadeamento {method} não suportado"))),
        }
        p.expect(b')')?;
    }
    p.take(b';')?;
    p.skip()?;
    if p.at != text.len() {
        return Err(p.error("selecione uma única consulta; conteúdo adicional não executado"));
    }
    Ok(request)
}
