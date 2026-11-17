//! Bounded catalog returned by the selected installed runtime, not a static model list.
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Model {
    pub id: String,
    pub name: String,
    pub description: String,
    pub is_default: bool,
    pub default_effort: String,
    pub efforts: Vec<String>,
    pub images: bool,
}
pub(crate) fn page(value: &Value) -> Result<Vec<Model>, String> {
    let data = value["data"]
        .as_array()
        .ok_or("Catálogo de modelos inválido")?;
    if data.len() > 128 {
        return Err("Catálogo excedeu128 modelos".into());
    }
    let mut models = Vec::new();
    for item in data {
        if item["hidden"] == true {
            continue;
        }
        let id = item["model"]
            .as_str()
            .or(item["id"].as_str())
            .ok_or("Modelo sem identificador")?;
        if id.len() > 256 {
            return Err("Identificador de modelo excessivo".into());
        }
        let name = item["displayName"].as_str().unwrap_or(id);
        let efforts = item["supportedReasoningEfforts"]
            .as_array()
            .map(|values| {
                values
                    .iter()
                    .filter_map(|v| v["reasoningEffort"].as_str())
                    .take(16)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        models.push(Model {
            id: id.into(),
            name: name.chars().take(128).collect(),
            description: item["description"]
                .as_str()
                .unwrap_or("")
                .chars()
                .take(1024)
                .collect(),
            is_default: item["isDefault"] == true,
            default_effort: item["defaultReasoningEffort"].as_str().unwrap_or("").into(),
            efforts,
            images: item["inputModalities"]
                .as_array()
                .is_none_or(|a| a.iter().any(|v| v == "image")),
        });
    }
    Ok(models)
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn catalog_uses_runtime_metadata_and_filters_hidden() {
        let m=page(&json!({"data":[{"id":"i","model":"runtime-model","displayName":"Runtime","isDefault":true,"supportedReasoningEfforts":[{"reasoningEffort":"high"}],"inputModalities":["text"]},{"id":"hidden","hidden":true}]})).unwrap();
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].id, "runtime-model");
        assert_eq!(m[0].efforts, ["high"]);
        assert!(!m[0].images);
    }
}
