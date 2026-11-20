//! Project-scoped drawing links. All filesystem work runs on a bounded worker.
use codisseu_draw::{DrawingDocument, Scene};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::mpsc::{self, Receiver, SyncSender, TryRecvError},
};

const MAX_PATHS: usize = 128;
const MAX_METADATA: u64 = 1024 * 1024;

#[derive(Clone)]
struct Context {
    project: PathBuf,
    roots: Vec<PathBuf>,
}

enum Action {
    Load(Option<Context>),
    Register(Context, PathBuf),
    Create(Context, String),
}

struct Reply {
    generation: u64,
    context: Option<Context>,
    paths: Option<Vec<PathBuf>>,
    open: Vec<PathBuf>,
    error: Option<String>,
}

/// Links belong to the canonical project, independently of editor tab closure.
pub struct DrawingCatalog {
    pub project: Option<PathBuf>,
    pub paths: Vec<PathBuf>,
    pub error: Option<String>,
    roots: Vec<PathBuf>,
    open: Vec<PathBuf>,
    busy: bool,
    generation: u64,
    send: SyncSender<(u64, Action)>,
    receive: Receiver<Reply>,
}

impl DrawingCatalog {
    pub fn new(directory: PathBuf) -> Self {
        let (send, requests) = mpsc::sync_channel::<(u64, Action)>(1);
        let (replies, receive) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            while let Ok((generation, action)) = requests.recv() {
                let reply = perform(&directory, generation, action);
                if replies.send(reply).is_err() {
                    break;
                }
            }
        });
        Self {
            project: None,
            paths: vec![],
            error: None,
            roots: vec![],
            open: vec![],
            busy: false,
            generation: 0,
            send,
            receive,
        }
    }

    pub fn is_busy(&self) -> bool {
        self.busy
    }

    pub fn poll(&mut self) -> bool {
        match self.receive.try_recv() {
            Ok(reply) => {
                if reply.generation != self.generation {
                    return false;
                }
                self.busy = false;
                if let Some(context) = reply.context {
                    self.project = Some(context.project);
                    self.roots = context.roots;
                }
                if let Some(paths) = reply.paths {
                    self.paths = paths;
                }
                self.open.extend(reply.open);
                self.error = reply.error;
                true
            }
            Err(TryRecvError::Disconnected) if self.busy => {
                self.busy = false;
                self.error = Some("O serviço de desenhos foi encerrado".into());
                true
            }
            Err(_) => false,
        }
    }

    pub fn set_project(
        &mut self,
        project: Option<PathBuf>,
        roots: Vec<PathBuf>,
    ) -> Result<(), String> {
        let context = project.clone().map(|project| Context { project, roots });
        self.submit(Action::Load(context))?;
        self.project = project;
        self.paths.clear();
        self.roots.clear();
        self.open.clear();
        Ok(())
    }

    pub fn register(&mut self, path: PathBuf) -> Result<(), String> {
        self.submit(Action::Register(self.context()?, path))
    }

    /// A basename is placed in `drawings/`; an explicit relative path is kept.
    pub fn create(&mut self, relative_path: String) -> Result<(), String> {
        self.submit(Action::Create(self.context()?, relative_path))
    }

    pub fn take_open(&mut self) -> Vec<PathBuf> {
        std::mem::take(&mut self.open)
    }

    fn context(&self) -> Result<Context, String> {
        let project = self
            .project
            .clone()
            .ok_or("Abra um projeto para usar desenhos")?;
        if self.roots.is_empty() {
            return Err("O projeto não possui uma pasta disponível para desenhos".into());
        }
        Ok(Context {
            project,
            roots: self.roots.clone(),
        })
    }

    fn submit(&mut self, action: Action) -> Result<(), String> {
        if self.busy {
            return Err("Aguarde a operação de desenhos em andamento".into());
        }
        let generation = self.generation.wrapping_add(1);
        self.send
            .try_send((generation, action))
            .map_err(|_| "Serviço de desenhos indisponível")?;
        self.generation = generation;
        self.busy = true;
        self.error = None;
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Metadata {
    version: u32,
    project: PathBuf,
    paths: Vec<PathBuf>,
}

fn perform(directory: &Path, generation: u64, action: Action) -> Reply {
    let mut reply = Reply {
        generation,
        context: None,
        paths: None,
        open: vec![],
        error: None,
    };
    let result = (|| {
        let context = match &action {
            Action::Load(None) => {
                reply.paths = Some(vec![]);
                return Ok(());
            }
            Action::Load(Some(context))
            | Action::Register(context, _)
            | Action::Create(context, _) => canonical_context(context)?,
        };
        let path = metadata_path(directory, &context.project);
        reply.context = Some(context.clone());
        match action {
            Action::Load(_) => {
                let metadata = read_metadata(&path, &context)?.0;
                reply.open = existing_paths(&metadata.paths)?;
                reply.paths = Some(metadata.paths);
            }
            Action::Register(_, drawing) => {
                let drawing = existing_drawing(&drawing, &context)?;
                let _lock = lock_metadata(&path)?;
                let (mut metadata, expected) = read_metadata(&path, &context)?;
                if !metadata.paths.contains(&drawing) {
                    if metadata.paths.len() >= MAX_PATHS {
                        return Err("O projeto já possui 128 desenhos vinculados".into());
                    }
                    metadata.paths.push(drawing);
                    persist_metadata(&path, &metadata, expected.as_deref())?;
                }
                reply.paths = Some(metadata.paths);
            }
            Action::Create(_, relative) => {
                let root = context
                    .roots
                    .first()
                    .ok_or("O projeto não possui uma pasta disponível para desenhos")?;
                let relative = creation_relative(&relative)?;
                let _lock = lock_metadata(&path)?;
                let (mut metadata, expected) = read_metadata(&path, &context)?;
                let drawing = prepare_destination(root, &relative)?;
                if metadata.paths.len() >= MAX_PATHS && !metadata.paths.contains(&drawing) {
                    return Err("O projeto já possui 128 desenhos vinculados".into());
                }
                DrawingDocument::create(&drawing, Scene::default())?;
                if !metadata.paths.contains(&drawing) {
                    metadata.paths.push(drawing.clone());
                }
                reply.open.push(drawing);
                reply.paths = Some(metadata.paths.clone());
                persist_metadata(&path, &metadata, expected.as_deref()).map_err(|error| {
                    format!("Desenho criado; falha ao salvar o vínculo com o projeto: {error}")
                })?;
            }
        }
        Ok(())
    })();
    reply.error = result.err();
    reply
}

fn canonical_context(context: &Context) -> Result<Context, String> {
    let project =
        fs::canonicalize(&context.project).map_err(|_| "Projeto de desenhos indisponível")?;
    if context.roots.len() > 32 {
        return Err("O projeto excede 32 pastas".into());
    }
    let mut roots = vec![];
    for root in &context.roots {
        match fs::canonicalize(root) {
            Ok(root) if root.is_dir() => {
                if !roots.contains(&root) {
                    roots.push(root);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            _ => return Err("Pasta do projeto inválida para desenhos".into()),
        }
    }
    Ok(Context { project, roots })
}

fn metadata_path(directory: &Path, project: &Path) -> PathBuf {
    let id = format!(
        "{:x}",
        Sha256::digest(project.as_os_str().as_encoded_bytes())
    );
    directory.join(format!("{id}.json"))
}

pub fn is_drawing(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("excalidraw"))
        || path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.to_ascii_lowercase().ends_with(".excalidraw.json"))
}

fn relative_components(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

fn creation_relative(value: &str) -> Result<PathBuf, String> {
    if value.trim().is_empty() || value.len() > 4096 || value.chars().any(char::is_control) {
        return Err("Informe um nome ou caminho relativo para o desenho".into());
    }
    let mut path = PathBuf::from(value.trim());
    if !relative_components(&path) {
        return Err("O desenho deve ficar dentro de uma pasta do projeto, sem ../".into());
    }
    if path.extension().is_none() {
        path.set_extension("excalidraw");
    }
    if !is_drawing(&path) {
        return Err("Use a extensão .excalidraw ou .excalidraw.json".into());
    }
    if path.components().count() == 1 {
        path = Path::new("drawings").join(path);
    }
    Ok(path)
}

fn checked_drawing(path: &Path, context: &Context) -> Result<(), String> {
    if !path.is_absolute() || path.as_os_str().len() > 8192 || !is_drawing(path) {
        return Err("Vínculo de desenho inválido; catálogo preservado".into());
    }
    let root = context
        .roots
        .iter()
        .find(|root| path.starts_with(root))
        .ok_or("Desenho fora das pastas do projeto")?;
    let relative = path
        .strip_prefix(root)
        .map_err(|_| "Caminho de desenho inválido")?;
    if !relative_components(relative) {
        return Err("Caminho de desenho inválido".into());
    }
    let mut current = root.clone();
    let total = relative.components().count();
    for (index, component) in relative.components().enumerate() {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err("Desenhos não seguem links simbólicos".into());
            }
            Ok(metadata) if index + 1 < total && !metadata.is_dir() => {
                return Err("Diretório de desenhos inválido".into());
            }
            Ok(metadata) if index + 1 == total && !metadata.is_file() => {
                return Err("Desenho deve ser um arquivo regular".into());
            }
            Ok(_) => (),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(_) => return Err("Não foi possível verificar o desenho".into()),
        }
    }
    Ok(())
}

fn existing_drawing(path: &Path, context: &Context) -> Result<PathBuf, String> {
    checked_drawing(path, context)?;
    let canonical = fs::canonicalize(path).map_err(|_| "Desenho indisponível")?;
    checked_drawing(&canonical, context)?;
    if !canonical.is_file() {
        return Err("Desenho deve ser um arquivo regular".into());
    }
    Ok(canonical)
}

fn existing_paths(paths: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    let mut existing = vec![];
    for path in paths {
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                existing.push(path.clone())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            _ => return Err("Desenho inválido ou inacessível; catálogo preservado".into()),
        }
    }
    Ok(existing)
}

fn prepare_destination(root: &Path, relative: &Path) -> Result<PathBuf, String> {
    let parent = relative.parent().ok_or("Diretório de desenho inválido")?;
    let mut current = root.to_path_buf();
    for component in parent.components() {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => (),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&current)
                    .map_err(|_| "Não foi possível criar o diretório do desenho")?;
            }
            _ => return Err("Diretório de desenhos inválido ou com link simbólico".into()),
        }
    }
    let canonical = fs::canonicalize(&current).map_err(|_| "Diretório do desenho indisponível")?;
    if !canonical.starts_with(root) {
        return Err("O desenho deve permanecer dentro do projeto".into());
    }
    Ok(canonical.join(relative.file_name().ok_or("Nome de desenho inválido")?))
}

fn read_bytes(path: &Path) -> Result<Option<Vec<u8>>, String> {
    checked_profile_directory(path.parent().ok_or("Perfil de desenhos inválido")?, false)?;
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Ok(metadata)
            if metadata.is_file()
                && !metadata.file_type().is_symlink()
                && metadata.len() <= MAX_METADATA => {}
        _ => return Err("Catálogo de desenhos inválido; arquivo preservado".into()),
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options
        .open(path)
        .map_err(|_| "Não foi possível ler o catálogo de desenhos")?;
    if !file
        .metadata()
        .map_err(|_| "Catálogo indisponível")?
        .is_file()
    {
        return Err("Catálogo de desenhos deve ser um arquivo regular".into());
    }
    let mut bytes = vec![];
    file.take(MAX_METADATA + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Não foi possível ler o catálogo de desenhos")?;
    if bytes.len() as u64 > MAX_METADATA {
        return Err("Catálogo de desenhos excede 1 MiB".into());
    }
    Ok(Some(bytes))
}

fn read_metadata(path: &Path, context: &Context) -> Result<(Metadata, Option<Vec<u8>>), String> {
    let bytes = read_bytes(path)?;
    let metadata: Metadata = match &bytes {
        Some(bytes) => serde_json::from_slice(bytes)
            .map_err(|_| "Catálogo de desenhos inválido; arquivo preservado")?,
        None => Metadata {
            version: 1,
            project: context.project.clone(),
            paths: vec![],
        },
    };
    if metadata.version != 1
        || metadata.project != context.project
        || metadata.paths.len() > MAX_PATHS
    {
        return Err("Catálogo de desenhos inválido; arquivo preservado".into());
    }
    let mut unique = HashSet::new();
    for drawing in &metadata.paths {
        checked_drawing(drawing, context)?;
        if !unique.insert(drawing) {
            return Err("Catálogo contém desenhos repetidos; arquivo preservado".into());
        }
    }
    Ok((metadata, bytes))
}

fn lock_metadata(path: &Path) -> Result<File, String> {
    let parent = path.parent().ok_or("Perfil de desenhos inválido")?;
    checked_profile_directory(parent, true)?;
    let lock_path = path.with_extension("lock");
    if fs::symlink_metadata(&lock_path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err("Trava do catálogo não pode ser um link simbólico".into());
    }
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .mode(0o600);
    }
    let lock = options
        .open(lock_path)
        .map_err(|_| "Não foi possível bloquear o catálogo de desenhos")?;
    if !lock
        .metadata()
        .map_err(|_| "Trava de desenhos indisponível")?
        .is_file()
    {
        return Err("Trava do catálogo deve ser um arquivo regular".into());
    }
    lock.try_lock()
        .map_err(|_| "Catálogo de desenhos em uso por outra janela; tente novamente")?;
    Ok(lock)
}

fn checked_profile_directory(path: &Path, create: bool) -> Result<(), String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|_| "Diretório atual indisponível")?
            .join(path)
    };
    let mut current = PathBuf::new();
    for component in absolute.components() {
        if matches!(component, Component::CurDir | Component::ParentDir) {
            return Err("Diretório do perfil de desenhos inválido".into());
        }
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => (),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if !create {
                    return Ok(());
                }
                fs::create_dir(&current)
                    .map_err(|_| "Não foi possível criar o perfil de desenhos")?;
            }
            _ => return Err("Perfil de desenhos inválido ou com link simbólico".into()),
        }
    }
    Ok(())
}

fn persist_metadata(
    path: &Path,
    metadata: &Metadata,
    expected: Option<&[u8]>,
) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(metadata)
        .map_err(|_| "Não foi possível serializar o catálogo")?;
    if bytes.len() as u64 > MAX_METADATA {
        return Err("Catálogo de desenhos excede 1 MiB".into());
    }
    let parent = path.parent().ok_or("Perfil de desenhos inválido")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .map_err(|_| "Não foi possível preparar o catálogo")?;
    temporary
        .write_all(&bytes)
        .and_then(|_| temporary.as_file().sync_all())
        .map_err(|_| "Não foi possível gravar o catálogo")?;
    if read_bytes(path)?.as_deref() != expected {
        return Err("Catálogo alterado externamente; arquivo preservado".into());
    }
    temporary
        .persist(path)
        .map_err(|_| "Não foi possível publicar o catálogo de desenhos")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn settle(catalog: &mut DrawingCatalog) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while catalog.is_busy() {
            catalog.poll();
            assert!(Instant::now() < deadline, "drawing worker timed out");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn select(catalog: &mut DrawingCatalog, project: &Path) {
        catalog
            .set_project(Some(project.to_path_buf()), vec![project.to_path_buf()])
            .unwrap();
        settle(catalog);
        assert_eq!(catalog.error, None);
    }

    #[test]
    fn drawings_restore_for_project_after_switch_and_restart() {
        let temporary = tempfile::tempdir().unwrap();
        let a = temporary.path().join("A ação");
        let b = temporary.path().join("B");
        fs::create_dir(&a).unwrap();
        fs::create_dir(&b).unwrap();
        let profile = temporary.path().join("profile.drawings");
        let mut catalog = DrawingCatalog::new(profile.clone());
        select(&mut catalog, &a);
        catalog.create("Arquitetura".into()).unwrap();
        assert!(
            catalog
                .set_project(Some(b.clone()), vec![b.clone()])
                .is_err()
        );
        settle(&mut catalog);
        assert_eq!(catalog.error, None);
        let drawing = a.join("drawings/Arquitetura.excalidraw");
        assert_eq!(catalog.take_open(), vec![drawing.clone()]);
        assert!(catalog.take_open().is_empty());
        assert!(DrawingDocument::open(&drawing).is_ok());
        select(&mut catalog, &b);
        assert!(catalog.paths.is_empty());
        assert!(catalog.take_open().is_empty());
        select(&mut catalog, &a);
        assert_eq!(catalog.take_open(), vec![drawing.clone()]);
        drop(catalog);
        let mut restarted = DrawingCatalog::new(profile);
        select(&mut restarted, &a);
        assert_eq!(restarted.take_open(), vec![drawing]);
        restarted.set_project(None, vec![]).unwrap();
        settle(&mut restarted);
        assert!(restarted.project.is_none());
        assert!(restarted.paths.is_empty());
        assert!(is_drawing(Path::new("workflow.EXCALIDRAW.JSON")));
        assert!(!is_drawing(Path::new("workflow.json")));
        select(&mut restarted, &a);
        restarted
            .create("docs/fluxo.excalidraw.json".into())
            .unwrap();
        settle(&mut restarted);
        assert_eq!(restarted.error, None);
        let json_path = a.join("docs/fluxo.excalidraw.json");
        assert_eq!(restarted.take_open().last(), Some(&json_path));
        assert!(DrawingDocument::open(&json_path).is_ok());
        select(&mut restarted, &b);
        select(&mut restarted, &a);
        assert!(restarted.take_open().contains(&json_path));
    }

    #[test]
    fn concurrent_windows_merge_distinct_links_and_keep_missing_links() {
        let temporary = tempfile::tempdir().unwrap();
        let project = temporary.path().join("project");
        fs::create_dir(&project).unwrap();
        let a = project.join("a.excalidraw");
        let b = project.join("b.excalidraw");
        DrawingDocument::create(&a, Scene::default()).unwrap();
        DrawingDocument::create(&b, Scene::default()).unwrap();
        let profile = temporary.path().join("profile");
        let mut first = DrawingCatalog::new(profile.clone());
        let mut second = DrawingCatalog::new(profile.clone());
        select(&mut first, &project);
        select(&mut second, &project);
        first.register(a.clone()).unwrap();
        settle(&mut first);
        second.register(b.clone()).unwrap();
        settle(&mut second);
        assert_eq!(second.paths, vec![a.clone(), b.clone()]);
        assert_eq!(second.error, None);
        second.register(a.clone()).unwrap();
        settle(&mut second);
        assert_eq!(second.paths.len(), 2);
        fs::remove_file(&a).unwrap();
        let mut restarted = DrawingCatalog::new(profile);
        select(&mut restarted, &project);
        assert_eq!(restarted.paths, vec![a, b.clone()]);
        assert_eq!(restarted.take_open(), vec![b]);
    }

    #[test]
    fn invalid_metadata_is_preserved_and_creation_does_not_overwrite() {
        let temporary = tempfile::tempdir().unwrap();
        let project = temporary.path().join("project");
        fs::create_dir(&project).unwrap();
        let profile = temporary.path().join("profile");
        let mut catalog = DrawingCatalog::new(profile.clone());
        select(&mut catalog, &project);
        catalog.create("docs/plan.excalidraw".into()).unwrap();
        settle(&mut catalog);
        let drawing = project.join("docs/plan.excalidraw");
        let original = fs::read(&drawing).unwrap();
        catalog.create("docs/plan.excalidraw".into()).unwrap();
        settle(&mut catalog);
        assert!(catalog.error.is_some());
        assert_eq!(fs::read(&drawing).unwrap(), original);
        fs::remove_file(&drawing).unwrap();
        catalog.create("docs/plan.excalidraw".into()).unwrap();
        settle(&mut catalog);
        assert_eq!(catalog.error, None);
        assert_eq!(catalog.paths, vec![drawing]);
        let path = metadata_path(&profile, &project.canonicalize().unwrap());
        fs::write(&path, b"invalid metadata").unwrap();
        catalog.create("never-created".into()).unwrap();
        settle(&mut catalog);
        assert!(catalog.error.is_some());
        assert!(!project.join("drawings/never-created.excalidraw").exists());
        assert_eq!(fs::read(&path).unwrap(), b"invalid metadata");
    }

    #[test]
    fn workspace_files_have_separate_catalogs_even_with_identical_roots() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("src");
        fs::create_dir(&root).unwrap();
        let a = temporary.path().join("a.code-workspace");
        let b = temporary.path().join("b.code-workspace");
        fs::write(&a, "{}").unwrap();
        fs::write(&b, "{}").unwrap();
        let mut catalog = DrawingCatalog::new(temporary.path().join("profile"));
        catalog
            .set_project(Some(a.clone()), vec![root.clone()])
            .unwrap();
        settle(&mut catalog);
        catalog.create("flow".into()).unwrap();
        settle(&mut catalog);
        catalog.take_open();
        catalog.set_project(Some(b), vec![root.clone()]).unwrap();
        settle(&mut catalog);
        assert!(catalog.paths.is_empty());
        catalog.set_project(Some(a), vec![root]).unwrap();
        settle(&mut catalog);
        assert_eq!(catalog.paths.len(), 1);
        assert_eq!(catalog.take_open().len(), 1);
    }

    #[test]
    fn external_catalog_edit_and_locked_catalog_are_not_replaced() {
        let temporary = tempfile::tempdir().unwrap();
        let project = temporary.path().join("project");
        fs::create_dir(&project).unwrap();
        let context = Context {
            project: project.clone(),
            roots: vec![project.clone()],
        };
        let path = metadata_path(&temporary.path().join("profile"), &project);
        let lock = lock_metadata(&path).unwrap();
        let metadata = Metadata {
            version: 1,
            project: project.clone(),
            paths: vec![],
        };
        fs::write(&path, b"external edit").unwrap();
        assert!(persist_metadata(&path, &metadata, None).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"external edit");
        fs::write(&path, serde_json::to_vec(&metadata).unwrap()).unwrap();
        let reply = perform(
            path.parent().unwrap(),
            1,
            Action::Create(context, "blocked".into()),
        );
        assert!(
            reply
                .error
                .as_deref()
                .is_some_and(|error| error.contains("em uso"))
        );
        assert!(!project.join("drawings/blocked.excalidraw").exists());
        drop(lock);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_traversal_outside_paths_and_symbolic_links() {
        use std::os::unix::fs::symlink;
        let temporary = tempfile::tempdir().unwrap();
        let project = temporary.path().join("project");
        let outside = temporary.path().join("outside");
        fs::create_dir(&project).unwrap();
        fs::create_dir(&outside).unwrap();
        let external = outside.join("external.excalidraw");
        DrawingDocument::create(&external, Scene::default()).unwrap();
        symlink(&outside, project.join("linked")).unwrap();
        symlink(&external, project.join("linked.excalidraw")).unwrap();
        fs::create_dir(project.join("drawings")).unwrap();
        symlink(&external, project.join("drawings/linked.excalidraw")).unwrap();
        let mut catalog = DrawingCatalog::new(temporary.path().join("profile"));
        select(&mut catalog, &project);
        for relative in [
            "../escaped",
            "/tmp/escaped",
            "linked/escaped",
            "linked.excalidraw",
            "not-a-scene.txt",
        ] {
            catalog.create(relative.into()).unwrap();
            settle(&mut catalog);
            assert!(catalog.error.is_some(), "accepted {relative}");
        }
        for path in [external.clone(), project.join("linked.excalidraw")] {
            catalog.register(path).unwrap();
            settle(&mut catalog);
            assert!(catalog.error.is_some());
        }
        assert!(!outside.join("escaped.excalidraw").exists());
        assert!(!temporary.path().join("escaped.excalidraw").exists());
        assert!(DrawingDocument::open(&external).is_ok());
        assert!(catalog.paths.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn symbolic_profile_directory_and_metadata_are_not_followed() {
        use std::os::unix::fs::symlink;
        let temporary = tempfile::tempdir().unwrap();
        let project = temporary.path().join("project");
        let outside = temporary.path().join("outside");
        fs::create_dir(&project).unwrap();
        fs::create_dir(&outside).unwrap();
        let linked = temporary.path().join("profile-link");
        symlink(&outside, &linked).unwrap();
        let mut catalog = DrawingCatalog::new(linked);
        catalog
            .set_project(Some(project.clone()), vec![project.clone()])
            .unwrap();
        settle(&mut catalog);
        assert!(catalog.error.is_some());
        assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
        let profile = temporary.path().join("profile");
        fs::create_dir(&profile).unwrap();
        let external = outside.join("private.json");
        fs::write(&external, b"preserved").unwrap();
        let metadata = metadata_path(&profile, &project);
        symlink(&external, &metadata).unwrap();
        let mut catalog = DrawingCatalog::new(profile);
        catalog
            .set_project(Some(project.clone()), vec![project])
            .unwrap();
        settle(&mut catalog);
        assert!(catalog.error.is_some());
        assert_eq!(fs::read(&external).unwrap(), b"preserved");
    }
}
