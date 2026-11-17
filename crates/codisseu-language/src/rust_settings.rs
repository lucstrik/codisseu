//! Explicit analysis-only defaults. Cargo build/check and procedural macros
//! execute project code and belong to the user's explicit Cargo actions.
use serde_json::{Value, json};

pub(crate) fn options() -> Value {
    json!({
        "cargo": {
            "buildScripts": { "enable": false },
            "extraArgs": ["--offline"]
        },
        "procMacro": { "enable": false },
        "checkOnSave": false,
        "cachePriming": { "numThreads": 2 },
        "numThreads": 2
    })
}

pub(crate) fn configuration(items: Option<&Value>) -> Vec<Value> {
    let options = options();
    items
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|item| {
            let Some(section) = item.get("section").and_then(Value::as_str) else {
                return Value::Null;
            };
            if section == "rust-analyzer" {
                return options.clone();
            }
            let Some(path) = section.strip_prefix("rust-analyzer.") else {
                return Value::Null;
            };
            let mut value = &options;
            for field in path.split('.') {
                let Some(next) = value.get(field) else {
                    return Value::Null;
                };
                value = next;
            }
            value.clone()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn initialization_and_configuration_keep_project_execution_disabled() {
        let values = configuration(Some(&json!([
            {"section":"rust-analyzer"},
            {"section":"rust-analyzer.cargo.buildScripts.enable"},
            {"section":"rust-analyzer.procMacro.enable"},
            {"section":"rust-analyzer.checkOnSave"},
            {"section":"unrelated"}
        ])));
        assert_eq!(values[0], options());
        assert_eq!(values[1..4], [false, false, false]);
        assert_eq!(values[4], Value::Null);
        assert_eq!(values[0]["cargo"]["extraArgs"], json!(["--offline"]));
    }
}
