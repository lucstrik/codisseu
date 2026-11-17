use crate::*;
use serde_json::Value;

/// Imports OpenAPI 3.x JSON/YAML. External refs are never fetched.
pub fn import_openapi(source: &str) -> Result<(ApiSpec, Collection), String> {
    if source.len() > 16 * 1024 * 1024 {
        return Err("OpenAPI exceeds 16 MiB".into());
    }
    let value: Value = serde_yaml::from_str(source).map_err(|_| "Invalid OpenAPI JSON/YAML")?;
    if !value["openapi"]
        .as_str()
        .is_some_and(|s| s.starts_with("3."))
    {
        return Err("OpenAPI 3.x document is required".into());
    }
    let name = value["info"]["title"]
        .as_str()
        .unwrap_or("Imported API")
        .to_owned();
    let base = value["servers"][0]["url"]
        .as_str()
        .unwrap_or("{{base_url}}");
    let paths = value["paths"]
        .as_object()
        .ok_or("OpenAPI paths object is required")?;
    let mut collection = Collection {
        name: name.clone(),
        auth: security_auth(&value, &value["security"])?,
        ..Collection::default()
    };
    collection.variables.push(KeyValue::new(
        "base_url",
        if base.starts_with("http") {
            base
        } else {
            "http://localhost"
        },
    ));
    if let Some(vars) = value["servers"][0]["variables"].as_object() {
        for (key, v) in vars {
            collection
                .variables
                .push(KeyValue::new(key, v["default"].as_str().unwrap_or("")));
        }
    }
    for (path, path_item) in paths {
        for method in [
            "get", "post", "put", "patch", "delete", "head", "options", "trace",
        ] {
            let operation = &path_item[method];
            if !operation.is_object() {
                continue;
            }
            if collection.items.len() >= 10_000 {
                return Err("OpenAPI exceeds 10000 operations".into());
            }
            let mut request = ApiRequest {
                name: operation["summary"]
                    .as_str()
                    .or(operation["operationId"].as_str())
                    .unwrap_or(path)
                    .into(),
                method: method.to_uppercase(),
                url: format!("{{{{base_url}}}}{path}"),
                auth: if operation.get("security").is_some() {
                    security_auth(&value, &operation["security"])?
                } else {
                    Auth::Inherit
                },
                description: operation["description"].as_str().unwrap_or("").into(),
                ..ApiRequest::default()
            };
            for parameter in path_item["parameters"]
                .as_array()
                .into_iter()
                .flatten()
                .chain(operation["parameters"].as_array().into_iter().flatten())
            {
                let parameter = if let Some(reference) = parameter["$ref"].as_str() {
                    if let Some(ptr) = reference.strip_prefix('#') {
                        value
                            .pointer(ptr)
                            .ok_or("Unresolved OpenAPI parameter reference")?
                    } else {
                        return Err(
                            "External OpenAPI references are unsupported and were not fetched"
                                .into(),
                        );
                    }
                } else {
                    parameter
                };
                let key = parameter["name"].as_str().unwrap_or("");
                let sample = &parameter["example"];
                let sample = if sample.is_null() {
                    &parameter["schema"]["default"]
                } else {
                    sample
                };
                let sample = sample.as_str().map(str::to_owned).unwrap_or_else(|| {
                    if sample.is_null() {
                        String::new()
                    } else {
                        sample.to_string()
                    }
                });
                match parameter["in"].as_str() {
                    Some("query") => request.params.push(KeyValue::new(key, sample)),
                    Some("header") => request.headers.push(KeyValue::new(key, sample)),
                    Some("path") => {
                        request.url = request
                            .url
                            .replace(&format!("{{{key}}}"), &format!("{{{{{key}}}}}"));
                        if !collection.variables.iter().any(|r| r.key == key) {
                            collection.variables.push(KeyValue::new(key, sample));
                        }
                    }
                    _ => {}
                }
            }
            if let Some(content) = operation["requestBody"]["content"].as_object()
                && let Some((kind, v)) = content.iter().next()
            {
                let body = if v["example"].is_null() {
                    String::new()
                } else if let Some(s) = v["example"].as_str() {
                    s.to_owned()
                } else {
                    serde_json::to_string_pretty(&v["example"]).unwrap_or_default()
                };
                request.body = RequestBody::Raw {
                    text: body,
                    content_type: kind.clone(),
                };
            }
            collection
                .items
                .push(CollectionItem::Request(Box::new(request)));
        }
    }
    Ok((
        ApiSpec {
            id: new_id(),
            name,
            source: source.into(),
        },
        collection,
    ))
}

fn security_auth(document: &Value, security: &Value) -> Result<Auth, String> {
    let Some(requirements) = security.as_array() else {
        return Ok(Auth::None);
    };
    if requirements.is_empty() {
        return Ok(Auth::None);
    }
    if requirements.len() != 1 {
        return Err(
            "Multiple OpenAPI security alternatives require manual configuration; import stopped"
                .into(),
        );
    }
    let requirement = requirements[0]
        .as_object()
        .ok_or("Invalid OpenAPI security requirement")?;
    if requirement.is_empty() {
        return Ok(Auth::None);
    }
    if requirement.len() != 1 {
        return Err(
            "Combined OpenAPI authorization schemes are unsupported; import stopped".into(),
        );
    }
    let key = requirement.keys().next().expect("one security requirement");
    let mut scheme = &document["components"]["securitySchemes"][key];
    if let Some(reference) = scheme["$ref"].as_str() {
        scheme = reference
            .strip_prefix('#')
            .and_then(|p| document.pointer(p))
            .ok_or("External or unresolved OpenAPI security reference")?;
    }
    match scheme["type"].as_str(){
        Some("http")=>match scheme["scheme"].as_str().map(str::to_ascii_lowercase).as_deref(){Some("bearer")=>Ok(Auth::Bearer{token:"{{access_token}}".into()}),Some("basic")=>Ok(Auth::Basic{username:"{{username}}".into(),password:"{{password}}".into()}),_=>Err("Unsupported OpenAPI HTTP authorization scheme; import stopped".into())},
        Some("apiKey")=>{let location=scheme["in"].as_str().unwrap_or("");if !matches!(location,"header"|"query"){return Err("OpenAPI API key cookie authorization is unsupported".into());}Ok(Auth::ApiKey{key:scheme["name"].as_str().ok_or("API key scheme requires a name")?.into(),value:"{{api_key}}".into(),in_query:location=="query"})},
        _=>Err("Unsupported OpenAPI authorization scheme; import stopped to avoid sending unauthenticated requests".into())
    }
}
