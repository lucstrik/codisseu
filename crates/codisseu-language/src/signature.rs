//! Bounded, inert LSP signature data. Ranges refer to UTF-8 boundaries after
//! validating the provider's UTF-16 offsets; no source-code parsing is used.
use crate::{
    SignatureHelpCapabilities, SignatureHelpResult, SignatureInformation, SignatureParameter,
};
use serde_json::Value;

pub(crate) fn capabilities(value: Option<&Value>) -> Option<SignatureHelpCapabilities> {
    let value = value?.as_object()?;
    let chars = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|s| s.chars().count() == 1)
            .take(32)
            .map(str::to_owned)
            .collect()
    };
    Some(SignatureHelpCapabilities {
        trigger_characters: chars("triggerCharacters"),
        retrigger_characters: chars("retriggerCharacters"),
    })
}
fn documentation(value: Option<&Value>) -> Result<Option<String>, String> {
    let Some(value) = value else { return Ok(None) };
    let text = if let Some(text) = value.as_str() {
        text
    } else {
        let kind = value.get("kind").and_then(Value::as_str);
        if !matches!(kind, Some("plaintext" | "markdown")) {
            return Err("Documentação de assinatura inválida".into());
        }
        value
            .get("value")
            .and_then(Value::as_str)
            .ok_or("Documentação de assinatura inválida")?
    };
    if text.len() > 32768 || text.contains('\0') {
        return Err("Documentação de assinatura excede limites".into());
    }
    Ok(Some(text.to_owned()))
}
fn index(value: Option<&Value>) -> Result<Option<usize>, String> {
    value
        .map(|v| {
            v.as_u64()
                .filter(|v| *v <= i32::MAX as u64)
                .map(|v| v as usize)
                .ok_or_else(|| "Índice de assinatura inválido".into())
        })
        .transpose()
}
fn offset(label: &str, units: usize) -> Option<usize> {
    let mut at = 0;
    for (byte, ch) in label.char_indices() {
        if at == units {
            return Some(byte);
        }
        at += ch.len_utf16();
        if at > units {
            return None;
        }
    }
    (at == units).then_some(label.len())
}
pub(crate) fn decode(
    value: Option<&Value>,
    mut result: SignatureHelpResult,
) -> Result<SignatureHelpResult, String> {
    let value = value.ok_or("Resposta de assinatura sem result")?;
    if value.is_null() {
        return Ok(result);
    }
    let signatures = value
        .get("signatures")
        .and_then(Value::as_array)
        .ok_or("Resposta de assinatura inválida")?;
    result.truncated = signatures.len() > 32;
    let mut total = 0usize;
    for signature in signatures.iter().take(32) {
        let label = signature
            .get("label")
            .and_then(Value::as_str)
            .ok_or("Label de assinatura inválido")?;
        if label.len() > 8192 || label.contains('\0') {
            return Err("Label de assinatura excede limites".into());
        }
        let doc = documentation(signature.get("documentation"))?;
        total += label.len() + doc.as_ref().map_or(0, String::len);
        let mut parameters = Vec::new();
        let mut previous_end = 0;
        if let Some(items) = signature.get("parameters") {
            let items = items
                .as_array()
                .ok_or("Parâmetros de assinatura inválidos")?;
            if items.len() > 128 {
                return Err("Assinatura excede 128 parâmetros".into());
            }
            for item in items {
                let value = item.get("label").ok_or("Parâmetro sem label")?;
                let range = if let Some(text) = value.as_str() {
                    let start = label[previous_end..]
                        .find(text)
                        .map(|i| i + previous_end)
                        .ok_or("Label do parâmetro ausente da assinatura")?;
                    start..start + text.len()
                } else {
                    let pair = value
                        .as_array()
                        .filter(|a| a.len() == 2)
                        .ok_or("Range de parâmetro inválido")?;
                    let start = index(pair.first())?.ok_or("Início de parâmetro inválido")?;
                    let end = index(pair.get(1))?.ok_or("Fim de parâmetro inválido")?;
                    let start =
                        offset(label, start).ok_or("Início UTF-16 de parâmetro inválido")?;
                    let end = offset(label, end).ok_or("Fim UTF-16 de parâmetro inválido")?;
                    if start > end {
                        return Err("Range de parâmetro invertido".into());
                    }
                    start..end
                };
                if range.start < previous_end {
                    return Err("Ranges de parâmetros sobrepostos".into());
                }
                previous_end = range.end;
                let doc = documentation(item.get("documentation"))?;
                total += doc.as_ref().map_or(0, String::len);
                if total > 262144 {
                    return Err("Assinaturas excedem 256 KiB".into());
                }
                parameters.push(SignatureParameter {
                    label_range: range,
                    documentation: doc,
                });
            }
        }
        if total > 262144 {
            return Err("Assinaturas excedem 256 KiB".into());
        }
        result.signatures.push(SignatureInformation {
            label: label.into(),
            documentation: doc,
            parameters,
            active_parameter: index(signature.get("activeParameter"))?,
        });
    }
    let active_signature = index(value.get("activeSignature"))?.unwrap_or(0);
    result.active_signature =
        (!result.signatures.is_empty()).then_some(if active_signature < result.signatures.len() {
            active_signature
        } else {
            0
        });
    let global = index(value.get("activeParameter"))?;
    if let Some(active) = result.active_signature {
        let signature = &result.signatures[active];
        let parameter = signature.active_parameter.or(global).unwrap_or(0);
        result.active_parameter = (!signature.parameters.is_empty()).then_some(
            if parameter < signature.parameters.len() {
                parameter
            } else {
                0
            },
        );
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn result() -> SignatureHelpResult {
        SignatureHelpResult {
            path: "a.ts".into(),
            version: 1,
            line: 0,
            character: 0,
            signatures: vec![],
            active_signature: None,
            active_parameter: None,
            truncated: false,
        }
    }
    #[test]
    fn labels_utf16_overloads_and_parameter_precedence() {
        let r = decode(Some(&json!({"signatures":[{"label":"f(🙂: string, n: number)","parameters":[{"label":[2,12]},{"label":"n: number"}],"activeParameter":1,"documentation":{"kind":"markdown","value":"<script>inert</script>"}},{"label":"f()"}],"activeSignature":0,"activeParameter":0})), result()).unwrap();
        assert_eq!(
            &r.signatures[0].label[r.signatures[0].parameters[0].label_range.clone()],
            "🙂: string"
        );
        assert_eq!(r.active_parameter, Some(1));
        assert_eq!(
            r.signatures[0].documentation.as_deref(),
            Some("<script>inert</script>")
        );
        let r = decode(Some(&json!({"signatures":[{"label":"f(a)","parameters":[{"label":"a"}]}],"activeSignature":99,"activeParameter":99})), result()).unwrap();
        assert_eq!((r.active_signature, r.active_parameter), (Some(0), Some(0)));
        assert!(
            decode(Some(&Value::Null), result())
                .unwrap()
                .signatures
                .is_empty()
        );
    }
    #[test]
    fn malformed_or_excessive_signature_data_is_rejected() {
        for value in [
            json!({"signatures":[{"label":"f(🙂)","parameters":[{"label":[3,4]}]}]}),
            json!({"signatures":[{"label":"f(a)","parameters":[{"label":"absent"}]}]}),
            json!({"signatures":[{"label":"f(a)","parameters":[{"label":[3,2]}]}]}),
            json!({"signatures":[{"label":"x".repeat(8193)}]}),
            json!({"signatures":[{"label":"f()","documentation":{"kind":"html","value":"x"}}]}),
            json!({"signatures":[],"activeSignature":-1}),
            json!({"signatures":[{"label":"f()","parameters":false}]}),
            json!({"signatures":[{"label":"f()","documentation":"x".repeat(32769)}]}),
            json!({"signatures":[{"label":"f()","parameters":vec![json!({"label":""});129]}]}),
            json!({"signatures":vec![json!({"label":"f()","documentation":"x".repeat(32768)});9]}),
        ] {
            assert!(decode(Some(&value), result()).is_err(), "{value}");
        }
        assert!(decode(None, result()).is_err());
        assert!(
            decode(
                Some(&json!({"signatures":vec![json!({"label":"f()"});33]})),
                result()
            )
            .unwrap()
            .truncated
        );
    }
}
