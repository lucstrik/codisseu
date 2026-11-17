use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

pub type Variables = BTreeMap<String, String>;
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
pub fn new_id() -> String {
    format!(
        "{:x}-{:x}-{:x}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    )
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct KeyValue {
    pub file_path: Option<std::path::PathBuf>,
    pub enabled: bool,
    pub key: String,
    pub value: String,
    pub description: String,
    pub secret: bool,
}
impl Default for KeyValue {
    fn default() -> Self {
        Self {
            file_path: None,
            enabled: true,
            key: String::new(),
            value: String::new(),
            description: String::new(),
            secret: false,
        }
    }
}
impl KeyValue {
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
            ..Self::default()
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum Auth {
    #[default]
    Inherit,
    None,
    Bearer {
        token: String,
    },
    Basic {
        username: String,
        password: String,
    },
    ApiKey {
        key: String,
        value: String,
        in_query: bool,
    },
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum RequestBody {
    #[default]
    None,
    Raw {
        text: String,
        content_type: String,
    },
    UrlEncoded(Vec<KeyValue>),
    FormData(Vec<KeyValue>),
    /// Binary data is stored in memory; importers never read local paths implicitly.
    Binary(Vec<u8>),
    BinaryFile {
        path: std::path::PathBuf,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct RequestSettings {
    pub timeout_ms: u64,
    pub follow_redirects: bool,
    pub verify_tls: bool,
    pub use_cookies: bool,
    pub max_response_bytes: usize,
}
impl Default for RequestSettings {
    fn default() -> Self {
        Self {
            timeout_ms: 30_000,
            follow_redirects: true,
            verify_tls: true,
            use_cookies: true,
            max_response_bytes: 4 * 1024 * 1024,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ApiRequest {
    #[serde(skip)]
    pub upload_root: Option<std::path::PathBuf>,
    pub id: String,
    pub name: String,
    pub description: String,
    pub method: String,
    pub url: String,
    pub params: Vec<KeyValue>,
    pub path_variables: Vec<KeyValue>,
    pub headers: Vec<KeyValue>,
    pub auth: Auth,
    pub body: RequestBody,
    pub pre_request_script: String,
    pub post_response_script: String,
    pub settings: RequestSettings,
}
impl Default for ApiRequest {
    fn default() -> Self {
        Self {
            upload_root: None,
            id: new_id(),
            name: "New Request".into(),
            description: String::new(),
            method: "GET".into(),
            url: String::new(),
            params: vec![],
            path_variables: vec![],
            headers: vec![],
            auth: Auth::default(),
            body: RequestBody::None,
            pre_request_script: String::new(),
            post_response_script: String::new(),
            settings: RequestSettings::default(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum CollectionItem {
    Request(Box<ApiRequest>),
    Folder {
        id: String,
        name: String,
        items: Vec<CollectionItem>,
        auth: Auth,
        pre_request_script: String,
        post_response_script: String,
    },
}
impl CollectionItem {
    pub fn id(&self) -> &str {
        match self {
            Self::Request(r) => &r.id,
            Self::Folder { id, .. } => id,
        }
    }
    pub fn name(&self) -> &str {
        match self {
            Self::Request(r) => &r.name,
            Self::Folder { name, .. } => name,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Collection {
    pub id: String,
    pub name: String,
    pub description: String,
    pub items: Vec<CollectionItem>,
    pub variables: Vec<KeyValue>,
    pub auth: Auth,
    pub pre_request_script: String,
    pub post_response_script: String,
}
impl Default for Collection {
    fn default() -> Self {
        Self {
            id: new_id(),
            name: "New Collection".into(),
            description: String::new(),
            items: vec![],
            variables: vec![],
            auth: Auth::None,
            pre_request_script: String::new(),
            post_response_script: String::new(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Environment {
    pub id: String,
    pub name: String,
    pub variables: Vec<KeyValue>,
}
impl Default for Environment {
    fn default() -> Self {
        Self {
            id: new_id(),
            name: "New Environment".into(),
            variables: vec![],
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApiSpec {
    pub id: String,
    pub name: String,
    pub source: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MockRoute {
    pub method: String,
    pub path: String,
    pub status: u16,
    pub headers: Vec<KeyValue>,
    pub body: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MockDefinition {
    pub id: String,
    pub name: String,
    pub routes: Vec<MockRoute>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ApiWorkspace {
    pub schema_version: u32,
    pub collections: Vec<Collection>,
    pub globals: Vec<KeyValue>,
    pub environments: Vec<Environment>,
    pub active_environment: Option<String>,
    pub specs: Vec<ApiSpec>,
    pub mocks: Vec<MockDefinition>,
}
impl Default for ApiWorkspace {
    fn default() -> Self {
        Self {
            schema_version: 1,
            collections: vec![],
            globals: vec![],
            environments: vec![],
            active_environment: None,
            specs: vec![],
            mocks: vec![],
        }
    }
}
fn find<'a>(items: &'a [CollectionItem], id: &str) -> Option<&'a ApiRequest> {
    items.iter().find_map(|item| match item {
        CollectionItem::Request(r) if r.id == id => Some(r.as_ref()),
        CollectionItem::Folder { items, .. } => find(items, id),
        _ => None,
    })
}
fn find_mut<'a>(items: &'a mut [CollectionItem], id: &str) -> Option<&'a mut ApiRequest> {
    items.iter_mut().find_map(|item| match item {
        CollectionItem::Request(r) if r.id == id => Some(r.as_mut()),
        CollectionItem::Folder { items, .. } => find_mut(items, id),
        _ => None,
    })
}
impl ApiWorkspace {
    pub fn find_request(&self, id: &str) -> Option<&ApiRequest> {
        self.collections.iter().find_map(|c| find(&c.items, id))
    }
    pub fn find_request_mut(&mut self, id: &str) -> Option<&mut ApiRequest> {
        self.collections
            .iter_mut()
            .find_map(|c| find_mut(&mut c.items, id))
    }
    pub fn active_environment(&self) -> Option<&Environment> {
        self.environments
            .iter()
            .find(|e| Some(&e.id) == self.active_environment.as_ref())
    }
    pub fn collection_for_request(&self, id: &str) -> Option<&Collection> {
        self.collections
            .iter()
            .find(|c| find(&c.items, id).is_some())
    }
    pub fn variables_for(&self, id: &str) -> Variables {
        let mut result = Variables::new();
        merge_variables(&mut result, &self.globals);
        if let Some(collection) = self
            .collections
            .iter()
            .find(|c| find(&c.items, id).is_some())
        {
            merge_variables(&mut result, &collection.variables);
        }
        if let Some(environment) = self.active_environment() {
            merge_variables(&mut result, &environment.variables);
        }
        result
    }
    pub fn request_context(&self, id: &str) -> Option<(ApiRequest, Variables)> {
        fn walk(
            items: &[CollectionItem],
            id: &str,
            auth: &Auth,
            pre: &str,
            post: &str,
        ) -> Option<ApiRequest> {
            for item in items {
                match item {
                    CollectionItem::Request(r) if r.id == id => {
                        let mut r = (**r).clone();
                        if r.auth == Auth::Inherit {
                            r.auth = auth.clone();
                        }
                        r.pre_request_script =
                            [pre.to_owned(), scoped_script(&r.pre_request_script)].join("\n");
                        r.post_response_script =
                            [post.to_owned(), scoped_script(&r.post_response_script)].join("\n");
                        return Some(r);
                    }
                    CollectionItem::Folder {
                        items,
                        auth: a,
                        pre_request_script: p,
                        post_response_script: q,
                        ..
                    } => {
                        if let Some(r) = walk(
                            items,
                            id,
                            if *a == Auth::Inherit { auth } else { a },
                            &[pre.to_owned(), scoped_script(p)].join("\n"),
                            &[post.to_owned(), scoped_script(q)].join("\n"),
                        ) {
                            return Some(r);
                        }
                    }
                    _ => {}
                }
            }
            None
        }
        self.collections
            .iter()
            .find_map(|c| {
                walk(
                    &c.items,
                    id,
                    &c.auth,
                    &scoped_script(&c.pre_request_script),
                    &scoped_script(&c.post_response_script),
                )
            })
            .map(|r| (r, self.variables_for(id)))
    }
    pub fn new_collection(&mut self, name: impl Into<String>) -> String {
        let c = Collection {
            name: name.into(),
            ..Collection::default()
        };
        let id = c.id.clone();
        self.collections.push(c);
        id
    }
    pub fn new_request(&mut self, collection_id: &str, name: impl Into<String>) -> Option<String> {
        if self.item_count() >= 10_000 {
            return None;
        }
        let c = self.destination_mut(collection_id)?;
        let r = ApiRequest {
            name: name.into(),
            ..ApiRequest::default()
        };
        let id = r.id.clone();
        c.push(CollectionItem::Request(Box::new(r)));
        Some(id)
    }
    pub fn new_environment(&mut self, name: impl Into<String>) -> String {
        let e = Environment {
            name: name.into(),
            ..Environment::default()
        };
        let id = e.id.clone();
        self.environments.push(e);
        id
    }
    /// Script values are session credentials by default. Never persist them in profile JSON.
    pub fn apply_environment_updates(&mut self, environment_id: &str, updates: &Variables) {
        if let Some(env) = self
            .environments
            .iter_mut()
            .find(|e| e.id == environment_id)
        {
            for (key, value) in updates {
                if let Some(row) = env.variables.iter_mut().find(|r| r.key == *key) {
                    row.value = value.clone();
                    row.secret = true;
                } else {
                    env.variables.push(KeyValue {
                        key: key.clone(),
                        value: value.clone(),
                        secret: true,
                        ..KeyValue::default()
                    });
                }
            }
        }
    }
}
pub fn merge_variables(target: &mut Variables, rows: &[KeyValue]) {
    for row in rows.iter().filter(|r| r.enabled && !r.key.is_empty()) {
        target.insert(row.key.clone(), row.value.clone());
    }
}
pub fn interpolate(text: &str, variables: &Variables) -> Result<String, String> {
    if text.len() > 8 * 1024 * 1024 {
        return Err("Input exceeds 8 MiB".into());
    }
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        rest = &rest[start + 2..];
        let Some(end) = rest.find("}}") else {
            return Err("Unclosed variable {{…}}".into());
        };
        let key = rest[..end].trim();
        let generated;
        let value = if let Some(value) = variables.get(key) {
            value
        } else {
            generated = dynamic_variable(key)
                .ok_or_else(|| format!("Variable {{{{{key}}}}} is undefined"))?;
            &generated
        };
        if out.len() + value.len() > 8 * 1024 * 1024 {
            return Err("Expanded input exceeds 8 MiB".into());
        }
        out.push_str(value);
        rest = &rest[end + 2..];
    }
    out.push_str(rest);
    Ok(out)
}

fn scoped_script(script: &str) -> String {
    if script.trim().is_empty() {
        String::new()
    } else {
        format!("(function() {{\n{script}\n}})();")
    }
}

/// Dynamic test values, never credentials. Generated in the HTTP/cURL worker path.
fn dynamic_variable(key: &str) -> Option<String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    match key {
        "$timestamp" => Some(now.as_secs().to_string()),
        "$guid" | "$randomUUID" => {
            use sha2::{Digest, Sha256};
            let mut bytes = Sha256::digest(new_id().as_bytes());
            bytes[6] = (bytes[6] & 15) | 64;
            bytes[8] = (bytes[8] & 63) | 128;
            let hex = bytes[..16]
                .iter()
                .map(|v| format!("{v:02x}"))
                .collect::<String>();
            Some(format!(
                "{}-{}-{}-{}-{}",
                &hex[..8],
                &hex[8..12],
                &hex[12..16],
                &hex[16..20],
                &hex[20..]
            ))
        }
        "$randomInt" => {
            use std::hash::{BuildHasher, Hasher};
            let mut h = std::collections::hash_map::RandomState::new().build_hasher();
            h.write_u128(now.as_nanos());
            Some((h.finish() % 1001).to_string())
        }
        _ => None,
    }
}
