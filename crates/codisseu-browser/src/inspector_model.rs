use super::*;
use serde_json::Value;
pub(super) const TEXT: usize = 4096;
pub(super) fn text(v: &Value, limit: usize) -> String {
    bounded(v.as_str().unwrap_or(""), limit)
}
pub(super) fn bounded(s: &str, n: usize) -> String {
    let mut at = s.len().min(n);
    while !s.is_char_boundary(at) {
        at -= 1;
    }
    s[..at].to_owned()
}
pub(super) fn pairs(v: &Value, limit: usize) -> Vec<(String, String)> {
    v.as_object()
        .map(|m| {
            m.iter()
                .take(limit)
                .map(|(k, v)| {
                    (
                        bounded(k, 256),
                        if v.is_string() {
                            text(v, TEXT)
                        } else {
                            bounded(&v.to_string(), TEXT)
                        },
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}
pub(super) fn remote(v: &Value) -> RemoteValue {
    RemoteValue {
        kind: text(&v["type"], 64),
        subtype: v["subtype"].as_str().map(|s| bounded(s, 64)),
        description: if let Some(s) = v["description"].as_str() {
            bounded(s, 2048)
        } else if v.get("value").is_some() {
            bounded(&v["value"].to_string(), 2048)
        } else {
            text(&v["unserializableValue"], 2048)
        },
        object_id: v["objectId"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 1024)
            .map(str::to_owned),
    }
}
pub(super) fn properties(v: &Value) -> Vec<Property> {
    v.as_array()
        .map(|a| {
            a.iter()
                .take(200)
                .map(|p| Property {
                    name: text(&p["name"], 256),
                    value: remote(&p["value"]),
                    writable: p["writable"].as_bool().unwrap_or(false),
                    enumerable: p["enumerable"].as_bool().unwrap_or(false),
                    getter: p.get("get").is_some(),
                    setter: p.get("set").is_some(),
                })
                .collect()
        })
        .unwrap_or_default()
}
pub(super) fn origin(url: &str) -> Result<String, String> {
    let canonical = crate::validate_url(url)?;
    let u = url::Url::parse(&canonical).map_err(|_| "Origem inválida")?;
    if !matches!(u.scheme(), "http" | "https") {
        return Err("Abra uma página HTTP/HTTPS para inspecionar".into());
    }
    Ok(u.origin().ascii_serialization())
}
pub(super) fn storage(v: &Value) -> Vec<(String, String)> {
    let mut size = 0;
    v.as_array()
        .map(|a| {
            a.iter()
                .take(256)
                .filter_map(|v| {
                    let item = v.as_array()?;
                    if item.len() != 2 {
                        return None;
                    }
                    let a = text(&item[0], 4096);
                    let b = text(&item[1], 16384);
                    size += a.len() + b.len();
                    (size <= 1024 * 1024).then_some((a, b))
                })
                .collect()
        })
        .unwrap_or_default()
}
pub(super) fn cookie(v: &Value) -> Cookie {
    Cookie {
        name: text(&v["name"], 1024),
        value: text(&v["value"], 8192),
        domain: text(&v["domain"], 512),
        path: text(&v["path"], 2048),
        secure: v["secure"].as_bool().unwrap_or(false),
        http_only: v["httpOnly"].as_bool().unwrap_or(false),
        same_site: v["sameSite"].as_str().map(|s| bounded(s, 32)),
        expires: v["expires"].as_f64().unwrap_or(-1.),
        session: v["session"].as_bool().unwrap_or(false),
    }
}
pub(super) fn native_pause(v: &Value) -> Pause {
    let frames = v["callFrames"]
        .as_array()
        .map(|a| {
            a.iter()
                .take(32)
                .map(|f| CallFrame {
                    id: text(&f["callFrameId"], 1024),
                    function_name: text(&f["functionName"], 256),
                    url: text(&f["url"], 8192),
                    line: (f["location"]["lineNumber"].as_u64().unwrap_or(0)).min(u32::MAX as u64)
                        as u32,
                    column: (f["location"]["columnNumber"].as_u64().unwrap_or(0))
                        .min(u32::MAX as u64) as u32,
                    scopes: f["scopeChain"]
                        .as_array()
                        .map(|s| {
                            s.iter()
                                .take(32)
                                .filter_map(|p| {
                                    p["object"]["objectId"]
                                        .as_str()
                                        .filter(|id| id.len() <= 1024)
                                        .map(|id| Scope {
                                            name: text(&p["name"], 256),
                                            kind: text(&p["type"], 64),
                                            object_id: id.into(),
                                        })
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let selected = frames.first().map(|f| f.id.clone());
    Pause {
        reason: text(&v["reason"], 128),
        frames,
        selected,
    }
}
pub(super) fn walk_nodes(v: &Value, generation: u64) -> Vec<DomNode> {
    let mut out = Vec::new();
    let mut bytes = 0;
    let base = (generation % 1_000_000_000) * 1024;
    if let Some(nodes) = v.as_array() {
        for (index, n) in nodes.iter().take(500).enumerate() {
            let depth = n["depth"].as_u64().unwrap_or(0).min(8) as usize;
            let parent = n["parent"]
                .as_u64()
                .filter(|p| (*p as usize) < index)
                .map(|p| (base + p + 1) as i64);
            let attributes = n["attributes"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .take(32)
                        .filter_map(|v| {
                            v.as_array()
                                .filter(|p| p.len() == 2)
                                .map(|p| (text(&p[0], 256), text(&p[1], 1024)))
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let name = text(&n["name"], 256);
            let local_name = text(&n["localName"], 256);
            let value = text(&n["value"], 512);
            bytes += name.len()
                + local_name.len()
                + value.len()
                + attributes
                    .iter()
                    .map(|(a, b)| a.len() + b.len())
                    .sum::<usize>();
            if bytes > 1024 * 1024 {
                break;
            }
            out.push(DomNode {
                id: (base + index as u64 + 1) as i64,
                parent,
                depth,
                node_type: n["nodeType"].as_u64().unwrap_or(0),
                name,
                custom_element: local_name.contains('-'),
                local_name,
                value,
                attributes,
                child_count: n["childCount"].as_u64().unwrap_or(0).min(1_000_000) as usize,
                shadow_root: n["shadow"].as_bool().unwrap_or(false),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn dom_bounds_preserve_unicode_custom_shadow_and_epoch_identity() {
        let mut nodes = vec![
            json!({"name":"#document","depth":0}),
            json!({"name":"FIXTURE-COUNTER","localName":"fixture-counter","depth":1,"parent":0}),
            json!({"name":"#document-fragment","depth":2,"parent":1,"shadow":true,"value":"😀".repeat(900)}),
        ];
        nodes.extend(
            (0..20_000).map(|_| json!({"name":"DIV","localName":"div","depth":3,"parent":2})),
        );
        let a = walk_nodes(&json!(nodes), 1);
        let b = walk_nodes(&json!(nodes), 2);
        assert_eq!(a.len(), 500);
        assert!(a[1].custom_element && a[2].shadow_root);
        assert!(a[2].value.is_char_boundary(a[2].value.len()));
        assert!(a[2].value.len() <= 512);
        assert_eq!(a[2].parent, Some(a[1].id));
        assert_ne!(a[0].id, b[0].id);
    }
    #[test]
    fn own_properties_mark_getters_without_calling_or_exposing_accessor_functions() {
        let p = properties(
            &json!([{"name":"answer","value":{"type":"number","value":42}}, {"name":"secret","get":{"type":"function","objectId":"getter"}}]),
        );
        assert_eq!(p[0].value.description, "42");
        assert!(p[1].getter);
        assert!(p[1].value.object_id.is_none());
    }
    #[test]
    fn storage_and_frames_are_bounded_and_locations_are_zero_based() {
        let entries = (0..300)
            .map(|n| json!([n.to_string(), "😀".repeat(9000)]))
            .collect::<Vec<_>>();
        let p = storage(&json!(entries));
        assert!(p.len() <= 256);
        assert!(p.iter().map(|(k, v)| k.len() + v.len()).sum::<usize>() <= 1024 * 1024);
        let pause = native_pause(
            &json!({"callFrames":[{"callFrameId":"0","location":{"lineNumber":0,"columnNumber":0},"scopeChain":[]}]}),
        );
        assert_eq!(pause.frames[0].line, 0);
        assert_eq!(pause.frames[0].column, 0);
        assert!(origin("file:///etc/passwd").is_err());
        assert!(origin("https://user:pass@example.com").is_err());
    }
}
