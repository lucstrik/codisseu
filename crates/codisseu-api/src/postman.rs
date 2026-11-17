use crate::*;
use serde_json::{Value, json};

const LIMIT: usize = 16 * 1024 * 1024;
fn parse(source: &str) -> Result<Value, String> {
    if source.len() > LIMIT {
        return Err("Import exceeds 16 MiB".into());
    }
    serde_json::from_str(source).map_err(|_| "Invalid JSON document".into())
}
fn text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        v => v.to_string(),
    }
}
fn description(value: &Value) -> String {
    if value.is_object() {
        text(&value["content"])
    } else {
        text(value)
    }
}
fn rows(value: &Value) -> Vec<KeyValue> {
    value
        .as_array()
        .map(|a| {
            a.iter()
                .map(|v| KeyValue {
                    file_path: v["src"].as_str().map(std::path::PathBuf::from),
                    key: text(&v["key"]),
                    value: text(&v["value"]),
                    description: description(&v["description"]),
                    enabled: !v["disabled"].as_bool().unwrap_or(false)
                        && v["enabled"].as_bool().unwrap_or(true),
                    secret: v["type"] == "secret",
                })
                .collect()
        })
        .unwrap_or_default()
}
fn scripts(value: &Value) -> (String, String) {
    let mut pre = vec![];
    let mut post = vec![];
    if let Some(events) = value.as_array() {
        for event in events {
            let script = match &event["script"]["exec"] {
                Value::Array(lines) => lines.iter().map(text).collect::<Vec<_>>().join("\n"),
                value => text(value),
            };
            match event["listen"].as_str() {
                Some("prerequest") => pre.push(script),
                Some("test") => post.push(script),
                _ => {}
            }
        }
    }
    (pre.join("\n"), post.join("\n"))
}
fn auth(v: &Value) -> Result<Auth, String> {
    let Some(kind) = v["type"].as_str() else {
        return Ok(Auth::Inherit);
    };
    let fields = rows(&v[kind]);
    let field = |key: &str| {
        fields
            .iter()
            .find(|r| r.key == key)
            .map(|r| r.value.clone())
            .unwrap_or_default()
    };
    Ok(match kind {
        "noauth" => Auth::None,
        "bearer" => Auth::Bearer {
            token: field("token"),
        },
        "oauth2" if !field("accessToken").is_empty() => Auth::Bearer {
            token: field("accessToken"),
        },
        "basic" => Auth::Basic {
            username: field("username"),
            password: field("password"),
        },
        "apikey" => Auth::ApiKey {
            key: field("key"),
            value: field("value"),
            in_query: field("in") == "query",
        },
        other => {
            return Err(format!(
                "Authorization {other} is not supported; import stopped to avoid sending an unauthenticated request"
            ));
        }
    })
}
fn import_items(
    value: &Value,
    depth: usize,
    count: &mut usize,
) -> Result<Vec<CollectionItem>, String> {
    if depth > 32 {
        return Err("Collection folder depth exceeds 32".into());
    }
    let values = value.as_array().ok_or("Collection item must be an array")?;
    let mut result = vec![];
    for item in values {
        *count += 1;
        if *count > 10_000 {
            return Err("Collection exceeds 10000 items".into());
        }
        let (pre, post) = scripts(&item["event"]);
        if item["item"].is_array() {
            result.push(CollectionItem::Folder {
                id: new_id(),
                name: text(&item["name"]),
                items: import_items(&item["item"], depth + 1, count)?,
                auth: auth(&item["auth"])?,
                pre_request_script: pre,
                post_response_script: post,
            });
            continue;
        }
        let v = &item["request"];
        let url = if v.is_string() {
            text(v)
        } else if v["url"].is_string() {
            text(&v["url"])
        } else {
            text(&v["url"]["raw"])
        };
        let mut request = ApiRequest {
            name: text(&item["name"]),
            method: v["method"].as_str().unwrap_or("GET").to_uppercase(),
            url,
            description: description(&v["description"]),
            headers: rows(&v["header"]),
            auth: auth(&v["auth"])?,
            pre_request_script: pre,
            post_response_script: post,
            ..ApiRequest::default()
        };
        request.set_url(&request.url.clone());
        if v["url"]["variable"].is_array() {
            request.path_variables = rows(&v["url"]["variable"]);
        }
        if v["url"]["query"].is_array() {
            request.params = rows(&v["url"]["query"]);
        }
        request.body=match v["body"]["mode"].as_str(){None=>RequestBody::None,Some("raw")=>RequestBody::Raw{text:text(&v["body"]["raw"]),content_type:request.headers.iter().find(|h|h.key.eq_ignore_ascii_case("Content-Type")).map(|h|h.value.clone()).unwrap_or_else(||match v["body"]["options"]["raw"]["language"].as_str(){Some("json")=>"application/json".into(),Some("xml")=>"application/xml".into(),Some("html")=>"text/html".into(),_=>"text/plain".into()})},Some("urlencoded")=>RequestBody::UrlEncoded(rows(&v["body"]["urlencoded"])),Some("formdata")=>{if v["body"]["formdata"].as_array().is_some_and(|a|a.iter().any(|r|r["type"]=="file"&&!r["src"].is_string())){return Err("Multipart file requires a single explicit source path".into());}RequestBody::FormData(rows(&v["body"]["formdata"]))},Some("graphql")=>RequestBody::Raw{text:json!({"query":v["body"]["graphql"]["query"],"variables":serde_json::from_str::<Value>(v["body"]["graphql"]["variables"].as_str().unwrap_or("{}")).unwrap_or(json!({}))}).to_string(),content_type:"application/json".into()},Some("file")=>RequestBody::BinaryFile{path:std::path::PathBuf::from(v["body"]["file"]["src"].as_str().ok_or("Postman binary file path is missing")?)},Some(mode)=>return Err(format!("Unsupported Postman body mode {mode}"))};
        result.push(CollectionItem::Request(Box::new(request)));
    }
    Ok(result)
}
pub fn import_collection(source: &str) -> Result<Collection, String> {
    let v = parse(source)?;
    if !v["info"].is_object() {
        return Err("Expected a Postman collection with info and item".into());
    }
    let (pre, post) = scripts(&v["event"]);
    Ok(Collection {
        name: text(&v["info"]["name"]),
        description: description(&v["info"]["description"]),
        items: import_items(&v["item"], 0, &mut 0)?,
        variables: rows(&v["variable"]),
        auth: auth(&v["auth"])?,
        pre_request_script: pre,
        post_response_script: post,
        ..Collection::default()
    })
}
fn export_rows(rows: &[KeyValue]) -> Value {
    json!(rows.iter().map(|r|json!({"key":r.key,"value":r.value,"description":r.description,"disabled":!r.enabled,"type":if r.file_path.is_some(){"file"}else if r.secret{"secret"}else{"default"},"src":r.file_path})).collect::<Vec<_>>())
}
fn export_auth(a: &Auth) -> Value {
    match a {
        Auth::Inherit => Value::Null,
        Auth::None => json!({"type":"noauth"}),
        Auth::Bearer { token } => {
            json!({"type":"bearer","bearer":[{"key":"token","value":token,"type":"string"}]})
        }
        Auth::Basic { username, password } => {
            json!({"type":"basic","basic":[{"key":"username","value":username,"type":"string"},{"key":"password","value":password,"type":"string"}]})
        }
        Auth::ApiKey {
            key,
            value,
            in_query,
        } => {
            json!({"type":"apikey","apikey":[{"key":"key","value":key,"type":"string"},{"key":"value","value":value,"type":"string"},{"key":"in","value":if *in_query{"query"}else{"header"},"type":"string"}]})
        }
    }
}
fn export_scripts(pre: &str, post: &str) -> Value {
    let mut events = vec![];
    for (name, script) in [("prerequest", pre), ("test", post)] {
        if !script.trim().is_empty() {
            events.push(json!({"listen":name,"script":{"type":"text/javascript","exec":script.lines().collect::<Vec<_>>()}}));
        }
    }
    json!(events)
}
fn export_items(items: &[CollectionItem]) -> Result<Vec<Value>, String> {
    items.iter().map(|item|Ok(match item{CollectionItem::Folder{name,items,auth,pre_request_script,post_response_script,..}=>json!({"name":name,"item":export_items(items)?,"auth":export_auth(auth),"event":export_scripts(pre_request_script,post_response_script)}),CollectionItem::Request(r)=>{
    let raw=r.display_url();
    let mut headers=r.headers.clone();let body=match &r.body{RequestBody::None=>Value::Null,RequestBody::Raw{text,content_type}=>{if !content_type.is_empty()&&!headers.iter().any(|h|h.key.eq_ignore_ascii_case("Content-Type")){headers.push(KeyValue::new("Content-Type",content_type));}json!({"mode":"raw","raw":text,"options":{"raw":{"language":if content_type.contains("json"){"json"}else{"text"}}}})},RequestBody::UrlEncoded(rows)=>json!({"mode":"urlencoded","urlencoded":export_rows(rows)}),RequestBody::FormData(rows)=>json!({"mode":"formdata","formdata":export_rows(rows)}),RequestBody::BinaryFile{path}=>json!({"mode":"file","file":{"src":path}}),RequestBody::Binary(_)=>return Err("Postman export of inline binary bodies is unsupported".into())};
    json!({"name":r.name,"event":export_scripts(&r.pre_request_script,&r.post_response_script),"request":{"method":r.method,"description":r.description,"header":export_rows(&headers),"auth":export_auth(&r.auth),"url":{"raw":raw,"query":export_rows(&r.params),"variable":export_rows(&r.path_variables)},"body":body}})
}})).collect()
}
pub fn export_collection(collection: &Collection) -> Result<String, String> {
    serde_json::to_string_pretty(&json!({"info":{"name":collection.name,"description":collection.description,"schema":"https://schema.getpostman.com/json/collection/v2.1.0/collection.json"},"item":export_items(&collection.items)?,"variable":export_rows(&collection.variables),"auth":export_auth(&collection.auth),"event":export_scripts(&collection.pre_request_script,&collection.post_response_script)})).map_err(|_|"Could not export collection".into())
}
pub fn import_environment(source: &str) -> Result<Environment, String> {
    let v = parse(source)?;
    if !v["values"].is_array() {
        return Err("Expected a Postman environment values array".into());
    }
    Ok(Environment {
        name: text(&v["name"]),
        variables: rows(&v["values"]),
        ..Environment::default()
    })
}
pub fn export_environment(environment: &Environment) -> Result<String, String> {
    serde_json::to_string_pretty(&json!({"id":environment.id,"name":environment.name,"values":environment.variables.iter().map(|r|json!({"key":r.key,"value":r.value,"type":if r.file_path.is_some(){"file"}else if r.secret{"secret"}else{"default"},"src":r.file_path,"enabled":r.enabled})).collect::<Vec<_>>(),"_postman_variable_scope":"environment"})).map_err(|_|"Could not export environment".into())
}
