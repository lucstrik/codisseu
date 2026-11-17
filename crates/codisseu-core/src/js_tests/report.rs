use super::*;
use serde_json::Value;
fn text(value: &Value, field: &str) -> String {
    value[field].as_str().unwrap_or_default().to_owned()
}
fn limited(mut s: String) -> String {
    if s.len() > 65536 {
        let mut n = 65536;
        while !s.is_char_boundary(n) {
            n -= 1;
        }
        s.truncate(n);
        s.push_str("\n[detalhe limitado]");
    }
    process::inert(s.as_bytes())
}
pub(super) fn parse(
    bytes: &[u8],
    root: &Path,
    framework: Framework,
) -> Result<Vec<TestCase>, String> {
    let json: Value =
        serde_json::from_slice(bytes).map_err(|e| format!("JSON de testes inválido: {e}"))?;
    let files = json["testResults"]
        .as_array()
        .ok_or("JSON sem testResults")?;
    let vitest5 = framework == Framework::Vitest
        && bounded(&root.join("node_modules/vitest/package.json"), 65536)
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .and_then(|j| {
                j["version"]
                    .as_str()?
                    .split('.')
                    .next()?
                    .parse::<u32>()
                    .ok()
            })
            .is_some_and(|major| major >= 5);
    let mut cases = vec![];
    for file in files {
        let filename = text(file, "name");
        let path = local_file(root, Path::new(&filename)).ok();
        let assertions = file["assertionResults"]
            .as_array()
            .ok_or("Resultado sem assertionResults")?;
        if assertions.is_empty() && !text(file, "message").is_empty() {
            return Err(limited(text(file, "message")));
        }
        for case in assertions {
            if cases.len() >= 10_000 {
                return Err("Relatório excede10000 testes".into());
            }
            let name = if vitest5 {
                let mut segments = case["ancestorTitles"]
                    .as_array()
                    .ok_or("Vitest sans ancestorTitles")?
                    .iter()
                    .map(|v| v.as_str().ok_or("Nome de suite inválido"))
                    .collect::<Result<Vec<_>, _>>()?;
                segments.push(case["title"].as_str().ok_or("Vitest sans title")?);
                segments.join(" > ")
            } else {
                text(case, "fullName")
            };
            if name.is_empty() || name.len() > 16384 || name.contains('\0') {
                return Err("Nome de teste inválido".into());
            }
            let outcome = match case["status"].as_str() {
                Some("passed") => Outcome::Passed,
                Some("failed") => Outcome::Failed,
                Some("pending" | "todo" | "skipped" | "disabled") => Outcome::Skipped,
                _ => return Err("Estado de teste desconhecido no JSON".into()),
            };
            let details = limited(
                case["failureMessages"]
                    .as_array()
                    .map(|v| {
                        v.iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .unwrap_or_default(),
            );
            let line = case
                .pointer("/location/line")
                .and_then(Value::as_u64)
                .and_then(|v| u32::try_from(v).ok())
                .filter(|n| *n > 0 && *n <= 1_000_000)
                .or_else(|| {
                    // Stack locations are used only for the canonical test file itself.
                    path.as_ref().and_then(|p| {
                        let prefix = format!("{}:", p.display());
                        details.lines().find_map(|l| {
                            l.split_once(&prefix)
                                .and_then(|(_, tail)| tail.split(':').next()?.parse::<u32>().ok())
                                .filter(|n| *n > 0 && *n <= 1_000_000)
                        })
                    })
                });
            let class = path
                .as_ref()
                .and_then(|p| p.strip_prefix(root).ok())
                .map_or_else(|| filename.clone(), |p| p.display().to_string());
            let ancestors = case["ancestorTitles"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if ancestors.len() > 64 || ancestors.iter().any(|s| s.len() > 4096) {
                return Err("Hierarquia de testes excede limites".into());
            }
            cases.push(TestCase {
                ancestors,
                title: text(case, "title"),
                name,
                class,
                path: path.clone(),
                line,
                outcome,
                seconds: case["duration"]
                    .as_f64()
                    .filter(|n| n.is_finite() && *n >= 0.)
                    .unwrap_or(0.)
                    / 1000.,
                details,
            });
        }
    }
    if json["numRuntimeErrorTestSuites"].as_u64().unwrap_or(0) > 0 {
        return Err("Erro de inicialização em uma suite; confira console".into());
    }
    if json["success"] == false && !cases.iter().any(|c| c.outcome.is_failure()) {
        return Err("Runner informou execução sem sucesso; confira console".into());
    }
    Ok(cases)
}
