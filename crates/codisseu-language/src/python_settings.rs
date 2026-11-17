//! Basedpyright configuration: explicit interpreter, bounded background analysis,
//! native hints/import suggestions. No script discovery, dependency installs or execution.
use serde_json::{Value, json};
use std::path::Path;
pub fn settings(python: Option<&Path>) -> Value {
    json!({"python":{"pythonPath":python},"basedpyright":{
        "disableOrganizeImports":true,
        "analysis":{"diagnosticMode":"openFilesOnly","autoImportCompletions":true,
            "autoFormatStrings":false,"baselineMode":"discard",
            "inlayHints":{"variableTypes":true,"callArgumentNames":true,"functionReturnTypes":true,"genericTypes":true}}
    }})
}
pub fn configuration(items: Option<&Value>, python: Option<&Path>) -> Vec<Value> {
    let settings = settings(python);
    items
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(128)
        .map(|item| {
            item.get("section")
                .and_then(Value::as_str)
                .map_or(Value::Null, |section| {
                    let mut value = &settings;
                    for field in section.split('.') {
                        let Some(next) = value.get(field) else {
                            return Value::Null;
                        };
                        value = next;
                    }
                    value.clone()
                })
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn settings_return_exact_sections_and_never_executable_metadata() {
        let items =
            json!([{"section":"python"},{"section":"basedpyright.analysis"},{"section":"unknown"}]);
        let result = configuration(Some(&items), Some(Path::new("/tmp/venv ação/bin/python")));
        assert_eq!(result[0]["pythonPath"], "/tmp/venv ação/bin/python");
        assert_eq!(result[1]["autoImportCompletions"], true);
        assert_eq!(result[1]["baselineMode"], "discard");
        assert!(result[2].is_null());
    }
}
