//! Worker-only AST resource proof and immutable preview revalidation.
use super::*;

pub(super) fn preview(
    workspace: PathBuf,
    mut search: rename::RenameSearch,
    new_name: String,
    buffers: Vec<(PathBuf, Arc<str>)>,
    cancel: &std::sync::atomic::AtomicBool,
    token: &Cancellation<'_>,
) -> Result<rename::PreparedRename, String> {
    let root = fs::canonicalize(workspace).map_err(|e| e.to_string())?;
    let started = std::time::Instant::now();
    let corpus = collect_rename_corpus(&root, token, cancel, started)?;
    if let Some(error) = &corpus.limitation {
        return Err(error.clone());
    }
    if !corpus.paths.contains(&search.origin) {
        return Err("Arquivo de origem fora do corpus local".into());
    }
    let mut snapshots = vec![];
    let mut bytes = 0;
    for path in &corpus.paths {
        reference_check(token, cancel)?;
        if started.elapsed() > std::time::Duration::from_secs(2) {
            return Err("Snapshots de arquivo excederam 2 segundos".into());
        }
        let text = if let Some((_, text)) = buffers.iter().find(|(p, _)| p == path) {
            text.clone()
        } else {
            Arc::from(read_semantic_document(path, token)?.text)
        };
        bytes += text.len() * 2;
        if text.len() > MAX_DOCUMENT_BYTES || bytes > 8 * 1024 * 1024 {
            return Err("Snapshots excedem 1 MiB/8 MiB".into());
        }
        snapshots.push(codisseu_syntax::RenameSnapshot {
            path: path.clone(),
            before: text.clone(),
            after: text,
            edits: vec![],
        });
    }
    let text_buffers = buffers
        .iter()
        .map(|(p, t)| (p.clone(), 0, t.clone()))
        .collect::<Vec<_>>();
    search.text_snapshots = collect_search_snapshots(&root, &search, &text_buffers, token, cancel)?;
    let (files, resources, validation) = plan(
        &root,
        &snapshots,
        &mut search,
        &new_name,
        None,
        cancel,
        token,
    )?;
    Ok(rename::PreparedRename {
        search,
        files,
        snapshots,
        validation,
        corpus,
        resources,
    })
}

fn plan(
    root: &Path,
    snapshots: &[codisseu_syntax::RenameSnapshot],
    search: &mut rename::RenameSearch,
    new_name: &str,
    expected: Option<&[rename::resources::ResourceMove]>,
    cancel: &std::sync::atomic::AtomicBool,
    token: &Cancellation<'_>,
) -> Result<
    (
        Vec<RenamePreviewFile>,
        Vec<rename::resources::ResourceMove>,
        RenameValidation,
    ),
    String,
> {
    codisseu_syntax::validate_file_rename_name(new_name).map_err(str::to_owned)?;
    if snapshots
        .iter()
        .any(|s| !s.edits.is_empty() || s.before != s.after)
    {
        return Err("Prova de recurso exige snapshots originais sem edições".into());
    }
    let source = search.origin.clone();
    let target = source.with_file_name(new_name);
    if target == source || target.extension() != source.extension() {
        return Err("Destino exige outro nome e mesma extensão".into());
    }
    let expected_disk = if let Some(previous) = expected {
        if previous.len() != 1 || previous[0].source != source || previous[0].target != target {
            return Err("Destino privado divergiu do preview".into());
        }
        previous[0].expected_disk.clone()
    } else {
        Arc::from(read_semantic_document(&source, token)?.text.into_bytes())
    };
    let resources = vec![rename::resources::ResourceMove {
        source: source.clone(),
        target: target.clone(),
        expected_disk,
    }];
    let references = search.options.search_file_references;
    let mut validation = RenameValidation {
        coverage:Coverage::Verified,
        scope:if references {ValidationScope::ResourceReferences} else {ValidationScope::ResourceOperationOnly},
        conflicts:vec![],issues:vec![],checked_declarations:0,checked_override_pairs:0,
        related_methods:vec![],checked_files:snapshots.len(),incremental_reparses:0,
        note:if references {"Arquivo local: literais de import/export/require/include relativos comprovados por AST no corpus declarado; identificadores preservados. Não verifica reflexão ou módulos externos."}
            else {"Somente recurso: Search for references desmarcado. Imports/includes e configurações NÃO serão atualizados e podem deixar de resolver. A aprovação verifica apenas a operação de arquivo e textos opcionais."}.into(),
    };
    let mut all = snapshots.to_vec();
    all.extend(search.text_snapshots.iter().cloned());
    let mut edits = vec![];
    if references {
        let source_text = &snapshots
            .iter()
            .find(|s| s.path == source)
            .ok_or("Origem fora do corpus")?
            .before;
        let result = codisseu_syntax::validate_standalone_file_source(
            &source,
            source_text,
            cancel,
            codisseu_syntax::ValidationLimits::default(),
        )
        .and_then(|()| {
            codisseu_syntax::plan_named_file_rename(
                snapshots,
                &source,
                &target,
                cancel,
                codisseu_syntax::ValidationLimits::default(),
            )
        });
        match result {
            Ok(Some(proposal)) => edits = proposal.edits,
            Ok(None) => return Err("Rename de arquivo não produziu movimento".into()),
            Err(message) => validation.issues.push(ValidationIssue {
                path: source.clone(),
                range: None,
                kind: IssueKind::UnsupportedEdit,
                message,
            }),
        }
        let config_result = if search.resource_config_dirty {
            Err("Salve as configurações de módulos/autoload antes de mover arquivos".into())
        } else if expected.is_some() {
            rename::resource_config::recheck(&search.resource_configs)
        } else {
            rename::resource_config::capture(root, snapshots, &resources).map(|configs| {
                search.resource_configs = Arc::new(configs);
            })
        };
        if let Err(message) = config_result {
            validation.issues.push(ValidationIssue {
                path: source.clone(),
                range: None,
                kind: IssueKind::UnsupportedEdit,
                message,
            });
        }
    } else {
        search.resource_configs = Arc::default();
    }
    for (path, edit) in &edits {
        all.iter_mut()
            .find(|s| s.path == *path)
            .ok_or("Módulo fora do corpus")?
            .edits
            .push(edit.clone());
    }
    if search.options.search_comments_strings || search.options.search_text {
        match codisseu_syntax::plan_file_noncode_rename(
            &all,
            &source,
            &search.old,
            new_name,
            search.options,
            cancel,
            codisseu_syntax::ValidationLimits::default(),
        ) {
            Ok(optional) => edits.extend(optional),
            Err(message) => validation.issues.push(ValidationIssue {
                path: source.clone(),
                range: None,
                kind: IssueKind::UnsupportedEdit,
                message,
            }),
        }
    }
    let mut files = vec![];
    for (path, edit) in edits {
        if !files.iter().any(|f: &RenamePreviewFile| f.path == path) {
            let before = all
                .iter()
                .find(|s| s.path == path)
                .ok_or("Texto fora do corpus")?
                .before
                .clone();
            files.push(RenamePreviewFile {
                encoding: FileEncoding::Utf8,
                path: path.clone(),
                before: before.clone(),
                after: before,
                edits: vec![],
                presentation: vec![],
            });
        }
        files
            .iter_mut()
            .find(|f| f.path == path)
            .unwrap()
            .edits
            .push(syntax_edit(&edit));
    }
    for file in &mut files {
        file.edits
            .sort_by_key(|e| (e.start_line, e.start_character, e.end_line, e.end_character));
        file.after = Arc::from(rename::build_after(&file.before, &file.edits)?);
    }
    if let Err(message) = rename::resources::ResourcePlan::prepare(root, &resources) {
        validation.issues.push(ValidationIssue {
            path: source,
            range: None,
            kind: IssueKind::UnsupportedEdit,
            message,
        });
    }
    if !validation.issues.is_empty() {
        validation.coverage = Coverage::Unsupported;
    }
    if files.len() > 32
        || files.iter().map(|f| f.edits.len()).sum::<usize>() > 1024
        || files
            .iter()
            .map(|f| f.before.len() + f.after.len())
            .sum::<usize>()
            > 8 * 1024 * 1024
    {
        return Err("Rename de módulo excede 32 arquivos/1024 edições/8 MiB".into());
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    present_rename_files(&mut files, cancel);
    reference_check(token, cancel)?;
    Ok((files, resources, validation))
}

fn recheck(
    root: &Path,
    snapshots: &[codisseu_syntax::RenameSnapshot],
    corpus: &rename::RenameCorpus,
    open: &[PathBuf],
    cancel: &std::sync::atomic::AtomicBool,
    token: &Cancellation<'_>,
) -> Result<(), String> {
    let started = std::time::Instant::now();
    let current = collect_rename_corpus(root, token, cancel, started)?;
    if corpus.limitation.is_some()
        || current.limitation.is_some()
        || current.paths != corpus.paths
        || snapshots.len() != corpus.paths.len()
        || snapshots
            .iter()
            .zip(&corpus.paths)
            .any(|(s, p)| s.path != *p)
    {
        return Err("Corpus de Rename de arquivo mudou ou está incompleto".into());
    }
    for s in snapshots {
        reference_check(token, cancel)?;
        if started.elapsed() > std::time::Duration::from_secs(2) {
            return Err("Revalidação de arquivo excedeu 2 segundos".into());
        }
        if fs::canonicalize(&s.path).map_err(|e| e.to_string())? != s.path
            || !s.path.starts_with(root)
        {
            return Err("Caminho de contexto mudou".into());
        }
        if !open.contains(&s.path) {
            let actual = read_semantic_document(&s.path, token)?;
            if actual.path != s.path || actual.text.as_str() != s.before.as_ref() {
                return Err("Arquivo mudou externamente; refaça preview".into());
            }
        }
    }
    Ok(())
}

fn recheck_text(
    root: &Path,
    search: &rename::RenameSearch,
    open: &[PathBuf],
    cancel: &std::sync::atomic::AtomicBool,
    token: &Cancellation<'_>,
) -> Result<(), String> {
    let started = std::time::Instant::now();
    if search.options.search_text && search.options.scope == RenameSearchScope::Project {
        let current = collect_text_corpus(root, token, cancel, started)?;
        if current.limitation.is_some()
            || current.paths
                != search
                    .text_snapshots
                    .iter()
                    .map(|s| s.path.clone())
                    .collect::<Vec<_>>()
        {
            return Err("Corpus textual mudou; refaça preview".into());
        }
    }
    for snapshot in &search.text_snapshots {
        reference_check(token, cancel)?;
        if started.elapsed() > std::time::Duration::from_secs(2) {
            return Err("Revalidação textual excedeu 2 segundos".into());
        }
        if !open.contains(&snapshot.path) {
            let actual = read_semantic_document(&snapshot.path, token)?;
            if actual.path != snapshot.path || actual.text.as_str() != snapshot.before.as_ref() {
                return Err("Texto opcional mudou externamente; refaça preview".into());
            }
        }
    }
    Ok(())
}

pub(super) fn verify(
    workspace: PathBuf,
    mut prepared: rename::PreparedRename,
    new_name: &str,
    open: &[PathBuf],
    cancel: &std::sync::atomic::AtomicBool,
    token: &Cancellation<'_>,
) -> Result<rename::PreparedRename, String> {
    let root = fs::canonicalize(workspace).map_err(|e| e.to_string())?;
    recheck(
        &root,
        &prepared.snapshots,
        &prepared.corpus,
        open,
        cancel,
        token,
    )?;
    recheck_text(&root, &prepared.search, open, cancel, token)?;
    let (files, resources, validation) = plan(
        &root,
        &prepared.snapshots,
        &mut prepared.search,
        new_name,
        Some(&prepared.resources),
        cancel,
        token,
    )?;
    if files != prepared.files || resources != prepared.resources {
        return Err("Proposta de recurso mudou; refaça preview".into());
    }
    recheck(
        &root,
        &prepared.snapshots,
        &prepared.corpus,
        open,
        cancel,
        token,
    )?;
    if prepared.search.options.search_file_references {
        rename::resource_config::recheck(&prepared.search.resource_configs)?;
    }
    recheck_text(&root, &prepared.search, open, cancel, token)?;
    prepared.validation = validation;
    Ok(prepared)
}
