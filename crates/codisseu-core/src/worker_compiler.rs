//! Compiler fallback for exact TypeScript proposals unsupported by the nominal checker.
use super::*;
use serde::Deserialize;
use std::{fs, io::Read, path::Path, sync::atomic::AtomicBool};

/// Identity as well as contents participate in Preview/Apply revalidation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Configuration {
    name: &'static str,
    text: String,
}
#[derive(Clone, Deserialize, PartialEq, Eq)]
struct WireRange {
    start_line: u32,
    start_character: u32,
    end_line: u32,
    end_character: u32,
}
impl From<WireRange> for TextRange {
    fn from(r: WireRange) -> Self {
        Self {
            start_line: r.start_line,
            start_character: r.start_character,
            end_line: r.end_line,
            end_character: r.end_character,
        }
    }
}
#[derive(Deserialize)]
struct Location {
    path: PathBuf,
    range: WireRange,
}
#[derive(Deserialize)]
struct Finding {
    path: PathBuf,
    range: Option<WireRange>,
    message: String,
    related: Option<Location>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    coverage: String,
    conflicts: Vec<Finding>,
    issues: Vec<Finding>,
    note: String,
    compiler_version: String,
    #[serde(default)]
    checked_declarations: usize,
    #[serde(default)]
    missing_references: Vec<Location>,
}

pub(super) fn fallback(
    report: &mut RenameValidation,
    snapshots: &[codisseu_syntax::RenameSnapshot],
    new_name: &str,
    cancel: &AtomicBool,
    root: &Path,
    search: &mut rename::RenameSearch,
) {
    search.compiler_missing.clear();
    if report.coverage != Coverage::Unsupported
        || !report.conflicts.is_empty()
        || Language::for_path(&search.origin) != Some(Language::TypeScript)
        || report.issues.iter().any(|i| {
            matches!(
                i.kind,
                IssueKind::Limit
                    | IssueKind::Cancelled
                    | IssueKind::InvalidSnapshot
                    | IssueKind::SyntaxBefore
                    | IssueKind::Parser
            )
        })
    {
        return;
    }
    let Some(runtime) = search.compiler.clone() else {
        return;
    };
    let verified = (|| -> Result<Report, String> {
        validate_structure(root, snapshots, cancel)?;
        let config = read_config(root, snapshots)?;
        if let Some(previous) = &search.compiler_config {
            if previous != &config {
                return Err("Configuração TypeScript mudou; refaça preview".into());
            }
        } else {
            search.compiler_config = Some(config.clone());
        }
        let value = runtime.verify(root, serde_json::json!({
            "root": root, "origin": {"path": search.origin, "line": search.line, "character":search.character},
            "newName":new_name, "configText": config.as_ref().map(|config| &config.text),
            "configName": config.as_ref().map_or("tsconfig.json", |config| config.name),
            "snapshots":snapshots.iter().map(|s| serde_json::json!({"path":s.path,"before":s.before.as_ref(),"after":s.after.as_ref(),"edits":s.edits.iter().map(|e|serde_json::json!({"range":e.range,"new_text":e.new_text})).collect::<Vec<_>>()})).collect::<Vec<_>>()
        }), cancel)?;
        if read_config(root, snapshots)? != config {
            return Err(
                "Configuração TypeScript mudou durante a verificação; refaça preview".into(),
            );
        }
        let parsed: Report = serde_json::from_value(value)
            .map_err(|e| format!("Resposta TypeScript inválida: {e}"))?;
        if parsed.conflicts.len() + parsed.issues.len() > 256
            || parsed.missing_references.len() > 256
            || parsed.note.len() > 8192
            || parsed.compiler_version.len() > 100
        {
            return Err("Relatório TypeScript excede limites".into());
        }
        for finding in parsed.conflicts.iter().chain(&parsed.issues) {
            if finding.message.len() > 8192
                || !finding.path.starts_with(root)
                || finding
                    .related
                    .as_ref()
                    .is_some_and(|r| !r.path.starts_with(root))
            {
                return Err("Relatório TypeScript inválido".into());
            }
        }
        for location in &parsed.missing_references {
            let snapshot = snapshots
                .iter()
                .find(|s| s.path == location.path)
                .ok_or("Referência omitida fora dos snapshots")?;
            let start = utf16_byte_offset(
                &snapshot.before,
                location.range.start_line,
                location.range.start_character,
            )
            .ok_or("Início de referência omitida inválido")?;
            let end = utf16_byte_offset(
                &snapshot.before,
                location.range.end_line,
                location.range.end_character,
            )
            .ok_or("Fim de referência omitida inválido")?;
            if start >= end
                || !parsed
                    .conflicts
                    .iter()
                    .any(|f| f.path == location.path && f.range.as_ref() == Some(&location.range))
            {
                return Err("Referência omitida não corresponde ao conflito comprovado".into());
            }
        }
        Ok(parsed)
    })();
    match verified {
        Err(message) => {
            report.issues.truncate(255);
            report.issues.push(ValidationIssue {
                path: search.origin.clone(),
                range: None,
                message,
                kind: IssueKind::UnsupportedClass,
            });
        }
        Ok(result) => {
            let missing: Vec<rename::RenameQueryTarget> = result
                .missing_references
                .into_iter()
                .map(|location| rename::RenameQueryTarget {
                    path: location.path,
                    range: location.range.into(),
                })
                .collect();
            let coverage = match result.coverage.as_str() {
                "Verified" if result.conflicts.is_empty() && result.issues.is_empty() => {
                    Coverage::Verified
                }
                "Rejected" if !result.conflicts.is_empty() => Coverage::Rejected,
                _ => Coverage::Unsupported,
            };
            if coverage == Coverage::Unsupported
                && result.issues.is_empty()
                && result.conflicts.is_empty()
            {
                return;
            }
            report.coverage = coverage;
            report.scope = ValidationScope::CompilerBindings;
            report.related_methods.clear();
            report.issues = result
                .issues
                .into_iter()
                .map(|f| ValidationIssue {
                    path: f.path,
                    range: f.range.map(Into::into),
                    message: f.message,
                    kind: IssueKind::UnsupportedClass,
                })
                .collect();
            report.conflicts = result
                .conflicts
                .into_iter()
                .map(|f| {
                    let range = f.range.map(Into::into).unwrap_or(TextRange {
                        start_line: 0,
                        start_character: 0,
                        end_line: 0,
                        end_character: 0,
                    });
                    let kind = if missing.iter().any(|m| m.path == f.path && m.range == range) {
                        ConflictKind::MissingReference
                    } else {
                        ConflictKind::BindingChanged
                    };
                    Conflict {
                        path: f.path,
                        range,
                        message: f.message,
                        related: f.related.map(|r| ConflictLocation {
                            path: r.path,
                            range: r.range.into(),
                        }),
                        kind,
                        container_name: None,
                    }
                })
                .collect();
            if report.coverage == Coverage::Rejected
                && report.issues.is_empty()
                && report
                    .conflicts
                    .iter()
                    .all(|c| c.kind == ConflictKind::MissingReference)
            {
                search.compiler_missing = missing;
            }
            report.checked_files = snapshots.len();
            report.checked_declarations = result.checked_declarations;
            report.note = format!(
                "TypeScript {}: {} Corpus limitado aos snapshots; não prova referências externas ou reflexão em runtime.",
                result.compiler_version, result.note
            );
        }
    }
}
fn validate_structure(
    root: &Path,
    snapshots: &[codisseu_syntax::RenameSnapshot],
    cancel: &AtomicBool,
) -> Result<(), String> {
    use std::collections::HashSet;
    use std::path::Component;
    use std::sync::atomic::Ordering;
    if snapshots.is_empty() || snapshots.len() > 64 {
        return Err("Corpus do compilador exige 1..64 arquivos".into());
    }
    let mut paths = HashSet::new();
    let mut bytes = 0usize;
    let mut count = 0usize;
    for snapshot in snapshots {
        if cancel.load(Ordering::Acquire) {
            return Err("Verificação cancelada".into());
        }
        if !snapshot.path.is_absolute()
            || !snapshot.path.starts_with(root)
            || snapshot
                .path
                .components()
                .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
            || !paths.insert(&snapshot.path)
        {
            return Err("Caminho inválido/duplicado no corpus do compilador".into());
        }
        bytes = bytes
            .saturating_add(snapshot.before.len())
            .saturating_add(snapshot.after.len());
        count = count.saturating_add(snapshot.edits.len());
        if snapshot.before.len() > 1024 * 1024
            || snapshot.after.len() > 1024 * 1024
            || bytes > 8 * 1024 * 1024
            || count > 1024
        {
            return Err("Corpus do compilador excede limites de bytes/edições".into());
        }
        let edits: Vec<_> = snapshot
            .edits
            .iter()
            .map(|e| RenameTextEdit {
                start_line: e.range.start_line,
                start_character: e.range.start_character,
                end_line: e.range.end_line,
                end_character: e.range.end_character,
                new_text: e.new_text.clone(),
            })
            .collect();
        if edits.iter().any(|e| e.new_text.len() > 4096)
            || (if edits.is_empty() {
                snapshot.before.as_ref() != snapshot.after.as_ref()
            } else {
                rename::build_after(&snapshot.before, &edits)?.as_str() != snapshot.after.as_ref()
            })
        {
            return Err("Snapshot posterior diverge da proposta exata".into());
        }
    }
    Ok(())
}
pub(super) fn recheck_configuration(
    root: &Path,
    snapshots: &[codisseu_syntax::RenameSnapshot],
    search: &rename::RenameSearch,
) -> Result<(), String> {
    if let Some(expected) = &search.compiler_config
        && &read_config(root, snapshots)? != expected
    {
        return Err("Configuração TypeScript mudou durante a verificação; refaça preview".into());
    }
    Ok(())
}
fn read_config(
    root: &Path,
    snapshots: &[codisseu_syntax::RenameSnapshot],
) -> Result<Option<Configuration>, String> {
    for s in snapshots
        .iter()
        .filter(|s| Language::for_path(&s.path) == Some(Language::TypeScript))
    {
        let mut parent = s.path.parent();
        while let Some(dir) = parent.filter(|dir| *dir != root && dir.starts_with(root)) {
            if dir
                .join("tsconfig.json")
                .try_exists()
                .map_err(|e| e.to_string())?
                || dir
                    .join("jsconfig.json")
                    .try_exists()
                    .map_err(|e| e.to_string())?
            {
                return Err("Configuração TypeScript aninhada exige corpus próprio".into());
            }
            parent = dir.parent();
        }
    }
    // TypeScript's project lookup prefers tsconfig when both are present.
    // Missing files remain a captured state, so later creation is detected.
    for name in ["tsconfig.json", "jsconfig.json"] {
        let path = root.join(name);
        let file = match fs::File::open(&path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.to_string()),
        };
        if path.canonicalize().map_err(|e| e.to_string())? != path
            || !file.metadata().map_err(|e| e.to_string())?.is_file()
        {
            return Err("Configuração JavaScript/TypeScript não é arquivo regular local".into());
        }
        let mut text = String::new();
        file.take(128 * 1024 + 1)
            .read_to_string(&mut text)
            .map_err(|e| e.to_string())?;
        if text.len() > 128 * 1024 {
            return Err("Configuração JavaScript/TypeScript excede 128 KiB".into());
        }
        return Ok(Some(Configuration { name, text }));
    }
    Ok(None)
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::{
        sync::Arc,
        thread,
        time::{Duration, Instant},
    };

    #[test]
    fn named_configuration_and_missing_configuration_are_private_apply_snapshots() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let snapshots = vec![codisseu_syntax::RenameSnapshot {
            path: root.join("source.js"),
            before: Arc::from("class Weather {}"),
            after: Arc::from("class Weather {}"),
            edits: vec![],
        }];
        let missing = read_config(&root, &snapshots).unwrap();
        assert!(missing.is_none());
        let mut search = rename::RenameSearch {
            compiler_config: Some(missing),
            ..Default::default()
        };
        fs::write(root.join("jsconfig.json"), "{}").unwrap();
        assert!(recheck_configuration(&root, &snapshots, &search).is_err());
        let js = read_config(&root, &snapshots).unwrap();
        assert_eq!(js.as_ref().unwrap().name, "jsconfig.json");
        search.compiler_config = Some(js.clone());
        assert!(recheck_configuration(&root, &snapshots, &search).is_ok());
        fs::write(root.join("tsconfig.json"), "{}").unwrap();
        let ts = read_config(&root, &snapshots).unwrap();
        assert_eq!(ts.as_ref().unwrap().name, "tsconfig.json");
        assert_ne!(
            js, ts,
            "equal bytes under another config name have different semantics"
        );
        assert!(recheck_configuration(&root, &snapshots, &search).is_err());
        fs::remove_file(root.join("tsconfig.json")).unwrap();
        fs::write(
            root.join("jsconfig.json"),
            "{\"compilerOptions\":{\"checkJs\":false}}",
        )
        .unwrap();
        assert!(recheck_configuration(&root, &snapshots, &search).is_err());
        fs::remove_file(root.join("jsconfig.json")).unwrap();
        assert!(recheck_configuration(&root, &snapshots, &search).is_err());
    }

    #[test]
    fn javascript_configuration_keeps_local_nested_and_size_guards() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        fs::create_dir(&root).unwrap();
        let source = codisseu_syntax::RenameSnapshot {
            path: root.join("source.js"),
            before: Arc::from("class Weather {}"),
            after: Arc::from("class Weather {}"),
            edits: vec![],
        };
        let outside = dir.path().join("outside.json");
        fs::write(&outside, "{}").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("jsconfig.json")).unwrap();
        assert!(read_config(&root, std::slice::from_ref(&source)).is_err());
        fs::remove_file(root.join("jsconfig.json")).unwrap();
        fs::write(root.join("jsconfig.json"), " ".repeat(128 * 1024 + 1)).unwrap();
        assert!(read_config(&root, std::slice::from_ref(&source)).is_err());
        fs::write(root.join("jsconfig.json"), "{}").unwrap();
        fs::create_dir(root.join("nested")).unwrap();
        fs::write(root.join("nested/jsconfig.json"), "{}").unwrap();
        let nested = codisseu_syntax::RenameSnapshot {
            path: root.join("nested/source.js"),
            ..source
        };
        assert!(
            read_config(&root, &[nested])
                .unwrap_err()
                .contains("aninhada")
        );
    }
    #[test]
    fn exact_structure_accepts_shorthand_and_rejects_forgery_and_duplicates() {
        let root = Path::new("/workspace");
        let snapshot = codisseu_syntax::RenameSnapshot {
            path: root.join("weather.ts"),
            before: Arc::from("const box = { rainfall };"),
            after: Arc::from("const box = { rainfall: precipitation };"),
            edits: vec![codisseu_syntax::RenameEdit {
                range: TextRange {
                    start_line: 0,
                    start_character: 14,
                    end_line: 0,
                    end_character: 22,
                },
                new_text: "rainfall: precipitation".into(),
            }],
        };
        let cancel = AtomicBool::new(false);
        assert!(validate_structure(root, std::slice::from_ref(&snapshot), &cancel).is_ok());
        let mut unchanged = snapshot.clone();
        unchanged.edits.clear();
        unchanged.after = unchanged.before.clone();
        assert!(validate_structure(root, &[unchanged.clone()], &cancel).is_ok());
        unchanged.after = Arc::from("modified without edits");
        assert!(validate_structure(root, &[unchanged], &cancel).is_err());
        let mut forged = snapshot.clone();
        forged.after = Arc::from("const box = { precipitation }; unexpected();");
        assert!(validate_structure(root, &[forged], &cancel).is_err());
        assert!(validate_structure(root, &[snapshot.clone(), snapshot.clone()], &cancel).is_err());
        let mut overlap = snapshot.clone();
        overlap.edits.push(overlap.edits[0].clone());
        assert!(validate_structure(root, &[overlap], &cancel).is_err());
        let mut outside = snapshot.clone();
        outside.path = root.join("../outside.ts");
        assert!(validate_structure(root, &[outside], &cancel).is_err());
        cancel.store(true, std::sync::atomic::Ordering::Release);
        assert!(validate_structure(root, &[snapshot], &cancel).is_err());
    }
    #[test]
    #[ignore = "real TS process/config race; CODISSEU_PROVIDER_CONFIG"]
    fn configuration_changed_while_compiler_runs_never_verifies() {
        let configs: Vec<ProviderConfig> = serde_json::from_slice(
            &fs::read(std::env::var_os("CODISSEU_PROVIDER_CONFIG").unwrap()).unwrap(),
        )
        .unwrap();
        let runtime = configs
            .iter()
            .find_map(codisseu_language::semantic::TypeScriptRuntime::from_config)
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let path = root.join("source.ts");
        let config = root.join("tsconfig.json");
        fs::write(&config, "{}").unwrap();
        let source = "export class Weather<T> { rainfall(v: T): T { return v; } } const w = new Weather<string>(); w.rainfall('x');";
        let edits = source
            .match_indices("rainfall")
            .map(|(i, _)| codisseu_syntax::RenameEdit {
                range: TextRange {
                    start_line: 0,
                    start_character: i as u32,
                    end_line: 0,
                    end_character: (i + 8) as u32,
                },
                new_text: "forecast".into(),
            })
            .collect();
        let snapshots = vec![codisseu_syntax::RenameSnapshot {
            path: path.clone(),
            before: Arc::from(source),
            after: Arc::from(source.replace("rainfall", "forecast")),
            edits,
        }];
        let cancel = AtomicBool::new(false);
        let mut report = codisseu_syntax::validate_rename(&snapshots, "forecast", &cancel);
        assert_eq!(report.coverage, Coverage::Unsupported);
        let mut search = rename::RenameSearch {
            compiler: Some(Arc::new(runtime)),
            origin: path,
            line: 0,
            character: source.find("rainfall").unwrap() as u32 + 1,
            ..Default::default()
        };
        // Observe only children of this test process. Spawn implies the first
        // configuration snapshot was already read; mutation then occurs while
        // the real compiler is running, not before its initial guard.
        let mutation = thread::spawn(move || {
            let start = Instant::now();
            while start.elapsed() < Duration::from_secs(5) {
                for task in fs::read_dir("/proc/self/task").unwrap().flatten() {
                    let children =
                        fs::read_to_string(task.path().join("children")).unwrap_or_default();
                    for pid in children.split_whitespace() {
                        let command = fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
                        if command
                            .windows(b"--max-old-space-size=256".len())
                            .any(|w| w == b"--max-old-space-size=256")
                        {
                            fs::write(&config, "{\"compilerOptions\":{\"strict\":true}}").unwrap();
                            return true;
                        }
                    }
                }
                thread::sleep(Duration::from_millis(5));
            }
            false
        });
        fallback(
            &mut report,
            &snapshots,
            "forecast",
            &cancel,
            &root,
            &mut search,
        );
        assert!(
            mutation.join().unwrap(),
            "compiler subprocess was not observed"
        );
        assert_eq!(report.coverage, Coverage::Unsupported, "{report:?}");
        assert!(
            report
                .issues
                .iter()
                .any(|i| i.message.contains("mudou durante")),
            "{report:?}"
        );
    }
}
