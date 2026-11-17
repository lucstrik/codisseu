//! Workspace-scoped creation and recoverable deletion; never permanently removes files.
use super::{
    project_files::{self, FileReceipt, FileResultKind},
    *,
};
use std::{
    fs,
    path::Path,
    process::{Command as Process, Stdio},
    time::{Duration, Instant},
};

fn checked_root(workspace: &Path) -> Result<PathBuf, String> {
    let root = fs::canonicalize(workspace).map_err(|error| error.to_string())?;
    if !root.is_dir() {
        return Err("O projeto não é uma pasta".into());
    }
    Ok(root)
}
fn checked_parent(root: &Path, parent: &Path) -> Result<PathBuf, String> {
    if !parent.starts_with(root)
        || parent
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
        || parent
            .strip_prefix(root)
            .ok()
            .is_some_and(|path| path.components().any(|part| part.as_os_str() == ".git"))
    {
        return Err("A pasta deve estar dentro do projeto".into());
    }
    let canonical = fs::canonicalize(parent).map_err(|error| error.to_string())?;
    if !canonical.starts_with(root) || canonical != parent || !canonical.is_dir() {
        return Err(
            "A pasta selecionada aponta para fora do projeto ou contém um link simbólico".into(),
        );
    }
    Ok(canonical)
}
pub(super) fn perform(
    workspace: PathBuf,
    command: ProjectFileCommand,
    open_paths: &[PathBuf],
) -> Result<FileReceipt, String> {
    let root = checked_root(&workspace)?;
    match command {
        ProjectFileCommand::Refresh => Ok(FileReceipt {
            workspace: root.clone(),
            path: root,
            kind: FileResultKind::Refresh,
        }),
        ProjectFileCommand::CreateFile { parent, name } => {
            let target = parent.join(project_files::relative_name(&name)?);
            guard_open_target(&target, None, open_paths)?;
            create(root, parent, name, false)
        }
        ProjectFileCommand::CreateFolder { parent, name } => create(root, parent, name, true),
        ProjectFileCommand::RenameFile { path, name } => {
            let target = path.parent().ok_or("Item sem pasta")?.join(&name);
            guard_open_target(&target, Some(&path), open_paths)?;
            rename_file(root, path, name)
        }
        ProjectFileCommand::Transfer {
            source,
            parent,
            move_item,
        } => transfer(root, source, parent, move_item, open_paths),
        ProjectFileCommand::Trash { path } => trash(root, path),
        ProjectFileCommand::TrashMany { paths } => trash_many(&[root], paths, &|| false),
    }
}
fn guard_open_target(
    target: &Path,
    source: Option<&Path>,
    open_paths: &[PathBuf],
) -> Result<(), String> {
    if open_paths.iter().any(|path| {
        path.starts_with(target) && source.is_none_or(|source| !path.starts_with(source))
    }) {
        return Err("Feche o documento aberto no destino antes de transferir ou criar".into());
    }
    Ok(())
}
pub(super) fn create(
    root: PathBuf,
    parent: PathBuf,
    name: String,
    folder: bool,
) -> Result<FileReceipt, String> {
    let relative = project_files::relative_name(&name)?;
    let parent = checked_parent(&root, &parent)?;
    let target = parent.join(&relative);
    let mut current = parent;
    let parts: Vec<_> = relative.components().collect();
    for (index, part) in parts.iter().enumerate() {
        current.push(part.as_os_str());
        if index + 1 == parts.len() {
            break;
        }
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => return Err("Um componente do caminho é arquivo ou link simbólico".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&current).map_err(|error| error.to_string())?
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    if folder {
        fs::create_dir(&target)
            .map_err(|error| format!("Não foi possível criar pasta: {error}"))?;
    } else {
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .map_err(|error| format!("Não foi possível criar arquivo: {error}"))?;
        file.sync_all().map_err(|error| error.to_string())?;
    }
    Ok(FileReceipt {
        workspace: root,
        path: target,
        kind: if folder {
            FileResultKind::Folder
        } else {
            FileResultKind::File
        },
    })
}
fn validate_trash(root: &Path, path: &Path) -> Result<(), String> {
    if path == root
        || !path.starts_with(root)
        || path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err(
            "A raiz do projeto e caminhos externos não podem ser movidos para a Lixeira".into(),
        );
    }
    let relative = path.strip_prefix(root).map_err(|error| error.to_string())?;
    if relative.components().any(|part| part.as_os_str() == ".git") {
        return Err("A pasta interna .git é protegida".into());
    }
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink() || (!metadata.is_file() && !metadata.is_dir()) {
        return Err("Selecione um arquivo ou pasta regular do projeto".into());
    }
    let parent = path.parent().ok_or("Caminho inválido")?;
    checked_parent(root, parent)?;
    Ok(())
}

pub(super) fn trash_many(
    roots: &[PathBuf],
    paths: Vec<PathBuf>,
    cancelled: &dyn Fn() -> bool,
) -> Result<FileReceipt, String> {
    trash_many_with(roots, paths, cancelled, trash)
}

fn trash_many_with(
    roots: &[PathBuf],
    paths: Vec<PathBuf>,
    cancelled: &dyn Fn() -> bool,
    mut move_item: impl FnMut(PathBuf, PathBuf) -> Result<FileReceipt, String>,
) -> Result<FileReceipt, String> {
    if paths.is_empty() || paths.len() > 10_000 {
        return Err("Selecione de 1 a 10000 itens para mover para a Lixeira".into());
    }
    let roots = roots
        .iter()
        .map(|root| checked_root(root))
        .collect::<Result<Vec<_>, _>>()?;
    let workspace = roots.first().ok_or("Abra uma pasta")?.clone();
    let mut checked = Vec::with_capacity(paths.len());
    // Validate every supplied entry before folding parent/child selections: a protected
    // or stale selected child must not silently disappear behind its selected parent.
    for path in paths {
        if cancelled() {
            return Err("Operação cancelada antes de mover os itens".into());
        }
        let root = roots
            .iter()
            .find(|root| path.starts_with(root))
            .ok_or("Item fora das pastas do workspace")?;
        validate_trash(root, &path).map_err(|error| format!("{}: {error}", path.display()))?;
        checked.push((path, root.clone()));
    }
    checked.sort_by(|a, b| a.0.cmp(&b.0));
    checked.dedup_by(|a, b| a.0 == b.0);
    let mut effective: Vec<(PathBuf, PathBuf)> = Vec::new();
    for (path, root) in checked {
        if effective
            .last()
            .is_none_or(|(ancestor, _)| !path.starts_with(ancestor))
        {
            effective.push((path, root));
        }
    }
    let mut moved = Vec::new();
    let mut failures = Vec::new();
    for (path, root) in effective {
        if cancelled() {
            failures.push((
                path,
                "Operação cancelada; item não enviado à Lixeira".into(),
            ));
            continue;
        }
        // The individual operation revalidates immediately before GIO. A failure after
        // earlier successes is data in the receipt, never an Err that would lose ACKs.
        match move_item(root, path.clone()) {
            Ok(_) => moved.push(path),
            Err(error) => failures.push((path, error)),
        }
    }
    Ok(FileReceipt {
        workspace: workspace.clone(),
        path: workspace,
        kind: FileResultKind::TrashedMany {
            paths: moved,
            failures,
        },
    })
}

fn trash(root: PathBuf, path: PathBuf) -> Result<FileReceipt, String> {
    validate_trash(&root, &path)?;
    #[cfg(target_os = "linux")]
    {
        // GIO delegates to the desktop's recoverable Trash implementation. No rm fallback.
        let mut process = Process::new("/usr/bin/gio")
            .arg("trash")
            .arg("--")
            .arg(&path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| {
                format!("Lixeira do sistema indisponível; arquivo preservado: {error}")
            })?;
        let until = Instant::now() + Duration::from_secs(15);
        let status = loop {
            if let Some(status) = process.try_wait().map_err(|error| error.to_string())? {
                break status;
            }
            if Instant::now() >= until {
                let _ = process.kill();
                let _ = process.wait();
                return Err(
                    "A Lixeira não respondeu em15s; confira o arquivo e a Lixeira antes de repetir"
                        .into(),
                );
            }
            std::thread::sleep(Duration::from_millis(15));
        };
        if !status.success() {
            use std::io::Read;
            let mut detail = String::new();
            if let Some(stderr) = process.stderr.take() {
                let _ = stderr.take(8192).read_to_string(&mut detail);
            }
            return Err(format!(
                "Não foi possível mover para a Lixeira: {}",
                detail.trim()
            ));
        }
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Ok(_) => {
                return Err(
                    "O sistema não confirmou a remoção da origem; atualize a árvore".into(),
                );
            }
            Err(error) => {
                return Err(format!(
                    "Não foi possível confirmar a origem; confira o arquivo e a Lixeira: {error}"
                ));
            }
        }
        Ok(FileReceipt {
            workspace: root,
            path,
            kind: FileResultKind::Trash,
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err("Lixeira ainda não implementada nesta plataforma; arquivo preservado".into())
    }
}

#[cfg(test)]
mod trash_batch_tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn trash_batch_validates_every_member_before_any_move() {
        let owned = tempfile::tempdir().unwrap();
        let root = owned.path().join("project");
        fs::create_dir_all(root.join(".git")).unwrap();
        let first = root.join("first.txt");
        fs::write(&first, "original").unwrap();
        fs::write(root.join(".git/config"), "protected").unwrap();
        let called = Cell::new(false);
        let result = trash_many_with(
            std::slice::from_ref(&root),
            vec![first.clone(), root.join(".git/config")],
            &|| false,
            |_, _| {
                called.set(true);
                panic!("preflight must reject before any move")
            },
        );
        assert!(result.is_err());
        assert!(!called.get());
        assert_eq!(fs::read_to_string(first).unwrap(), "original");
    }

    #[test]
    fn trash_batch_partial_failure_keeps_committed_receipts_and_continues_other_items() {
        let owned = tempfile::tempdir().unwrap();
        let root = owned.path().join("project");
        let sink = owned.path().join("test-sink");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&sink).unwrap();
        let paths = [root.join("a.txt"), root.join("b.txt"), root.join("c.txt")];
        for (index, path) in paths.iter().enumerate() {
            fs::write(path, index.to_string()).unwrap();
        }
        let receipt = trash_many_with(
            std::slice::from_ref(&root),
            paths.to_vec(),
            &|| false,
            |workspace, path| {
                if path == paths[1] {
                    return Err("controlled desktop failure".into());
                }
                fs::rename(&path, sink.join(path.file_name().unwrap())).unwrap();
                Ok(FileReceipt {
                    workspace,
                    path,
                    kind: FileResultKind::Trash,
                })
            },
        )
        .unwrap();
        let FileResultKind::TrashedMany {
            paths: moved,
            failures,
        } = receipt.kind
        else {
            panic!("batch receipt")
        };
        assert_eq!(moved, [paths[0].clone(), paths[2].clone()]);
        assert_eq!(
            failures,
            [(paths[1].clone(), "controlled desktop failure".into())]
        );
        assert!(!paths[0].exists() && !paths[2].exists());
        assert_eq!(fs::read_to_string(&paths[1]).unwrap(), "1");
        assert_eq!(fs::read_to_string(sink.join("a.txt")).unwrap(), "0");
        assert_eq!(fs::read_to_string(sink.join("c.txt")).unwrap(), "2");
    }

    #[test]
    fn trash_batch_cancel_between_items_preserves_committed_ack_and_remaining_sources() {
        let owned = tempfile::tempdir().unwrap();
        let root = owned.path().join("project");
        fs::create_dir(&root).unwrap();
        let paths = [root.join("a.txt"), root.join("b.txt")];
        for path in &paths {
            fs::write(path, "original").unwrap();
        }
        let cancelled = Cell::new(false);
        let receipt = trash_many_with(
            std::slice::from_ref(&root),
            paths.to_vec(),
            &|| cancelled.get(),
            |workspace, path| {
                fs::rename(&path, owned.path().join("test-sink.txt")).unwrap();
                cancelled.set(true);
                Ok(FileReceipt {
                    workspace,
                    path,
                    kind: FileResultKind::Trash,
                })
            },
        )
        .unwrap();
        let FileResultKind::TrashedMany {
            paths: moved,
            failures,
        } = receipt.kind
        else {
            panic!("batch receipt")
        };
        assert_eq!(moved, [paths[0].clone()]);
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].0, paths[1]);
        assert!(failures[0].1.contains("cancelada"));
        assert!(!paths[0].exists());
        assert_eq!(fs::read_to_string(&paths[1]).unwrap(), "original");
        assert_eq!(
            fs::read_to_string(owned.path().join("test-sink.txt")).unwrap(),
            "original"
        );
    }
}

fn rename_file(root: PathBuf, path: PathBuf, name: String) -> Result<FileReceipt, String> {
    use std::io::Read;
    let relative = project_files::relative_name(&name)?;
    if relative.components().count() != 1 {
        return Err("Renomear aceita somente um nome, sem pastas".into());
    }
    if path
        .strip_prefix(&root)
        .ok()
        .is_none_or(|relative| relative.components().any(|part| part.as_os_str() == ".git"))
    {
        return Err("Arquivo fora do projeto ou protegido".into());
    }
    let parent = checked_parent(&root, path.parent().ok_or("Arquivo sem pasta")?)?;
    let metadata = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        if path == root {
            return Err("A raiz do projeto não pode ser renomeada".into());
        }
        let target = parent.join(relative);
        rename_item_no_replace(&root, &path, &target)?;
        return Ok(FileReceipt {
            workspace: root,
            path: target,
            kind: FileResultKind::Renamed { source: path },
        });
    }
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 16 * 1024 * 1024
    {
        return Err("Renomear exige arquivo regular de até16MiB".into());
    }
    let target = parent.join(relative);
    match fs::symlink_metadata(&target) {
        Ok(_) => return Err(format!("Destino já existe; escolha outro nome: {name}")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }
    let mut bytes = Vec::new();
    fs::File::open(&path)
        .map_err(|error| error.to_string())?
        .take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    let change = rename::resources::ResourceMove {
        source: path.clone(),
        target: target.clone(),
        expected_disk: Arc::from(bytes),
    };
    rename::resources::ResourcePlan::prepare(&root, &[change])?
        .apply()
        .map_err(|error| error.to_string())?;
    Ok(FileReceipt {
        workspace: root,
        path: target,
        kind: FileResultKind::Renamed { source: path },
    })
}

fn checked_item(root: &Path, path: &Path) -> Result<(), String> {
    if path == root
        || !path.starts_with(root)
        || path
            .strip_prefix(root)
            .is_ok_and(|relative| relative.components().any(|part| part.as_os_str() == ".git"))
    {
        return Err("Selecione um item do projeto, sem a raiz ou .git".into());
    }
    checked_parent(root, path.parent().ok_or("Item sem pasta")?)?;
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink() || (!metadata.is_file() && !metadata.is_dir()) {
        return Err("Links simbólicos e arquivos especiais não são transferidos".into());
    }
    Ok(())
}

fn transfer(
    root: PathBuf,
    source: PathBuf,
    parent: PathBuf,
    move_item: bool,
    open_paths: &[PathBuf],
) -> Result<FileReceipt, String> {
    checked_item(&root, &source)?;
    let parent = checked_parent(&root, &parent)?;
    if parent.starts_with(&source) {
        return Err("O destino não pode estar dentro da origem".into());
    }
    let mut target = parent.join(source.file_name().ok_or("Item sem nome")?);
    if !move_item && target == source {
        let is_file = source.is_file();
        let stem = if is_file {
            source.file_stem()
        } else {
            source.file_name()
        }
        .ok_or("Item sem nome")?;
        for number in 1..=1000 {
            let mut name = stem.to_os_string();
            name.push(if number == 1 {
                " copy".to_owned()
            } else {
                format!(" copy {number}")
            });
            if is_file && let Some(extension) = source.extension() {
                name.push(".");
                name.push(extension);
            }
            target = parent.join(name);
            if fs::symlink_metadata(&target)
                .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
            {
                break;
            }
        }
    }
    match fs::symlink_metadata(&target) {
        Ok(_) => return Err("Destino já existe; escolha outra pasta".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }
    guard_open_target(&target, move_item.then_some(source.as_path()), open_paths)?;
    if move_item {
        rename_item_no_replace(&root, &source, &target)?;
        return Ok(FileReceipt {
            workspace: root,
            path: target,
            kind: FileResultKind::Renamed { source },
        });
    }
    let scope = ScopedRoot::new(&root)?;
    let destination_directory = scope.open(&parent, true)?;
    let staging = CopyStaging(
        tempfile::Builder::new()
            .prefix(".codisseu-copy-")
            .tempdir_in(descriptor_path(&destination_directory)?)
            .map_err(|error| error.to_string())?,
    );
    let staged = staging.0.path().join("item");
    let mut budget = CopyBudget {
        entries: 0,
        bytes: 0,
        deadline: Instant::now() + Duration::from_secs(30),
    };
    copy_item(&scope, &source, &staged, &mut budget, 0)?;
    if Instant::now() >= budget.deadline {
        return Err("Cópia excedeu30s; origem preservada".into());
    }
    let stage_directory = fs::File::open(staging.0.path()).map_err(|error| error.to_string())?;
    let destination_now = scope.open(&parent, true)?;
    same_identity(
        &destination_directory
            .metadata()
            .map_err(|e| e.to_string())?,
        &destination_now.metadata().map_err(|e| e.to_string())?,
    )?;
    rename_in_directories(
        &stage_directory,
        std::ffi::OsStr::new("item"),
        &destination_now,
        target.file_name().ok_or("Destino sem nome")?,
    )?;
    Ok(FileReceipt {
        workspace: root,
        path: target,
        kind: FileResultKind::Copied,
    })
}

struct CopyBudget {
    entries: usize,
    bytes: u64,
    deadline: Instant,
}
struct CopyStaging(tempfile::TempDir);
impl Drop for CopyStaging {
    fn drop(&mut self) {
        fn allow_removal(path: &Path) {
            let Ok(metadata) = fs::symlink_metadata(path) else {
                return;
            };
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(
                    path,
                    fs::Permissions::from_mode(metadata.permissions().mode() | 0o700),
                );
            }
            if let Ok(entries) = fs::read_dir(path) {
                for entry in entries.flatten() {
                    allow_removal(&entry.path());
                }
            }
        }
        // Completed read-only subdirectories must not prevent failed-copy cleanup.
        allow_removal(self.0.path());
    }
}
fn copy_item(
    scope: &ScopedRoot,
    source: &Path,
    target: &Path,
    budget: &mut CopyBudget,
    depth: usize,
) -> Result<(), String> {
    let expected = fs::symlink_metadata(source).map_err(|error| error.to_string())?;
    if expected.file_type().is_symlink() {
        return Err("A cópia contém link simbólico; origem preservada".into());
    }
    if !expected.is_file() && !expected.is_dir() {
        return Err("A cópia contém arquivo especial; origem preservada".into());
    }
    let mut input = scope.open(source, expected.is_dir())?;
    let metadata = input.metadata().map_err(|error| error.to_string())?;
    same_identity(&expected, &metadata)?;
    budget.entries += 1;
    if metadata.is_file() {
        budget.bytes = budget.bytes.saturating_add(metadata.len());
    }
    if depth > 128
        || budget.entries > 10_000
        || budget.bytes > 256 * 1024 * 1024
        || Instant::now() >= budget.deadline
    {
        return Err("Cópia excedeu o limite de10.000 itens,256MiB ou30s; origem preservada".into());
    }
    if metadata.file_type().is_symlink() {
        return Err("A cópia contém link simbólico; origem preservada".into());
    }
    if metadata.is_dir() {
        fs::create_dir(target).map_err(|error| error.to_string())?;
        for entry in fs::read_dir(descriptor_path(&input)?).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            if entry.file_name() == ".git" {
                return Err("A cópia contém um repositório .git; origem preservada".into());
            }
            copy_item(
                scope,
                &source.join(entry.file_name()),
                &target.join(entry.file_name()),
                budget,
                depth + 1,
            )?;
        }
        // Tighten only after children were copied. A failure can leave read-only
        // directories, so the staging guard restores owner access before removal.
        fs::set_permissions(target, metadata.permissions()).map_err(|error| error.to_string())?;
    } else if metadata.is_file() {
        use std::io::Read;
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(target)
            .map_err(|error| error.to_string())?;
        let copied = std::io::copy(&mut (&mut input).take(metadata.len() + 1), &mut output)
            .map_err(|error| error.to_string())?;
        if copied != metadata.len()
            || input
                .metadata()
                .map_err(|error| error.to_string())?
                .modified()
                .ok()
                != metadata.modified().ok()
        {
            return Err("Origem mudou durante a cópia; tente novamente".into());
        }
        fs::set_permissions(target, metadata.permissions()).map_err(|error| error.to_string())?;
    } else {
        return Err("A cópia contém arquivo especial; origem preservada".into());
    }
    let current = scope.open(source, metadata.is_dir())?;
    let current_metadata = current.metadata().map_err(|error| error.to_string())?;
    same_identity(&metadata, &current_metadata)?;
    if metadata.modified().ok() != current_metadata.modified().ok() {
        return Err("Origem mudou durante a cópia; tente novamente".into());
    }
    Ok(())
}

fn same_identity(expected: &fs::Metadata, actual: &fs::Metadata) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if expected.dev() != actual.dev() || expected.ino() != actual.ino() {
            return Err("Item mudou durante a operação; origem preservada".into());
        }
    }
    if expected.is_dir() != actual.is_dir() || expected.is_file() != actual.is_file() {
        return Err("Tipo do item mudou durante a operação".into());
    }
    Ok(())
}

/// Resolve every path beneath a held workspace root, forbidding symlinks in
/// every component in the kernel rather than checking and following later.
struct ScopedRoot {
    path: PathBuf,
    directory: fs::File,
}
impl ScopedRoot {
    fn new(root: &Path) -> Result<Self, String> {
        let directory = scoped_open_at(-100, root, true, false)?;
        Ok(Self {
            path: root.into(),
            directory,
        })
    }
    fn open(&self, path: &Path, directory: bool) -> Result<fs::File, String> {
        let relative = path
            .strip_prefix(&self.path)
            .map_err(|_| "Caminho fora do projeto")?;
        if relative
            .components()
            .any(|part| !matches!(part, std::path::Component::Normal(_)))
        {
            return Err("Caminho relativo inválido".into());
        }
        let relative = if relative.as_os_str().is_empty() {
            Path::new(".")
        } else {
            relative
        };
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            scoped_open_at(self.directory.as_raw_fd(), relative, directory, true)
        }
        #[cfg(not(unix))]
        {
            let _ = relative;
            let _ = directory;
            Err("Transferência segura indisponível nesta plataforma".into())
        }
    }
}

#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
fn scoped_open_at(
    fd: i32,
    path: &Path,
    directory: bool,
    beneath: bool,
) -> Result<fs::File, String> {
    use std::{
        ffi::{CString, c_long},
        os::{fd::FromRawFd, unix::ffi::OsStrExt},
    };
    #[repr(C)]
    struct OpenHow {
        flags: u64,
        mode: u64,
        resolve: u64,
    }
    unsafe extern "C" {
        fn syscall(number: c_long, ...) -> c_long;
    }
    let path = CString::new(path.as_os_str().as_bytes()).map_err(|_| "NUL no caminho")?;
    let how = OpenHow {
        // O_CLOEXEC | O_NOFOLLOW | O_NONBLOCK and optionally O_DIRECTORY.
        flags: 0x80000 | 0x20000 | 0x800 | if directory { 0x10000 } else { 0 },
        mode: 0,
        // RESOLVE_NO_SYMLINKS | NO_MAGICLINKS, and BENEATH for relative paths.
        resolve: 0x4 | 0x2 | if beneath { 0x8 } else { 0 },
    };
    let result = unsafe {
        syscall(
            437,
            fd as c_long,
            path.as_ptr(),
            &how,
            std::mem::size_of::<OpenHow>(),
        )
    };
    if result < 0 {
        return Err(format!(
            "Não foi possível abrir item sem links dentro do projeto: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(unsafe { fs::File::from_raw_fd(result as i32) })
}
#[cfg(not(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
)))]
fn scoped_open_at(_: i32, _: &Path, _: bool, _: bool) -> Result<fs::File, String> {
    Err("Transferência segura ainda não certificada nesta plataforma".into())
}
#[cfg(target_os = "linux")]
fn descriptor_path(file: &fs::File) -> Result<PathBuf, String> {
    use std::os::fd::AsRawFd;
    Ok(PathBuf::from(format!("/proc/self/fd/{}", file.as_raw_fd())))
}
#[cfg(not(target_os = "linux"))]
fn descriptor_path(_: &fs::File) -> Result<PathBuf, String> {
    Err("Transferência segura ainda não certificada nesta plataforma".into())
}

#[cfg(target_os = "linux")]
fn rename_item_no_replace(root: &Path, source: &Path, target: &Path) -> Result<(), String> {
    let scope = ScopedRoot::new(root)?;
    let old_dir = scope.open(source.parent().ok_or("Origem sem pasta")?, true)?;
    let new_dir = scope.open(target.parent().ok_or("Destino sem pasta")?, true)?;
    rename_in_directories(
        &old_dir,
        source.file_name().ok_or("Origem sem nome")?,
        &new_dir,
        target.file_name().ok_or("Destino sem nome")?,
    )
}
#[cfg(target_os = "linux")]
fn rename_in_directories(
    old_dir: &fs::File,
    source: &std::ffi::OsStr,
    new_dir: &fs::File,
    target: &std::ffi::OsStr,
) -> Result<(), String> {
    use std::{
        ffi::{CString, c_char, c_int, c_uint},
        os::{fd::AsRawFd, unix::ffi::OsStrExt},
    };
    unsafe extern "C" {
        fn renameat2(
            oldfd: c_int,
            old: *const c_char,
            newfd: c_int,
            new: *const c_char,
            flags: c_uint,
        ) -> c_int;
    }
    let old = CString::new(source.as_bytes()).map_err(|_| "NUL no caminho")?;
    let new = CString::new(target.as_bytes()).map_err(|_| "NUL no caminho")?;
    // Held directory descriptors and RENAME_NOREPLACE prevent replacement of an existing target.
    if unsafe {
        renameat2(
            old_dir.as_raw_fd(),
            old.as_ptr(),
            new_dir.as_raw_fd(),
            new.as_ptr(),
            1,
        )
    } != 0
    {
        return Err(format!(
            "Não foi possível mover; origem preservada: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}
#[cfg(not(target_os = "linux"))]
fn rename_item_no_replace(_: &Path, _: &Path, _: &Path) -> Result<(), String> {
    Err("Mover pastas ainda não certificado nesta plataforma".into())
}
#[cfg(not(target_os = "linux"))]
fn rename_in_directories(
    _: &fs::File,
    _: &std::ffi::OsStr,
    _: &fs::File,
    _: &std::ffi::OsStr,
) -> Result<(), String> {
    Err("Transferência segura ainda não certificada nesta plataforma".into())
}

#[cfg(all(test, target_os = "linux"))]
mod transfer_integrity_tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[test]
    fn scoped_open_and_move_reject_replaced_ancestor_without_touching_outside() {
        let owned = tempfile::tempdir().unwrap();
        let root = owned.path().join("workspace");
        let outside = owned.path().join("outside");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::create_dir(root.join("parent")).unwrap();
        fs::write(root.join("parent/item"), "inside").unwrap();
        fs::write(outside.join("item"), "outside").unwrap();
        let scope = ScopedRoot::new(&root).unwrap();
        fs::rename(root.join("parent"), root.join("original")).unwrap();
        symlink(&outside, root.join("parent")).unwrap();
        assert!(scope.open(&root.join("parent/item"), false).is_err());
        assert!(
            rename_item_no_replace(&root, &root.join("original/item"), &root.join("parent/new"))
                .is_err()
        );
        assert_eq!(
            fs::read_to_string(root.join("original/item")).unwrap(),
            "inside"
        );
        assert_eq!(fs::read_to_string(outside.join("item")).unwrap(), "outside");
        assert!(!outside.join("new").exists());
    }

    #[test]
    fn copied_private_and_read_only_directories_keep_permissions() {
        let owned = tempfile::tempdir().unwrap();
        let root = owned.path().to_owned();
        fs::create_dir_all(root.join("source/read-only")).unwrap();
        fs::create_dir(root.join("destination")).unwrap();
        fs::write(root.join("source/read-only/item"), "preserve").unwrap();
        fs::set_permissions(root.join("source"), fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(
            root.join("source/read-only"),
            fs::Permissions::from_mode(0o500),
        )
        .unwrap();
        transfer(
            root.clone(),
            root.join("source"),
            root.join("destination"),
            false,
            &[],
        )
        .unwrap();
        assert_eq!(
            fs::metadata(root.join("destination/source"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(root.join("destination/source/read-only"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o500
        );
        assert_eq!(
            fs::read_to_string(root.join("destination/source/read-only/item")).unwrap(),
            "preserve"
        );
        fs::set_permissions(
            root.join("source/read-only"),
            fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        fs::set_permissions(
            root.join("destination/source/read-only"),
            fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        assert_eq!(fs::read_dir(root.join("destination")).unwrap().count(), 1);
    }

    #[test]
    fn failed_copy_cleans_completed_read_only_staging_directories() {
        let owned = tempfile::tempdir().unwrap();
        let staging = CopyStaging(tempfile::tempdir_in(owned.path()).unwrap());
        let path = staging.0.path().to_owned();
        fs::create_dir(path.join("read-only")).unwrap();
        fs::write(path.join("read-only/item"), "staged").unwrap();
        fs::set_permissions(path.join("read-only"), fs::Permissions::from_mode(0o500)).unwrap();
        drop(staging);
        assert!(!path.exists());
        assert_eq!(fs::read_dir(owned.path()).unwrap().count(), 0);
    }
}
