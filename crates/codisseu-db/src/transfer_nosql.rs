//! Portable, bounded NoSQL transfer records. No executable commands in files.
//! Mongo uses Canonical Extended JSON; cache values retain arbitrary bytes.
use crate::remote::Engine;
use mongodb::bson::{Bson, Document};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const RECORD_LIMIT: usize = 500;
pub const RECORD_BYTES: usize = 64 * 1024;
pub const DECODED_BYTES: usize = 4 * 1024 * 1024;
pub const FILE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "encoding",
    content = "data",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum TransferBytes {
    Utf8(String),
    Hex(String),
}
impl TransferBytes {
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        match String::from_utf8(bytes) {
            Ok(text) => Self::Utf8(text),
            Err(error) => Self::Hex(hex(&error.into_bytes())),
        }
    }
    pub fn decode(&self) -> Result<Vec<u8>, String> {
        match self {
            Self::Utf8(text) if text.len() <= RECORD_BYTES => Ok(text.as_bytes().to_vec()),
            Self::Hex(value) if value.len() <= RECORD_BYTES * 2 => unhex(value),
            _ => Err("Valor de transferência excede 64 KiB".into()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "unix_ms",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum TransferExpiry {
    Never,
    AtUnixMs(u64),
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "data",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum RedisValue {
    String(TransferBytes),
    List(Vec<TransferBytes>),
    Hash(Vec<(TransferBytes, TransferBytes)>),
    Set(Vec<TransferBytes>),
    Zset(Vec<(TransferBytes, String)>),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum NoSqlRecord {
    Mongo {
        document: serde_json::Value,
    },
    Redis {
        key: TransferBytes,
        value: RedisValue,
        expiry: TransferExpiry,
    },
    Memcached {
        key: String,
        value: TransferBytes,
        flags: u32,
        expiry: TransferExpiry,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoSqlTransfer {
    pub version: u32,
    pub engine: Engine,
    pub source_truncated: bool,
    pub records: Vec<NoSqlRecord>,
}
impl NoSqlTransfer {
    pub fn to_json(&self) -> Result<String, String> {
        validate_transfer(self)?;
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        if json.len() > FILE_BYTES {
            return Err("Arquivo de transferência excede 8 MiB".into());
        }
        Ok(json)
    }
    pub fn from_json(json: &str) -> Result<Self, String> {
        if json.len() > FILE_BYTES {
            return Err("Arquivo de transferência excede 8 MiB".into());
        }
        let data: Self =
            serde_json::from_str(json).map_err(|e| format!("Transferência NoSQL inválida: {e}"))?;
        validate_transfer(&data)?;
        Ok(data)
    }
}

#[derive(Clone, Debug)]
pub enum NoSqlExportRequest {
    Mongo {
        collection: String,
        filter: serde_json::Value,
        limit: usize,
    },
    Keys {
        keys: Vec<TransferBytes>,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NoSqlImportTarget {
    MongoCollection(String),
    RedisKeys,
    MemcachedKeys,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum UnknownTtlPolicy {
    #[default]
    Reject,
    NeverExpire,
    AfterSeconds(u64),
}

#[derive(Clone, Debug)]
pub struct PreparedNoSqlImport {
    pub(crate) data: NoSqlTransfer,
    pub(crate) target: NoSqlImportTarget,
    pub(crate) ttl_policy: UnknownTtlPolicy,
}
impl PreparedNoSqlImport {
    pub fn engine(&self) -> Engine {
        self.data.engine
    }
    pub fn len(&self) -> usize {
        self.data.records.len()
    }
    pub fn is_empty(&self) -> bool {
        self.data.records.is_empty()
    }
    pub fn target(&self) -> &NoSqlImportTarget {
        &self.target
    }
    pub fn summary(&self) -> String {
        format!(
            "{} itens · criar somente · sem rollback de grupo",
            self.len()
        )
    }
}

#[derive(Clone, Debug)]
pub struct NoSqlImportPreview {
    pub records: usize,
    /// Zero-based source row indices; preview never changes these items.
    pub conflicts: Vec<usize>,
    pub warnings: Vec<String>,
    pub atomic_group: bool,
}
#[derive(Clone, Debug, Default)]
pub struct NoSqlImportReport {
    pub applied: usize,
    pub failed_index: Option<usize>,
    pub error: Option<String>,
    /// The failing request may already have changed the current item.
    pub outcome_unknown: bool,
    pub atomic_group: bool,
}

pub fn prepare_import(
    json: &str,
    engine: Engine,
    target: NoSqlImportTarget,
    ttl_policy: UnknownTtlPolicy,
) -> Result<PreparedNoSqlImport, String> {
    let data = if engine == Engine::Mongo {
        mongo_input(json)?
    } else {
        NoSqlTransfer::from_json(json)?
    };
    if data.records.is_empty() {
        return Err("Arquivo não contém itens para importar".into());
    }
    if data.engine != engine {
        return Err("Engine do arquivo difere do datasource de destino".into());
    }
    match (&target, engine) {
        (NoSqlImportTarget::MongoCollection(name), Engine::Mongo) => validate_collection(name)?,
        (NoSqlImportTarget::RedisKeys, Engine::Redis)
        | (NoSqlImportTarget::MemcachedKeys, Engine::Memcached) => {}
        _ => return Err("Destino de importação não corresponde ao engine".into()),
    }
    if matches!(
        ttl_policy,
        UnknownTtlPolicy::AfterSeconds(0) | UnknownTtlPolicy::AfterSeconds(3_153_600_001..)
    ) {
        return Err("TTL explícito deve ser 1–3153600000 segundos".into());
    }
    for record in &data.records {
        let expiry = match record {
            NoSqlRecord::Redis { expiry, .. } | NoSqlRecord::Memcached { expiry, .. } => {
                Some(expiry)
            }
            _ => None,
        };
        if expiry == Some(&TransferExpiry::Unknown) && ttl_policy == UnknownTtlPolicy::Reject {
            return Err(
                "TTL desconhecido: escolha explicitamente duração ou sem expiração na prévia"
                    .into(),
            );
        }
    }
    Ok(PreparedNoSqlImport {
        data,
        target,
        ttl_policy,
    })
}

fn mongo_input(input: &str) -> Result<NoSqlTransfer, String> {
    if input.len() > FILE_BYTES {
        return Err("Arquivo de transferência excede 8 MiB".into());
    }
    let envelope_key = |value: &serde_json::Value| {
        value.as_object().is_some_and(|fields| {
            ["version", "engine", "records"]
                .iter()
                .any(|key| fields.contains_key(*key))
        })
    };
    let documents = match serde_json::from_str::<serde_json::Value>(input) {
        Ok(serde_json::Value::Array(documents)) => documents,
        Ok(value) if envelope_key(&value) => return NoSqlTransfer::from_json(input),
        Ok(value)
            if value
                .as_object()
                .is_some_and(|fields| fields.contains_key("_id")) =>
        {
            vec![value]
        }
        Ok(_) => return Err(
            "Mongo aceita envelope NoSQL, documento com _id, array ou NDJSON de documentos com _id"
                .into(),
        ),
        Err(_) => {
            let mut documents = Vec::new();
            for line in input.lines().filter(|line| !line.trim().is_empty()) {
                if documents.len() == RECORD_LIMIT {
                    return Err("NDJSON Mongo excede 500 documentos".into());
                }
                let document: serde_json::Value = serde_json::from_str(line)
                    .map_err(|e| format!("Mongo JSON/NDJSON inválido: {e}"))?;
                if envelope_key(&document) {
                    return Err(
                        "Envelope NoSQL inválido não pode ser reinterpretado como NDJSON".into(),
                    );
                }
                documents.push(document);
            }
            documents
        }
    };
    let data = NoSqlTransfer {
        version: 1,
        engine: Engine::Mongo,
        source_truncated: false,
        records: documents
            .into_iter()
            .map(|document| NoSqlRecord::Mongo { document })
            .collect(),
    };
    validate_transfer(&data)?;
    Ok(data)
}

pub(crate) fn validate_collection(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > 255 || name.contains('\0') || name.starts_with("system.") {
        Err("Coleção inválida ou reservada".into())
    } else {
        Ok(())
    }
}

pub(crate) fn mongo_document(value: &serde_json::Value) -> Result<Document, String> {
    fn unsigned_out_of_range(value: &serde_json::Value) -> bool {
        match value {
            serde_json::Value::Number(number) => {
                number.as_u64().is_some_and(|n| n > i64::MAX as u64)
            }
            serde_json::Value::Array(values) => values.iter().any(unsigned_out_of_range),
            serde_json::Value::Object(fields) => fields.values().any(unsigned_out_of_range),
            _ => false,
        }
    }
    if unsigned_out_of_range(value) {
        return Err(
            "Inteiro Mongo fora de i64; use Extended JSON Decimal128 para preservar precisão"
                .into(),
        );
    }
    let Bson::Document(document) = Bson::try_from(value.clone()).map_err(|e| e.to_string())? else {
        return Err("Cada registro Mongo deve ser um documento Extended JSON".into());
    };
    if !document.contains_key("_id")
        || matches!(document.get("_id"), Some(Bson::Null | Bson::Array(_)))
    {
        return Err("Importação Mongo exige _id explícito, não nulo e não array".into());
    }
    Ok(document)
}

pub(crate) fn validate_mem_key(key: &str) -> Result<(), String> {
    if key.is_empty() || key.len() > 250 || key.bytes().any(|b| b <= b' ' || b == 127) {
        Err("Chave Memcached deve ter 1–250 bytes, sem espaços ou controles".into())
    } else {
        Ok(())
    }
}

pub(crate) fn now_ms() -> Result<u64, String> {
    let value = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "Relógio anterior a 1970")?
        .as_millis();
    u64::try_from(value).map_err(|_| "Relógio fora do intervalo".into())
}

pub(crate) fn resolve_expiry(
    expiry: &TransferExpiry,
    policy: &UnknownTtlPolicy,
) -> Result<Option<u64>, String> {
    let now = now_ms()?;
    let expiry = match expiry {
        TransferExpiry::Never => return Ok(None),
        TransferExpiry::AtUnixMs(value) => *value,
        TransferExpiry::Unknown => match policy {
            UnknownTtlPolicy::Reject => {
                return Err("TTL desconhecido sem política explícita".into());
            }
            UnknownTtlPolicy::NeverExpire => return Ok(None),
            UnknownTtlPolicy::AfterSeconds(seconds) => now
                .checked_add(seconds.checked_mul(1000).ok_or("TTL excede intervalo")?)
                .ok_or("TTL excede intervalo")?,
        },
    };
    if expiry <= now {
        return Err(
            "Registro já expirou; nenhum item expirado será importado como permanente".into(),
        );
    }
    if expiry > i64::MAX as u64 {
        return Err("Timestamp de expiração fora do intervalo".into());
    }
    Ok(Some(expiry))
}

pub(crate) fn validate_transfer(data: &NoSqlTransfer) -> Result<(), String> {
    if data.version != 1 {
        return Err("Versão de transferência NoSQL não suportada".into());
    }
    if !matches!(
        data.engine,
        Engine::Mongo | Engine::Redis | Engine::Memcached
    ) {
        return Err("Formato nativo NoSQL exige MongoDB, Redis ou Memcached".into());
    }
    if data.records.len() > RECORD_LIMIT {
        return Err("Transferência excede 500 registros".into());
    }
    let mut total = 0usize;
    let mut identities = BTreeSet::new();
    for record in &data.records {
        let (size, identity) = match (data.engine, record) {
            (Engine::Mongo, NoSqlRecord::Mongo { document }) => {
                let document = mongo_document(document)?;
                let bytes = mongodb::bson::to_vec(&document).map_err(|e| e.to_string())?;
                (
                    bytes.len(),
                    document
                        .get("_id")
                        .unwrap()
                        .clone()
                        .into_canonical_extjson()
                        .to_string()
                        .into_bytes(),
                )
            }
            (Engine::Redis, NoSqlRecord::Redis { key, value, expiry }) => {
                validate_expiry(expiry)?;
                let key = key.decode()?;
                if key.len() > 1024 {
                    return Err("Chave Redis excede 1 KiB".into());
                }
                let size = key.len() + redis_bytes(value)?;
                (size, key)
            }
            (
                Engine::Memcached,
                NoSqlRecord::Memcached {
                    key, value, expiry, ..
                },
            ) => {
                validate_mem_key(key)?;
                validate_expiry(expiry)?;
                (key.len() + value.decode()?.len(), key.as_bytes().to_vec())
            }
            _ => return Err("Registro não corresponde ao engine do arquivo".into()),
        };
        if size > RECORD_BYTES {
            return Err("Registro excede 64 KiB decodificados".into());
        }
        total = total.checked_add(size).ok_or("Payload excede intervalo")?;
        if total > DECODED_BYTES {
            return Err("Transferência excede 4 MiB decodificados".into());
        }
        if !identities.insert(identity) {
            return Err("Arquivo contém identidade/chave duplicada".into());
        }
    }
    Ok(())
}

fn validate_expiry(expiry: &TransferExpiry) -> Result<(), String> {
    if matches!(expiry, TransferExpiry::AtUnixMs(0)) {
        return Err("Expiração zero não significa permanente; use Never".into());
    }
    if matches!(expiry, TransferExpiry::AtUnixMs(value) if *value > i64::MAX as u64) {
        return Err("Timestamp de expiração fora do intervalo".into());
    }
    Ok(())
}

fn redis_bytes(value: &RedisValue) -> Result<usize, String> {
    fn cardinality(count: usize) -> Result<(), String> {
        if count == 0 || count > RECORD_LIMIT {
            Err("Coleção Redis deve ter 1–500 membros".into())
        } else {
            Ok(())
        }
    }
    let mut seen = BTreeSet::new();
    match value {
        RedisValue::String(value) => Ok(value.decode()?.len()),
        RedisValue::List(values) | RedisValue::Set(values) => {
            cardinality(values.len())?;
            let mut size = 0;
            for member in values {
                let bytes = member.decode()?;
                size += bytes.len();
                seen.insert(bytes);
            }
            // Lists may repeat values; Redis sets may not.
            if matches!(value, RedisValue::Set(_)) && seen.len() != values.len() {
                return Err("Set contém membro duplicado".into());
            }
            Ok(size)
        }
        RedisValue::Hash(values) => {
            cardinality(values.len())?;
            let mut size = 0;
            for (field, value) in values {
                let field = field.decode()?;
                size += field.len() + value.decode()?.len();
                if !seen.insert(field) {
                    return Err("Hash contém campo duplicado".into());
                }
            }
            Ok(size)
        }
        RedisValue::Zset(values) => {
            cardinality(values.len())?;
            let mut size = 0;
            for (member, score) in values {
                let member = member.decode()?;
                let numeric: f64 = score.parse().map_err(|_| "Score Redis inválido")?;
                if !numeric.is_finite() {
                    return Err("Score deve ser finito".into());
                }
                size += member.len() + score.len();
                if !seen.insert(member) {
                    return Err("Zset contém membro duplicado".into());
                }
            }
            Ok(size)
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(result, "{byte:02X}");
    }
    result
}
fn unhex(value: &str) -> Result<Vec<u8>, String> {
    if !value.len().is_multiple_of(2) {
        return Err("Hexadecimal deve ter número par de dígitos".into());
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let digit = |b: u8| match b {
                b'0'..=b'9' => Some(b - b'0'),
                b'a'..=b'f' => Some(b - b'a' + 10),
                b'A'..=b'F' => Some(b - b'A' + 10),
                _ => None,
            };
            Ok(digit(pair[0]).ok_or("Hexadecimal inválido")? * 16
                + digit(pair[1]).ok_or("Hexadecimal inválido")?)
        })
        .collect()
}
