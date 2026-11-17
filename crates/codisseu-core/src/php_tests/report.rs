use super::{MAX_CASES, MAX_REPORT, Outcome, TestCase, local_file, process::inert};
use quick_xml::{Reader, events::Event};
use std::{collections::BTreeMap, path::Path};

const MAX_DETAIL: usize = 64 * 1024;
fn add_details(case: &mut TestCase, value: &str) {
    let remaining = MAX_DETAIL.saturating_sub(case.details.len());
    let mut n = remaining.min(value.len());
    while !value.is_char_boundary(n) {
        n -= 1;
    }
    case.details.push_str(&value[..n]);
}
pub(super) fn parse(bytes: &[u8], root: &Path) -> Result<Vec<TestCase>, String> {
    if bytes.len() as u64 > MAX_REPORT {
        return Err("JUnit excede16MiB".into());
    }
    let input = std::str::from_utf8(bytes).map_err(|_| "JUnit deve ser UTF-8")?;
    let mut reader = Reader::from_str(input);
    reader.config_mut().expand_empty_elements = true;
    reader.config_mut().check_comments = true;
    let mut depth = 0usize;
    let mut roots = 0usize;
    let mut suites: Vec<Option<String>> = Vec::new();
    let mut cases = Vec::new();
    let mut current: Option<TestCase> = None;
    let mut in_detail = false;
    let mut has_outcome = false;
    loop {
        match reader
            .read_event()
            .map_err(|e| format!("JUnit inválido: {e}"))?
        {
            Event::Start(start) => {
                depth += 1;
                if depth > 64 {
                    return Err("JUnit excede profundidade64".into());
                }
                let name = String::from_utf8_lossy(start.local_name().as_ref()).into_owned();
                if depth == 1 {
                    roots += 1;
                    if roots != 1 || !matches!(name.as_str(), "testsuites" | "testsuite") {
                        return Err("Raiz JUnit inválida".into());
                    }
                }
                let mut attrs = BTreeMap::new();
                for attr in start.attributes() {
                    if attrs.len() >= 32 {
                        return Err("JUnit excede32 atributos".into());
                    }
                    let attr = attr.map_err(|e| e.to_string())?;
                    let key = String::from_utf8_lossy(attr.key.as_ref()).into_owned();
                    let value = attr
                        .decoded_and_normalized_value(
                            quick_xml::XmlVersion::Implicit1_0,
                            reader.decoder(),
                        )
                        .map_err(|e| e.to_string())?
                        .into_owned();
                    if value.len() > 4096 {
                        return Err("Atributo JUnit excede4KiB".into());
                    }
                    if attrs.insert(key, value).is_some() {
                        return Err("Atributo JUnit duplicado".into());
                    }
                }
                match name.as_str() {
                    "testsuite" => suites.push(
                        attrs
                            .get("file")
                            .cloned()
                            .or_else(|| suites.last().cloned().flatten()),
                    ),
                    "testcase" => {
                        if current.is_some() || suites.is_empty() || cases.len() >= MAX_CASES {
                            return Err(
                                "Casos JUnit aninhados, fora de suite ou acima do limite".into()
                            );
                        }
                        let name = attrs
                            .get("name")
                            .filter(|s| !s.is_empty())
                            .cloned()
                            .ok_or("Caso JUnit sem nome")?;
                        // Pest appends ::description to file; this is metadata, never a filename.
                        let file = attrs
                            .get("file")
                            .cloned()
                            .or_else(|| suites.last().cloned().flatten());
                        let path = file.and_then(|p| {
                            local_file(root, Path::new(&p)).ok().or_else(|| {
                                p.rmatch_indices("::").find_map(|(index, _)| {
                                    local_file(root, Path::new(&p[..index])).ok()
                                })
                            })
                        });
                        let line = attrs
                            .get("line")
                            .and_then(|s| s.parse::<u32>().ok())
                            .filter(|n| *n > 0);
                        let seconds = attrs
                            .get("time")
                            .and_then(|s| s.parse::<f64>().ok())
                            .filter(|s| s.is_finite() && *s >= 0.)
                            .unwrap_or(0.);
                        has_outcome = false;
                        current = Some(TestCase {
                            ancestors: vec![],
                            title: String::new(),
                            name,
                            class: attrs
                                .get("class")
                                .or_else(|| attrs.get("classname"))
                                .cloned()
                                .unwrap_or_default(),
                            path,
                            line,
                            seconds,
                            outcome: Outcome::Passed,
                            details: String::new(),
                        });
                    }
                    "failure" | "error" | "skipped" => {
                        let case = current.as_mut().ok_or("Estado JUnit fora de um caso")?;
                        if has_outcome {
                            return Err("Múltiplos estados no mesmo teste JUnit".into());
                        }
                        has_outcome = true;
                        case.outcome = match name.as_str() {
                            "failure" => Outcome::Failed,
                            "error" => Outcome::Error,
                            _ => Outcome::Skipped,
                        };
                        for key in ["type", "message"] {
                            if let Some(value) = attrs.get(key) {
                                add_details(case, value);
                                add_details(case, "\n");
                            }
                        }
                        in_detail = true;
                    }
                    "system-out" | "system-err" => in_detail = current.is_some(),
                    _ => {}
                }
            }
            Event::End(end) => {
                if depth == 0 {
                    return Err("JUnit com fechamento inválido".into());
                }
                match end.local_name().as_ref() {
                    b"testcase" => {
                        let mut case = current.take().ok_or("JUnit sem caso aberto")?;
                        case.details = inert(case.details.as_bytes());
                        locate_failure(&mut case, root);
                        cases.push(case);
                    }
                    b"testsuite" => {
                        suites.pop();
                    }
                    b"failure" | b"error" | b"skipped" | b"system-out" | b"system-err" => {
                        in_detail = false
                    }
                    _ => {}
                }
                depth -= 1;
            }
            Event::Text(t) if depth == 0 => {
                if !t.decode().map_err(|e| e.to_string())?.trim().is_empty() {
                    return Err("Texto fora da raiz JUnit".into());
                }
            }
            Event::CData(_) if depth == 0 => return Err("CDATA fora da raiz JUnit".into()),
            Event::Text(t) if in_detail => {
                if let Some(case) = &mut current {
                    add_details(case, &t.decode().map_err(|e| e.to_string())?);
                }
            }
            Event::CData(t) if in_detail => {
                if let Some(case) = &mut current {
                    add_details(case, &t.decode().map_err(|e| e.to_string())?);
                }
            }
            Event::GeneralRef(t) => {
                let text = if let Some(c) = t.resolve_char_ref().map_err(|e| e.to_string())? {
                    c.to_string()
                } else {
                    match t.decode().map_err(|e| e.to_string())?.as_ref() {
                        "amp" => "&".into(),
                        "lt" => "<".into(),
                        "gt" => ">".into(),
                        "quot" => "\"".into(),
                        "apos" => "'".into(),
                        _ => return Err("Entidade JUnit não permitida".into()),
                    }
                };
                if in_detail && let Some(case) = &mut current {
                    add_details(case, &text);
                }
            }
            Event::DocType(_) => return Err("DTD não é permitido em JUnit".into()),
            Event::Eof => break,
            _ => {}
        }
    }
    if depth != 0 || roots != 1 || current.is_some() {
        return Err("JUnit incompleto".into());
    }
    Ok(cases)
}
fn locate_failure(case: &mut TestCase, root: &Path) {
    if !case.outcome.is_failure() {
        return;
    }
    for line in case.details.lines() {
        let line = line.trim().strip_prefix("at ").unwrap_or(line.trim());
        if let Some((path, number)) = line.rsplit_once(':')
            && let Ok(number) = number.parse::<u32>()
            && number > 0
            && let Ok(path) = local_file(root, Path::new(path))
            && (case.path.as_ref() == Some(&path) || case.path.is_none())
        {
            case.path = Some(path);
            case.line = Some(number);
            return;
        }
    }
}
