#[path = "worker_file_rename.rs"]
mod file_rename;
#[path = "worker_project_files.rs"]
mod project_files_worker;
use super::*;
use codisseu_editor::{MAX_DOCUMENT_BYTES, MAX_EDITOR_DOCUMENT_BYTES};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::Path,
};

const MAX_SESSION_BYTES: usize = 128 * 1024;

struct Cancellation<'a> {
    workspace: Option<&'a Path>,
    encoding_profile: Option<&'a Path>,
    generation: &'a AtomicU64,
    expected: u64,
}
impl Cancellation<'_> {
    fn check(&self) -> Result<(), String> {
        if self.generation.load(Ordering::SeqCst) != self.expected {
            Err("Operação cancelada antes de concluir".into())
        } else {
            Ok(())
        }
    }
}

pub(super) fn run(
    jobs: Receiver<Job>,
    replies: SyncSender<Reply>,
    generation: Arc<AtomicU64>,
    encoding_profile: PathBuf,
) {
    while let Ok(job) = jobs.recv() {
        let token = Cancellation {
            encoding_profile: Some(&encoding_profile),
            workspace: job.workspace.as_deref(),
            generation: &generation,
            expected: job.generation,
        };
        let result = token
            .check()
            .and_then(|()| execute(job.work, &token))
            .and_then(|mut completed| {
                if let Completed::Rename { prepared, .. } = &mut completed {
                    for file in &mut prepared.files {
                        file.encoding = if file.before.starts_with('\u{feff}') {
                            FileEncoding::Utf8
                        } else {
                            encoding::resolve_for(
                                token.encoding_profile,
                                &file.path,
                                token.workspace,
                            )?
                        };
                        encoding::validate_rename_ranges(
                            &file.path,
                            file.encoding,
                            &file.before,
                            &file.edits,
                        )?;
                    }
                }
                Ok(completed)
            });
        if replies
            .send(Reply {
                id: job.id,
                generation: job.generation,
                result,
            })
            .is_err()
        {
            break;
        }
    }
}

fn execute(work: Work, token: &Cancellation<'_>) -> Result<Completed, String> {
    match work {
        Work::Encoding {
            root,
            encoding,
            files,
        } => {
            let canonical = fs::canonicalize(&root).map_err(|e| contextual(&root, e))?;
            if canonical != root || !canonical.is_dir() {
                return Err("Pasta do projeto mudou; preferência e buffers preservados".into());
            }
            let mut loaded = Vec::new();
            for (id, path) in files {
                let file =
                    read_encoded_document(&path, token, MAX_EDITOR_DOCUMENT_BYTES, encoding)?;
                if file.path != path {
                    return Err(contextual(
                        &path,
                        "Caminho mudou por link simbólico; buffers preservados",
                    ));
                }
                let mut file = file;
                file.project_root = root.clone();
                loaded.push((id, file));
            }
            let profile = token
                .encoding_profile
                .ok_or("Perfil de encoding indisponível")?;
            token.check()?;
            encoding::persist(profile, root.clone(), encoding)?;
            Ok(Completed::Encoding {
                root,
                encoding,
                files: loaded,
            })
        }

        Work::Directory {
            root,
            path,
            revision,
        } => {
            let result = if let Some(file) = token
                .workspace
                .filter(|p| workspace_file::is_workspace_file(p))
            {
                workspace_file::read_folders(file).and_then(|folders| {
                    let folder = folders
                        .iter()
                        .find(|f| path.starts_with(&f.path))
                        .ok_or("Pasta fora do workspace")?;
                    let mut children = read_directory(&folder.path, &path, token)?;
                    for child in &mut children {
                        child.depth += 1;
                    }
                    Ok(children)
                })
            } else {
                read_directory(&root, &path, token)
            };
            Ok(Completed::Directory {
                root,
                path,
                revision,
                result,
            })
        }
        Work::ProjectFile {
            mut directories,
            workspace,
            command,
            open_paths,
        } => {
            token.check()?;
            let mut operation_root = workspace.clone();
            let mut operation_roots = vec![workspace.clone()];
            if let Some(file) = token
                .workspace
                .filter(|p| workspace_file::is_workspace_file(p))
            {
                let folders = workspace_file::read_folders(file)?;
                operation_roots = folders
                    .iter()
                    .filter(|folder| folder.available)
                    .map(|folder| folder.path.clone())
                    .collect();
                let target = match &command {
                    ProjectFileCommand::CreateFile { parent, .. }
                    | ProjectFileCommand::CreateFolder { parent, .. } => Some(parent),
                    ProjectFileCommand::Trash { path }
                    | ProjectFileCommand::RenameFile { path, .. } => Some(path),
                    ProjectFileCommand::Transfer { source, .. } => Some(source),
                    ProjectFileCommand::Refresh | ProjectFileCommand::TrashMany { .. } => None,
                };
                if let Some(target) = target {
                    operation_root = folders
                        .iter()
                        .find(|f| target.starts_with(&f.path))
                        .ok_or("Item fora das pastas do workspace")?
                        .path
                        .clone();
                }
            }
            let mut receipt = if let ProjectFileCommand::TrashMany { paths } = command {
                project_files_worker::trash_many(&operation_roots, paths, &|| {
                    token.check().is_err()
                })?
            } else {
                project_files_worker::perform(operation_root.clone(), command, &open_paths)?
            };
            receipt.workspace = workspace.clone();
            // Mutation receipts survive cancellation; only refresh can be cancelled.
            if let crate::project_files::FileResultKind::Renamed { source } = &receipt.kind {
                for path in &mut directories {
                    if let Ok(relative) = path.strip_prefix(source) {
                        *path = receipt.path.join(relative);
                    }
                }
            }
            directories.extend(
                receipt
                    .path
                    .ancestors()
                    .skip(1)
                    .filter(|p| p.starts_with(&operation_root))
                    .map(PathBuf::from),
            );
            if let crate::project_files::FileResultKind::TrashedMany { paths, .. } = &receipt.kind {
                directories.extend(paths.iter().flat_map(|path| {
                    path.ancestors()
                        .skip(1)
                        .filter(|ancestor| {
                            operation_roots
                                .iter()
                                .any(|root| ancestor.starts_with(root))
                        })
                        .map(PathBuf::from)
                }));
            }
            let refresh = read_workspace_directories(
                token.workspace.unwrap_or(&workspace),
                &directories,
                token,
            );
            let opened = matches!(receipt.kind, crate::project_files::FileResultKind::File)
                .then(|| read_document(&receipt.path, token));
            let project_roots =
                if let crate::project_files::FileResultKind::Renamed { source } = &receipt.kind {
                    open_paths
                        .iter()
                        .filter_map(|old| {
                            old.strip_prefix(source)
                                .ok()
                                .map(|relative| receipt.path.join(relative))
                        })
                        .map(|path| {
                            let root =
                                encoding::context(token.encoding_profile, &path, Some(&workspace))
                                    .map_or_else(
                                        |_| projects::resolve_root(Some(&workspace), &path),
                                        |(root, _)| root,
                                    );
                            (path, root)
                        })
                        .collect()
                } else {
                    Vec::new()
                };
            Ok(Completed::ProjectFile {
                receipt,
                opened,
                project_roots,
                refresh,
            })
        }
        Work::RenameFilePreview {
            token: rename_token,
            workspace,
            search,
            new_name,
            buffers,
            cancel,
        } => {
            let prepared =
                file_rename::preview(workspace, search, new_name, buffers, &cancel, token)?;
            Ok(Completed::Rename {
                token: rename_token,
                prepared: Box::new(prepared),
                apply: false,
            })
        }
        Work::RenameResources { workspace, moves } => {
            // Cancellation is checked before execution only. Once the first move
            // starts, always deliver its receipt/recovery to the staged transaction.
            let result = rename::resources::ResourcePlan::prepare(&workspace, &moves)
                .map_err(rename::resources::ResourceError::from)
                .and_then(|plan| plan.apply());
            Ok(Completed::RenameResources(result))
        }
        Work::RenameInspect {
            workspace,
            buffers,
            token: rename_token,
            path,
            text,
            line,
            character,
            cancel,
        } => {
            reference_check(token, &cancel)?;
            let mut kind = if Language::for_path(&path) == Some(Language::Rust) {
                codisseu_syntax::rust_rename::classify(&text, line, character, &cancel)?
            } else if Language::for_path(&path) == Some(Language::Python) {
                codisseu_syntax::python_rename::classify(&text, line, character, &cancel)?
            } else {
                codisseu_syntax::classify_rename_symbol(&path, &text, line, character, &cancel)?
            };
            let mut name = if Language::for_path(&path) == Some(Language::Php) {
                match codisseu_syntax::prepare_global_rename_with_limits(
                    &path,
                    &text,
                    line,
                    character,
                    &cancel,
                    codisseu_syntax::ValidationLimits::default(),
                )? {
                    Some(name) => Some(name),
                    None => {
                        let name = codisseu_syntax::prepare_trait_rename_with_limits(
                            &path,
                            &text,
                            line,
                            character,
                            &cancel,
                            codisseu_syntax::ValidationLimits::default(),
                        )?;
                        if name.is_some() {
                            kind = codisseu_syntax::RenameSymbolKind::Method;
                        }
                        name
                    }
                }
            } else {
                None
            };
            if name.is_none()
                && Language::for_path(&path) == Some(Language::Php)
                && kind == codisseu_syntax::RenameSymbolKind::Method
            {
                let started = std::time::Instant::now();
                let root = fs::canonicalize(&workspace).map_err(|e| contextual(&workspace, e))?;
                let corpus = collect_rename_corpus(&root, token, &cancel, started)?;
                if let Some(limitation) = corpus.limitation {
                    return Err(limitation);
                }
                let mut snapshots = vec![];
                let mut bytes = 0usize;
                for candidate in corpus
                    .paths
                    .into_iter()
                    .filter(|p| Language::for_path(p) == Some(Language::Php))
                {
                    reference_check(token, &cancel)?;
                    if started.elapsed() > std::time::Duration::from_secs(2) {
                        return Err("Preparação de traits excedeu 2 segundos".into());
                    }
                    let text =
                        if let Some((_, text)) = buffers.iter().find(|(p, _)| *p == candidate) {
                            text.clone()
                        } else {
                            let file = read_semantic_document(&candidate, token)?;
                            if file.path != candidate {
                                return Err("Caminho de trait mudou".into());
                            }
                            Arc::from(file.text)
                        };
                    bytes += text.len();
                    if text.len() > MAX_DOCUMENT_BYTES || bytes > 4 * 1024 * 1024 {
                        return Err("Preparação de traits excede 1 MiB/4 MiB".into());
                    }
                    snapshots.push(codisseu_syntax::RenameSnapshot {
                        path: candidate,
                        before: text.clone(),
                        after: text,
                        edits: vec![],
                    });
                }
                // No trait declaration means the ordinary provider owns this method.
                if snapshots.iter().any(|s| s.before.contains("trait")) {
                    name = codisseu_syntax::prepare_trait_rename_in_corpus_with_limits(
                        &snapshots,
                        &path,
                        &TextRange {
                            start_line: line,
                            start_character: character,
                            end_line: line,
                            end_character: character,
                        },
                        &cancel,
                        codisseu_syntax::ValidationLimits::default(),
                    )?;
                }
            }
            reference_check(token, &cancel)?;
            Ok(Completed::RenameInspected {
                token: rename_token,
                name,
                kind,
            })
        }
        Work::RenamePreview {
            token: rename_token,
            workspace,
            changes,
            new_name,
            buffers,
            mut context_limitation,
            local_origin,
            mut search,
            cancel,
        } => {
            let root = fs::canonicalize(workspace).map_err(|e| e.to_string())?;
            if (changes.is_empty() && local_origin.is_none()) || changes.len() > 32 {
                return Err("Rename exige 1..32 arquivos".into());
            }
            let mut files = Vec::new();
            let mut bytes = 0;
            let mut count = 0;
            for change in changes {
                reference_check(token, &cancel)?;
                let path = fs::canonicalize(&change.path).map_err(|e| e.to_string())?;
                if Language::for_path(&search.origin) == Some(Language::Python)
                    && codisseu_language::python_library_root(
                        &path,
                        search.python_interpreter.as_deref(),
                    )
                    .is_some()
                {
                    return Err("Rename não pode alterar biblioteca Python somente leitura".into());
                }
                if path != change.path || !path.starts_with(&root) {
                    return Err("Rename fora do workspace ou caminho mudou".into());
                }
                if files.iter().any(|f: &RenamePreviewFile| f.path == path) {
                    return Err("Caminho duplicado".into());
                }
                let before = if let Some((_, version, text)) = buffers.iter().find(|b| b.0 == path)
                {
                    if change.version.is_some_and(|v| v != *version) {
                        return Err("Versão de rename obsoleta".into());
                    }
                    text.clone()
                } else {
                    if change.version.is_some_and(|v| v != 0) {
                        return Err("Versão de arquivo fechado desconhecida".into());
                    }
                    let f = read_semantic_document(&path, token)?;
                    if f.path != path {
                        return Err("Caminho mudou".into());
                    }
                    Arc::from(f.text)
                };
                if fs::canonicalize(&path).map_err(|e| e.to_string())? != path {
                    return Err("Caminho mudou durante preview".into());
                }
                count += change.edits.len();
                if count > 1024 {
                    return Err("Rename excede 1024 edições".into());
                }
                if before.len() > MAX_DOCUMENT_BYTES {
                    return Err("Rename excede 1 MiB por texto".into());
                }
                let after: Arc<str> = Arc::from(rename::build_after(&before, &change.edits)?);
                bytes += before.len() + after.len();
                if bytes > 8 * 1024 * 1024 {
                    return Err("Rename excede cache 8 MiB".into());
                }
                files.push(RenamePreviewFile {
                    encoding: FileEncoding::Utf8,
                    presentation: vec![],
                    path,
                    before,
                    after,
                    edits: change.edits,
                });
            }
            let mut snapshots: Vec<_> = files
                .iter()
                .map(|f| codisseu_syntax::RenameSnapshot {
                    path: f.path.clone(),
                    before: f.before.clone(),
                    after: f.after.clone(),
                    edits: f
                        .edits
                        .iter()
                        .map(|e| codisseu_syntax::RenameEdit {
                            range: TextRange {
                                start_line: e.start_line,
                                start_character: e.start_character,
                                end_line: e.end_line,
                                end_character: e.end_character,
                            },
                            new_text: e.new_text.clone(),
                        })
                        .collect(),
                })
                .collect();
            let open_buffers = buffers.clone();
            for (path, _, text) in buffers {
                if matches!(
                    Language::for_path(&search.origin),
                    Some(Language::Rust | Language::Python)
                ) && Language::for_path(&path) != Language::for_path(&search.origin)
                {
                    continue;
                }
                if snapshots.iter().any(|s| s.path == path) {
                    continue;
                }
                if Language::for_path(&path).is_none() {
                    continue;
                }
                let canonical = fs::canonicalize(&path).map_err(|e| e.to_string())?;
                if canonical != path || !path.starts_with(&root) {
                    return Err("Contexto de rename fora do workspace ou caminho mudou".into());
                }
                if bytes + text.len() * 2 > 8 * 1024 * 1024 {
                    context_limitation = Some("Contexto aberto excede cache 8 MiB".into());
                    break;
                }
                bytes += text.len() * 2;
                snapshots.push(codisseu_syntax::RenameSnapshot {
                    path,
                    before: text.clone(),
                    after: text,
                    edits: vec![],
                });
            }
            let started = std::time::Instant::now();
            let mut corpus = if context_limitation.is_some() {
                rename::RenameCorpus {
                    paths: vec![],
                    limitation: context_limitation,
                }
            } else {
                collect_symbol_rename_corpus(&root, &search.origin, token, &cancel, started)?
            };
            if corpus.limitation.is_none() {
                for path in &corpus.paths {
                    reference_check(token, &cancel)?;
                    if started.elapsed() > std::time::Duration::from_secs(2) {
                        corpus.limitation = Some("Coleta de contexto excedeu 2 segundos".into());
                        break;
                    }
                    if snapshots.iter().any(|s| s.path == *path) {
                        continue;
                    }
                    if snapshots.len() >= 64 {
                        corpus.limitation = Some("Contexto de rename excede 64 snapshots".into());
                        break;
                    }
                    let metadata = fs::metadata(path).map_err(|e| contextual(path, e))?;
                    if metadata.len() > MAX_DOCUMENT_BYTES as u64 {
                        corpus.limitation =
                            Some("Contexto de rename excede 1 MiB por texto".into());
                        break;
                    }
                    let file = read_semantic_document(path, token)?;
                    if file.path != *path
                        || fs::canonicalize(path).map_err(|e| contextual(path, e))? != *path
                    {
                        return Err("Caminho de contexto mudou durante coleta".into());
                    }
                    if bytes + file.text.len() * 2 > 8 * 1024 * 1024 {
                        corpus.limitation = Some("Contexto de rename excede cache 8 MiB".into());
                        break;
                    }
                    bytes += file.text.len() * 2;
                    let text: Arc<str> = Arc::from(file.text);
                    snapshots.push(codisseu_syntax::RenameSnapshot {
                        path: path.clone(),
                        before: text.clone(),
                        after: text,
                        edits: vec![],
                    });
                }
                if started.elapsed() > std::time::Duration::from_secs(2) {
                    corpus.limitation = Some("Coleta de contexto excedeu 2 segundos".into());
                }
            }
            reference_check(token, &cancel)?;
            if let Some((origin_path, origin)) = local_origin {
                if let Some(message) = &corpus.limitation {
                    return Err(message.clone());
                }
                snapshots = match codisseu_syntax::propose_global_rename_with_limits(
                    &snapshots,
                    &origin_path,
                    &origin,
                    &new_name,
                    &cancel,
                    codisseu_syntax::ValidationLimits::default(),
                )? {
                    Some(proposal) => proposal,
                    None => codisseu_syntax::propose_trait_rename_with_limits(
                        &snapshots,
                        &origin_path,
                        &origin,
                        &new_name,
                        &cancel,
                        codisseu_syntax::ValidationLimits::default(),
                    )?
                    .ok_or("Origem sem classe/função/trait/alias resolvido no corpus semântico")?,
                };
                files = snapshots
                    .iter()
                    .filter(|s| !s.edits.is_empty())
                    .map(|s| RenamePreviewFile {
                        encoding: FileEncoding::Utf8,
                        presentation: vec![],
                        path: s.path.clone(),
                        before: s.before.clone(),
                        after: s.after.clone(),
                        edits: s
                            .edits
                            .iter()
                            .map(|e| RenameTextEdit {
                                start_line: e.range.start_line,
                                start_character: e.range.start_character,
                                end_line: e.range.end_line,
                                end_character: e.range.end_character,
                                new_text: e.new_text.clone(),
                            })
                            .collect(),
                    })
                    .collect();
                if files.is_empty() || files.len() > 32 {
                    return Err("Rename semântico exige 1..32 arquivos alterados".into());
                }
            }
            if corpus.limitation.is_none()
                && Language::for_path(&search.origin) == Some(Language::Php)
                && let Some(complete) = codisseu_syntax::complete_php_named_argument_rename(
                    &snapshots,
                    &new_name,
                    &cancel,
                    codisseu_syntax::ValidationLimits::default(),
                )?
            {
                snapshots = complete;
                files = semantic_files(&snapshots);
            }
            apply_javascript_rename_option(&mut snapshots, &mut files, &search);
            let mut validation = rename_corpus_validation(
                &snapshots,
                &new_name,
                &cancel,
                &corpus,
                &root,
                &mut search,
            );
            reference_check(token, &cancel)?;
            let mut resources = vec![];
            search.text_snapshots =
                collect_search_snapshots(&root, &search, &open_buffers, token, &cancel)?;
            if validation.coverage == Coverage::Verified {
                let additional = rename_extra_proposal(
                    &root,
                    &snapshots,
                    &new_name,
                    &mut search,
                    None,
                    token,
                    &cancel,
                )?;
                merge_accessor_validation(&mut validation, &additional);
                files = additional.files;
                resources = additional.resources;
            }
            present_rename_files(&mut files, &cancel);
            Ok(Completed::Rename {
                token: rename_token,
                prepared: Box::new(rename::PreparedRename {
                    files,
                    snapshots,
                    validation,
                    corpus,
                    resources,
                    search,
                }),
                apply: false,
            })
        }
        Work::RenameChoices {
            token: rename_token,
            workspace,
            mut prepared,
            new_name,
            cancel,
        } => {
            let root = fs::canonicalize(workspace).map_err(|e| e.to_string())?;
            let mut validation = rename_corpus_validation(
                &prepared.snapshots,
                &new_name,
                &cancel,
                &prepared.corpus,
                &root,
                &mut prepared.search,
            );
            if validation.coverage == Coverage::Verified {
                let additional = rename_extra_proposal(
                    &root,
                    &prepared.snapshots,
                    &new_name,
                    &mut prepared.search,
                    None,
                    token,
                    &cancel,
                )?;
                merge_accessor_validation(&mut validation, &additional);
                prepared.files = additional.files;
                prepared.resources = additional.resources;
            }
            prepared.validation = validation;
            present_rename_files(&mut prepared.files, &cancel);
            Ok(Completed::Rename {
                token: rename_token,
                prepared,
                apply: false,
            })
        }
        Work::RenameExpand {
            token: rename_token,
            workspace,
            mut prepared,
            result,
            open_versions,
            new_name,
            cancel,
        } => {
            let root = fs::canonicalize(&workspace).map_err(|e| e.to_string())?;
            let started = std::time::Instant::now();
            let corpus = collect_symbol_rename_corpus(
                &root,
                &prepared.search.origin,
                token,
                &cancel,
                started,
            )?;
            if corpus.limitation.is_some() || corpus.paths != prepared.corpus.paths {
                return Err(
                    "Contexto mudou ou excedeu limites durante consultas relacionadas".into(),
                );
            }
            // Revalidate the ORIGINAL bytes, including unchanged closed context. A
            // new provider response must never silently replace the earlier baseline.
            for snapshot in &prepared.snapshots {
                reference_check(token, &cancel)?;
                if started.elapsed() > std::time::Duration::from_secs(2) {
                    return Err("Revalidação relacionada excedeu 2 segundos".into());
                }
                if fs::canonicalize(&snapshot.path).map_err(|e| e.to_string())? != snapshot.path
                    || !snapshot.path.starts_with(&root)
                {
                    return Err("Caminho de contexto mudou durante consultas relacionadas".into());
                }
                if !open_versions.iter().any(|(p, _)| *p == snapshot.path) {
                    let actual = read_semantic_document(&snapshot.path, token)?;
                    if actual.path != snapshot.path
                        || actual.text.as_str() != snapshot.before.as_ref()
                    {
                        return Err(
                            "Arquivo mudou durante consultas relacionadas; refaça preview".into(),
                        );
                    }
                }
            }
            prepared.files = semantic_files(&prepared.snapshots);
            for change in result.changes {
                reference_check(token, &cancel)?;
                let snapshot = prepared
                    .snapshots
                    .iter()
                    .find(|s| s.path == change.path)
                    .ok_or("Consulta relacionada retornou arquivo fora do corpus original")?;
                let expected = open_versions
                    .iter()
                    .find(|(p, _)| *p == change.path)
                    .map(|(_, v)| *v)
                    .or_else(|| {
                        (change.path == result.path)
                            .then_some(result.transient_document_version)
                            .flatten()
                    })
                    .unwrap_or(0);
                if change.version.is_some_and(|v| v != expected) {
                    return Err("Versão de consulta relacionada divergiu do snapshot".into());
                }
                let index = if let Some(index) =
                    prepared.files.iter().position(|f| f.path == change.path)
                {
                    index
                } else {
                    if prepared.files.len() >= 32 {
                        return Err("Rename relacionado excede 32 arquivos".into());
                    }
                    prepared.files.push(RenamePreviewFile {
                        encoding: FileEncoding::Utf8,
                        presentation: vec![],
                        path: change.path.clone(),
                        before: snapshot.before.clone(),
                        after: snapshot.before.clone(),
                        edits: vec![],
                    });
                    prepared.files.len() - 1
                };
                let file = &mut prepared.files[index];
                for edit in change.edits {
                    if file.edits.contains(&edit) {
                        continue;
                    }
                    file.edits.push(edit);
                }
                // build_after rejects conflicting same-range and overlapping edits.
                file.after = Arc::from(rename::build_after(&file.before, &file.edits)?);
                file.edits.sort_by_key(|e| {
                    (e.start_line, e.start_character, e.end_line, e.end_character)
                });
            }
            prepared.files.sort_by(|a, b| a.path.cmp(&b.path));
            if prepared.files.iter().map(|f| f.edits.len()).sum::<usize>() > 1024 {
                return Err("Rename relacionado excede 1024 edições".into());
            }
            for snapshot in &mut prepared.snapshots {
                if let Some(file) = prepared.files.iter().find(|f| f.path == snapshot.path) {
                    snapshot.after = file.after.clone();
                    snapshot.edits = file
                        .edits
                        .iter()
                        .map(|e| codisseu_syntax::RenameEdit {
                            range: TextRange {
                                start_line: e.start_line,
                                start_character: e.start_character,
                                end_line: e.end_line,
                                end_character: e.end_character,
                            },
                            new_text: e.new_text.clone(),
                        })
                        .collect();
                }
            }
            let original_declarations = prepared.validation.checked_declarations;
            apply_javascript_rename_option(
                &mut prepared.snapshots,
                &mut prepared.files,
                &prepared.search,
            );
            prepared.validation = rename_corpus_validation(
                &prepared.snapshots,
                &new_name,
                &cancel,
                &prepared.corpus,
                &root,
                &mut prepared.search,
            );
            if prepared.validation.checked_declarations != original_declarations {
                return Err(
                    "Consulta relacionada acrescentou declarações fora do conjunto original".into(),
                );
            }
            reference_check(token, &cancel)?;
            if prepared.validation.coverage == Coverage::Verified {
                let additional = rename_extra_proposal(
                    &root,
                    &prepared.snapshots,
                    &new_name,
                    &mut prepared.search,
                    None,
                    token,
                    &cancel,
                )?;
                merge_accessor_validation(&mut prepared.validation, &additional);
                prepared.files = additional.files;
                prepared.resources = additional.resources;
            }
            present_rename_files(&mut prepared.files, &cancel);
            Ok(Completed::Rename {
                token: rename_token,
                prepared,
                apply: false,
            })
        }
        Work::RenameVerify {
            token: rename_token,
            workspace,
            files,
            resources,
            mut search,
            snapshots,
            mut corpus,
            new_name,
            open,
            cancel,
        } => {
            if search.file_rename {
                let prepared = file_rename::verify(
                    workspace,
                    rename::PreparedRename {
                        search,
                        files,
                        snapshots,
                        corpus,
                        resources,
                        validation: RenameValidation {
                            coverage: Coverage::Unsupported,
                            scope: ValidationScope::ResourceReferences,
                            conflicts: vec![],
                            issues: vec![],
                            checked_declarations: 0,
                            checked_override_pairs: 0,
                            related_methods: vec![],
                            checked_files: 0,
                            incremental_reparses: 0,
                            note: String::new(),
                        },
                    },
                    &new_name,
                    &open,
                    &cancel,
                    token,
                )?;
                return Ok(Completed::Rename {
                    token: rename_token,
                    prepared: Box::new(prepared),
                    apply: true,
                });
            }
            let root = fs::canonicalize(workspace).map_err(|e| e.to_string())?;
            if Language::for_path(&search.origin) == Some(Language::Php)
                && let Some(repeated) = codisseu_syntax::complete_php_named_argument_rename(
                    &snapshots,
                    &new_name,
                    &cancel,
                    codisseu_syntax::ValidationLimits::default(),
                )?
                && (repeated.len() != snapshots.len()
                    || repeated.iter().zip(&snapshots).any(|(a, b)| {
                        a.path != b.path
                            || a.before != b.before
                            || a.after != b.after
                            || a.edits.len() != b.edits.len()
                            || a.edits
                                .iter()
                                .zip(&b.edits)
                                .any(|(a, b)| a.range != b.range || a.new_text != b.new_text)
                    }))
            {
                return Err("Complemento semântico dos parâmetros mudou; refaça preview".into());
            }
            let started = std::time::Instant::now();
            if corpus.limitation.is_none() {
                let current =
                    collect_symbol_rename_corpus(&root, &search.origin, token, &cancel, started)?;
                if current.limitation.is_some() {
                    corpus = current;
                } else if current.paths != corpus.paths {
                    return Err("Conjunto de arquivos do contexto mudou; refaça preview".into());
                }
            }
            for f in &snapshots {
                reference_check(token, &cancel)?;
                if corpus.limitation.is_some() {
                    break;
                }
                if started.elapsed() > std::time::Duration::from_secs(2) {
                    corpus.limitation = Some("Revalidação de contexto excedeu 2 segundos".into());
                    break;
                }
                let path = fs::canonicalize(&f.path).map_err(|e| e.to_string())?;
                if path != f.path || !path.starts_with(&root) {
                    return Err("Caminho rename mudou".into());
                }
                if !open.contains(&path) {
                    let actual = read_semantic_document(&path, token)?;
                    if actual.path != path || actual.text.as_str() != f.before.as_ref() {
                        return Err("Arquivo mudou externamente; refaça preview".into());
                    }
                    if fs::canonicalize(&path).map_err(|e| e.to_string())? != path {
                        return Err("Caminho mudou durante verificação".into());
                    }
                }
            }
            reference_check(token, &cancel)?;
            if started.elapsed() > std::time::Duration::from_secs(2) {
                corpus.limitation = Some("Revalidação de contexto excedeu 2 segundos".into());
            }
            let mut validation = rename_corpus_validation(
                &snapshots,
                &new_name,
                &cancel,
                &corpus,
                &root,
                &mut search,
            );
            reference_check(token, &cancel)?;
            if validation.coverage == Coverage::Verified {
                if search.options.search_text && search.options.scope == RenameSearchScope::Project
                {
                    let current =
                        collect_text_corpus(&root, token, &cancel, std::time::Instant::now())?;
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
                    if !open.contains(&snapshot.path) {
                        let actual = read_semantic_document(&snapshot.path, token)?;
                        if actual.path != snapshot.path
                            || actual.text.as_str() != snapshot.before.as_ref()
                        {
                            return Err("Texto opcional mudou externamente; refaça preview".into());
                        }
                    }
                }
                let additional = rename_extra_proposal(
                    &root,
                    &snapshots,
                    &new_name,
                    &mut search,
                    Some(&resources),
                    token,
                    &cancel,
                )?;
                merge_accessor_validation(&mut validation, &additional);
                if additional.files != files || additional.resources != resources {
                    return Err("Proposta privada mudou; refaça preview".into());
                }
                // Compiler/accessor verification can take seconds. Check disk again
                // after that work, before authorizing the in-memory transaction.
                let final_started = std::time::Instant::now();
                let current = collect_symbol_rename_corpus(
                    &root,
                    &search.origin,
                    token,
                    &cancel,
                    final_started,
                )?;
                if current.limitation.is_some() || current.paths != corpus.paths {
                    return Err("Corpus mudou durante a verificação; refaça preview".into());
                }
                for snapshot in snapshots.iter().chain(&search.text_snapshots) {
                    reference_check(token, &cancel)?;
                    if final_started.elapsed() > std::time::Duration::from_secs(2) {
                        return Err("Conferência final do contexto excedeu 2 segundos".into());
                    }
                    if !open.contains(&snapshot.path) {
                        let actual = read_semantic_document(&snapshot.path, token)?;
                        if actual.path != snapshot.path
                            || actual.text.as_str() != snapshot.before.as_ref()
                        {
                            return Err(
                                "Arquivo mudou durante a verificação; refaça preview".into()
                            );
                        }
                    }
                }
                if search.options.search_text && search.options.scope == RenameSearchScope::Project
                {
                    let current = collect_text_corpus(&root, token, &cancel, final_started)?;
                    if current.limitation.is_some()
                        || current.paths
                            != search
                                .text_snapshots
                                .iter()
                                .map(|s| s.path.clone())
                                .collect::<Vec<_>>()
                    {
                        return Err(
                            "Corpus textual mudou durante a verificação; refaça preview".into()
                        );
                    }
                }
                if !resources.is_empty() {
                    rename::resource_config::recheck(&search.resource_configs)?;
                }
                worker_compiler::recheck_configuration(&root, &snapshots, &search)?;
            }
            Ok(Completed::Rename {
                token: rename_token,
                prepared: Box::new(rename::PreparedRename {
                    files,
                    snapshots,
                    validation,
                    corpus,
                    resources,
                    search,
                }),
                apply: true,
            })
        }
        Work::ReferencePreviews {
            token: reference_token,
            targets,
            workspace,
            open_buffers,
            cancel,
            allow_rust_libraries,
            python_interpreter,
        } => {
            let root = fs::canonicalize(&workspace).map_err(|e| contextual(&workspace, e))?;
            let mut cache: HashMap<PathBuf, Arc<str>> = HashMap::new();
            let mut items = Vec::new();
            let mut bytes = 0usize;
            for mut target in targets {
                reference_check(token, &cancel)?;
                let canonical =
                    fs::canonicalize(&target.path).map_err(|e| contextual(&target.path, e))?;
                navigation_read_only(
                    &canonical,
                    &root,
                    allow_rust_libraries,
                    python_interpreter.as_deref(),
                )?;
                if canonical != target.path {
                    return Err("Caminho de referência mudou desde a resposta do provider".into());
                }
                target.path = canonical.clone();
                if !cache.contains_key(&canonical) {
                    let text =
                        if let Some(buffer) = open_buffers.iter().find(|b| b.path == canonical) {
                            buffer.text.clone()
                        } else {
                            let file = read_semantic_document(&canonical, token)?;
                            if file.path != canonical {
                                return Err("Caminho de referência mudou durante leitura".into());
                            }
                            file.text
                        };
                    bytes += text.len();
                    if bytes > references::MAX_REFERENCE_CACHE {
                        return Err("Referências excedem cache de8MiB".into());
                    }
                    if fs::canonicalize(&canonical).map_err(|e| contextual(&canonical, e))?
                        != canonical
                    {
                        return Err("Caminho de referência mudou durante leitura".into());
                    }
                    navigation_read_only(
                        &canonical,
                        &root,
                        allow_rust_libraries,
                        python_interpreter.as_deref(),
                    )?;
                    cache.insert(canonical.clone(), Arc::from(text));
                }
                let text = &cache[&canonical];
                let preview = references::preview(text, &target)?;
                let (preview_text, preview_first_line) = references::preview_context(text, &target);
                items.push(ReferenceItem {
                    target,
                    preview,
                    preview_text,
                    preview_first_line,
                });
            }
            reference_check(token, &cancel)?;
            Ok(Completed::ReferencePreviews {
                token: reference_token,
                items,
                cache,
            })
        }
        Work::ReferenceOpen {
            token: reference_token,
            mut target,
            workspace,
            expected,
            open,
            cancel,
            allow_rust_libraries,
            python_interpreter,
        } => {
            reference_check(token, &cancel)?;
            let root = fs::canonicalize(&workspace).map_err(|e| contextual(&workspace, e))?;
            let canonical =
                fs::canonicalize(&target.path).map_err(|e| contextual(&target.path, e))?;
            let read_only = navigation_read_only(
                &canonical,
                &root,
                allow_rust_libraries,
                python_interpreter.as_deref(),
            )?;
            if canonical != target.path {
                return Err("Caminho de referência mudou ou está fora do workspace".into());
            }
            target.path = canonical;
            let file = if open {
                None
            } else {
                let file = read_semantic_document(&target.path, token)?;
                if file.path != target.path {
                    return Err("Caminho de referência mudou durante abertura".into());
                }
                if file.text.as_str() != expected.as_ref() {
                    return Err(
                        "Arquivo da referência mudou externamente; busque referências novamente"
                            .into(),
                    );
                }
                if fs::canonicalize(&target.path).map_err(|e| contextual(&target.path, e))?
                    != target.path
                {
                    return Err("Caminho de referência mudou durante abertura".into());
                }
                Some(file)
            };
            reference_check(token, &cancel)?;
            navigation_read_only(
                &target.path,
                &root,
                allow_rust_libraries,
                python_interpreter.as_deref(),
            )?;
            Ok(Completed::ReferenceOpen {
                token: reference_token,
                target,
                file,
                read_only,
            })
        }
        Work::Completion {
            token: completion_token,
            id,
            version,
            text,
            caret,
            primary,
            additional,
            php_callable,
        } => {
            token.check()?;
            let (text, caret) =
                completion::build_call_edits(&text, caret, primary, additional, php_callable)?;
            token.check()?;
            Ok(Completed::Completion {
                token: completion_token,
                id,
                version,
                text,
                caret,
            })
        }
        Work::Navigation {
            token: navigation_token,
            mut target,
            workspace,
            open_paths,
            allow_rust_libraries,
            python_interpreter,
        } => {
            token.check()?;
            let root = fs::canonicalize(&workspace).map_err(|e| contextual(&workspace, e))?;
            let canonical =
                fs::canonicalize(&target.path).map_err(|e| contextual(&target.path, e))?;
            let read_only = navigation_read_only(
                &canonical,
                &root,
                allow_rust_libraries,
                python_interpreter.as_deref(),
            )?;
            target.path = canonical;
            let file = if open_paths.contains(&target.path) {
                None
            } else {
                let file = read_semantic_document(&target.path, token)?;
                if file.path != target.path {
                    return Err("Caminho da definição mudou durante leitura".into());
                }
                Some(file)
            };
            if fs::canonicalize(&target.path).map_err(|e| contextual(&target.path, e))?
                != target.path
            {
                return Err("Caminho da definição mudou durante abertura".into());
            }
            token.check()?;
            navigation_read_only(
                &target.path,
                &root,
                allow_rust_libraries,
                python_interpreter.as_deref(),
            )?;
            Ok(Completed::Navigation {
                token: navigation_token,
                target,
                file,
                read_only,
            })
        }

        Work::ProviderConfig { root, path } => {
            let configs: Vec<ProviderConfig> =
                serde_json::from_slice(&read_limited(&path, MAX_SESSION_BYTES)?)
                    .map_err(|e| e.to_string())?;
            token.check()?;
            if configs.len() > 4 {
                return Err(
                    "São aceitos até quatro providers: PHP, TypeScript, Rust e Python".into(),
                );
            }
            Ok(Completed::ProviderConfig { root, configs })
        }
        Work::GitReload(files) => {
            let mut loaded = Vec::with_capacity(files.len());
            for (id, version, path) in files {
                token.check()?;
                let result = match fs::symlink_metadata(&path) {
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                    Err(e) => Err(contextual(&path, e)),
                    Ok(meta) if meta.file_type().is_symlink() => {
                        Err("Git transformou arquivo em link simbólico; buffer preservado".into())
                    }
                    Ok(_) => read_document(&path, token).map(Some),
                };
                loaded.push((id, version, result));
            }
            Ok(Completed::GitReload(loaded))
        }
        Work::Create(path) => {
            let name = path
                .file_name()
                .filter(|name| !name.is_empty())
                .ok_or("Informe um nome de arquivo")?;
            let parent = path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new("."));
            let parent = fs::canonicalize(parent).map_err(|e| contextual(parent, e))?;
            let path = parent.join(name);
            token.check()?;
            let encoding = encoding::resolve_for(token.encoding_profile, &path, token.workspace)?;
            let file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .map_err(|e| contextual(&path, e))?;
            file.sync_all().map_err(|e| contextual(&path, e))?;
            // Once creation starts, report its receipt rather than erasing a
            // user-requested file if a newer workspace generation cancels opening.
            let (project_root, _) =
                encoding::context(token.encoding_profile, &path, token.workspace)?;
            Ok(Completed::Created(OpenedFile {
                project_root,
                path,
                text: String::new(),
                encoding,
            }))
        }
        Work::Open(path) => read_document(&path, token).map(Completed::Open),
        Work::Workspace(path) => read_workspace(&path, token).map(Completed::Workspace),
        Work::SaveNew {
            id,
            path,
            text,
            identity,
        } => {
            token.check()?;
            let parent = path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new("."));
            let parent = fs::canonicalize(parent).map_err(|e| contextual(parent, e))?;
            let path = parent.join(path.file_name().ok_or("Nome de arquivo ausente")?);
            let text: Arc<str> = Arc::from(text.to_string());
            let mut temporary =
                tempfile::NamedTempFile::new_in(&parent).map_err(|e| contextual(&parent, e))?;
            temporary
                .write_all(text.as_bytes())
                .map_err(|e| contextual(&path, e))?;
            temporary
                .as_file()
                .sync_all()
                .map_err(|e| contextual(&path, e))?;
            token.check()?;
            temporary
                .persist_noclobber(&path)
                .map_err(|e| contextual(&path, e.error))?;
            #[cfg(unix)]
            let warning = File::open(&parent)
                .and_then(|f| f.sync_all())
                .err()
                .map(|e| format!("Arquivo salvo; sincronização do diretório falhou: {e}"));
            #[cfg(not(unix))]
            let warning = None;
            Ok(Completed::Saved {
                new_path: Some(path),
                previous: None,
                id,
                text,
                identity,
                warning,
            })
        }
        Work::Save {
            encoding,
            id,
            path,
            text,
            expected,
            baseline,
            identity,
        } => {
            token.check()?;
            let text: Arc<str> = Arc::from(text.to_string());
            let warning = match save_encoded_document(&path, &text, &expected, encoding, token) {
                Ok(warning) => warning,
                Err(error) => {
                    // Publish a resolvable snapshot with the failed Save ACK,
                    // rather than waiting for the periodic watcher. Never turn
                    // a cancellation or a rejected path into a resolution.
                    token.check()?;
                    if let Ok(file) = external_files::stable_document(&path, encoding)
                        && file.path == path
                        && file.text != expected.as_ref()
                    {
                        return Ok(Completed::SaveConflict {
                            id,
                            baseline,
                            file,
                            error,
                        });
                    }
                    return Err(error);
                }
            };
            Ok(Completed::Saved {
                new_path: None,
                previous: Some(expected),
                id,
                text,
                identity,
                warning,
            })
        }
        Work::ReloadDocument {
            id,
            version,
            external_revision,
            path,
            encoding,
            baseline,
            observed,
        } => {
            token.check()?;
            let file = external_files::stable_document(&path, encoding)?;
            if file.path != path {
                return Err("O caminho mudou; conteúdo aberto preservado".into());
            }
            if file.text != observed.as_ref() || file.encoding != encoding {
                return Ok(Completed::SaveConflict {
                    id, baseline, file,
                    error: "Conflito: o arquivo mudou novamente no disco; confira a nova versão antes de decidir".into(),
                });
            }
            token.check()?;
            Ok(Completed::ReloadDocument {
                id,
                version,
                external_revision,
                baseline,
                file,
            })
        }
        Work::Persist { path, session } => {
            save_session(&path, &session, token)?;
            Ok(Completed::Persisted)
        }
        Work::Restore {
            path,
            legacy,
            profile_anchor,
        } => {
            let source = match fs::symlink_metadata(&path) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    legacy.as_deref().unwrap_or(&path)
                }
                _ => &path,
            };
            restore_session(source, &profile_anchor, token).map(Completed::Restored)
        }
    }
}

fn contextual(path: &Path, error: impl std::fmt::Display) -> String {
    format!("{}: {error}", path.display())
}

fn read_limited(path: &Path, max_bytes: usize) -> Result<Vec<u8>, String> {
    let metadata = fs::metadata(path).map_err(|e| contextual(path, e))?;
    if !metadata.is_file() {
        return Err(contextual(
            path,
            "Somente arquivos regulares são suportados",
        ));
    }
    if metadata.len() > max_bytes as u64 {
        return Err(contextual(path, "Arquivo excede o limite de tamanho"));
    }
    let file = File::open(path).map_err(|e| contextual(path, e))?;
    if !file.metadata().map_err(|e| contextual(path, e))?.is_file() {
        return Err(contextual(path, "Arquivo deixou de ser regular"));
    }
    let mut bytes = Vec::new();
    file.take(max_bytes as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| contextual(path, e))?;
    if bytes.len() > max_bytes {
        return Err(contextual(path, "Arquivo excede o limite de tamanho"));
    }
    Ok(bytes)
}

fn navigation_read_only(
    path: &Path,
    root: &Path,
    allow_rust_libraries: bool,
    python_interpreter: Option<&Path>,
) -> Result<bool, String> {
    if python_interpreter.is_some()
        && codisseu_language::python_library_root(path, python_interpreter).is_some()
    {
        return Ok(true);
    }
    if path.starts_with(root) {
        return Ok(false);
    }
    if allow_rust_libraries && codisseu_language::rust_library_root(path).is_some() {
        return Ok(true);
    }
    Err("Destino de navegação fora do workspace e das bibliotecas Rust permitidas".into())
}

fn read_document(path: &Path, token: &Cancellation<'_>) -> Result<OpenedFile, String> {
    read_document_with_limit(path, token, MAX_EDITOR_DOCUMENT_BYTES)
}
pub(super) fn external_document(path: &Path, encoding: FileEncoding) -> Result<OpenedFile, String> {
    let generation = AtomicU64::new(0);
    read_encoded_document(
        path,
        &Cancellation {
            encoding_profile: None,
            workspace: None,
            generation: &generation,
            expected: 0,
        },
        MAX_EDITOR_DOCUMENT_BYTES,
        encoding,
    )
}
pub(super) fn external_workspace(
    path: &Path,
    directories: &[PathBuf],
    encoding_profile: Option<&Path>,
) -> Result<Workspace, String> {
    let generation = AtomicU64::new(0);
    read_workspace_directories(
        path,
        directories,
        &Cancellation {
            encoding_profile,
            workspace: None,
            generation: &generation,
            expected: 0,
        },
    )
}
fn read_semantic_document(path: &Path, token: &Cancellation<'_>) -> Result<OpenedFile, String> {
    read_document_with_limit(path, token, MAX_DOCUMENT_BYTES)
}
fn read_document_with_limit(
    path: &Path,
    token: &Cancellation<'_>,
    limit: usize,
) -> Result<OpenedFile, String> {
    let path = fs::canonicalize(path).map_err(|e| contextual(path, e))?;
    let (project_root, encoding) =
        encoding::context(token.encoding_profile, &path, token.workspace)?;
    let mut file = read_encoded_document(&path, token, limit, encoding)?;
    file.project_root = project_root;
    Ok(file)
}
fn read_encoded_document(
    path: &Path,
    token: &Cancellation<'_>,
    limit: usize,
    encoding: FileEncoding,
) -> Result<OpenedFile, String> {
    token.check()?;
    let path = fs::canonicalize(path).map_err(|e| contextual(path, e))?;
    let bytes = read_limited(&path, limit)?;
    token.check()?;
    let (text, encoding) = encoding.decode(&bytes).map_err(|e| contextual(&path, e))?;
    if text.len() > limit {
        return Err(contextual(
            &path,
            "Texto decodificado excede o limite de bytes do editor",
        ));
    }
    if text.contains('\0') {
        return Err(contextual(&path, "Arquivo binário (NUL) não é editável"));
    }
    codisseu_editor::validate_editor_line_count(&text).map_err(|error| contextual(&path, error))?;
    let project_root = projects::resolve_root(token.workspace, &path);
    Ok(OpenedFile {
        project_root,
        path,
        text,
        encoding,
    })
}

fn read_workspace(path: &Path, token: &Cancellation<'_>) -> Result<Workspace, String> {
    read_workspace_directories(path, &[], token)
}
fn read_workspace_directories(
    path: &Path,
    directories: &[PathBuf],
    token: &Cancellation<'_>,
) -> Result<Workspace, String> {
    token.check()?;
    let path = fs::canonicalize(path).map_err(|e| contextual(path, e))?;
    if workspace_file::is_workspace_file(&path) {
        let folders = workspace_file::read_folders(&path)?;
        let mut entries: Vec<_> = folders
            .iter()
            .map(|f| TreeEntry {
                path: f.path.clone(),
                is_dir: true,
                depth: 0,
            })
            .collect();
        let mut loaded = std::collections::BTreeSet::new();
        let mut errors = BTreeMap::new();
        let mut project_encodings = BTreeMap::new();
        let mut requested = directories.to_vec();
        requested.sort_by_key(|p| p.components().count());
        requested.dedup();
        for folder in &folders {
            if !folder.available {
                errors.insert(
                    folder.path.clone(),
                    format!("Pasta ausente: {} ({})", folder.name, folder.path.display()),
                );
                continue;
            }
            project_encodings.extend(encoding::project_contexts(
                token.encoding_profile,
                &folder.path,
                &[],
            )?);
        }
        for directory in requested {
            if !entries.iter().any(|e| e.path == directory && e.is_dir) {
                continue;
            }
            let Some(folder) = folders.iter().find(|f| directory.starts_with(&f.path)) else {
                continue;
            };
            match read_directory(&folder.path, &directory, token) {
                Ok(mut children) => {
                    for child in &mut children {
                        child.depth += 1;
                    }
                    project_tree::merge(&mut entries, &path, &directory, children);
                    loaded.insert(directory);
                }
                Err(error) => {
                    errors.insert(directory, error);
                }
            }
        }
        return Ok(Workspace {
            file: Some(path.clone()),
            folders,
            encoding: FileEncoding::default(),
            path: path.parent().ok_or("Workspace sem pasta")?.to_path_buf(),
            project_encodings,
            entries,
            loaded,
            errors,
            truncated: false,
        });
    }
    if !path.is_dir() {
        return Err(contextual(
            &path,
            "O workspace deve ser uma pasta ou arquivo .code-workspace",
        ));
    }
    let mut entries = read_directory(&path, &path, token)?;
    let mut loaded = std::collections::BTreeSet::from([path.clone()]);
    let mut errors = BTreeMap::new();
    let mut requested = directories.to_vec();
    requested.sort_by_key(|p| p.components().count());
    requested.dedup();
    for directory in requested {
        token.check()?;
        if loaded.contains(&directory) || !entries.iter().any(|e| e.path == directory && e.is_dir) {
            continue;
        }
        match read_directory(&path, &directory, token) {
            Ok(children) => {
                project_tree::merge(&mut entries, &path, &directory, children);
                loaded.insert(directory);
            }
            Err(error) => {
                errors.insert(directory, error);
            }
        }
    }
    token.check()?;
    let encoding = encoding::resolve(token.encoding_profile, &path)?;
    let project_encodings = encoding::project_contexts(token.encoding_profile, &path, &entries)?;
    Ok(Workspace {
        file: None,
        folders: Vec::new(),
        project_encodings,
        encoding,
        path,
        entries,
        truncated: false,
        loaded,
        errors,
    })
}
fn read_directory(
    root: &Path,
    path: &Path,
    token: &Cancellation<'_>,
) -> Result<Vec<TreeEntry>, String> {
    token.check()?;
    let relative = path
        .strip_prefix(root)
        .map_err(|_| "Diretório fora do projeto")?;
    if relative
        .components()
        .any(|p| matches!(p, std::path::Component::ParentDir))
    {
        return Err("Diretório inválido".into());
    }
    let depth = relative.components().count();
    let mut entries = Vec::new();
    for item in fs::read_dir(path).map_err(|e| contextual(path, e))? {
        token.check()?;
        let item = item.map_err(|e| contextual(path, e))?;
        let kind = item.file_type().map_err(|e| contextual(&item.path(), e))?;
        // Links are visible. Only an explicit expansion follows a directory link,
        // so cycles cannot trigger automatic recursive enumeration.
        if kind.is_dir() || kind.is_file() || kind.is_symlink() {
            let is_dir = kind.is_dir()
                || (kind.is_symlink() && fs::metadata(item.path()).is_ok_and(|m| m.is_dir()));
            entries.push(TreeEntry {
                path: item.path(),
                is_dir,
                depth,
            });
        }
    }
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.path.cmp(&b.path)));
    token.check()?;
    Ok(entries)
}

fn verify_save_target(path: &Path, expected: &[u8]) -> Result<fs::Permissions, String> {
    // Canonical paths are stored at open time. Reject replaced symlinks/parents.
    let current_path = fs::canonicalize(path).map_err(|e| contextual(path, e))?;
    if current_path != path {
        return Err(contextual(
            path,
            "Caminho mudou por link simbólico; reabra o arquivo",
        ));
    }
    let metadata = fs::symlink_metadata(path).map_err(|e| contextual(path, e))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(contextual(path, "Destino não é um arquivo regular"));
    }
    if metadata.permissions().readonly() {
        return Err(contextual(
            path,
            "Arquivo está marcado como somente leitura",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() > 1 {
            return Err(contextual(
                path,
                "Salvar arquivos com múltiplos hard links requer uma política explícita (M1)",
            ));
        }
    }
    if read_limited(path, MAX_EDITOR_DOCUMENT_BYTES)? != expected {
        return Err(contextual(
            path,
            "Conflito: arquivo mudou no disco; alterações locais foram preservadas",
        ));
    }
    Ok(metadata.permissions())
}

fn save_encoded_document(
    path: &Path,
    text: &str,
    expected: &str,
    encoding: FileEncoding,
    token: &Cancellation<'_>,
) -> Result<Option<String>, String> {
    token.check()?;
    let bytes = encoding.encode(text)?;
    let expected = encoding.encode(expected)?;
    let permissions = verify_save_target(path, &expected)?;
    let parent = path
        .parent()
        .ok_or_else(|| "Arquivo sem diretório pai".to_owned())?;
    let mut temporary =
        tempfile::NamedTempFile::new_in(parent).map_err(|e| contextual(parent, e))?;
    temporary
        .write_all(&bytes)
        .map_err(|e| contextual(path, e))?;
    temporary
        .as_file()
        .set_permissions(permissions)
        .map_err(|e| contextual(path, e))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|e| contextual(path, e))?;
    // Check again after writing the temporary file, before committing it.
    verify_save_target(path, &expected)?;
    token.check()?;
    temporary
        .persist(path)
        .map_err(|e| contextual(path, e.error))?;
    // A failure here means committed content with uncertain crash durability,
    // not an untouched original. Return a warning with the successful save.
    #[cfg(unix)]
    if let Err(error) = File::open(parent).and_then(|directory| directory.sync_all()) {
        return Ok(Some(format!(
            "Arquivo salvo; sincronização do diretório falhou: {error}"
        )));
    }
    Ok(None)
}

fn save_session(path: &Path, session: &Session, token: &Cancellation<'_>) -> Result<(), String> {
    token.check()?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|e| contextual(parent, e))?;
    if let Ok(metadata) = fs::symlink_metadata(path)
        && (!metadata.is_file() || metadata.file_type().is_symlink())
    {
        return Err(contextual(
            path,
            "Sessão não pode sobrescrever um link ou arquivo especial",
        ));
    }
    let bytes = serde_json::to_vec_pretty(session).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_SESSION_BYTES {
        return Err("Sessão excede 128 KiB".into());
    }
    let mut temporary =
        tempfile::NamedTempFile::new_in(parent).map_err(|e| contextual(parent, e))?;
    temporary
        .write_all(&bytes)
        .map_err(|e| contextual(path, e))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|e| contextual(path, e))?;
    token.check()?;
    temporary
        .persist(path)
        .map_err(|e| contextual(path, e.error))?;
    Ok(())
}

fn restore_session(
    path: &Path,
    profile_anchor: &Path,
    token: &Cancellation<'_>,
) -> Result<Restored, String> {
    token.check()?;
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Restored {
                omitted_database_consoles: false,
                shell_layout: ShellLayout::default(),
                git_drafts: BTreeMap::new(),
                editor_state: None,
                editor_preferences: EditorPreferences::default(),
                project_preferences: ProjectPreferences::default(),
                tool_window_preferences: ToolWindowPreferences::default(),
                workspace: None,
                files: Vec::new(),
                active: None,
                database_path: None,
                provider_configs: Vec::new(),
            });
        }
        Err(error) => return Err(contextual(path, error)),
        Ok(metadata) if !metadata.is_file() || metadata.file_type().is_symlink() => {
            return Err(contextual(
                path,
                "Sessão deve ser um arquivo regular sem link simbólico",
            ));
        }
        Ok(_) => {}
    }
    let session: Session = serde_json::from_slice(&read_limited(path, MAX_SESSION_BYTES)?)
        .map_err(|e| contextual(path, e))?;
    if !matches!(session.schema_version, 1 | 2) {
        return Err("Versão de sessão não suportada; arquivo preservado".into());
    }
    session.shell_layout.validate()?;
    if session.tabs.len() > MAX_DOCUMENTS {
        return Err("Sessão excede 32 documentos".into());
    }
    if let Some(state) = &session.editor_state {
        state.validate_paths(&session.tabs)?;
    }
    let workspace = session
        .workspace
        .map(|p| read_workspace(&p, token))
        .transpose()?;
    let mut files = Vec::new();
    let mut omitted_database_consoles = false;
    for path in &session.tabs {
        if !remote::restorable_project_console(
            profile_anchor,
            workspace
                .as_ref()
                .map(|w| w.file.as_deref().unwrap_or(&w.path)),
            path,
        ) {
            omitted_database_consoles = true;
            continue;
        }
        let encoding = encoding::resolve_for(
            token.encoding_profile,
            path,
            workspace
                .as_ref()
                .map(|w| w.file.as_deref().unwrap_or(&w.path)),
        )?;
        let mut file = read_encoded_document(path, token, MAX_EDITOR_DOCUMENT_BYTES, encoding)?;
        file.project_root = encoding::context(
            token.encoding_profile,
            &file.path,
            workspace
                .as_ref()
                .map(|w| w.file.as_deref().unwrap_or(&w.path)),
        )?
        .0;
        if !files
            .iter()
            .any(|other: &OpenedFile| other.path == file.path)
        {
            files.push(file);
        }
    }
    if !omitted_database_consoles && let Some(state) = &session.editor_state {
        state.validate(&files)?;
    }
    git_integration::validate_git_drafts(&session.git_drafts)?;
    token.check()?;
    if omitted_database_consoles {
        // Preserve the complete old tab references before a later normal save
        // persists the filtered session. Never replace an existing recovery copy.
        let backup = path.with_extension("before-project-isolation.json");
        if !backup.exists() {
            let mut temporary =
                tempfile::NamedTempFile::new_in(path.parent().unwrap_or(Path::new(".")))
                    .map_err(|e| contextual(path, e))?;
            temporary
                .write_all(&read_limited(path, MAX_SESSION_BYTES)?)
                .map_err(|e| contextual(path, e))?;
            temporary
                .as_file()
                .sync_all()
                .map_err(|e| contextual(path, e))?;
            match temporary.persist_noclobber(&backup) {
                Ok(_) => (),
                Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => (),
                Err(e) => return Err(contextual(&backup, e)),
            }
        }
    }
    Ok(Restored {
        omitted_database_consoles,
        shell_layout: session.shell_layout,
        git_drafts: session.git_drafts,
        editor_state: if omitted_database_consoles {
            None
        } else {
            session.editor_state
        },
        editor_preferences: session.editor_preferences,
        project_preferences: session.project_preferences,
        tool_window_preferences: session.tool_window_preferences,
        workspace,
        files,
        active: session.active,
        database_path: session.database_path,
        provider_configs: session.provider_configs,
    })
}

fn reference_check(
    token: &Cancellation<'_>,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<(), String> {
    token.check()?;
    if cancel.load(Ordering::Acquire) {
        Err("Referências canceladas".into())
    } else {
        Ok(())
    }
}

// This bounded corpus is explicitly smaller than a project-wide semantic index.
fn present_rename_files(files: &mut [RenamePreviewFile], cancel: &std::sync::atomic::AtomicBool) {
    for file in files {
        let ranges = file
            .edits
            .iter()
            .map(|e| TextRange {
                start_line: e.start_line,
                start_character: e.start_character,
                end_line: e.end_line,
                end_character: e.end_character,
            })
            .collect::<Vec<_>>();
        file.presentation = codisseu_syntax::rename_occurrence_presentation(
            &file.path,
            &file.before,
            &ranges,
            cancel,
            codisseu_syntax::ValidationLimits::default(),
        );
    }
}
fn semantic_files(snapshots: &[codisseu_syntax::RenameSnapshot]) -> Vec<RenamePreviewFile> {
    snapshots
        .iter()
        .filter(|s| !s.edits.is_empty())
        .map(|s| RenamePreviewFile {
            encoding: FileEncoding::Utf8,
            presentation: vec![],
            path: s.path.clone(),
            before: s.before.clone(),
            after: s.after.clone(),
            edits: s.edits.iter().map(syntax_edit).collect(),
        })
        .collect()
}
fn syntax_edit(edit: &codisseu_syntax::RenameEdit) -> RenameTextEdit {
    RenameTextEdit {
        start_line: edit.range.start_line,
        start_character: edit.range.start_character,
        end_line: edit.range.end_line,
        end_character: edit.range.end_character,
        new_text: edit.new_text.clone(),
    }
}
fn collect_search_snapshots(
    root: &Path,
    search: &rename::RenameSearch,
    buffers: &[(PathBuf, u64, Arc<str>)],
    token: &Cancellation<'_>,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<Vec<codisseu_syntax::RenameSnapshot>, String> {
    if !search.options.search_text || search.options.scope == RenameSearchScope::CurrentFile {
        return Ok(vec![]);
    }
    let corpus = collect_text_corpus(root, token, cancel, std::time::Instant::now())?;
    if let Some(error) = corpus.limitation {
        return Err(error);
    }
    let mut result = vec![];
    let mut bytes = 0;
    for path in corpus.paths {
        reference_check(token, cancel)?;
        let text = if let Some((_, _, text)) = buffers.iter().find(|(p, _, _)| *p == path) {
            text.clone()
        } else {
            Arc::from(read_semantic_document(&path, token)?.text)
        };
        bytes += text.len();
        if bytes > 8 * 1024 * 1024 || text.len() > MAX_DOCUMENT_BYTES {
            return Err("Busca textual excede 8 MiB/1 MiB por arquivo".into());
        }
        result.push(codisseu_syntax::RenameSnapshot {
            path,
            before: text.clone(),
            after: text,
            edits: vec![],
        });
    }
    Ok(result)
}
struct AdditionalRename {
    files: Vec<RenamePreviewFile>,
    resources: Vec<rename::resources::ResourceMove>,
    reports: Vec<RenameValidation>,
    issues: Vec<ValidationIssue>,
}
fn merge_accessor_validation(primary: &mut RenameValidation, additional: &AdditionalRename) {
    // Keep the primary symbol's scope/family: accessors were independently
    // checked with their own spelling and do not become property overrides.
    if !additional.issues.is_empty() {
        primary.coverage = Coverage::Unsupported;
    }
    primary.issues.extend(additional.issues.iter().cloned());
    for report in &additional.reports {
        if report.coverage == Coverage::Unsupported {
            primary.coverage = Coverage::Unsupported;
        } else if report.coverage == Coverage::Rejected && primary.coverage == Coverage::Verified {
            primary.coverage = Coverage::Rejected;
        }
        primary.conflicts.extend(report.conflicts.iter().cloned());
        primary.issues.extend(report.issues.iter().cloned());
    }
    if !additional.reports.is_empty() {
        primary.note.push_str(&format!(" {} subrenames adicionais verificados em seus contextos; conflitos preservam caminho e faixa originais.",additional.reports.len()));
    }
}
fn rename_extra_proposal(
    root: &Path,
    snapshots: &[codisseu_syntax::RenameSnapshot],
    new_name: &str,
    search: &mut rename::RenameSearch,
    expected: Option<&[rename::resources::ResourceMove]>,
    token: &Cancellation<'_>,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<AdditionalRename, String> {
    reference_check(token, cancel)?;
    if matches!(
        Language::for_path(&search.origin),
        Some(Language::Rust | Language::Python)
    ) {
        return Ok(AdditionalRename {
            files: semantic_files(snapshots),
            resources: vec![],
            reports: vec![],
            issues: vec![],
        });
    }
    let mut files = semantic_files(snapshots);
    let mut all = snapshots.to_vec();
    all.extend(search.text_snapshots.iter().cloned());
    let mut resources = vec![];
    let mut extra = vec![];
    let mut reports = vec![];
    let mut issues = vec![];
    let mut resource_proposals = vec![];
    if search.origin.extension().is_some_and(|e| e == "php") {
        if search.options.rename_inheritors {
            let plan = rename::inheritors::plan(snapshots, new_name, &search.inheritors, cancel)?;
            search.inheritors = plan.choices;
            files = semantic_files(&plan.snapshots);
            all = plan.snapshots;
            all.extend(search.text_snapshots.iter().cloned());
            resource_proposals = plan.resources;
            reports.extend(plan.reports);
            issues.extend(plan.issues);
        }
    } else if let Some(proposal) = codisseu_syntax::plan_class_file_rename(
        snapshots,
        new_name,
        cancel,
        codisseu_syntax::ValidationLimits::default(),
    )? {
        resource_proposals.push(proposal);
    }
    for proposal in resource_proposals {
        let expected_disk = if let Some(previous) = expected {
            previous
                .iter()
                .find(|m| m.source == proposal.source && m.target == proposal.target)
                .ok_or("Movimentação proposta divergiu do preview")?
                .expected_disk
                .clone()
        } else {
            Arc::from(
                read_semantic_document(&proposal.source, token)?
                    .text
                    .into_bytes(),
            )
        };
        resources.push(rename::resources::ResourceMove {
            source: proposal.source,
            target: proposal.target,
            expected_disk,
        });
        extra.extend(proposal.edits);
    }
    if search.options.rename_accessors && Language::for_path(&search.origin) == Some(Language::Php)
    {
        let proposals = match codisseu_syntax::plan_property_accessor_renames(
            snapshots,
            new_name,
            cancel,
            codisseu_syntax::ValidationLimits::default(),
        ) {
            Ok(proposals) => proposals,
            Err(message) => {
                issues.push(ValidationIssue {
                    path: search.origin.clone(),
                    range: None,
                    message: format!("Accessors: {message}"),
                    kind: IssueKind::UnsupportedEdit,
                });
                vec![]
            }
        };
        if proposals.len() > 4 {
            return Err("Mais de quatro famílias de accessors".into());
        }
        for proposal in proposals {
            reference_check(token, cancel)?;
            let mut report = codisseu_syntax::validate_rename_with_limits(
                &proposal.snapshots,
                &proposal.new_name,
                cancel,
                codisseu_syntax::ValidationLimits::default(),
            );
            let prefix = format!("Accessor {} → {}: ", proposal.old_name, proposal.new_name);
            for conflict in &mut report.conflicts {
                conflict.message = format!("{prefix}{}", conflict.message);
            }
            for issue in &mut report.issues {
                issue.message = format!("{prefix}{}", issue.message);
            }
            reports.push(report);
            for snapshot in proposal.snapshots {
                extra.extend(
                    snapshot
                        .edits
                        .into_iter()
                        .map(|edit| (snapshot.path.clone(), edit)),
                );
            }
        }
    }
    // Add the AST-proven import changes to the exclusion ranges before explicit
    // comment/string search, so module paths cannot receive overlapping edits.
    for (path, edit) in &extra {
        all.iter_mut()
            .find(|s| s.path == *path)
            .ok_or("Import fora do corpus")?
            .edits
            .push(edit.clone());
    }
    if search.options.search_comments_strings || search.options.search_text {
        extra.extend(codisseu_syntax::plan_noncode_rename(
            &all,
            &search.origin,
            &search.old,
            new_name,
            search.options,
            cancel,
            codisseu_syntax::ValidationLimits::default(),
        )?);
    }
    for (path, edit) in extra {
        if !files.iter().any(|f| f.path == path) {
            let snapshot = all
                .iter()
                .find(|s| s.path == path)
                .ok_or("Ocorrência adicional fora do corpus")?;
            files.push(RenamePreviewFile {
                encoding: FileEncoding::Utf8,
                presentation: vec![],
                path: path.clone(),
                before: snapshot.before.clone(),
                after: snapshot.before.clone(),
                edits: vec![],
            });
        }
        let file = files.iter_mut().find(|f| f.path == path).unwrap();
        let edit = syntax_edit(&edit);
        if !file.edits.contains(&edit) {
            file.edits.push(edit);
        }
    }
    let mut bytes = 0;
    for file in &mut files {
        file.edits
            .sort_by_key(|e| (e.start_line, e.start_character, e.end_line, e.end_character));
        file.after = Arc::from(rename::build_after(&file.before, &file.edits)?);
        bytes += file.before.len() + file.after.len();
        if file.after.len() > MAX_DOCUMENT_BYTES {
            return Err("Rename excede 1 MiB por texto".into());
        }
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    if files.len() > 32
        || files.iter().map(|f| f.edits.len()).sum::<usize>() > 1024
        || bytes > 8 * 1024 * 1024
    {
        return Err(
            "Rename com ocorrências adicionais excede 32 arquivos/1024 edições/8 MiB".into(),
        );
    }
    if !resources.is_empty() {
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
            issues.push(ValidationIssue {
                path: search.origin.clone(),
                range: None,
                kind: IssueKind::UnsupportedEdit,
                message,
            });
        }
    }
    if !resources.is_empty()
        && let Err(message) = rename::resources::ResourcePlan::prepare(root, &resources)
    {
        issues.push(ValidationIssue {
            path: search.origin.clone(),
            range: None,
            kind: IssueKind::UnsupportedEdit,
            message,
        });
    }
    reference_check(token, cancel)?;
    Ok(AdditionalRename {
        files,
        resources,
        reports,
        issues,
    })
}
fn collect_rename_corpus(
    root: &Path,
    token: &Cancellation<'_>,
    cancel: &std::sync::atomic::AtomicBool,
    started: std::time::Instant,
) -> Result<rename::RenameCorpus, String> {
    collect_rename_corpus_impl(root, token, cancel, started, None)
}
fn collect_symbol_rename_corpus(
    root: &Path,
    origin: &Path,
    token: &Cancellation<'_>,
    cancel: &std::sync::atomic::AtomicBool,
    started: std::time::Instant,
) -> Result<rename::RenameCorpus, String> {
    collect_rename_corpus_impl(
        root,
        token,
        cancel,
        started,
        Language::for_path(origin).filter(|l| matches!(l, Language::Rust | Language::Python)),
    )
}
fn collect_rename_corpus_impl(
    root: &Path,
    token: &Cancellation<'_>,
    cancel: &std::sync::atomic::AtomicBool,
    started: std::time::Instant,
    only: Option<Language>,
) -> Result<rename::RenameCorpus, String> {
    let mut result = rename::RenameCorpus::default();
    let mut pending = vec![(root.to_path_buf(), 0usize)];
    let mut entries = 0usize;
    while let Some((dir, depth)) = pending.pop() {
        reference_check(token, cancel)?;
        if started.elapsed() > std::time::Duration::from_secs(2) {
            result.limitation = Some("Coleta de contexto excedeu 2 segundos".into());
            break;
        }
        let canonical = fs::canonicalize(&dir).map_err(|e| contextual(&dir, e))?;
        if canonical != dir || !canonical.starts_with(root) {
            return Err("Diretório de contexto mudou ou está fora do workspace".into());
        }
        for entry in fs::read_dir(&dir).map_err(|e| contextual(&dir, e))? {
            reference_check(token, cancel)?;
            if started.elapsed() > std::time::Duration::from_secs(2) {
                result.limitation = Some("Coleta de contexto excedeu 2 segundos".into());
                break;
            }
            entries += 1;
            if entries > 512 {
                result.limitation = Some("Coleta de contexto excede 512 entradas".into());
                break;
            }
            let entry = entry.map_err(|e| contextual(&dir, e))?;
            let kind = entry
                .file_type()
                .map_err(|e| contextual(&entry.path(), e))?;
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                if (only == Some(Language::Python)
                    && matches!(
                        entry.file_name().to_str(),
                        Some(".venv" | "venv" | "__pycache__" | ".pytest_cache" | ".ruff_cache")
                    ))
                    || matches!(
                        entry.file_name().to_str(),
                        Some(
                            ".git"
                                | "target"
                                | "node_modules"
                                | "vendor"
                                | ".codisseu"
                                | ".storm"
                                | ".tools"
                                | "artifacts"
                        )
                    )
                {
                    continue;
                }
                if depth >= 8 {
                    result.limitation = Some("Coleta de contexto excede profundidade 8".into());
                    break;
                }
                pending.push((entry.path(), depth + 1));
            } else if kind.is_file()
                && if let Some(language) = only {
                    Language::for_path(&entry.path()) == Some(language)
                } else {
                    matches!(
                        entry.path().extension().and_then(|s| s.to_str()),
                        Some("php" | "ts" | "tsx" | "js" | "jsx")
                    )
                }
            {
                let path = entry.path();
                let canonical = fs::canonicalize(&path).map_err(|e| contextual(&path, e))?;
                if canonical != path || !canonical.starts_with(root) {
                    return Err("Arquivo de contexto mudou ou está fora do workspace".into());
                }
                result.paths.push(path);
                if result.paths.len() > 64 {
                    result.limitation = Some(
                        if only.is_some() {
                            "Coleta de contexto excede64arquivos da linguagem"
                        } else {
                            "Coleta de contexto excede 64 arquivos PHP/TS"
                        }
                        .into(),
                    );
                    break;
                }
            }
        }
        if result.limitation.is_some() {
            break;
        }
    }
    result.paths.sort();
    Ok(result)
}
fn collect_text_corpus(
    root: &Path,
    token: &Cancellation<'_>,
    cancel: &std::sync::atomic::AtomicBool,
    started: std::time::Instant,
) -> Result<rename::RenameCorpus, String> {
    let mut result = rename::RenameCorpus::default();
    let mut pending = vec![(root.to_path_buf(), 0usize)];
    let mut entries = 0usize;
    while let Some((dir, depth)) = pending.pop() {
        reference_check(token, cancel)?;
        if started.elapsed() > std::time::Duration::from_secs(2) {
            result.limitation = Some("Coleta de contexto excedeu 2 segundos".into());
            break;
        }
        let canonical = fs::canonicalize(&dir).map_err(|e| contextual(&dir, e))?;
        if canonical != dir || !canonical.starts_with(root) {
            return Err("Diretório de contexto mudou ou está fora do workspace".into());
        }
        for entry in fs::read_dir(&dir).map_err(|e| contextual(&dir, e))? {
            reference_check(token, cancel)?;
            if started.elapsed() > std::time::Duration::from_secs(2) {
                result.limitation = Some("Coleta de contexto excedeu 2 segundos".into());
                break;
            }
            entries += 1;
            if entries > 512 {
                result.limitation = Some("Coleta de contexto excede 512 entradas".into());
                break;
            }
            let entry = entry.map_err(|e| contextual(&dir, e))?;
            let kind = entry
                .file_type()
                .map_err(|e| contextual(&entry.path(), e))?;
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                if matches!(
                    entry.file_name().to_str(),
                    Some(
                        ".git"
                            | "target"
                            | "node_modules"
                            | "vendor"
                            | ".codisseu"
                            | ".storm"
                            | ".tools"
                            | "artifacts"
                    )
                ) {
                    continue;
                }
                if depth >= 8 {
                    result.limitation = Some("Coleta de contexto excede profundidade 8".into());
                    break;
                }
                pending.push((entry.path(), depth + 1));
            } else if kind.is_file()
                && matches!(
                    entry.path().extension().and_then(|s| s.to_str()),
                    Some("txt" | "md")
                )
            {
                let path = entry.path();
                let canonical = fs::canonicalize(&path).map_err(|e| contextual(&path, e))?;
                if canonical != path || !canonical.starts_with(root) {
                    return Err("Arquivo de contexto mudou ou está fora do workspace".into());
                }
                result.paths.push(path);
                if result.paths.len() > 128 {
                    result.limitation =
                        Some("Coleta de contexto excede 128 arquivos de texto".into());
                    break;
                }
            }
        }
        if result.limitation.is_some() {
            break;
        }
    }
    result.paths.sort();
    Ok(result)
}
fn apply_javascript_rename_option(
    snapshots: &mut [codisseu_syntax::RenameSnapshot],
    files: &mut Vec<RenamePreviewFile>,
    search: &rename::RenameSearch,
) {
    if search.options.search_javascript
        || Language::for_path(&search.origin) != Some(Language::TypeScript)
    {
        return;
    }
    let javascript = |path: &Path| {
        matches!(
            path.extension().and_then(|s| s.to_str()),
            Some("js" | "jsx")
        )
    };
    for snapshot in snapshots.iter_mut().filter(|s| javascript(&s.path)) {
        snapshot.after = snapshot.before.clone();
        snapshot.edits.clear();
    }
    files.retain(|f| !javascript(&f.path));
}

fn rename_corpus_validation(
    snapshots: &[codisseu_syntax::RenameSnapshot],
    new_name: &str,
    cancel: &std::sync::atomic::AtomicBool,
    corpus: &rename::RenameCorpus,
    root: &Path,
    search: &mut rename::RenameSearch,
) -> RenameValidation {
    if matches!(
        Language::for_path(&search.origin),
        Some(Language::Rust | Language::Python)
    ) {
        let validate = if Language::for_path(&search.origin) == Some(Language::Python) {
            codisseu_syntax::python_rename::validate
        } else {
            codisseu_syntax::rust_rename::validate
        };
        let mut report = validate(snapshots, &search.origin, &search.old, new_name, cancel);
        if let Some(message) = &corpus.limitation {
            report.coverage = Coverage::Unsupported;
            report.issues.push(ValidationIssue {
                path: root.into(),
                range: None,
                kind: IssueKind::Limit,
                message: message.clone(),
            });
        }
        return report;
    }
    let extended_web = |s: &codisseu_syntax::RenameSnapshot| {
        matches!(
            s.path.extension().and_then(|e| e.to_str()),
            Some("js" | "jsx" | "tsx")
        )
    };
    let compiler_required = Language::for_path(&search.origin) == Some(Language::TypeScript)
        && (snapshots.iter().any(extended_web)
            || snapshots
                .iter()
                .flat_map(|s| &s.edits)
                .any(|e| e.new_text != new_name));
    let php_context: Vec<_> = if Language::for_path(&search.origin) == Some(Language::Php) {
        snapshots
            .iter()
            .filter(|s| !extended_web(s) || !s.edits.is_empty())
            .cloned()
            .collect()
    } else {
        Vec::new()
    };
    let mut report = if compiler_required {
        RenameValidation {coverage: Coverage::Unsupported, scope: ValidationScope::CompilerBindings,
            conflicts:vec![],issues:vec![ValidationIssue {path:search.origin.clone(),range:None,kind:IssueKind::UnsupportedLanguage,message:"Consumidores JavaScript/JSX/TSX ou shorthand exigem verificação do compilador configurado".into()}],
            checked_declarations:0,checked_override_pairs:0,related_methods:vec![],checked_files:0,incremental_reparses:0,note:"Proposta exige gramática e vínculos do compilador TypeScript.".into()}
    } else {
        codisseu_syntax::validate_rename(
            if php_context.is_empty() {
                snapshots
            } else {
                &php_context
            },
            new_name,
            cancel,
        )
    };
    if corpus.limitation.is_none() {
        worker_compiler::fallback(&mut report, snapshots, new_name, cancel, root, search);
    }
    report.note.push_str(" Corpus local: arquivos .php/.ts/.tsx/.js/.jsx do workspace; exclui .git, target, node_modules, vendor, .codisseu, .storm, .tools, artifacts e links simbólicos. Não comprova ausência de descendentes fora desse corpus.");
    if let Some(message) = &corpus.limitation {
        report.coverage = Coverage::Unsupported;
        report.issues.truncate(255);
        report.issues.push(ValidationIssue {
            path: root.into(),
            range: None,
            message: message.clone(),
            kind: IssueKind::Limit,
        });
    }
    report
}

#[cfg(test)]
mod rename_collection_tests {
    use super::*;
    #[test]
    fn collection_deadline_and_cancellation_never_claim_complete() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.ts"), "class A {}").unwrap();
        let generation = AtomicU64::new(0);
        let token = Cancellation {
            encoding_profile: None,
            workspace: None,
            generation: &generation,
            expected: 0,
        };
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let expired = collect_rename_corpus(
            dir.path(),
            &token,
            &cancel,
            std::time::Instant::now() - std::time::Duration::from_secs(3),
        )
        .unwrap();
        assert!(expired.limitation.as_ref().unwrap().contains("2 segundos"));
        cancel.store(true, Ordering::Release);
        assert!(
            collect_rename_corpus(dir.path(), &token, &cancel, std::time::Instant::now()).is_err()
        );
    }

    fn expanded_case(mode: &str) -> Result<rename::PreparedRename, String> {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let path = root.join("weather.ts");
        let text = "class Weather { rainfall(): string { return 'rain'; } }\nclass WetWeather extends Weather { rainfall(): string { return 'rain'; } }\nconst wet = new WetWeather(); wet.rainfall(); wet.rainfall();\nclass Unrelated { rainfall(): string { return 'other'; } }\n";
        fs::write(&path, text).unwrap();
        let edits: Vec<_> = text
            .match_indices("rainfall")
            .map(|(offset, _)| {
                let prefix = &text[..offset];
                let line = prefix.bytes().filter(|b| *b == b'\n').count() as u32;
                let character = prefix.rsplit('\n').next().unwrap().encode_utf16().count() as u32;
                RenameTextEdit {
                    start_line: line,
                    start_character: character,
                    end_line: line,
                    end_character: character + 8,
                    new_text: "precipitation".into(),
                }
            })
            .collect();
        let generation = AtomicU64::new(0);
        let token = Cancellation {
            encoding_profile: None,
            workspace: None,
            generation: &generation,
            expected: 0,
        };
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let initial = Work::RenamePreview {
            token: 1,
            workspace: root.clone(),
            changes: vec![codisseu_language::RenameFileEdits {
                path: path.clone(),
                version: None,
                edits: edits[..3].to_vec(),
            }],
            new_name: "precipitation".into(),
            buffers: vec![],
            context_limitation: None,
            local_origin: None,
            search: rename::RenameSearch::default(),
            cancel: cancel.clone(),
        };
        let Completed::Rename { prepared, .. } = execute(initial, &token)? else {
            unreachable!()
        };
        assert_eq!(prepared.validation.coverage, Coverage::Rejected);
        assert!(!prepared.validation.related_methods.is_empty());
        assert!(prepared.validation.issues.is_empty());
        assert!(
            prepared
                .validation
                .conflicts
                .iter()
                .all(|c| c.kind == codisseu_syntax::ConflictKind::MissingReference)
        );
        let mut supplement = edits[..4].to_vec();
        match mode {
            "overlap" => {
                let mut overlap = edits[2].clone();
                overlap.start_character += 1;
                supplement.push(overlap);
            }
            "unrelated" => {
                supplement.push(edits[4].clone());
            }
            "changed" => {
                fs::write(&path, format!("{text}// changed after baseline\n")).unwrap();
            }
            "cancelled" => {
                cancel.store(true, Ordering::Release);
            }
            _ => {}
        }
        let result = codisseu_language::RenameResult {
            transient_document_version: Some(1),
            path: path.clone(),
            version: 0,
            line: 1,
            character: 34,
            new_name: "precipitation".into(),
            changes: vec![codisseu_language::RenameFileEdits {
                path: path.clone(),
                version: Some(1),
                edits: supplement,
            }],
        };
        let completed = execute(
            Work::RenameExpand {
                token: 1,
                workspace: root,
                prepared,
                result,
                open_versions: vec![],
                new_name: "precipitation".into(),
                cancel,
            },
            &token,
        )?;
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            text,
            "expansion never writes"
        );
        let Completed::Rename { prepared, .. } = completed else {
            unreachable!()
        };
        Ok(*prepared)
    }

    #[test]
    fn related_union_deduplicates_real_ranges_without_replacing_original_baselines() {
        let result = expanded_case("safe").unwrap();
        assert_eq!(result.files.len(), 1);
        assert_eq!(result.files[0].edits.len(), 4);
        assert_eq!(result.validation.coverage, Coverage::Verified);
        assert!(
            result.files[0]
                .after
                .contains("class Unrelated { rainfall()")
        );
        assert!(expanded_case("changed").is_err());
        assert!(expanded_case("cancelled").is_err());
    }

    #[test]
    fn related_union_rejects_overlaps_and_additional_unrelated_declarations() {
        assert!(expanded_case("overlap").is_err());
        let error = expanded_case("unrelated")
            .err()
            .expect("unrelated declaration must fail");
        assert!(
            error.contains("declarações fora do conjunto original"),
            "{error}"
        );
    }
}

#[cfg(test)]
#[path = "rename/accessor_tests.rs"]
mod rename_accessor_tests;
