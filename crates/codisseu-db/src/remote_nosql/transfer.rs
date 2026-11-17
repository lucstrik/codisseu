//! Transfer operations retain the session's existing client/socket.
use super::*;
use crate::transfer_nosql::{
    self as model, NoSqlExportRequest, NoSqlImportPreview, NoSqlImportReport, NoSqlImportTarget,
    NoSqlRecord, NoSqlTransfer, PreparedNoSqlImport, RedisValue, TransferBytes, TransferExpiry,
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

impl NoSqlSession {
    pub(crate) async fn export_source(
        &mut self,
        config: &ConnectionConfig,
        request: &NoSqlExportRequest,
    ) -> Result<NoSqlTransfer, String> {
        let data = match (self, request) {
            (
                Self::Mongo(client),
                NoSqlExportRequest::Mongo {
                    collection,
                    filter,
                    limit,
                },
            ) => {
                model::validate_collection(collection)?;
                if *limit == 0 || *limit > model::RECORD_LIMIT {
                    return Err("Limite Mongo deve ser 1–500".into());
                }
                // Reuse console validation to block server-side JavaScript.
                let input =
                    serde_json::json!({"collection":collection,"filter":filter,"limit":limit})
                        .to_string();
                let request = mongo_request(&input, model::RECORD_LIMIT)?;
                let collection = client
                    .database(&config.database)
                    .collection::<Document>(collection);
                let mut cursor = collection
                    .find(request.filter)
                    .limit((*limit + 1) as i64)
                    .batch_size(16)
                    .max_time(timeout(config))
                    .await
                    .map_err(|e| e.to_string())?;
                let mut data = NoSqlTransfer {
                    version: 1,
                    engine: Engine::Mongo,
                    source_truncated: false,
                    records: vec![],
                };
                while cursor.advance().await.map_err(|e| e.to_string())? {
                    if data.records.len() == *limit {
                        data.source_truncated = true;
                        break;
                    }
                    let document = cursor.deserialize_current().map_err(|e| e.to_string())?;
                    data.records.push(NoSqlRecord::Mongo {
                        document: Bson::Document(document).into_canonical_extjson(),
                    });
                    model::validate_transfer(&data)?;
                }
                data
            }
            (Self::Redis(connection), NoSqlExportRequest::Keys { keys }) => {
                validate_keys(keys)?;
                let mut data = NoSqlTransfer {
                    version: 1,
                    engine: Engine::Redis,
                    source_truncated: false,
                    records: vec![],
                };
                for key in keys {
                    let key = key.decode()?;
                    let record = redis_export(connection, &key).await;
                    // Never leave a WATCH attached to the shared session.
                    let cleanup = redis::cmd("UNWATCH").query_async::<()>(connection).await;
                    let record = record?;
                    cleanup.map_err(|e| e.to_string())?;
                    data.records.push(record);
                    model::validate_transfer(&data)?;
                }
                data
            }
            (Self::Memcached(socket), NoSqlExportRequest::Keys { keys }) => {
                validate_keys(keys)?;
                let mut connection = socket.take().ok_or("Memcached desconectado; reconecte")?;
                let response = async {
                    let mut data = NoSqlTransfer {
                        version: 1,
                        engine: Engine::Memcached,
                        source_truncated: false,
                        records: vec![],
                    };
                    for key in keys {
                        let key = String::from_utf8(key.decode()?)
                            .map_err(|_| "Memcached texto exige chave UTF-8")?;
                        model::validate_mem_key(&key)?;
                        let record = mem_export(&mut connection, &key).await?;
                        data.records.push(record);
                        model::validate_transfer(&data)?;
                    }
                    Ok::<_, String>(data)
                }
                .await;
                if response.is_ok() {
                    *socket = Some(connection);
                }
                response?
            }
            _ => return Err("Fonte de exportação não corresponde ao datasource".into()),
        };
        // Enforce encoded as well as decoded size before publishing anything.
        data.to_json()?;
        Ok(data)
    }

    pub(crate) async fn preview_import(
        &mut self,
        config: &ConnectionConfig,
        plan: &PreparedNoSqlImport,
    ) -> Result<NoSqlImportPreview, String> {
        validate_plan(config, plan)?;
        let mut conflicts = Vec::new();
        for (index, record) in plan.data.records.iter().enumerate() {
            let exists = match (&mut *self, record) {
                (Self::Mongo(client), NoSqlRecord::Mongo { document }) => {
                    let NoSqlImportTarget::MongoCollection(collection) = &plan.target else {
                        return Err("Destino Mongo inválido".into());
                    };
                    let document = model::mongo_document(document)?;
                    client
                        .database(&config.database)
                        .collection::<Document>(collection)
                        .find_one(doc! {"_id":document.get("_id").unwrap().clone()})
                        .projection(doc! {"_id":1})
                        .max_time(timeout(config))
                        .await
                        .map_err(|e| e.to_string())?
                        .is_some()
                }
                (Self::Redis(connection), NoSqlRecord::Redis { key, .. }) => redis::cmd("EXISTS")
                    .arg(key.decode()?)
                    .query_async::<bool>(connection)
                    .await
                    .map_err(|e| e.to_string())?,
                (Self::Memcached(socket), NoSqlRecord::Memcached { key, .. }) => {
                    let mut connection =
                        socket.take().ok_or("Memcached desconectado; reconecte")?;
                    let response = async {
                        connection
                            .write_all(format!("mg {key} c\r\n").as_bytes())
                            .await
                            .map_err(|e| e.to_string())?;
                        let line = mem_line(&mut connection).await?;
                        if line == "EN" {
                            Ok(false)
                        } else if line.starts_with("HD ") || line == "HD" {
                            Ok(true)
                        } else {
                            Err(format!(
                                "Memcached meta indisponível ou resposta inválida: {line}"
                            ))
                        }
                    }
                    .await;
                    if response.is_ok() {
                        *socket = Some(connection);
                    }
                    response?
                }
                _ => return Err("Registro não corresponde à sessão ativa".into()),
            };
            if exists {
                conflicts.push(index);
            }
        }
        let mut warnings = vec!["Criar somente: itens existentes nunca serão sobrescritos. O lote NoSQL não é atômico e não oferece rollback; mudanças já confirmadas permanecem após erro/cancelamento.".into()];
        if plan.data.source_truncated {
            warnings.push("O arquivo contém apenas um recorte limitado da fonte.".into());
        }
        if plan.data.records.iter().any(|record| {
            matches!(
                record,
                NoSqlRecord::Redis {
                    expiry: TransferExpiry::Unknown,
                    ..
                } | NoSqlRecord::Memcached {
                    expiry: TransferExpiry::Unknown,
                    ..
                }
            )
        }) {
            warnings.push(match plan.ttl_policy {
                model::UnknownTtlPolicy::NeverExpire => "TTL original desconhecido: os itens serão permanentes pela escolha explícita desta importação.".into(),
                model::UnknownTtlPolicy::AfterSeconds(seconds) => format!("TTL original desconhecido: os itens receberão {seconds} segundos a partir da criação."),
                model::UnknownTtlPolicy::Reject => "TTL desconhecido impede aplicação.".into(),
            });
        }
        if plan
            .data
            .records
            .iter()
            .any(|r| matches!(r, NoSqlRecord::Memcached { .. }))
        {
            warnings.push("Memcached arredonda a expiração para segundos; a expiração é encurtada em até um segundo, sem tornar TTL desconhecido permanente.".into());
        }
        Ok(NoSqlImportPreview {
            records: plan.len(),
            conflicts,
            warnings,
            atomic_group: false,
        })
    }

    pub(crate) async fn apply_import(
        &mut self,
        config: &ConnectionConfig,
        plan: &PreparedNoSqlImport,
        progress: Arc<AtomicUsize>,
    ) -> NoSqlImportReport {
        progress.store(0, Ordering::SeqCst);
        let mut report = NoSqlImportReport::default();
        if config.read_only {
            report.error = Some("Datasource somente leitura: importação não enviada".into());
            return report;
        }
        let check = self.preview_import(config, plan).await;
        match check {
            Err(error) => {
                report.error = Some(error);
                return report;
            }
            Ok(preview) if !preview.conflicts.is_empty() => {
                report.failed_index = preview.conflicts.first().copied();
                report.error =
                    Some("Conflito: chave/_id já existe; nenhum item foi importado".into());
                return report;
            }
            _ => {}
        }
        for (index, record) in plan.data.records.iter().enumerate() {
            let mut known_rejection = false;
            let result: Result<bool, String> = match (&mut *self, record) {
                (Self::Mongo(client), NoSqlRecord::Mongo { document }) => {
                    let NoSqlImportTarget::MongoCollection(collection) = &plan.target else {
                        unreachable!()
                    };
                    match model::mongo_document(document) {
                        Ok(document) => client
                            .database(&config.database)
                            .collection::<Document>(collection)
                            .insert_one(document)
                            .await
                            .map(|_| true)
                            .map_err(|e| {
                                known_rejection = matches!(
                                    e.kind.as_ref(),
                                    mongodb::error::ErrorKind::Write(
                                        mongodb::error::WriteFailure::WriteError(_)
                                    )
                                );
                                e.to_string()
                            }),
                        Err(e) => Err(e),
                    }
                }
                (Self::Redis(connection), NoSqlRecord::Redis { key, value, expiry }) => {
                    match (
                        key.decode(),
                        model::resolve_expiry(expiry, &plan.ttl_policy),
                    ) {
                        (Ok(key), Ok(expiry)) => {
                            redis_insert(connection, &key, value, expiry).await
                        }
                        (Err(e), _) | (_, Err(e)) => Err(e),
                    }
                }
                (
                    Self::Memcached(socket),
                    NoSqlRecord::Memcached {
                        key,
                        value,
                        flags,
                        expiry,
                    },
                ) => {
                    match (
                        value.decode(),
                        model::resolve_expiry(expiry, &plan.ttl_policy),
                    ) {
                        (Ok(value), Ok(expiry)) => {
                            if let Some(mut connection) = socket.take() {
                                let response =
                                    mem_insert(&mut connection, key, &value, *flags, expiry).await;
                                if response.is_ok() {
                                    *socket = Some(connection);
                                }
                                response
                            } else {
                                Err("Memcached desconectado; reconecte".into())
                            }
                        }
                        (Err(e), _) | (_, Err(e)) => Err(e),
                    }
                }
                _ => Err("Registro não corresponde à sessão ativa".into()),
            };
            match result {
                Ok(true) => {
                    report.applied += 1;
                    progress.store(report.applied, Ordering::SeqCst);
                }
                Ok(false) => {
                    report.failed_index = Some(index);
                    report.error = Some(
                        "Conflito ao criar item; itens anteriores permanecem aplicados".into(),
                    );
                    break;
                }
                Err(error) => {
                    report.failed_index = Some(index);
                    report.outcome_unknown = !known_rejection
                        && !error.contains("E11000")
                        && !error.contains("já expirou");
                    report.error = Some(format!(
                        "Importação interrompida: {error}. Itens confirmados anteriormente permanecem aplicados; sem rollback de grupo."
                    ));
                    break;
                }
            }
        }
        report
    }
}

fn validate_plan(config: &ConnectionConfig, plan: &PreparedNoSqlImport) -> Result<(), String> {
    if config.engine != plan.engine() {
        return Err("Plano pertence a outro engine".into());
    }
    model::validate_transfer(&plan.data)?;
    for record in &plan.data.records {
        if let NoSqlRecord::Redis { expiry, .. } | NoSqlRecord::Memcached { expiry, .. } = record {
            let expiry = model::resolve_expiry(expiry, &plan.ttl_policy)?;
            if matches!(record, NoSqlRecord::Memcached { .. })
                && expiry.is_some_and(|ms| {
                    ms / 1000 > u32::MAX as u64
                        || ms / 1000 <= model::now_ms().unwrap_or(u64::MAX) / 1000
                })
            {
                return Err("Expiração Memcached fora do intervalo seguro de segundos Unix".into());
            }
        }
    }
    Ok(())
}
fn validate_keys(keys: &[TransferBytes]) -> Result<(), String> {
    if keys.is_empty() || keys.len() > model::RECORD_LIMIT {
        return Err("Informe 1–500 chaves explícitas; nenhuma varredura será iniciada".into());
    }
    let mut seen = std::collections::BTreeSet::new();
    for key in keys {
        let key = key.decode()?;
        if key.len() > 1024 {
            return Err("Chave excede 1 KiB".into());
        }
        if !seen.insert(key) {
            return Err("Chaves explícitas duplicadas".into());
        }
    }
    Ok(())
}

async fn redis_export(
    connection: &mut redis::aio::MultiplexedConnection,
    key: &[u8],
) -> Result<NoSqlRecord, String> {
    redis::cmd("WATCH")
        .arg(key)
        .query_async::<()>(connection)
        .await
        .map_err(|e| e.to_string())?;
    let kind: String = redis::cmd("TYPE")
        .arg(key)
        .query_async(connection)
        .await
        .map_err(|e| e.to_string())?;
    let cardinality_command = match kind.as_str() {
        "string" => "STRLEN",
        "list" => "LLEN",
        "hash" => "HLEN",
        "set" => "SCARD",
        "zset" => "ZCARD",
        "none" => return Err("Chave explícita não existe ou expirou".into()),
        _ => return Err(format!("Tipo Redis não suportado: {kind}")),
    };
    let count: usize = redis::cmd(cardinality_command)
        .arg(key)
        .query_async(connection)
        .await
        .map_err(|e| e.to_string())?;
    if count
        > if kind == "string" {
            model::RECORD_BYTES
        } else {
            model::RECORD_LIMIT
        }
    {
        return Err("Chave Redis excede limite de transferência".into());
    }
    let value = match kind.as_str() {
        "string" => RedisValue::String(TransferBytes::from_bytes(
            redis::cmd("GETRANGE")
                .arg(key)
                .arg(0)
                .arg(model::RECORD_BYTES - 1)
                .query_async(connection)
                .await
                .map_err(|e| e.to_string())?,
        )),
        "list" => {
            let values: Vec<Vec<u8>> = redis::cmd("LRANGE")
                .arg(key)
                .arg(0)
                .arg(count.saturating_sub(1))
                .query_async(connection)
                .await
                .map_err(|e| e.to_string())?;
            RedisValue::List(values.into_iter().map(TransferBytes::from_bytes).collect())
        }
        "set" => {
            let values: Vec<Vec<u8>> = redis::cmd("SMEMBERS")
                .arg(key)
                .query_async(connection)
                .await
                .map_err(|e| e.to_string())?;
            RedisValue::Set(values.into_iter().map(TransferBytes::from_bytes).collect())
        }
        "hash" => {
            let values: Vec<(Vec<u8>, Vec<u8>)> = redis::cmd("HGETALL")
                .arg(key)
                .query_async(connection)
                .await
                .map_err(|e| e.to_string())?;
            RedisValue::Hash(
                values
                    .into_iter()
                    .map(|(k, v)| (TransferBytes::from_bytes(k), TransferBytes::from_bytes(v)))
                    .collect(),
            )
        }
        "zset" => {
            let values: Vec<(Vec<u8>, String)> = redis::cmd("ZRANGE")
                .arg(key)
                .arg(0)
                .arg(count.saturating_sub(1))
                .arg("WITHSCORES")
                .query_async(connection)
                .await
                .map_err(|e| e.to_string())?;
            RedisValue::Zset(
                values
                    .into_iter()
                    .map(|(k, v)| (TransferBytes::from_bytes(k), v))
                    .collect(),
            )
        }
        _ => unreachable!(),
    };
    // TIME before PTTL makes the computed deadline conservative on older Redis.
    let time: (u64, u64) = redis::cmd("TIME")
        .query_async(connection)
        .await
        .map_err(|e| e.to_string())?;
    let ttl: i64 = redis::cmd("PTTL")
        .arg(key)
        .query_async(connection)
        .await
        .map_err(|e| e.to_string())?;
    let expiry = match ttl {
        -1 => TransferExpiry::Never,
        -2 => return Err("Chave expirou durante exportação".into()),
        n if n >= 0 => TransferExpiry::AtUnixMs(
            time.0
                .checked_mul(1000)
                .and_then(|ms| ms.checked_add(time.1 / 1000))
                .and_then(|ms| ms.checked_add(n as u64))
                .ok_or("TTL Redis fora do intervalo")?,
        ),
        _ => return Err("TTL Redis inválido".into()),
    };
    let confirmation: Option<(String,)> = redis::pipe()
        .atomic()
        .cmd("PING")
        .query_async(connection)
        .await
        .map_err(|e| e.to_string())?;
    if confirmation.is_none() {
        return Err(
            "Chave alterada concorrentemente; exportação cancelada sem arquivo parcial".into(),
        );
    }
    Ok(NoSqlRecord::Redis {
        key: TransferBytes::from_bytes(key.to_vec()),
        value,
        expiry,
    })
}

async fn redis_insert(
    connection: &mut redis::aio::MultiplexedConnection,
    key: &[u8],
    value: &RedisValue,
    expiry: Option<u64>,
) -> Result<bool, String> {
    redis::cmd("WATCH")
        .arg(key)
        .query_async::<()>(connection)
        .await
        .map_err(|e| e.to_string())?;
    let existing: bool = redis::cmd("EXISTS")
        .arg(key)
        .query_async(connection)
        .await
        .map_err(|e| e.to_string())?;
    if existing {
        redis::cmd("UNWATCH")
            .query_async::<()>(connection)
            .await
            .map_err(|e| e.to_string())?;
        return Ok(false);
    }
    let mut pipeline = redis::pipe();
    pipeline.atomic();
    match value {
        RedisValue::String(v) => {
            pipeline.cmd("SET").arg(key).arg(v.decode()?);
        }
        RedisValue::List(values) => {
            pipeline.cmd("RPUSH").arg(key);
            for v in values {
                pipeline.arg(v.decode()?);
            }
        }
        RedisValue::Set(values) => {
            pipeline.cmd("SADD").arg(key);
            for v in values {
                pipeline.arg(v.decode()?);
            }
        }
        RedisValue::Hash(values) => {
            pipeline.cmd("HSET").arg(key);
            for (k, v) in values {
                pipeline.arg(k.decode()?).arg(v.decode()?);
            }
        }
        RedisValue::Zset(values) => {
            pipeline.cmd("ZADD").arg(key);
            for (k, score) in values {
                pipeline.arg(score).arg(k.decode()?);
            }
        }
    }
    if let Some(expiry) = expiry {
        pipeline.cmd("PEXPIREAT").arg(key).arg(expiry);
    }
    // MULTI guarantees isolation, not rollback of commands reporting errors.
    let response: Option<Vec<redis::Value>> = pipeline
        .query_async(connection)
        .await
        .map_err(|e| e.to_string())?;
    Ok(response.is_some())
}

async fn mem_export(
    connection: &mut BufReader<TcpStream>,
    key: &str,
) -> Result<NoSqlRecord, String> {
    let observed_at = model::now_ms()? / 1000 * 1000;
    connection
        .write_all(format!("mg {key} v f c t\r\n").as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    let header = mem_line(connection).await?;
    if header == "EN" {
        return Err("Chave explícita Memcached não existe ou expirou".into());
    }
    let fields: Vec<_> = header.split_whitespace().collect();
    if fields.len() < 2 || fields[0] != "VA" {
        return Err(format!(
            "Servidor Memcached precisa oferecer meta get para exportar TTL: {header}"
        ));
    }
    let size: usize = fields[1]
        .parse()
        .map_err(|_| "Tamanho Memcached inválido")?;
    if size > model::RECORD_BYTES {
        return Err(
            "Valor Memcached excede 64 KiB; conexão encerrada antes de carregar payload".into(),
        );
    }
    let mut flags = None;
    let mut ttl = None;
    for field in &fields[2..] {
        if let Some(v) = field.strip_prefix('f') {
            flags = Some(v.parse::<u32>().map_err(|_| "Flags Memcached inválidas")?);
        } else if let Some(v) = field.strip_prefix('t') {
            ttl = Some(v.parse::<i64>().map_err(|_| "TTL Memcached inválido")?);
        }
    }
    let mut bytes = vec![0; size];
    connection
        .read_exact(&mut bytes)
        .await
        .map_err(|e| e.to_string())?;
    let mut end = [0; 2];
    connection
        .read_exact(&mut end)
        .await
        .map_err(|e| e.to_string())?;
    if end != *b"\r\n" {
        return Err("Payload Memcached inválido".into());
    }
    let expiry = match ttl {
        Some(-1) => TransferExpiry::Never,
        Some(n) if n >= 0 => TransferExpiry::AtUnixMs(
            observed_at
                .checked_add(n as u64 * 1000)
                .ok_or("TTL fora do intervalo")?,
        ),
        None => TransferExpiry::Unknown,
        _ => return Err("TTL Memcached inválido".into()),
    };
    Ok(NoSqlRecord::Memcached {
        key: key.into(),
        value: TransferBytes::from_bytes(bytes),
        flags: flags.ok_or("Flags Memcached ausentes")?,
        expiry,
    })
}

async fn mem_insert(
    connection: &mut BufReader<TcpStream>,
    key: &str,
    value: &[u8],
    flags: u32,
    expiry: Option<u64>,
) -> Result<bool, String> {
    let expires = expiry.map_or(0, |ms| ms / 1000);
    connection
        .write_all(format!("add {key} {flags} {expires} {}\r\n", value.len()).as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    connection
        .write_all(value)
        .await
        .map_err(|e| e.to_string())?;
    connection
        .write_all(b"\r\n")
        .await
        .map_err(|e| e.to_string())?;
    match mem_line(connection).await?.as_str() {
        "STORED" => Ok(true),
        "NOT_STORED" => Ok(false),
        other => Err(format!("Memcached não confirmou criação: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transfer_nosql::{UnknownTtlPolicy, prepare_import};

    fn fixture(engine: Engine, variable: &str) -> ConnectionConfig {
        ConnectionConfig {
            id: "disposable-nosql-transfer".into(),
            name: "Disposable transfer fixture".into(),
            engine,
            host: "127.0.0.1".into(),
            port: std::env::var(variable)
                .unwrap_or_else(|_| panic!("{variable} must point only to a disposable fixture"))
                .parse()
                .unwrap(),
            database: if engine == Engine::Mongo {
                "codisseu_transfer_fixture".into()
            } else {
                "0".into()
            },
            tls: false,
            read_only: false,
            save_credentials: false,
            timeout_secs: 10,
            idle_secs: 60,
            ..Default::default()
        }
    }
    fn prefix() -> String {
        format!(
            "codisseu:{}:{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )
    }
    fn bytes(value: &str) -> TransferBytes {
        TransferBytes::Utf8(value.into())
    }

    #[tokio::test]
    #[ignore = "requires disposable Redis; CODISSEU_TEST_REDIS_PORT"]
    async fn real_redis_typed_binary_ttl_create_only_readonly_and_same_connection() {
        let config = fixture(Engine::Redis, "CODISSEU_TEST_REDIS_PORT");
        let mut session = NoSqlSession::connect(&config, "").await.unwrap();
        let prefix = prefix();
        let names =
            ["string", "list", "hash", "set", "zset"].map(|name| format!("{prefix}:{name}"));
        let values = [
            RedisValue::String(TransferBytes::Hex("00FF0D0A".into())),
            RedisValue::List(vec![
                bytes("same"),
                bytes("same"),
                TransferBytes::Hex("FF".into()),
            ]),
            RedisValue::Hash(vec![(
                TransferBytes::Hex("FF00".into()),
                bytes("9223372036854775807"),
            )]),
            RedisValue::Set(vec![bytes("a"), TransferBytes::Hex("FF".into())]),
            RedisValue::Zset(vec![(bytes("member"), "1.0000000000000002".into())]),
        ];
        let deadline = model::now_ms().unwrap() + 120_000;
        let transfer = NoSqlTransfer {
            version: 1,
            engine: Engine::Redis,
            source_truncated: false,
            records: names
                .iter()
                .zip(values)
                .map(|(key, value)| NoSqlRecord::Redis {
                    key: bytes(key),
                    value,
                    expiry: TransferExpiry::AtUnixMs(deadline),
                })
                .collect(),
        };
        let json = transfer.to_json().unwrap();
        let plan = prepare_import(
            &json,
            Engine::Redis,
            NoSqlImportTarget::RedisKeys,
            UnknownTtlPolicy::Reject,
        )
        .unwrap();
        assert!(
            session
                .preview_import(&config, &plan)
                .await
                .unwrap()
                .conflicts
                .is_empty()
        );
        let progress = Arc::new(AtomicUsize::new(0));
        let mut read_only = config.clone();
        read_only.read_only = true;
        let blocked = session
            .apply_import(&read_only, &plan, progress.clone())
            .await;
        assert_eq!(blocked.applied, 0);
        assert!(blocked.error.unwrap().contains("somente leitura"));
        assert!(
            session
                .preview_import(&config, &plan)
                .await
                .unwrap()
                .conflicts
                .is_empty()
        );
        let report = session.apply_import(&config, &plan, progress.clone()).await;
        assert_eq!(report.applied, 5, "{:?}", report.error);
        assert_eq!(progress.load(Ordering::SeqCst), 5);
        assert!(!report.atomic_group);
        let exported = session
            .export_source(
                &config,
                &NoSqlExportRequest::Keys {
                    keys: names.iter().map(|name| bytes(name)).collect(),
                },
            )
            .await
            .unwrap();
        assert_eq!(exported.records.len(), 5);
        for (index, record) in exported.records.iter().enumerate() {
            let NoSqlRecord::Redis { key, value, expiry } = record else {
                panic!("wrong type")
            };
            assert_eq!(key.decode().unwrap(), names[index].as_bytes());
            let TransferExpiry::AtUnixMs(actual) = expiry else {
                panic!("TTL lost")
            };
            assert!(
                *actual <= deadline && deadline - *actual < 1000,
                "{actual} {deadline}"
            );
            match (index, value) {
                (0, RedisValue::String(v)) => assert_eq!(v.decode().unwrap(), [0, 255, 13, 10]),
                (1, RedisValue::List(v)) => assert_eq!(v.len(), 3),
                (2, RedisValue::Hash(v)) => assert_eq!(v[0].0.decode().unwrap(), [255, 0]),
                (3, RedisValue::Set(v)) => assert_eq!(v.len(), 2),
                (4, RedisValue::Zset(v)) => assert_eq!(v[0].1, "1.0000000000000002"),
                _ => panic!("Redis type changed"),
            }
        }
        let conflicts = session
            .preview_import(&config, &plan)
            .await
            .unwrap()
            .conflicts;
        assert_eq!(conflicts, vec![0, 1, 2, 3, 4]);
        let again = session.apply_import(&config, &plan, progress.clone()).await;
        assert_eq!(again.applied, 0);
        assert!(again.error.unwrap().contains("Conflito"));
        assert_eq!(progress.load(Ordering::SeqCst), 0);
        session.close().await;
    }

    #[tokio::test]
    #[ignore = "requires disposable Memcached; CODISSEU_TEST_MEMCACHED_PORT"]
    async fn real_memcached_binary_flags_meta_ttl_roundtrip_conflict_and_no_enumeration() {
        let config = fixture(Engine::Memcached, "CODISSEU_TEST_MEMCACHED_PORT");
        let mut session = NoSqlSession::connect(&config, "").await.unwrap();
        let prefix = prefix();
        let keys = [format!("{prefix}:binary"), format!("{prefix}:permanent")];
        let deadline = model::now_ms().unwrap() + 120_000;
        let data = NoSqlTransfer {
            version: 1,
            engine: Engine::Memcached,
            source_truncated: false,
            records: vec![
                NoSqlRecord::Memcached {
                    key: keys[0].clone(),
                    value: TransferBytes::Hex("00FF0D0A".into()),
                    flags: u32::MAX,
                    expiry: TransferExpiry::AtUnixMs(deadline),
                },
                NoSqlRecord::Memcached {
                    key: keys[1].clone(),
                    value: bytes(""),
                    flags: 7,
                    expiry: TransferExpiry::Never,
                },
            ],
        };
        let plan = prepare_import(
            &data.to_json().unwrap(),
            Engine::Memcached,
            NoSqlImportTarget::MemcachedKeys,
            UnknownTtlPolicy::Reject,
        )
        .unwrap();
        let progress = Arc::new(AtomicUsize::new(0));
        let mut read_only = config.clone();
        read_only.read_only = true;
        assert_eq!(
            session
                .apply_import(&read_only, &plan, progress.clone())
                .await
                .applied,
            0
        );
        assert!(
            session
                .preview_import(&config, &plan)
                .await
                .unwrap()
                .conflicts
                .is_empty()
        );
        let report = session.apply_import(&config, &plan, progress.clone()).await;
        assert_eq!(report.applied, 2, "{:?}", report.error);
        let exported = session
            .export_source(
                &config,
                &NoSqlExportRequest::Keys {
                    keys: keys.iter().map(|name| bytes(name)).collect(),
                },
            )
            .await
            .unwrap();
        assert_eq!(exported.records.len(), 2);
        let NoSqlRecord::Memcached {
            value,
            flags,
            expiry,
            ..
        } = &exported.records[0]
        else {
            panic!("wrong type")
        };
        assert_eq!(value.decode().unwrap(), [0, 255, 13, 10]);
        assert_eq!(*flags, u32::MAX);
        let TransferExpiry::AtUnixMs(actual) = expiry else {
            panic!("TTL lost")
        };
        assert!(*actual <= deadline + 1000 && deadline.saturating_sub(*actual) < 3000);
        assert!(matches!(
            &exported.records[1],
            NoSqlRecord::Memcached {
                expiry: TransferExpiry::Never,
                ..
            }
        ));
        let again = session.apply_import(&config, &plan, progress).await;
        assert_eq!(again.applied, 0);
        assert!(!again.outcome_unknown);
        assert!(again.error.unwrap().contains("Conflito"));
        assert!(
            session
                .export_source(&config, &NoSqlExportRequest::Keys { keys: vec![] })
                .await
                .unwrap_err()
                .contains("explícitas")
        );
        session.close().await;
    }

    #[tokio::test]
    #[ignore = "requires disposable MongoDB; CODISSEU_TEST_MONGO_PORT"]
    async fn real_mongo_canonical_export_filter_limits_and_partial_prefix_survives_error() {
        let config = fixture(Engine::Mongo, "CODISSEU_TEST_MONGO_PORT");
        let mut session = NoSqlSession::connect(&config, "").await.unwrap();
        let NoSqlSession::Mongo(client) = &session else {
            unreachable!()
        };
        let collection = prefix().replace(':', "_");
        client
            .database(&config.database)
            .create_collection(&collection)
            .validator(doc! {"n":{"$type":"int"}})
            .await
            .unwrap();
        let data = NoSqlTransfer {
            version: 1,
            engine: Engine::Mongo,
            source_truncated: false,
            records: vec![
                NoSqlRecord::Mongo {
                    document: serde_json::json!({"_id":{"$oid":"0123456789abcdef01234567"},"n":1,"big":{"$numberLong":"9223372036854775807"},"binary":{"$binary":{"base64":"AP8=","subType":"00"}}}),
                },
                NoSqlRecord::Mongo {
                    document: serde_json::json!({"_id":"rejected","n":"invalid"}),
                },
            ],
        };
        let plan = prepare_import(
            &data.to_json().unwrap(),
            Engine::Mongo,
            NoSqlImportTarget::MongoCollection(collection.clone()),
            UnknownTtlPolicy::Reject,
        )
        .unwrap();
        let progress = Arc::new(AtomicUsize::new(0));
        assert!(
            session
                .preview_import(&config, &plan)
                .await
                .unwrap()
                .conflicts
                .is_empty()
        );
        let report = session.apply_import(&config, &plan, progress.clone()).await;
        assert_eq!(report.applied, 1, "{:?}", report.error);
        assert_eq!(report.failed_index, Some(1));
        assert!(!report.outcome_unknown);
        assert_eq!(progress.load(Ordering::SeqCst), 1);
        assert!(report.error.unwrap().contains("permanecem aplicados"));
        let exported = session
            .export_source(
                &config,
                &NoSqlExportRequest::Mongo {
                    collection: collection.clone(),
                    filter: serde_json::json!({"n":1}),
                    limit: 10,
                },
            )
            .await
            .unwrap();
        assert_eq!(exported.records.len(), 1);
        assert!(!exported.source_truncated);
        let json = exported.to_json().unwrap();
        assert!(json.contains("9223372036854775807"));
        assert!(json.contains("AP8="));
        let conflicts = session
            .preview_import(&config, &plan)
            .await
            .unwrap()
            .conflicts;
        assert_eq!(conflicts, vec![0]);
        session.close().await;
    }
}
