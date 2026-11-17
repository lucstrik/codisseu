//! Independently implemented, deliberately read-only NoSQL consoles.
//! Mongo uses the official driver, Redis uses redis-rs, Memcached implements
//! the documented basic text protocol with bounded headers and value lengths.
#[cfg(test)]
mod mongo_tests;
mod transfer;
use crate::remote::{
    Cell, ConnectionConfig, DbObject, Engine, MongoAuthentication, MongoReadPreference,
    RemoteResult,
};
use crate::{CELL_BYTE_LIMIT, QUERY_LIMIT, RESULT_BYTE_LIMIT, SQL_BYTE_LIMIT};
use mongodb::{
    Client,
    bson::{Bson, Document, doc},
    options::{
        AuthMechanism, ClientOptions, Credential, ReadPreference, ServerAddress, Tls, TlsOptions,
    },
};
use std::time::{Duration, Instant};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpStream,
};

pub(crate) enum NoSqlSession {
    Mongo(MongoSession),
    Redis(redis::aio::MultiplexedConnection),
    Memcached(Option<BufReader<TcpStream>>),
}

pub(crate) struct MongoSession {
    client: Client,
    // The official driver's TLS connector reads this for each new socket.
    _ca_file: Option<tempfile::NamedTempFile>,
}
impl std::ops::Deref for MongoSession {
    type Target = Client;
    fn deref(&self) -> &Client {
        &self.client
    }
}

fn timeout(config: &ConnectionConfig) -> Duration {
    Duration::from_secs(config.timeout_secs.clamp(1, 120))
}

fn row_limit(config: &ConnectionConfig) -> usize {
    config.row_limit.clamp(1, QUERY_LIMIT)
}

fn result(columns: &[&str]) -> RemoteResult {
    RemoteResult {
        columns: columns.iter().map(|s| (*s).to_owned()).collect(),
        rows: Vec::new(),
        truncated: false,
        elapsed_ms: 0,
        affected_rows: 0,
    }
}

fn cell_bytes(cell: &Cell) -> usize {
    match cell {
        Cell::Text(s) | Cell::Number(s) | Cell::Json(s) => s.len(),
        Cell::Binary(bytes) => bytes.len(),
        Cell::Null | Cell::Bool(_) => 1,
    }
}

fn push_row(out: &mut RemoteResult, row: Vec<Cell>, limit: usize, bytes: &mut usize) -> bool {
    let size = row.iter().map(cell_bytes).sum::<usize>();
    if out.rows.len() >= limit
        || row.iter().any(|c| cell_bytes(c) > CELL_BYTE_LIMIT)
        || size.saturating_add(*bytes) > RESULT_BYTE_LIMIT
    {
        out.truncated = true;
        return false;
    }
    *bytes += size;
    out.rows.push(row);
    true
}

fn mongo_options(config: &ConnectionConfig, password: &str) -> Result<ClientOptions, String> {
    if config.database.is_empty() {
        return Err("Informe um banco MongoDB explícito".into());
    }
    let deadline = timeout(config);
    let mut options = ClientOptions::default();
    options.hosts = vec![ServerAddress::Tcp {
        host: config.host.clone(),
        port: Some(config.port),
    }];
    options.app_name = Some("Codisseu".into());
    // One application connection per discovered server; SDAM uses extra sockets.
    // MongoDB also opens SDAM monitoring sockets; this is not a
    // promise of one total server-side socket.
    config.mongo.validate()?;
    options.direct_connection = Some(config.mongo.direct_connection);
    options.repl_set_name =
        (!config.mongo.replica_set.is_empty()).then(|| config.mongo.replica_set.clone());
    let preference = match config.mongo.read_preference {
        MongoReadPreference::Primary => ReadPreference::Primary,
        MongoReadPreference::PrimaryPreferred => ReadPreference::PrimaryPreferred { options: None },
        MongoReadPreference::Secondary => ReadPreference::Secondary { options: None },
        MongoReadPreference::SecondaryPreferred => {
            ReadPreference::SecondaryPreferred { options: None }
        }
        MongoReadPreference::Nearest => ReadPreference::Nearest { options: None },
    };
    options.selection_criteria = Some(preference.into());
    options.max_pool_size = Some(1);
    options.min_pool_size = Some(0);
    options.max_connecting = Some(1);
    options.max_idle_time = Some(Duration::from_secs(config.idle_secs.clamp(5, 300)));
    options.connect_timeout = Some(deadline);
    options.server_selection_timeout = Some(deadline);
    options.retry_reads = Some(false);
    options.retry_writes = Some(false);
    if !config.username.is_empty() || !password.is_empty() {
        options.credential = Some(
            Credential::builder()
                .username(config.username.clone())
                .password(password.to_owned())
                .mechanism(match config.mongo.authentication {
                    MongoAuthentication::Automatic => None,
                    MongoAuthentication::ScramSha1 => Some(AuthMechanism::ScramSha1),
                    MongoAuthentication::ScramSha256 => Some(AuthMechanism::ScramSha256),
                })
                .source(if config.auth_database.is_empty() {
                    config.database.clone()
                } else {
                    config.auth_database.clone()
                })
                .build(),
        );
    }
    if config.tls {
        let mut tls = TlsOptions::default();
        if !config.ca_file.is_empty() {
            tls.ca_file_path = Some(config.ca_file.clone().into());
        }
        options.tls = Some(Tls::Enabled(tls));
    }
    Ok(options)
}

impl NoSqlSession {
    pub(crate) async fn connect(config: &ConnectionConfig, password: &str) -> Result<Self, String> {
        let deadline = timeout(config);
        if (!config.ca_file.is_empty() || !config.ca_pem.is_empty()) && !config.tls {
            return Err("Arquivo CA exige TLS ativado".into());
        }
        tokio::time::timeout(deadline, async {
            match config.engine {
                Engine::Mongo => {
                    let mut options = mongo_options(config, password)?;
                    let ca = crate::tls_roots::material(config)?.map(|pem| crate::tls_roots::temporary_pem(&pem)).transpose()?;
                    if let Some(ca) = &ca && let Some(Tls::Enabled(tls)) = &mut options.tls {
                        tls.ca_file_path = Some(ca.path().into());
                    }
                    let client = Client::with_options(options).map_err(|e| e.to_string())?;
                    if let Err(error) = client.database(&config.database).run_command(doc! { "ping": 1 }).await {
                        client.shutdown().await;
                        return Err(error.to_string());
                    }
                    Ok(Self::Mongo(MongoSession {client, _ca_file:ca}))
                }
                Engine::Redis => {
                    let database = if config.database.is_empty() {
                        0
                    } else {
                        config.database.parse::<i64>().map_err(|_| {
                            "Banco Redis deve ser um número inteiro não negativo".to_owned()
                        })?
                    };
                    if database < 0 {
                        return Err("Banco Redis deve ser não negativo".into());
                    }
                    let info = redis::ConnectionInfo {
                        addr: if config.tls {
                            redis::ConnectionAddr::TcpTls {
                                host: config.host.clone(),
                                port: config.port,
                                insecure: false,
                                tls_params: None,
                            }
                        } else {
                            redis::ConnectionAddr::Tcp(config.host.clone(), config.port)
                        },
                        redis: redis::RedisConnectionInfo {
                            db: database,
                            username: (!config.username.is_empty()).then(|| config.username.clone()),
                            password: (!password.is_empty()).then(|| password.to_owned()),
                            ..Default::default()
                        },
                    };
                    let client = if let Some(pem)=crate::tls_roots::material(config)? {
                        redis::Client::build_with_tls(info,redis::TlsCertificates{client_tls:None,root_cert:Some(pem.to_vec())})
                    } else { redis::Client::open(info) }.map_err(|e|e.to_string())?;
                    let settings = redis::AsyncConnectionConfig::new()
                        .set_connection_timeout(deadline)
                        .set_response_timeout(deadline);
                    let mut connection = client
                        .get_multiplexed_async_connection_with_config(&settings)
                        .await
                        .map_err(|e| e.to_string())?;
                    redis::cmd("PING")
                        .query_async::<String>(&mut connection)
                        .await
                        .map_err(|e| e.to_string())?;
                    Ok(Self::Redis(connection))
                }
                Engine::Memcached => {
                    if config.tls || !config.username.is_empty() || !password.is_empty() {
                        return Err("Memcached: TLS e SASL ainda não são suportados; nenhuma conexão foi aberta".into());
                    }
                    let socket = TcpStream::connect((config.host.as_str(), config.port))
                        .await
                        .map_err(|e| e.to_string())?;
                    socket.set_nodelay(true).map_err(|e| e.to_string())?;
                    let mut stream = BufReader::new(socket);
                    stream
                        .write_all(b"version\r\n")
                        .await
                        .map_err(|e| e.to_string())?;
                    if !mem_line(&mut stream).await?.starts_with("VERSION ") {
                        return Err("Resposta Memcached inválida".into());
                    }
                    Ok(Self::Memcached(Some(stream)))
                }
                _ => Err("Driver não é NoSQL".into()),
            }
        })
        .await
        .map_err(|_| "Tempo limite ao conectar NoSQL".to_owned())?
    }

    pub(crate) async fn schema(
        &mut self,
        config: &ConnectionConfig,
    ) -> Result<Vec<DbObject>, String> {
        let Self::Mongo(client) = self else {
            // Key discovery must always be an explicit console command.
            return Ok(Vec::new());
        };
        tokio::time::timeout(timeout(config), async {
            let mut cursor = client
                .database(&config.database)
                .list_collections()
                .batch_size(32)
                .await
                .map_err(|e| e.to_string())?;
            let mut objects = Vec::new();
            while cursor.advance().await.map_err(|e| e.to_string())? {
                if objects.len() == 256 {
                    return Err("MongoDB: mais de 256 coleções; introspecção interrompida".into());
                }
                let collection = cursor.deserialize_current().map_err(|e| e.to_string())?;
                objects.push(DbObject {
                    namespace: config.database.clone(),
                    name: collection.name,
                    kind: "collection".into(),
                    columns: Vec::new(),
                });
            }
            Ok(objects)
        })
        .await
        .map_err(|_| "Tempo limite na introspecção MongoDB".to_owned())?
    }

    pub(crate) async fn execute(
        &mut self,
        query: &str,
        config: &ConnectionConfig,
    ) -> Result<RemoteResult, String> {
        if query.len() > SQL_BYTE_LIMIT {
            return Err("Comando excede 256 KiB".into());
        }
        let started = Instant::now();
        let operation = async {
            match self {
                Self::Mongo(client) => mongo_read(client, query, config).await,
                Self::Redis(connection) => redis_read(connection, query, config).await,
                Self::Memcached(stream) => {
                    // A truncated/protocol-error response cannot be reused.
                    let mut connection =
                        stream.take().ok_or("Memcached desconectado; reconecte")?;
                    let output = mem_read(&mut connection, query, config).await;
                    if output.is_ok() {
                        *stream = Some(connection);
                    }
                    output
                }
            }
        };
        let mut output = tokio::time::timeout(timeout(config), operation)
            .await
            .map_err(|_| {
                "Tempo limite NoSQL; desconecte a sessão antes de tentar novamente".to_owned()
            })??;
        output.elapsed_ms = started.elapsed().as_millis();
        Ok(output)
    }

    pub(crate) async fn close(self) {
        if let Self::Mongo(session) = self {
            session.client.shutdown().await;
        }
        // redis-rs aborts its driver task on last connection drop; TcpStream
        // closes on drop. No connection-manager or reconnect loop is used.
    }
}

use crate::mongo_shell::MongoRead;

fn mongo_request(query: &str, limit: usize) -> Result<MongoRead, String> {
    let request: MongoRead = if query.trim_start().starts_with('{') {
        serde_json::from_str(query)
            .map_err(|e| format!("MongoDB: JSON de consulta inválido: {e}"))?
    } else {
        crate::mongo_shell::parse(query)?
    };
    if request.collection.is_empty()
        || request.collection.len() > 255
        || request.collection.contains('\0')
        || request.collection.starts_with("system.")
    {
        return Err("Coleção inválida ou reservada".into());
    }
    if request.database.as_ref().is_some_and(|db| {
        db.is_empty()
            || db.len() > 63
            || db.contains([
                '\0', '/', '\\', '.', ' ', '\"', '$', '*', '<', '>', ':', '|', '?',
            ])
    }) {
        return Err("MongoDB: nome de database inválido".into());
    }
    if request.limit.is_some_and(|n| n == 0 || n > limit) {
        return Err(format!("MongoDB: limit deve estar entre 1 e {limit}"));
    }
    fn has_code(value: &Bson) -> bool {
        match value {
            Bson::Document(document) => document.iter().any(|(key, value)| {
                matches!(
                    key.as_str(),
                    "$where" | "$function" | "$accumulator" | "$out" | "$merge"
                ) || has_code(value)
            }),
            Bson::Array(items) => items.iter().any(has_code),
            Bson::JavaScriptCode(_) | Bson::JavaScriptCodeWithScope(_) => true,
            _ => false,
        }
    }
    let documents = std::iter::once(&request.filter)
        .chain(request.projection.iter())
        .chain(request.sort.iter())
        .chain(request.pipeline.iter().flatten());
    if documents.cloned().any(|d| has_code(&Bson::Document(d))) {
        return Err(
            "MongoDB: somente leitura; $out/$merge e JavaScript no servidor não são permitidos"
                .into(),
        );
    }
    if let Some(pipeline) = &request.pipeline {
        if !request.filter.is_empty()
            || request.projection.is_some()
            || request.sort.is_some()
            || request.limit.is_some()
        {
            return Err(
                "MongoDB: pipeline não pode ser combinado com filter/projection/sort/limit de find"
                    .into(),
            );
        }
        if pipeline.len() > 256
            || pipeline
                .iter()
                .any(|stage| stage.len() != 1 || !stage.keys().all(|key| key.starts_with('$')))
        {
            return Err("MongoDB: pipeline exige até 256 estágios, cada um com um operador".into());
        }
    }
    Ok(request)
}

fn mongo_error(error: mongodb::error::Error) -> String {
    // Error::Display includes raw wire-response bytes. The typed kind carries
    // the useful server code/message without that implementation detail.
    format!("MongoDB: {}", error.kind)
}

async fn mongo_read(
    client: &Client,
    query: &str,
    config: &ConnectionConfig,
) -> Result<RemoteResult, String> {
    let request = mongo_request(query, row_limit(config))?;
    let limit = request.limit.unwrap_or(row_limit(config));
    let collection = client
        .database(request.database.as_deref().unwrap_or(&config.database))
        .collection::<Document>(&request.collection);
    let mut cursor = if let Some(mut pipeline) = request.pipeline {
        // Limit only the final output, after grouping/sorting; never alter input stages.
        pipeline.push(doc! {"$limit": (limit + 1) as i64});
        collection
            .aggregate(pipeline)
            .batch_size(16)
            .max_time(timeout(config))
            .await
            .map_err(mongo_error)?
    } else {
        let mut find = collection
            .find(request.filter)
            .limit((limit + 1) as i64)
            .batch_size(16)
            .max_time(timeout(config));
        if let Some(projection) = request.projection {
            find = find.projection(projection);
        }
        if let Some(sort) = request.sort {
            find = find.sort(sort);
        }
        find.await.map_err(mongo_error)?
    };
    let mut output = result(&["_id", "document"]);
    let mut bytes = 0;
    while cursor.advance().await.map_err(mongo_error)? {
        let document = cursor.deserialize_current().map_err(mongo_error)?;
        let id = document.get("_id").cloned().unwrap_or(Bson::Null);
        let id = if id == Bson::Null {
            Cell::Null
        } else {
            Cell::Json(id.into_canonical_extjson().to_string())
        };
        let document = Bson::Document(document)
            .into_canonical_extjson()
            .to_string();
        if !push_row(
            &mut output,
            vec![id, Cell::Json(document)],
            limit,
            &mut bytes,
        ) {
            break;
        }
    }
    Ok(output)
}

/// Shell-like tokenization without shell expansion, pipelines or extra commands.
fn tokens(query: &str) -> Result<Vec<String>, String> {
    let mut result = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    let mut escaped = false;
    let mut active = false;
    for ch in query.chars() {
        if escaped {
            word.push(match ch {
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                other => other,
            });
            escaped = false;
            active = true;
        } else if ch == '\\' {
            escaped = true;
            active = true;
        } else if quote == Some(ch) {
            quote = None;
        } else if quote.is_some() {
            word.push(ch);
        } else if ch == '\'' || ch == '"' {
            quote = Some(ch);
            active = true;
        } else if ch.is_whitespace() {
            if active {
                result.push(std::mem::take(&mut word));
                active = false;
            }
        } else {
            word.push(ch);
            active = true;
        }
        if result.len() > QUERY_LIMIT {
            return Err("Número de argumentos excede 500".into());
        }
    }
    if escaped || quote.is_some() {
        return Err("Aspas ou escape incompleto".into());
    }
    if active {
        result.push(word);
    }
    if result.is_empty() {
        return Err("Informe um comando".into());
    }
    Ok(result)
}

fn redis_request(query: &str, limit: usize) -> Result<Vec<String>, String> {
    let mut args = tokens(query)?;
    args[0] = args[0].to_ascii_uppercase();
    let command = args[0].as_str();
    let arity = args.len();
    let valid = match command {
        "PING" | "DBSIZE" => arity == 1,
        "TYPE" | "TTL" | "PTTL" | "STRLEN" | "LLEN" | "HLEN" | "SCARD" | "ZCARD" | "GET" => arity == 2,
        "EXISTS" | "MGET" => arity >= 2 && arity <= limit + 1,
        "GETRANGE" | "LRANGE" | "ZRANGE" => {
            if arity != 4 { false } else {
                let start = args[2].parse::<usize>().map_err(|_| "Intervalo deve ser não negativo")?;
                let end = args[3].parse::<usize>().map_err(|_| "Intervalo deve ser não negativo e limitado")?;
                let bound = if command == "GETRANGE" { CELL_BYTE_LIMIT } else { limit };
                end >= start && end - start < bound
            }
        }
        "SCAN" | "HSCAN" | "SSCAN" | "ZSCAN" => {
            let cursor_index = if command == "SCAN" { 1 } else { 2 };
            if arity < cursor_index + 1 { return Err("Informe cursor explícito e COUNT limitado".into()); }
            args[cursor_index].parse::<u64>().map_err(|_| "Cursor inválido")?;
            let mut index = cursor_index + 1;
            let mut count = false;
            let mut seen_match = false;
            while index < arity {
                let option = args[index].to_ascii_uppercase();
                if index + 1 >= arity { return Err("Opção sem valor".into()); }
                match option.as_str() {
                    "COUNT" if !count => {
                        let n = args[index+1].parse::<usize>().map_err(|_| "COUNT inválido")?;
                        if n == 0 || n > limit { return Err(format!("COUNT deve estar entre 1 e {limit}")); }
                        count = true;
                    }
                    "MATCH" if !seen_match => seen_match = true,
                    _ => return Err("Somente MATCH e COUNT são aceitos no SCAN".into()),
                }
                args[index] = option;
                index += 2;
            }
            count
        }
        "HGETALL" => return Err("HGETALL é ilimitado; use HSCAN chave 0 COUNT 100 e continue pelo cursor retornado".into()),
        _ => return Err("Comando bloqueado. Console Redis somente leitura: GET/MGET, TYPE, TTL/PTTL, EXISTS, STRLEN, LLEN/HLEN/SCARD/ZCARD, GETRANGE, LRANGE/ZRANGE e SCAN explícito com COUNT. KEYS, scripts e escritas não são executados".into()),
    };
    if !valid {
        return Err("Argumentos inválidos ou intervalo excede o limite configurado".into());
    }
    Ok(args)
}

fn redis_cell(value: redis::Value) -> Result<Cell, String> {
    Ok(match value {
        redis::Value::Nil => Cell::Null,
        redis::Value::Int(v) => Cell::Number(v.to_string()),
        redis::Value::BulkString(bytes) => match String::from_utf8(bytes) {
            Ok(text) => Cell::Text(text),
            Err(error) => Cell::Binary(error.into_bytes()),
        },
        redis::Value::SimpleString(text) => Cell::Text(text),
        redis::Value::Okay => Cell::Text("OK".into()),
        redis::Value::Boolean(v) => Cell::Bool(v),
        _ => return Err("Formato Redis inesperado para o comando de leitura".into()),
    })
}

async fn redis_get(
    connection: &mut redis::aio::MultiplexedConnection,
    key: &str,
) -> Result<(Cell, bool), String> {
    let exists: bool = redis::cmd("EXISTS")
        .arg(key)
        .query_async(connection)
        .await
        .map_err(|e| e.to_string())?;
    if !exists {
        return Ok((Cell::Null, false));
    }
    let length: usize = redis::cmd("STRLEN")
        .arg(key)
        .query_async(connection)
        .await
        .map_err(|e| e.to_string())?;
    let value: redis::Value = redis::cmd("GETRANGE")
        .arg(key)
        .arg(0)
        .arg(CELL_BYTE_LIMIT - 1)
        .query_async(connection)
        .await
        .map_err(|e| e.to_string())?;
    Ok((redis_cell(value)?, length > CELL_BYTE_LIMIT))
}

async fn redis_read(
    connection: &mut redis::aio::MultiplexedConnection,
    query: &str,
    config: &ConnectionConfig,
) -> Result<RemoteResult, String> {
    let args = redis_request(query, row_limit(config))?;
    let mut output = result(&["value"]);
    let mut bytes = 0;
    if args[0] == "GET" || args[0] == "MGET" {
        output.columns = vec!["key".into(), "value".into()];
        for key in &args[1..] {
            let (value, truncated) = redis_get(connection, key).await?;
            output.truncated |= truncated;
            if !push_row(
                &mut output,
                vec![Cell::Text(key.clone()), value],
                row_limit(config),
                &mut bytes,
            ) {
                break;
            }
        }
        return Ok(output);
    }
    let value: redis::Value = redis::cmd(&args[0])
        .arg(&args[1..])
        .query_async(connection)
        .await
        .map_err(|e| e.to_string())?;
    if args[0].ends_with("SCAN") {
        let redis::Value::Array(mut parts) = value else {
            return Err("Cursor Redis inválido".into());
        };
        if parts.len() != 2 {
            return Err("Resposta SCAN inválida".into());
        }
        let values = parts.pop().unwrap();
        let cursor = redis_cell(parts.pop().unwrap())?;
        let redis::Value::Array(values) = values else {
            return Err("Página SCAN inválida".into());
        };
        output.columns = vec!["cursor".into(), "value".into()];
        let pairs = args[0] == "HSCAN" || args[0] == "ZSCAN";
        if pairs {
            output.columns.push("field value / score".into());
        }
        let mut values = values.into_iter();
        while let Some(value) = values.next() {
            let mut row = vec![cursor.clone(), redis_cell(value)?];
            if pairs {
                row.push(redis_cell(values.next().ok_or("Par SCAN incompleto")?)?);
            }
            // COUNT is a server hint, so never silently discard a cursor page:
            // report an explicit error when it cannot be shown in full.
            if !push_row(&mut output, row, row_limit(config), &mut bytes) {
                return Err("Página SCAN excede o limite de exibição; COUNT é sugestão do Redis. Reduza COUNT ou consulte uma chave explícita; nenhum cursor foi avançado automaticamente".into());
            }
        }
        if output.rows.is_empty() {
            let mut row = vec![cursor, Cell::Null];
            if pairs {
                row.push(Cell::Null);
            }
            output.rows.push(row);
        }
    } else if let redis::Value::Array(values) = value {
        for value in values {
            if !push_row(
                &mut output,
                vec![redis_cell(value)?],
                row_limit(config),
                &mut bytes,
            ) {
                break;
            }
        }
    } else {
        push_row(
            &mut output,
            vec![redis_cell(value)?],
            row_limit(config),
            &mut bytes,
        );
    }
    Ok(output)
}

async fn mem_line(stream: &mut BufReader<TcpStream>) -> Result<String, String> {
    let mut line = Vec::new();
    loop {
        if line.len() >= 8192 {
            return Err("Cabeçalho Memcached excede 8 KiB; conexão encerrada".into());
        }
        let byte = stream.read_u8().await.map_err(|e| e.to_string())?;
        line.push(byte);
        if byte == b'\n' {
            break;
        }
    }
    if !line.ends_with(b"\r\n") {
        return Err("Terminador Memcached inválido".into());
    }
    line.truncate(line.len() - 2);
    String::from_utf8(line).map_err(|_| "Cabeçalho Memcached inválido".into())
}

fn mem_request(query: &str, limit: usize) -> Result<Vec<String>, String> {
    let mut args = tokens(query)?;
    args[0] = args[0].to_ascii_lowercase();
    match args[0].as_str() {
        "version" | "stats" if args.len() == 1 => Ok(args),
        "get" | "gets" if args.len() > 1 && args.len() <= limit + 1 => {
            if args[1..].iter().any(|key| key.is_empty() || key.len() > 250 || key.bytes().any(|b| b <= b' ' || b == 127)) {
                return Err("Chave Memcached deve ter 1–250 bytes, sem espaços/controles".into());
            }
            Ok(args)
        }
        _ => Err("Memcached: use get/gets com chaves explícitas, stats ou version. Escritas, cachedump, stats sizes e varreduras são bloqueados".into()),
    }
}

async fn mem_read(
    stream: &mut BufReader<TcpStream>,
    query: &str,
    config: &ConnectionConfig,
) -> Result<RemoteResult, String> {
    let args = mem_request(query, row_limit(config))?;
    stream
        .write_all(format!("{}\r\n", args.join(" ")).as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    let mut output = if args[0] == "stats" {
        result(&["name", "value"])
    } else if args[0] == "version" {
        result(&["version"])
    } else {
        result(&["key", "value", "flags", "cas"])
    };
    let mut bytes = 0;
    loop {
        let line = mem_line(stream).await?;
        if line == "END" {
            break;
        }
        if let Some(version) = line.strip_prefix("VERSION ") {
            push_row(
                &mut output,
                vec![Cell::Text(version.into())],
                row_limit(config),
                &mut bytes,
            );
            break;
        }
        if let Some(stat) = line.strip_prefix("STAT ") {
            let (name, value) = stat.split_once(' ').ok_or("STAT Memcached inválido")?;
            if !push_row(
                &mut output,
                vec![Cell::Text(name.into()), Cell::Text(value.into())],
                row_limit(config),
                &mut bytes,
            ) {
                return Err("Estatísticas Memcached excedem o limite; conexão encerrada".into());
            }
            continue;
        }
        if line.starts_with("VALUE ") {
            let fields: Vec<_> = line.split_whitespace().collect();
            if fields.len() != 4 && fields.len() != 5 {
                return Err("VALUE Memcached inválido".into());
            }
            fields[2]
                .parse::<u32>()
                .map_err(|_| "Flags Memcached inválidas")?;
            if let Some(cas) = fields.get(4) {
                cas.parse::<u64>().map_err(|_| "CAS Memcached inválido")?;
            }
            let length = fields[3]
                .parse::<usize>()
                .map_err(|_| "Tamanho Memcached inválido")?;
            if length > CELL_BYTE_LIMIT || length.saturating_add(bytes) > RESULT_BYTE_LIMIT {
                return Err("Valor Memcached excede 64 KiB/resultado 4 MiB; conexão encerrada sem carregar o valor".into());
            }
            let mut value = vec![0; length];
            stream
                .read_exact(&mut value)
                .await
                .map_err(|e| e.to_string())?;
            let mut end = [0; 2];
            stream
                .read_exact(&mut end)
                .await
                .map_err(|e| e.to_string())?;
            if end != *b"\r\n" {
                return Err("Payload Memcached inválido".into());
            }
            let cell = match String::from_utf8(value) {
                Ok(s) => Cell::Text(s),
                Err(e) => Cell::Binary(e.into_bytes()),
            };
            if !push_row(
                &mut output,
                vec![
                    Cell::Text(fields[1].into()),
                    cell,
                    Cell::Number(fields[2].into()),
                    fields
                        .get(4)
                        .map_or(Cell::Null, |s| Cell::Number((*s).into())),
                ],
                row_limit(config),
                &mut bytes,
            ) {
                return Err("Resposta Memcached excede o limite; conexão encerrada".into());
            }
        } else {
            return Err(format!("Memcached: {line}"));
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn custom_ca_cannot_silently_disable_tls() {
        let config = ConnectionConfig {
            engine: Engine::Redis,
            host: "127.0.0.1".into(),
            port: 1,
            tls: false,
            ca_file: "/nonexistent/fixture-ca.pem".into(),
            ..Default::default()
        };
        let error = NoSqlSession::connect(&config, "").await.err().unwrap();
        assert!(error.contains("CA exige TLS"));
    }

    #[test]
    fn redis_allowlist_rejects_mutations_unbounded_and_incomplete_commands() {
        for query in [
            "KEYS *",
            "SET key value",
            "FLUSHALL",
            "EVAL 'return 1' 0",
            "HGETALL huge",
            "SCAN 0",
            "SCAN 0 COUNT 501",
            "LRANGE list 0 -1",
            "GET a\nSET b value",
            "GET 'bad",
        ] {
            assert!(redis_request(query, 500).is_err(), "{query}");
        }
        assert_eq!(redis_request("GET 'a b'", 500).unwrap(), ["GET", "a b"]);
        assert!(redis_request("SCAN 0 MATCH prefix:* COUNT 20", 100).is_ok());
        assert!(redis_request("HSCAN h 0 COUNT 20", 100).is_ok());
        assert!(redis_request("LRANGE list 0 99", 100).is_ok());
    }

    #[test]
    fn mongo_console_is_structured_bounded_and_cannot_run_server_code() {
        assert!(
            mongo_request(
                r#"{"collection":"users","filter":{"active":true},"limit":5}"#,
                100
            )
            .is_ok()
        );
        for query in [
            r#"{"collection":"users","limit":0}"#,
            r#"{"collection":"users","limit":101}"#,
            r#"{"collection":"users","filter":{"$where":"sleep(1000)"}}"#,
            r#"{"collection":"users","update":{}}"#,
            r#"{"collection":"system.users"}"#,
        ] {
            assert!(mongo_request(query, 100).is_err(), "{query}");
        }
    }

    #[test]
    fn memcached_blocks_protocol_injection_and_expensive_discovery() {
        for query in [
            "get 'key\\r\\nflush_all'",
            "stats sizes",
            "stats cachedump 1 0",
            "flush_all",
            "set k 0 0 1",
            "get ''",
        ] {
            assert!(mem_request(query, 100).is_err(), "{query}");
        }
        assert!(mem_request("gets first second", 100).is_ok());
        assert!(mem_request("stats", 100).is_ok());
    }
}
