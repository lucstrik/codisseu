//! Own, worktree-local shelves and changelists. Never implemented using Git stash.
//! Durable originals precede any worktree write; the index is published last.
use crate::{
    GitError, GitErrorKind, RepoSnapshot, Repository,
    repository::{args, cancelled, os_bytes, path_from_bytes},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

pub const MAX_SHELF_PATHS: usize = 128;
pub const MAX_SHELF_FILE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_SHELF_BYTES: usize = 32 * 1024 * 1024;
const MAX_PATCH_BYTES: usize = 8 * 1024 * 1024;
const MAX_STORE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_SHELVES: usize = 128;
const MAX_LISTS: usize = 64;
const MAX_METADATA: usize = 1024 * 1024;
const MAX_STORE_ENTRIES: usize = 65536;

#[derive(Clone, Debug)]
pub struct ShelfFileStatus {
    pub path: PathBuf,
    pub original_path: Option<PathBuf>,
    pub index_status: char,
    pub working_status: char,
    pub status: char,
}
#[derive(Clone, Debug)]
pub struct ShelfInfo {
    pub id: String,
    pub name: String,
    pub created_unix: u64,
    pub base: Option<String>,
    pub paths: Vec<PathBuf>,
    pub bytes: u64,
    pub files: Vec<ShelfFileStatus>,
    pub deleted_unix: Option<i64>,
}
#[derive(Clone, Debug)]
pub struct ShelfDetails {
    pub info: ShelfInfo,
    /// Concatenated layer patches for viewing, not an import/apply document.
    pub patch: Vec<u8>,
    pub staged_patch: Vec<u8>,
    pub working_patch: Vec<u8>,
}
#[derive(Clone, Debug)]
pub struct Changelist {
    pub comment: String,
    pub id: String,
    pub name: String,
    pub is_default: bool,
    pub paths: Vec<PathBuf>,
}
#[derive(Clone, Debug)]
pub enum ShelfAction {
    Shelve {
        name: String,
        paths: Vec<PathBuf>,
    },
    Unshelve {
        id: String,
    },
    UnshelveOptions {
        id: String,
        paths: Option<Vec<PathBuf>>,
        changelist_name: Option<String>,
        comment: Option<String>,
        set_active: bool,
        remove_applied: bool,
    },
    Delete {
        id: String,
    },
    RestoreDeleted {
        id: String,
    },
    DeletePermanently {
        id: String,
    },
    CreateChangelist {
        name: String,
        set_default: bool,
    },
    RenameChangelist {
        id: String,
        name: String,
    },
    DeleteChangelist {
        id: String,
    },
    MoveToChangelist {
        id: String,
        paths: Vec<PathBuf>,
    },
    SetDefaultChangelist {
        id: String,
    },
}
#[derive(Clone, Debug)]
pub struct ShelfPreview {
    pub summary: String,
    pub paths: Vec<PathBuf>,
    pub warnings: Vec<String>,
    pub destructive: bool,
    pub conflicts: Vec<PathBuf>,
}
#[derive(Clone, Debug)]
pub struct PreparedShelf {
    unshelve: Option<PreparedUnshelveOptions>,
    pub preview: ShelfPreview,
    snapshot: RepoSnapshot,
    store_hash: String,
    preview_hash: String,
    plan: Plan,
}
#[derive(Clone, Debug)]
struct PreparedUnshelveOptions {
    lists: Option<ListDocument>,
    remove_applied: bool,
    remaining: Option<Bundle>,
}
struct UnshelveOptions {
    paths: Option<Vec<PathBuf>>,
    changelist_name: Option<String>,
    comment: Option<String>,
    set_active: bool,
    remove_applied: bool,
}
#[derive(Clone, Debug)]
pub struct ShelfReport {
    pub success: bool,
    pub summary: String,
    pub output: String,
    pub outcome_unknown: bool,
    pub shelf_id: Option<String>,
    pub recovery_path: Option<PathBuf>,
    pub conflicts: Vec<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Item {
    mode: String,
    data: Vec<u8>,
    permissions: u32,
}
#[derive(Clone, Debug)]
struct SavedPath {
    path: PathBuf,
    base: Option<Item>,
    index: Option<Item>,
    working: Option<Item>,
}
#[derive(Clone, Debug)]
struct Bundle {
    info: ShelfInfo,
    entries: Vec<SavedPath>,
    staged_patch: Vec<u8>,
    working_patch: Vec<u8>,
}
#[derive(Clone, Debug)]
struct Change {
    path: PathBuf,
    before: Option<Item>,
    working: Option<Item>,
    index: Vec<(u8, Item)>,
}
#[derive(Clone, Debug)]
enum Plan {
    Shelve(Bundle, Vec<Change>),
    Unshelve(String, Vec<Change>),
    Delete(String),
    RestoreDeleted(String),
    DeletePermanently(String),
    Lists(ListDocument),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct BlobMeta {
    mode: String,
    permissions: u32,
    len: usize,
    hash: String,
}
#[derive(Serialize, Deserialize)]
struct StoredPath {
    path: Vec<u8>,
    base: Option<BlobMeta>,
    index: Option<BlobMeta>,
    working: Option<BlobMeta>,
}
#[derive(Serialize, Deserialize)]
struct StoredFileStatus {
    path: Vec<u8>,
    original_path: Option<Vec<u8>>,
    index_status: char,
    working_status: char,
    status: char,
}
#[derive(Serialize, Deserialize)]
struct Metadata {
    version: u32,
    id: String,
    name: String,
    created_unix: u64,
    base: Option<String>,
    entries: Vec<StoredPath>,
    #[serde(default)]
    files: Vec<StoredFileStatus>,
    staged_patch: BlobMeta,
    working_patch: BlobMeta,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct StoredList {
    #[serde(default)]
    comment: String,
    id: String,
    name: String,
    paths: Vec<Vec<u8>>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct ListDocument {
    version: u32,
    default: String,
    lists: Vec<StoredList>,
}
impl Default for ListDocument {
    fn default() -> Self {
        Self {
            version: 1,
            default: "default".into(),
            lists: vec![StoredList {
                id: "default".into(),
                name: "Changes".into(),
                comment: String::new(),
                paths: Vec::new(),
            }],
        }
    }
}

fn invalid(message: &str) -> GitError {
    GitError::invalid(message)
}
fn stale() -> GitError {
    GitError::new(
        GitErrorKind::Stale,
        "Estado do Git/Shelf mudou; prepare a operação novamente",
    )
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn fresh_id() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let seed = format!(
        "{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    hash(seed.as_bytes())[..32].into()
}
fn valid_id(id: &str) -> Result<(), GitError> {
    if id.len() != 32 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid("Identificador de Shelf inválido"));
    }
    Ok(())
}
fn name(value: &str) -> Result<(), GitError> {
    if value.trim().is_empty() || value.len() > 1024 || value.chars().any(char::is_control) {
        return Err(invalid(
            "Nome deve conter de 1 a 1024 bytes sem caracteres de controle",
        ));
    }
    Ok(())
}
fn shelf_message(value: &str) -> Result<(), GitError> {
    if value.trim().is_empty()
        || value.len() > 65536
        || value
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err(invalid(
            "Mensagem de Shelf deve conter de1a65536 bytes de texto",
        ));
    }
    Ok(())
}
fn store(repo: &Repository) -> Result<PathBuf, GitError> {
    let path = repo.git_dir.join("codisseu");
    check_directory(&path)?;
    Ok(path)
}
fn check_directory(path: &Path) -> Result<(), GitError> {
    match fs::symlink_metadata(path) {
        Ok(m) if !m.is_dir() || m.file_type().is_symlink() => Err(invalid(
            "Armazenamento/caminho Shelf não pode ser link simbólico ou arquivo",
        )),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}
fn ensure_directory(path: &Path) -> Result<(), GitError> {
    check_directory(path)?;
    if !path.exists() {
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(path)?;
    }
    Ok(())
}
fn safe_path(repo: &Repository, path: &Path) -> Result<PathBuf, GitError> {
    let relative = repo.relative_path(path)?;
    for component in relative.components() {
        if let Component::Normal(part) = component
            && os_bytes(part).eq_ignore_ascii_case(b".git")
        {
            return Err(invalid("Shelf não altera metadados .git"));
        }
    }
    let mut parent = repo.root.clone();
    if let Some(parts) = relative.parent() {
        for part in parts.components() {
            parent.push(part);
            check_directory(&parent)?;
        }
    }
    Ok(relative)
}
fn read_bounded(path: &Path, max: usize) -> Result<Vec<u8>, GitError> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > max as u64 {
        return Err(invalid("Arquivo de Shelf inválido ou excede o limite"));
    }
    let mut bytes = Vec::new();
    File::open(path)?
        .take(max as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > max {
        return Err(invalid("Arquivo de Shelf excede o limite"));
    }
    Ok(bytes)
}
fn sync_dir(path: &Path) -> Result<(), GitError> {
    #[cfg(unix)]
    {
        File::open(path)?.sync_all()?;
    }
    Ok(())
}
fn write_bytes(path: &Path, bytes: &[u8]) -> Result<(), GitError> {
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}
fn tree_hash(root: &Path, cancel: &AtomicBool) -> Result<String, GitError> {
    fn visit(
        path: &Path,
        digest: &mut Sha256,
        total: &mut u64,
        count: &mut usize,
        cancel: &AtomicBool,
        depth: usize,
    ) -> Result<(), GitError> {
        cancelled(cancel)?;
        if depth > 3 {
            return Err(invalid("Estrutura de Shelf inválida"));
        }
        let mut entries = match fs::read_dir(path) {
            Ok(entries) => entries
                .take(MAX_STORE_ENTRIES + 1)
                .collect::<Result<Vec<_>, _>>()?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        *count += entries.len();
        if *count > MAX_STORE_ENTRIES {
            return Err(invalid("Shelf excede o limite de arquivos armazenados"));
        }
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let n = entry.file_name();
            if depth == 0 && (n == "recovery" || n == ".lock" || os_bytes(&n).starts_with(b".tmp"))
            {
                continue;
            }
            let meta = fs::symlink_metadata(entry.path())?;
            if meta.file_type().is_symlink() {
                return Err(invalid("Shelf contém link simbólico inesperado"));
            }
            digest.update((os_bytes(&n).len() as u64).to_le_bytes());
            digest.update(os_bytes(&n));
            if meta.is_dir() {
                visit(&entry.path(), digest, total, count, cancel, depth + 1)?;
            } else {
                *total += meta.len();
                if *total > MAX_STORE_BYTES {
                    return Err(invalid("Armazenamento Shelf excede 256 MiB"));
                }
                let bytes = read_bounded(&entry.path(), MAX_SHELF_BYTES)?;
                digest.update((bytes.len() as u64).to_le_bytes());
                digest.update(bytes);
            }
        }
        Ok(())
    }
    let mut digest = Sha256::new();
    visit(root, &mut digest, &mut 0, &mut 0, cancel, 0)?;
    Ok(format!("{:x}", digest.finalize()))
}
fn preview_hash(preview: &ShelfPreview) -> String {
    hash(format!("{preview:?}").as_bytes())
}
fn matches_written(actual: &Option<Item>, desired: &Option<Item>) -> bool {
    if !item_equal(actual, desired) {
        return false;
    }
    match (actual, desired) {
        (Some(actual), Some(desired)) if desired.mode != "120000" => {
            let permissions =
                (desired.permissions & !0o111) | if desired.mode == "100755" { 0o111 } else { 0 };
            actual.permissions == permissions
        }
        _ => true,
    }
}
fn item_equal(a: &Option<Item>, b: &Option<Item>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.mode == b.mode && a.data == b.data,
        _ => false,
    }
}
fn blob(repo: &Repository, oid: &str, mode: &str, cancel: &AtomicBool) -> Result<Item, GitError> {
    if !matches!(mode, "100644" | "100755" | "120000") {
        return Err(GitError::unsupported(
            "Shelf não inclui submódulos ou modos de arquivo especiais",
        ));
    }
    let size = repo.run_checked(&args(&["cat-file", "-s", oid]), None, cancel)?;
    let size: usize = std::str::from_utf8(&size.stdout)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .ok_or_else(|| invalid("Tamanho de blob inválido"))?;
    if size > MAX_SHELF_FILE_BYTES {
        return Err(invalid("Shelf limita cada versão de arquivo a 4 MiB"));
    }
    let data = repo
        .run_checked(&args(&["cat-file", "blob", oid]), None, cancel)?
        .stdout;
    if data.len() != size {
        return Err(stale());
    }
    Ok(Item {
        mode: mode.into(),
        data,
        permissions: if mode == "100755" { 0o755 } else { 0o644 },
    })
}
fn read_index(
    repo: &Repository,
    path: &Path,
    cancel: &AtomicBool,
) -> Result<Option<Item>, GitError> {
    let mut a = args(&["ls-files", "--stage", "-z", "--"]);
    a.push(path.as_os_str().into());
    let out = repo.run_checked(&a, None, cancel)?.stdout;
    let mut item = None;
    for line in out.split(|b| *b == 0).filter(|s| !s.is_empty()) {
        let tab = line
            .iter()
            .position(|b| *b == b'\t')
            .ok_or_else(|| invalid("Índice Git inválido"))?;
        if &line[tab + 1..] != os_bytes(path.as_os_str()) {
            continue;
        }
        let fields = std::str::from_utf8(&line[..tab])
            .map_err(|_| invalid("Índice Git inválido"))?
            .split_whitespace()
            .collect::<Vec<_>>();
        if fields.len() != 3 || fields[2] != "0" || item.is_some() {
            return Err(invalid("Resolva conflitos existentes antes de usar Shelf"));
        }
        item = Some(blob(repo, fields[1], fields[0], cancel)?);
    }
    Ok(item)
}
fn read_base(
    repo: &Repository,
    head: Option<&str>,
    path: &Path,
    cancel: &AtomicBool,
) -> Result<Option<Item>, GitError> {
    let Some(head) = head else {
        return Ok(None);
    };
    let mut a = args(&["ls-tree", "-z", head, "--"]);
    a.push(path.as_os_str().into());
    let out = repo.run_checked(&a, None, cancel)?.stdout;
    for line in out.split(|b| *b == 0).filter(|s| !s.is_empty()) {
        let tab = line
            .iter()
            .position(|b| *b == b'\t')
            .ok_or_else(|| invalid("Árvore Git inválida"))?;
        if &line[tab + 1..] != os_bytes(path.as_os_str()) {
            continue;
        }
        let fields = std::str::from_utf8(&line[..tab])
            .map_err(|_| invalid("Árvore Git inválida"))?
            .split_whitespace()
            .collect::<Vec<_>>();
        if fields.len() != 3 || fields[1] != "blob" {
            return Err(GitError::unsupported(
                "Shelf não inclui diretórios ou submódulos",
            ));
        }
        return Ok(Some(blob(repo, fields[2], fields[0], cancel)?));
    }
    Ok(None)
}
fn read_work(repo: &Repository, path: &Path) -> Result<Option<Item>, GitError> {
    let relative = safe_path(repo, path)?;
    let full = repo.root.join(relative);
    let meta = match fs::symlink_metadata(&full) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    if meta.file_type().is_symlink() {
        let data = os_bytes(fs::read_link(&full)?.as_os_str()).to_vec();
        if data.len() > MAX_SHELF_FILE_BYTES {
            return Err(invalid("Link excede o limite Shelf"));
        }
        return Ok(Some(Item {
            mode: "120000".into(),
            data,
            permissions: 0o777,
        }));
    }
    #[cfg(unix)]
    let permissions = {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o777
    };
    #[cfg(not(unix))]
    let permissions = 0o644;
    let data = read_bounded(&full, MAX_SHELF_FILE_BYTES)?;
    Ok(Some(Item {
        mode: if permissions & 0o111 != 0 {
            "100755"
        } else {
            "100644"
        }
        .into(),
        data,
        permissions,
    }))
}
fn metadata_for(item: &Item) -> BlobMeta {
    BlobMeta {
        mode: item.mode.clone(),
        permissions: item.permissions,
        len: item.data.len(),
        hash: hash(&item.data),
    }
}
fn patch_metadata(bytes: &[u8]) -> BlobMeta {
    BlobMeta {
        mode: "patch".into(),
        permissions: 0o600,
        len: bytes.len(),
        hash: hash(bytes),
    }
}
fn blob_status(base: &Option<BlobMeta>, next: &Option<BlobMeta>) -> char {
    match (base, next) {
        (None, None) => '.',
        (None, Some(_)) => 'A',
        (Some(_), None) => 'D',
        (Some(a), Some(b)) if a.mode == b.mode && a.hash == b.hash => '.',
        _ => 'M',
    }
}
fn item_status(base: &Option<Item>, next: &Option<Item>) -> char {
    match (base, next) {
        (None, None) => '.',
        (None, Some(_)) => 'A',
        (Some(_), None) => 'D',
        (Some(a), Some(b)) if a.mode == b.mode && a.data == b.data => '.',
        _ => 'M',
    }
}
fn file_statuses(entries: &[SavedPath], snapshot: &RepoSnapshot) -> Vec<ShelfFileStatus> {
    let renamed = snapshot
        .entries
        .iter()
        .filter(|entry| entries.iter().any(|saved| saved.path == entry.path))
        .filter_map(|entry| entry.original_path.clone())
        .collect::<BTreeSet<_>>();
    entries
        .iter()
        .filter(|entry| !renamed.contains(&entry.path))
        .map(|entry| {
            let existing = snapshot
                .entries
                .iter()
                .find(|change| change.path == entry.path);
            let index_status = existing
                .map(|change| change.index)
                .filter(|status| *status != ' ')
                .unwrap_or_else(|| item_status(&entry.base, &entry.index));
            let working_status = existing
                .map(|change| change.worktree)
                .filter(|status| *status != ' ')
                .unwrap_or_else(|| item_status(&entry.index, &entry.working));
            ShelfFileStatus {
                path: entry.path.clone(),
                original_path: existing.and_then(|change| change.original_path.clone()),
                index_status,
                working_status,
                status: if !matches!(index_status, '.' | ' ') {
                    index_status
                } else {
                    working_status
                },
            }
        })
        .collect()
}
fn info(metadata: &Metadata) -> Result<ShelfInfo, GitError> {
    valid_id(&metadata.id)?;
    shelf_message(&metadata.name)?;
    if metadata.version != 1
        || metadata.entries.is_empty()
        || metadata.entries.len() > MAX_SHELF_PATHS
    {
        return Err(invalid("Versão/metadados de Shelf inválidos"));
    }
    if metadata.base.as_ref().is_some_and(|oid| {
        !matches!(oid.len(), 40 | 64) || !oid.bytes().all(|b| b.is_ascii_hexdigit())
    }) {
        return Err(invalid("Commit base de Shelf inválido"));
    }
    let paths = metadata
        .entries
        .iter()
        .map(|e| path_from_bytes(&e.path))
        .collect::<Result<Vec<_>, _>>()?;
    let bytes = metadata
        .entries
        .iter()
        .flat_map(|e| [&e.base, &e.index, &e.working])
        .flatten()
        .try_fold(0usize, |total, m| {
            total
                .checked_add(m.len)
                .ok_or_else(|| invalid("Tamanho de Shelf inválido"))
        })?;
    if bytes > MAX_SHELF_BYTES {
        return Err(invalid("Shelf excede 32 MiB de versões de arquivos"));
    }
    Ok(ShelfInfo {
        id: metadata.id.clone(),
        name: metadata.name.clone(),
        created_unix: metadata.created_unix,
        base: metadata.base.clone(),
        files: if metadata.files.is_empty() {
            metadata
                .entries
                .iter()
                .map(|entry| {
                    let index_status = blob_status(&entry.base, &entry.index);
                    let working_status = blob_status(&entry.index, &entry.working);
                    Ok(ShelfFileStatus {
                        path: path_from_bytes(&entry.path)?,
                        original_path: None,
                        index_status,
                        working_status,
                        status: if index_status != '.' {
                            index_status
                        } else {
                            working_status
                        },
                    })
                })
                .collect::<Result<Vec<_>, GitError>>()?
        } else {
            metadata
                .files
                .iter()
                .map(|entry| {
                    Ok(ShelfFileStatus {
                        path: path_from_bytes(&entry.path)?,
                        original_path: entry
                            .original_path
                            .as_ref()
                            .map(|path| path_from_bytes(path))
                            .transpose()?,
                        index_status: entry.index_status,
                        working_status: entry.working_status,
                        status: entry.status,
                    })
                })
                .collect::<Result<Vec<_>, GitError>>()?
        },
        deleted_unix: None,
        paths,
        bytes: bytes as u64,
    })
}
fn read_metadata(path: &Path) -> Result<Metadata, GitError> {
    check_directory(path)?;
    serde_json::from_slice(&read_bounded(&path.join("metadata.json"), MAX_METADATA)?)
        .map_err(|_| invalid("Metadados de Shelf inválidos"))
}
pub fn list_shelves(repo: &Repository, cancel: &AtomicBool) -> Result<Vec<ShelfInfo>, GitError> {
    let root = store(repo)?.join("shelves");
    check_directory(&root)?;
    let dirs = match fs::read_dir(&root) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let mut shelves = Vec::new();
    for entry in dirs {
        cancelled(cancel)?;
        let entry = entry?;
        let id = entry.file_name();
        if os_bytes(&id).starts_with(b".") {
            continue;
        }
        valid_id(
            id.to_str()
                .ok_or_else(|| invalid("Identificador Shelf inválido"))?,
        )?;
        let metadata = read_metadata(&entry.path())?;
        if metadata.id != id.to_string_lossy() {
            return Err(invalid("Identidade de Shelf não corresponde ao arquivo"));
        }
        let shelf = info(&metadata)?;
        for path in &shelf.paths {
            safe_path(repo, path)?;
        }
        shelves.push(shelf);
        if shelves.len() > MAX_SHELVES {
            return Err(invalid("Limite de 128 shelves atingido"));
        }
    }
    shelves.sort_by(|a, b| b.created_unix.cmp(&a.created_unix).then(a.id.cmp(&b.id)));
    Ok(shelves)
}
pub fn list_deleted_shelves(
    repo: &Repository,
    cancel: &AtomicBool,
) -> Result<Vec<ShelfInfo>, GitError> {
    let root = store(repo)?.join("deleted");
    check_directory(&root)?;
    let entries = match fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(error) => return Err(error.into()),
    };
    let mut result = Vec::new();
    for entry in entries {
        cancelled(cancel)?;
        let entry = entry?;
        let id = entry.file_name();
        let id = id
            .to_str()
            .ok_or_else(|| invalid("Identificador Shelf inválido"))?;
        valid_id(id)?;
        let metadata = read_metadata(&entry.path())?;
        if metadata.id != id {
            return Err(invalid("Identidade Shelf inválida"));
        }
        let mut shelf = info(&metadata)?;
        for path in &shelf.paths {
            safe_path(repo, path)?;
        }
        shelf.deleted_unix = Some(
            serde_json::from_slice::<i64>(&read_bounded(
                &entry.path().join("deleted-at.json"),
                128,
            )?)
            .map_err(|_| invalid("Data de exclusão Shelf inválida"))?,
        );
        result.push(shelf);
        if result.len() > MAX_SHELVES {
            return Err(invalid("Recently Deleted excede128 shelves"));
        }
    }
    result.sort_by_key(|item| std::cmp::Reverse(item.deleted_unix));
    Ok(result)
}
fn move_deleted(root: &Path, id: &str, restore: bool) -> Result<(), GitError> {
    valid_id(id)?;
    let active = root.join("shelves");
    let deleted = root.join("deleted");
    ensure_directory(&active)?;
    ensure_directory(&deleted)?;
    let (source, destination) = if restore {
        (deleted.join(id), active.join(id))
    } else {
        (active.join(id), deleted.join(id))
    };
    check_directory(&source)?;
    if destination.exists() {
        return Err(invalid("Destino Shelf já existe; nada foi substituído"));
    }
    if !restore {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| invalid("Relógio inválido"))?
            .as_secs();
        let timestamp = i64::try_from(timestamp).map_err(|_| invalid("Data inválida"))?;
        let mut file = tempfile::NamedTempFile::new_in(&source)?;
        file.write_all(timestamp.to_string().as_bytes())?;
        file.as_file().sync_all()?;
        file.persist(source.join("deleted-at.json"))
            .map_err(|error| GitError::from(error.error))?;
        sync_dir(&source)?;
    }
    fs::rename(source, destination)?;
    sync_dir(&active)?;
    sync_dir(&deleted)
}
fn load_bundle(repo: &Repository, id: &str, cancel: &AtomicBool) -> Result<Bundle, GitError> {
    load_bundle_at(repo, id, false, cancel)
}
fn load_bundle_at(
    repo: &Repository,
    id: &str,
    deleted: bool,
    cancel: &AtomicBool,
) -> Result<Bundle, GitError> {
    valid_id(id)?;
    let root = store(repo)?.join(if deleted { "deleted" } else { "shelves" });
    check_directory(&root)?;
    let root = root.join(id);
    let metadata = read_metadata(&root)?;
    if metadata.id != id {
        return Err(invalid("Identidade Shelf incorreta"));
    }
    let mut info = info(&metadata)?;
    if deleted {
        info.deleted_unix = Some(
            serde_json::from_slice::<i64>(&read_bounded(&root.join("deleted-at.json"), 128)?)
                .map_err(|_| invalid("Data de exclusão inválida"))?,
        );
    }
    let mut paths = BTreeSet::new();
    let mut entries = Vec::new();
    for (n, e) in metadata.entries.iter().enumerate() {
        cancelled(cancel)?;
        let path = safe_path(repo, &path_from_bytes(&e.path)?)?;
        if !paths.insert(path.clone()) {
            return Err(invalid("Shelf contém caminhos duplicados"));
        }
        let load = |label: &str, m: &Option<BlobMeta>| -> Result<Option<Item>, GitError> {
            let Some(m) = m else {
                return Ok(None);
            };
            if !matches!(m.mode.as_str(), "100644" | "100755" | "120000")
                || m.permissions & !0o777 != 0
                || m.len > MAX_SHELF_FILE_BYTES
            {
                return Err(invalid("Versão de arquivo Shelf inválida"));
            }
            let data = read_bounded(&root.join(format!("{n}-{label}")), MAX_SHELF_FILE_BYTES)?;
            if data.len() != m.len || hash(&data) != m.hash {
                return Err(invalid("Conteúdo Shelf não corresponde ao hash salvo"));
            }
            Ok(Some(Item {
                mode: m.mode.clone(),
                data,
                permissions: m.permissions,
            }))
        };
        entries.push(SavedPath {
            path,
            base: load("base", &e.base)?,
            index: load("index", &e.index)?,
            working: load("working", &e.working)?,
        });
    }
    let read_patch = |label: &str, m: &BlobMeta| -> Result<Vec<u8>, GitError> {
        let data = read_bounded(&root.join(label), MAX_PATCH_BYTES)?;
        if data.len() != m.len || hash(&data) != m.hash {
            return Err(invalid("Patch Shelf inválido"));
        }
        Ok(data)
    };
    let staged_patch = read_patch("staged.patch", &metadata.staged_patch)?;
    let working_patch = read_patch("working.patch", &metadata.working_patch)?;
    Ok(Bundle {
        info,
        entries,
        staged_patch,
        working_patch,
    })
}
pub fn shelf_deleted_details(
    repo: &Repository,
    id: &str,
    cancel: &AtomicBool,
) -> Result<ShelfDetails, GitError> {
    let bundle = load_bundle_at(repo, id, true, cancel)?;
    let mut patch = bundle.staged_patch.clone();
    if patch.len() + bundle.working_patch.len() > MAX_PATCH_BYTES {
        return Err(invalid("Detalhes Shelf excedem8MiB"));
    }
    patch.extend_from_slice(&bundle.working_patch);
    Ok(ShelfDetails {
        info: bundle.info,
        patch,
        staged_patch: bundle.staged_patch,
        working_patch: bundle.working_patch,
    })
}
pub fn shelf_details(
    repo: &Repository,
    id: &str,
    cancel: &AtomicBool,
) -> Result<ShelfDetails, GitError> {
    let b = load_bundle(repo, id, cancel)?;
    let mut patch = b.staged_patch.clone();
    patch.extend_from_slice(&b.working_patch);
    Ok(ShelfDetails {
        info: b.info,
        patch,
        staged_patch: b.staged_patch,
        working_patch: b.working_patch,
    })
}

fn load_lists(repo: &Repository) -> Result<ListDocument, GitError> {
    let file = store(repo)?.join("changelists.json");
    if !file.exists() {
        if fs::symlink_metadata(&file).is_ok() {
            return Err(invalid("Changelists não pode ser symlink"));
        }
        return Ok(ListDocument::default());
    }
    let doc: ListDocument = serde_json::from_slice(&read_bounded(&file, MAX_METADATA)?)
        .map_err(|_| invalid("Changelists inválido"))?;
    if doc.version != 1 || doc.lists.is_empty() || doc.lists.len() > MAX_LISTS {
        return Err(invalid("Versão/limite de changelists inválido"));
    }
    let mut ids = BTreeSet::new();
    let mut names = BTreeSet::new();
    let mut paths = BTreeSet::new();
    for list in &doc.lists {
        name(&list.name)?;
        if list.comment.len() > 65536 || list.comment.contains('\0') {
            return Err(invalid("Comentário de changelist inválido"));
        }
        if (list.id != "default" && valid_id(&list.id).is_err())
            || !ids.insert(&list.id)
            || !names.insert(&list.name)
        {
            return Err(invalid(
                "Identidade/nome de changelist duplicado ou inválido",
            ));
        }
        for path in &list.paths {
            let path = safe_path(repo, &path_from_bytes(path)?)?;
            if !paths.insert(path) || paths.len() > 10000 {
                return Err(invalid("Caminho duplicado/limite em changelists"));
            }
        }
    }
    if !ids.contains(&doc.default) {
        return Err(invalid("Changelist padrão não existe"));
    }
    Ok(doc)
}
pub fn list_changelists(
    repo: &Repository,
    cancel: &AtomicBool,
) -> Result<Vec<Changelist>, GitError> {
    cancelled(cancel)?;
    let doc = load_lists(repo)?;
    let snapshot = repo.snapshot(cancel)?;
    let mut assigned = BTreeSet::new();
    let mut out = Vec::new();
    for list in doc.lists {
        let mut paths = Vec::new();
        for path in list.paths {
            let path = path_from_bytes(&path)?;
            assigned.insert(path.clone());
            if snapshot.entries.iter().any(|e| e.path == path) {
                paths.push(path);
            }
        }
        out.push(Changelist {
            is_default: list.id == doc.default,
            id: list.id,
            name: list.name,
            comment: list.comment,
            paths,
        });
    }
    if let Some(default) = out.iter_mut().find(|l| l.is_default) {
        default.paths.extend(
            snapshot
                .entries
                .iter()
                .filter(|e| !assigned.contains(&e.path))
                .map(|e| e.path.clone()),
        );
    }
    for list in &mut out {
        list.paths.sort();
    }
    Ok(out)
}
fn prepare_lists(
    repo: &Repository,
    snapshot: &RepoSnapshot,
    action: ShelfAction,
) -> Result<ListDocument, GitError> {
    let mut doc = load_lists(repo)?;
    // Making another list active affects future changes, not existing grouping.
    let assigned = doc
        .lists
        .iter()
        .flat_map(|l| l.paths.iter().cloned())
        .collect::<BTreeSet<_>>();
    let current = doc.lists.iter_mut().find(|l| l.id == doc.default).unwrap();
    current.paths.extend(
        snapshot
            .entries
            .iter()
            .map(|e| os_bytes(e.path.as_os_str()).to_vec())
            .filter(|p| !assigned.contains(p)),
    );
    let find = |doc: &ListDocument, id: &str| {
        doc.lists
            .iter()
            .position(|l| l.id == id)
            .ok_or_else(|| invalid("Changelist não existe"))
    };
    match action {
        ShelfAction::CreateChangelist {
            name: n,
            set_default,
        } => {
            name(&n)?;
            if doc.lists.len() >= MAX_LISTS || doc.lists.iter().any(|l| l.name == n) {
                return Err(invalid("Nome existente ou limite de 64 changelists"));
            }
            let id = fresh_id();
            if set_default {
                doc.default = id.clone();
            }
            doc.lists.push(StoredList {
                id,
                name: n,
                comment: String::new(),
                paths: vec![],
            });
        }
        ShelfAction::RenameChangelist { id, name: n } => {
            name(&n)?;
            let i = find(&doc, &id)?;
            if doc.lists.iter().any(|l| l.id != id && l.name == n) {
                return Err(invalid("Nome de changelist já existe"));
            }
            doc.lists[i].name = n;
        }
        ShelfAction::DeleteChangelist { id } => {
            let i = find(&doc, &id)?;
            if id == doc.default {
                return Err(invalid(
                    "Defina outra changelist padrão antes de excluir esta",
                ));
            }
            let removed = doc.lists.remove(i);
            let default = doc.lists.iter_mut().find(|l| l.id == doc.default).unwrap();
            default.paths.extend(removed.paths);
        }
        ShelfAction::SetDefaultChangelist { id } => {
            find(&doc, &id)?;
            doc.default = id;
        }
        ShelfAction::MoveToChangelist { id, paths } => {
            find(&doc, &id)?;
            if paths.is_empty() || paths.len() > 512 {
                return Err(invalid("Selecione de 1 a 512 arquivos para mover"));
            }
            let mut selected = BTreeSet::new();
            for path in paths {
                let p = safe_path(repo, &path)?;
                if !snapshot.entries.iter().any(|e| e.path == p) {
                    return Err(invalid("Arquivo não está no snapshot de alterações"));
                }
                selected.insert(os_bytes(p.as_os_str()).to_vec());
            }
            for list in &mut doc.lists {
                list.paths.retain(|p| !selected.contains(p));
            }
            doc.lists
                .iter_mut()
                .find(|l| l.id == id)
                .unwrap()
                .paths
                .extend(selected);
        }
        _ => return Err(invalid("Ação de changelist inválida")),
    }
    if doc.lists.iter().map(|l| l.paths.len()).sum::<usize>() > 10000 {
        return Err(invalid(
            "Changelists limita a organização persistida a 10000 caminhos",
        ));
    }
    Ok(doc)
}

fn selected_paths(
    repo: &Repository,
    snapshot: &RepoSnapshot,
    input: &[PathBuf],
) -> Result<Vec<PathBuf>, GitError> {
    if input.is_empty() || input.len() > MAX_SHELF_PATHS {
        return Err(invalid(
            "Selecione de 1 a 128 caminhos rastreados para Shelf",
        ));
    }
    let mut selected = BTreeSet::new();
    for path in input {
        let path = safe_path(repo, path)?;
        let entry = snapshot
            .entries
            .iter()
            .find(|e| e.path == path || e.original_path.as_ref() == Some(&path))
            .ok_or_else(|| invalid("Arquivo não está no snapshot de alterações"))?;
        if entry.untracked || entry.conflict || entry.submodule != "N..." {
            return Err(GitError::unsupported(
                "Shelf exige alterações rastreadas sem conflito/submódulo; arquivos não rastreados permanecem no projeto",
            ));
        }
        selected.insert(entry.path.clone());
        if let Some(old) = &entry.original_path {
            selected.insert(old.clone());
        }
    }
    if selected.len() > MAX_SHELF_PATHS {
        return Err(invalid("Renomeações expandidas excedem 128 caminhos"));
    }
    let selected = selected.into_iter().collect::<Vec<_>>();
    for p in &selected {
        safe_path(repo, p)?;
    }
    Ok(selected)
}
fn collect_bundle(
    repo: &Repository,
    snapshot: &RepoSnapshot,
    name: String,
    paths: &[PathBuf],
    cancel: &AtomicBool,
) -> Result<Bundle, GitError> {
    let mut entries = Vec::new();
    let mut bytes = 0usize;
    for path in paths {
        cancelled(cancel)?;
        let base = read_base(repo, snapshot.head.as_deref(), path, cancel)?;
        let index = read_index(repo, path, cancel)?;
        let working = read_work(repo, path)?;
        for item in [&base, &index, &working].into_iter().flatten() {
            bytes = bytes
                .checked_add(item.data.len())
                .ok_or_else(|| invalid("Shelf excede o limite"))?;
        }
        if bytes > MAX_SHELF_BYTES {
            return Err(invalid(
                "Shelf limita as versões de arquivos a 32 MiB agregados",
            ));
        }
        entries.push(SavedPath {
            path: path.clone(),
            base,
            index,
            working,
        });
    }
    let patch = |staged: bool| -> Result<Vec<u8>, GitError> {
        let mut a = args(&[
            "diff",
            "--binary",
            "--full-index",
            "--no-ext-diff",
            "--no-textconv",
            "--no-renames",
        ]);
        if staged {
            a.push("--cached".into());
        }
        a.push("--".into());
        a.extend(paths.iter().map(|p| p.as_os_str().into()));
        let data = repo.run_checked(&a, None, cancel)?.stdout;
        if data.len() > MAX_PATCH_BYTES {
            return Err(invalid("Patch Shelf excede 8 MiB"));
        }
        Ok(data)
    };
    Ok(Bundle {
        info: ShelfInfo {
            id: fresh_id(),
            name,
            created_unix: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            base: snapshot.head.clone(),
            paths: paths.to_vec(),
            bytes: bytes as u64,
            files: file_statuses(&entries, snapshot),
            deleted_unix: None,
        },
        entries,
        staged_patch: patch(true)?,
        working_patch: patch(false)?,
    })
}
struct MergeResult {
    working: Option<Item>,
    conflict: bool,
}
fn merge_item(
    repo: &Repository,
    base: &Option<Item>,
    ours: &Option<Item>,
    theirs: &Option<Item>,
    cancel: &AtomicBool,
) -> Result<MergeResult, GitError> {
    if item_equal(ours, base) || item_equal(ours, theirs) {
        return Ok(MergeResult {
            working: theirs.clone(),
            conflict: false,
        });
    }
    if item_equal(theirs, base) {
        return Ok(MergeResult {
            working: ours.clone(),
            conflict: false,
        });
    }
    let text = |v: &Item| {
        v.mode != "120000" && !v.data.contains(&0) && std::str::from_utf8(&v.data).is_ok()
    };
    if let (Some(base), Some(ours), Some(theirs)) = (base, ours, theirs)
        && text(base)
        && text(ours)
        && text(theirs)
    {
        let mode = if ours.mode == base.mode {
            Some(theirs.mode.clone())
        } else if theirs.mode == base.mode || ours.mode == theirs.mode {
            Some(ours.mode.clone())
        } else {
            None
        };
        if let Some(mode) = mode {
            let dir = tempfile::tempdir()?;
            for (label, item) in [("ours", ours), ("base", base), ("theirs", theirs)] {
                write_bytes(&dir.path().join(label), &item.data)?;
            }
            let mut a = args(&[
                "merge-file",
                "--stdout",
                "--diff3",
                "-L",
                "Current worktree",
                "-L",
                "Shelf base",
                "-L",
                "Shelved changes",
            ]);
            for label in ["ours", "base", "theirs"] {
                a.push(dir.path().join(label).into_os_string());
            }
            let result = repo.run(&a, None, cancel)?;
            let code = result
                .status
                .code()
                .ok_or_else(|| invalid("Merge de Shelf interrompido"))?;
            if !(0..=127).contains(&code) {
                return Err(invalid("Não foi possível comparar Shelf em três vias"));
            }
            if result.stdout.len() > MAX_SHELF_FILE_BYTES {
                return Err(invalid("Resultado do merge de Shelf excede 4 MiB"));
            }
            return Ok(MergeResult {
                working: Some(Item {
                    mode,
                    data: result.stdout,
                    permissions: ours.permissions,
                }),
                conflict: code != 0,
            });
        }
    }
    Ok(MergeResult {
        working: ours.clone(),
        conflict: true,
    })
}
fn select_unshelve_paths(
    repo: &Repository,
    bundle: &Bundle,
    requested: Option<&[PathBuf]>,
) -> Result<BTreeSet<PathBuf>, GitError> {
    let mut selected = if let Some(paths) = requested {
        if paths.is_empty() || paths.len() > MAX_SHELF_PATHS {
            return Err(invalid("Selecione de1a128 caminhos do Shelf"));
        }
        paths
            .iter()
            .map(|path| safe_path(repo, path))
            .collect::<Result<BTreeSet<_>, _>>()?
    } else {
        bundle
            .entries
            .iter()
            .map(|entry| entry.path.clone())
            .collect()
    };
    for file in &bundle.info.files {
        if selected.contains(&file.path)
            || file
                .original_path
                .as_ref()
                .is_some_and(|path| selected.contains(path))
        {
            selected.insert(file.path.clone());
            if let Some(original) = &file.original_path {
                selected.insert(original.clone());
            }
        }
    }
    if selected
        .iter()
        .any(|path| !bundle.entries.iter().any(|entry| entry.path == *path))
    {
        return Err(invalid("Caminho não pertence ao Shelf selecionado"));
    }
    Ok(selected)
}
fn unshelve_lists(
    repo: &Repository,
    snapshot: &RepoSnapshot,
    paths: &[PathBuf],
    options: &UnshelveOptions,
) -> Result<Option<ListDocument>, GitError> {
    if options.changelist_name.is_none() && options.comment.is_none() && !options.set_active {
        return Ok(None);
    }
    let mut doc = load_lists(repo)?;
    let assigned = doc
        .lists
        .iter()
        .flat_map(|list| list.paths.iter().cloned())
        .collect::<BTreeSet<_>>();
    let previous = doc
        .lists
        .iter_mut()
        .find(|list| list.id == doc.default)
        .unwrap();
    previous.paths.extend(
        snapshot
            .entries
            .iter()
            .map(|entry| os_bytes(entry.path.as_os_str()).to_vec())
            .filter(|path| !assigned.contains(path)),
    );
    let index = if let Some(name_value) = &options.changelist_name {
        name(name_value)?;
        if let Some(index) = doc.lists.iter().position(|list| list.name == *name_value) {
            index
        } else {
            if doc.lists.len() >= MAX_LISTS {
                return Err(invalid("Limite de64 changelists"));
            }
            doc.lists.push(StoredList {
                id: fresh_id(),
                name: name_value.clone(),
                comment: String::new(),
                paths: vec![],
            });
            doc.lists.len() - 1
        }
    } else {
        doc.lists
            .iter()
            .position(|list| list.id == doc.default)
            .unwrap()
    };
    if let Some(comment) = &options.comment {
        if comment.len() > 65536 || comment.contains('\0') {
            return Err(invalid("Comentário de changelist excede64KiB"));
        }
        doc.lists[index].comment = comment.clone();
    }
    let selected = paths
        .iter()
        .map(|path| os_bytes(path.as_os_str()).to_vec())
        .collect::<BTreeSet<_>>();
    for list in &mut doc.lists {
        list.paths.retain(|path| !selected.contains(path));
    }
    doc.lists[index].paths.extend(selected);
    if options.set_active {
        doc.default = doc.lists[index].id.clone();
    }
    if serde_json::to_vec(&doc)
        .map_err(|_| invalid("Changelists inválido"))?
        .len()
        > MAX_METADATA
    {
        return Err(invalid("Changelists excederia1MiB; nada foi aplicado"));
    }
    Ok(Some(doc))
}
fn remaining_bundle(
    repo: &Repository,
    bundle: &Bundle,
    selected: &BTreeSet<PathBuf>,
    conflicts: &[PathBuf],
    cancel: &AtomicBool,
) -> Result<Option<Bundle>, GitError> {
    let mut remove = selected
        .iter()
        .filter(|path| !conflicts.contains(path))
        .cloned()
        .collect::<BTreeSet<_>>();
    for file in &bundle.info.files {
        if conflicts.contains(&file.path)
            || file
                .original_path
                .as_ref()
                .is_some_and(|path| conflicts.contains(path))
        {
            remove.remove(&file.path);
            if let Some(original) = &file.original_path {
                remove.remove(original);
            }
        }
    }
    let mut remaining = bundle.clone();
    remaining
        .entries
        .retain(|entry| !remove.contains(&entry.path));
    if remaining.entries.is_empty() {
        return Ok(None);
    }
    remaining.info.paths = remaining
        .entries
        .iter()
        .map(|entry| entry.path.clone())
        .collect();
    remaining
        .info
        .files
        .retain(|file| !remove.contains(&file.path));
    remaining.info.bytes = remaining
        .entries
        .iter()
        .flat_map(|entry| [&entry.base, &entry.index, &entry.working])
        .flatten()
        .map(|item| item.data.len() as u64)
        .sum();
    remaining.staged_patch = bundle_diff(repo, &remaining.entries, 0, 1, cancel)?;
    remaining.working_patch = bundle_diff(repo, &remaining.entries, 1, 2, cancel)?;
    Ok(Some(remaining))
}
fn bundle_diff(
    repo: &Repository,
    entries: &[SavedPath],
    from: usize,
    to: usize,
    cancel: &AtomicBool,
) -> Result<Vec<u8>, GitError> {
    let tree = |layer: usize| -> Result<String, GitError> {
        let changes = entries
            .iter()
            .map(|entry| {
                let item = [&entry.base, &entry.index, &entry.working][layer];
                Change {
                    path: entry.path.clone(),
                    before: None,
                    working: None,
                    index: item.clone().map(|item| vec![(0, item)]).unwrap_or_default(),
                }
            })
            .collect::<Vec<_>>();
        let (_temp, path) = prepare_index(repo, &changes, &[], cancel)?;
        let output = repo.run_env(
            &args(&["write-tree"]),
            None,
            cancel,
            &[(OsString::from("GIT_INDEX_FILE"), path.as_os_str().into())],
        )?;
        String::from_utf8(output.stdout)
            .map(|value| value.trim().to_owned())
            .map_err(|_| invalid("Árvore Shelf inválida"))
    };
    let from = tree(from)?;
    let to = tree(to)?;
    let output = repo
        .run_checked(
            &args(&[
                "diff",
                "--binary",
                "--full-index",
                "--no-ext-diff",
                "--no-textconv",
                "--no-renames",
                &from,
                &to,
                "--",
            ]),
            None,
            cancel,
        )?
        .stdout;
    if output.len() > MAX_PATCH_BYTES {
        return Err(invalid("Patch Shelf restante excede8MiB"));
    }
    Ok(output)
}
fn save_lists(root: &Path, doc: &ListDocument, cancel: &AtomicBool) -> Result<(), GitError> {
    let bytes = serde_json::to_vec_pretty(doc)
        .map_err(|_| invalid("Não foi possível salvar changelists"))?;
    if bytes.len() > MAX_METADATA {
        return Err(invalid("Changelists excede1MiB"));
    }
    cancelled(cancel)?;
    let mut temporary = tempfile::NamedTempFile::new_in(root)?;
    temporary.write_all(&bytes)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(root.join("changelists.json"))
        .map_err(|error| GitError::from(error.error))?;
    sync_dir(root)
}
fn replace_bundle(root: &Path, bundle: &Bundle) -> Result<PathBuf, GitError> {
    if storage_size(root)?
        .saturating_add(bundle.info.bytes)
        .saturating_add(bundle.staged_patch.len() as u64)
        .saturating_add(bundle.working_patch.len() as u64)
        .saturating_add(MAX_METADATA as u64)
        > MAX_STORE_BYTES
    {
        return Err(invalid(
            "Atualizar Shelf excederia256MiB; original preservado",
        ));
    }
    let temporary = tempfile::Builder::new()
        .prefix(".tmp-shelf-replacement-")
        .tempdir_in(root)?;
    let replacement = save_bundle(temporary.path(), bundle, None)?;
    let shelves = root.join("shelves");
    let current = shelves.join(&bundle.info.id);
    let recovery = root.join("recovery");
    ensure_directory(&recovery)?;
    let backup = recovery.join(fresh_id());
    fs::rename(&current, &backup)?;
    sync_dir(&recovery)?;
    if let Err(error) = fs::rename(&replacement, &current) {
        let restored = fs::rename(&backup, &current).is_ok();
        return Err(GitError::new(
            GitErrorKind::Io,
            format!(
                "Não foi possível atualizar Shelf: {error}; original {} em {}",
                if restored { "restaurado" } else { "preservado" },
                if restored {
                    current.display()
                } else {
                    backup.display()
                }
            ),
        ));
    }
    sync_dir(&shelves)?;
    Ok(backup)
}
fn stages(base: &Option<Item>, ours: &Option<Item>, theirs: &Option<Item>) -> Vec<(u8, Item)> {
    [(1, base), (2, ours), (3, theirs)]
        .into_iter()
        .filter_map(|(n, v)| v.clone().map(|v| (n, v)))
        .collect()
}

pub fn prepare_shelf(
    repo: &Repository,
    action: ShelfAction,
    cancel: &AtomicBool,
) -> Result<PreparedShelf, GitError> {
    cancelled(cancel)?;
    if repo.bare {
        return Err(GitError::unsupported("Shelf exige uma árvore de trabalho"));
    }
    let (action, options) = match action {
        ShelfAction::UnshelveOptions {
            id,
            paths,
            changelist_name,
            comment,
            set_active,
            remove_applied,
        } => (
            ShelfAction::Unshelve { id },
            Some(UnshelveOptions {
                paths,
                changelist_name,
                comment,
                set_active,
                remove_applied,
            }),
        ),
        action => (action, None),
    };
    let mut unshelve = None;
    let store = store(repo)?;
    let snapshot = repo.snapshot(cancel)?;
    let store_hash = tree_hash(&store, cancel)?;
    let mut preview = ShelfPreview {
        summary: String::new(),
        paths: vec![],
        warnings: vec![],
        destructive: false,
        conflicts: vec![],
    };
    let plan = match action {
        ShelfAction::Shelve { name: n, paths } => {
            shelf_message(&n)?;
            if snapshot.operation.is_some() {
                return Err(invalid("Finalize a operação Git antes de Shelve"));
            }
            if list_shelves(repo, cancel)?.len() >= MAX_SHELVES {
                return Err(invalid("Limite de 128 shelves atingido"));
            }
            let paths = selected_paths(repo, &snapshot, &paths)?;
            let bundle = collect_bundle(repo, &snapshot, n, &paths, cancel)?;
            let changes = bundle
                .entries
                .iter()
                .map(|e| {
                    let mut working = e.base.clone();
                    if let (Some(base), Some(current)) = (&mut working, &e.working) {
                        // Git tracks the executable bit, not the user's private rw bits.
                        base.permissions = current.permissions;
                    }
                    Change {
                        path: e.path.clone(),
                        before: e.working.clone(),
                        working,
                        index: e.base.clone().map(|b| vec![(0, b)]).unwrap_or_default(),
                    }
                })
                .collect();
            preview.summary = format!(
                "Shelve · {} · {} arquivos; retirar somente as alterações selecionadas",
                bundle.info.name,
                paths.len()
            );
            preview.paths = paths;
            preview.destructive = true;
            preview.warnings.push("As camadas do índice e dos arquivos serão preservadas separadamente no Shelf antes de retirar a seleção. Arquivos não selecionados permanecem intactos.".into());
            Plan::Shelve(bundle, changes)
        }
        ShelfAction::Unshelve { id } => {
            if snapshot.operation.is_some() || snapshot.entries.iter().any(|e| e.conflict) {
                return Err(invalid(
                    "Resolva a operação/conflitos existentes antes de Unshelve",
                ));
            }
            let bundle = load_bundle(repo, &id, cancel)?;
            let selected = select_unshelve_paths(
                repo,
                &bundle,
                options
                    .as_ref()
                    .and_then(|options| options.paths.as_deref()),
            )?;
            let mut changes = Vec::new();
            let mut total = 0;
            for e in bundle
                .entries
                .iter()
                .filter(|entry| selected.contains(&entry.path))
            {
                cancelled(cancel)?;
                let index = read_index(repo, &e.path, cancel)?;
                let working = read_work(repo, &e.path)?;
                for item in [&index, &working].into_iter().flatten() {
                    total += item.data.len();
                }
                if total > MAX_SHELF_BYTES {
                    return Err(invalid("Backup Unshelve excede 32 MiB"));
                }
                let staged = merge_item(repo, &e.base, &index, &e.index, cancel)?;
                let result = merge_item(repo, &e.base, &working, &e.working, cancel)?;
                let index = if result.conflict {
                    stages(&e.base, &working, &e.working)
                } else if staged.conflict {
                    stages(&e.base, &index, &e.index)
                } else {
                    staged.working.map(|i| vec![(0, i)]).unwrap_or_default()
                };
                if result.conflict || staged.conflict {
                    preview.conflicts.push(e.path.clone());
                }
                changes.push(Change {
                    path: e.path.clone(),
                    before: working,
                    working: result.working,
                    index,
                });
            }
            preview.summary = format!(
                "Unshelve · {} · {} arquivos",
                bundle.info.name,
                changes.len()
            );
            preview.paths = selected.iter().cloned().collect();
            preview.destructive = true;
            preview.warnings.push("Será criado backup durável dos arquivos/índice atuais. O Shelf será mantido até exclusão explícita.".into());
            if !preview.conflicts.is_empty() {
                preview.warnings.push(format!("{} conflitos: serão criados estágios Base/Ours/Theirs no índice; revise e resolva os arquivos. Nenhuma alteração alheia será removida.",preview.conflicts.len()));
            }
            if let Some(options) = &options {
                let lists = unshelve_lists(repo, &snapshot, &preview.paths, options)?;
                if let Some(name) = &options.changelist_name {
                    preview.warnings.push(format!(
                        "Destino: {name}{}",
                        if options.set_active {
                            " · tornar ativa"
                        } else {
                            ""
                        }
                    ));
                }
                let remaining = if options.remove_applied {
                    remaining_bundle(repo, &bundle, &selected, &preview.conflicts, cancel)?
                } else {
                    None
                };
                if options.remove_applied
                    && remaining.is_none()
                    && list_deleted_shelves(repo, cancel)?.len() >= MAX_SHELVES
                {
                    return Err(invalid(
                        "Recently Deleted atingiu128 shelves; preserve ou exclua explicitamente itens antigos",
                    ));
                }
                if options.remove_applied {
                    preview.warnings.push("Somente arquivos aplicados sem conflito serão retirados do Shelf; os demais e as duas camadas permanecem preservados. O Shelf anterior será recuperável.".into());
                }
                unshelve = Some(PreparedUnshelveOptions {
                    lists,
                    remove_applied: options.remove_applied,
                    remaining,
                });
            }
            Plan::Unshelve(id, changes)
        }
        ShelfAction::Delete { id } => {
            if list_deleted_shelves(repo, cancel)?.len() >= MAX_SHELVES {
                return Err(invalid(
                    "Recently Deleted atingiu128 shelves; preserve ou exclua explicitamente itens antigos",
                ));
            }
            let b = load_bundle(repo, &id, cancel)?;
            preview.summary = format!("Excluir Shelf · {}", b.info.name);
            preview.paths = b.info.paths;
            preview.destructive = true;
            preview.warnings.push("Mover para Recently Deleted; arquivos de trabalho e índice não mudam. Recuperar e excluir definitivamente são ações separadas.".into());
            Plan::Delete(id)
        }
        ShelfAction::RestoreDeleted { id } => {
            if list_shelves(repo, cancel)?.len() >= MAX_SHELVES {
                return Err(invalid("Limite de128 shelves ativos"));
            }
            let info = list_deleted_shelves(repo, cancel)?
                .into_iter()
                .find(|entry| entry.id == id)
                .ok_or_else(|| invalid("Shelf excluído não existe"))?;
            if store.join("shelves").join(&id).exists() {
                return Err(invalid("Já existe Shelf ativo com esta identidade"));
            }
            preview.summary = format!("Recuperar Shelf · {}", info.name);
            preview.paths = info.paths;
            Plan::RestoreDeleted(id)
        }
        ShelfAction::DeletePermanently { id } => {
            let info = list_deleted_shelves(repo, cancel)?
                .into_iter()
                .find(|entry| entry.id == id)
                .ok_or_else(|| invalid("Shelf excluído não existe"))?;
            preview.summary = format!("Excluir Shelf definitivamente · {}", info.name);
            preview.paths = info.paths;
            preview.destructive = true;
            preview.warnings.push("Remove definitivamente somente o Shelf selecionado de Recently Deleted. Esta ação não pode ser desfeita pelo Codisseu.".into());
            Plan::DeletePermanently(id)
        }
        other => {
            let doc = prepare_lists(repo, &snapshot, other)?;
            preview.summary = "Atualizar changelists · somente organização de alterações".into();
            preview
                .warnings
                .push("O conteúdo dos arquivos e o índice Git permanecem intactos.".into());
            Plan::Lists(doc)
        }
    };
    if repo.snapshot(cancel)? != snapshot || tree_hash(&store, cancel)? != store_hash {
        return Err(stale());
    }
    let preview_hash = preview_hash(&preview);
    Ok(PreparedShelf {
        unshelve,
        preview,
        snapshot,
        store_hash,
        preview_hash,
        plan,
    })
}

struct Lock {
    path: PathBuf,
    file: File,
    published: bool,
}
impl Lock {
    fn acquire(path: PathBuf) -> Result<Self, GitError> {
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(&path).map_err(|e| {
            GitError::new(
                GitErrorKind::Io,
                format!("Não foi possível obter bloqueio de Shelf/índice: {e}"),
            )
        })?;
        Ok(Self {
            path,
            file,
            published: false,
        })
    }
    fn publish(&mut self, bytes: &[u8], target: &Path) -> Result<(), GitError> {
        self.file.write_all(bytes)?;
        self.file.sync_all()?;
        fs::rename(&self.path, target)?;
        self.published = true;
        sync_dir(target.parent().unwrap())
    }
}
impl Drop for Lock {
    fn drop(&mut self) {
        if !self.published {
            let _ = fs::remove_file(&self.path);
        }
    }
}
fn save_bundle(root: &Path, b: &Bundle, index_backup: Option<&[u8]>) -> Result<PathBuf, GitError> {
    ensure_directory(root)?;
    let dir = tempfile::Builder::new().prefix(".tmp-").tempdir_in(root)?;
    let mut entries = Vec::new();
    for (n, e) in b.entries.iter().enumerate() {
        let mut metas = Vec::new();
        for (label, item) in [
            ("base", &e.base),
            ("index", &e.index),
            ("working", &e.working),
        ] {
            if let Some(item) = item {
                write_bytes(&dir.path().join(format!("{n}-{label}")), &item.data)?;
                metas.push(Some(metadata_for(item)));
            } else {
                metas.push(None);
            }
        }
        entries.push(StoredPath {
            path: os_bytes(e.path.as_os_str()).to_vec(),
            base: metas[0].take(),
            index: metas[1].take(),
            working: metas[2].take(),
        });
    }
    write_bytes(&dir.path().join("staged.patch"), &b.staged_patch)?;
    write_bytes(&dir.path().join("working.patch"), &b.working_patch)?;
    if let Some(index) = index_backup {
        write_bytes(&dir.path().join("index-original"), index)?;
    }
    let metadata = Metadata {
        version: 1,
        id: b.info.id.clone(),
        name: b.info.name.clone(),
        created_unix: b.info.created_unix,
        base: b.info.base.clone(),
        entries,
        files: b
            .info
            .files
            .iter()
            .map(|entry| StoredFileStatus {
                path: os_bytes(entry.path.as_os_str()).to_vec(),
                original_path: entry
                    .original_path
                    .as_ref()
                    .map(|path| os_bytes(path.as_os_str()).to_vec()),
                index_status: entry.index_status,
                working_status: entry.working_status,
                status: entry.status,
            })
            .collect(),
        staged_patch: patch_metadata(&b.staged_patch),
        working_patch: patch_metadata(&b.working_patch),
    };
    let json = serde_json::to_vec_pretty(&metadata)
        .map_err(|_| invalid("Não foi possível serializar Shelf"))?;
    if json.len() > MAX_METADATA {
        return Err(invalid("Metadados Shelf excedem 1 MiB"));
    }
    write_bytes(&dir.path().join("metadata.json"), &json)?;
    sync_dir(dir.path())?;
    let destination = root.join(&b.info.id);
    if destination.exists() {
        return Err(invalid("Identificador Shelf já existe"));
    }
    fs::rename(dir.path(), &destination)?;
    sync_dir(root)?;
    Ok(destination)
}
pub(crate) fn storage_size(root: &Path) -> Result<u64, GitError> {
    fn size(path: &Path, depth: usize, count: &mut usize) -> Result<u64, GitError> {
        if depth > 4 {
            return Err(invalid("Armazenamento Shelf inválido"));
        }
        let mut total = 0u64;
        for entry in fs::read_dir(path)? {
            *count += 1;
            if *count > MAX_STORE_ENTRIES {
                return Err(invalid(
                    "Shelf/recovery excede o limite de arquivos armazenados",
                ));
            }
            let entry = entry?;
            let meta = fs::symlink_metadata(entry.path())?;
            if meta.file_type().is_symlink() {
                return Err(invalid("Shelf contém link simbólico inesperado"));
            }
            total = total
                .checked_add(if meta.is_dir() {
                    size(&entry.path(), depth + 1, count)?
                } else {
                    meta.len()
                })
                .ok_or_else(|| invalid("Armazenamento Shelf excede limite"))?;
            if total > MAX_STORE_BYTES {
                return Err(invalid(
                    "Armazenamento Shelf/recovery excede 256 MiB; preserve os backups e libere espaço explicitamente",
                ));
            }
        }
        Ok(total)
    }
    size(root, 0, &mut 0)
}
fn write_work(repo: &Repository, path: &Path, value: &Option<Item>) -> Result<(), GitError> {
    let path = safe_path(repo, path)?;
    let full = repo.root.join(&path);
    if let Some(value) = value {
        let mut parent = repo.root.clone();
        if let Some(parts) = path.parent() {
            for part in parts.components() {
                parent.push(part);
                ensure_directory(&parent)?;
            }
        }
        let parent = full.parent().unwrap();
        if value.mode == "120000" {
            #[cfg(unix)]
            {
                let dir = tempfile::Builder::new()
                    .prefix(".codisseu-shelf-")
                    .tempdir_in(parent)?;
                let link = dir.path().join("link");
                std::os::unix::fs::symlink(path_from_bytes(&value.data)?, &link)?;
                fs::rename(&link, &full)?;
            }
            #[cfg(not(unix))]
            {
                return Err(GitError::unsupported(
                    "Restaurar symlink por Shelf exige Unix",
                ));
            }
        } else {
            let mut file = tempfile::Builder::new()
                .prefix(".codisseu-shelf-")
                .tempfile_in(parent)?;
            file.write_all(&value.data)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut permissions = value.permissions & 0o777;
                permissions =
                    (permissions & !0o111) | if value.mode == "100755" { 0o111 } else { 0 };
                file.as_file()
                    .set_permissions(fs::Permissions::from_mode(permissions))?;
            }
            file.as_file().sync_all()?;
            file.persist(&full).map_err(|e| GitError::from(e.error))?;
        }
        sync_dir(parent)?;
    } else {
        match fs::remove_file(&full) {
            Ok(()) => sync_dir(full.parent().unwrap())?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}
fn index_bytes(repo: &Repository) -> Result<Vec<u8>, GitError> {
    let path = repo.git_dir.join("index");
    if !path.exists() {
        if fs::symlink_metadata(&path).is_ok() {
            return Err(invalid("Índice não pode ser symlink"));
        }
        return Ok(Vec::new());
    }
    read_bounded(&path, MAX_SHELF_BYTES)
}
fn prepare_index(
    repo: &Repository,
    changes: &[Change],
    original: &[u8],
    cancel: &AtomicBool,
) -> Result<(tempfile::TempDir, PathBuf), GitError> {
    let dir = tempfile::Builder::new()
        .prefix("codisseu-shelf-index-")
        .tempdir_in(&repo.git_dir)?;
    let path = dir.path().join("index");
    let env = [(OsString::from("GIT_INDEX_FILE"), path.as_os_str().into())];
    if original.is_empty() {
        repo.run_env(&args(&["read-tree", "--empty"]), None, cancel, &env)?;
    } else {
        write_bytes(&path, original)?;
    }
    let hash_format = repo
        .run_checked(&args(&["rev-parse", "--show-object-format"]), None, cancel)?
        .stdout;
    let zeros = if hash_format.starts_with(b"sha256") {
        "0".repeat(64)
    } else {
        "0".repeat(40)
    };
    let mut input = Vec::new();
    for change in changes {
        cancelled(cancel)?;
        input.extend_from_slice(format!("0 {zeros}\t").as_bytes());
        input.extend_from_slice(os_bytes(change.path.as_os_str()));
        input.push(0);
        for (stage, item) in &change.index {
            let oid = repo
                .run_checked(
                    &args(&["hash-object", "-w", "--stdin", "--no-filters"]),
                    Some(&item.data),
                    cancel,
                )?
                .stdout;
            let oid = std::str::from_utf8(&oid)
                .map_err(|_| invalid("OID inválido"))?
                .trim();
            if !matches!(oid.len(), 40 | 64) || !oid.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(invalid("OID inválido"));
            }
            input.extend_from_slice(format!("{} {oid} {stage}\t", item.mode).as_bytes());
            input.extend_from_slice(os_bytes(change.path.as_os_str()));
            input.push(0);
        }
    }
    repo.run_env(
        &args(&["update-index", "-z", "--index-info"]),
        Some(&input),
        cancel,
        &env,
    )?;
    Ok((dir, path))
}
fn apply_changes(
    repo: &Repository,
    changes: &[Change],
    index_path: &Path,
    index_lock: &mut Lock,
    original_index: &[u8],
    original_snapshot: &RepoSnapshot,
    cancel: &AtomicBool,
) -> Result<(), (GitError, bool)> {
    let mut written: Vec<&Change> = Vec::new();
    let result = (|| -> Result<(), GitError> {
        for change in changes {
            cancelled(cancel)?;
            if read_work(repo, &change.path)? != change.before {
                return Err(stale());
            }
            written.push(change);
            write_work(repo, &change.path, &change.working)?;
        }
        cancelled(cancel)?;
        for change in changes {
            if !matches_written(&read_work(repo, &change.path)?, &change.working) {
                return Err(stale());
            }
        }
        if index_bytes(repo)? != original_index {
            return Err(stale());
        }
        let current = repo.snapshot(cancel)?;
        if current.head != original_snapshot.head || current.head_ref != original_snapshot.head_ref
        {
            return Err(stale());
        }
        let bytes = read_bounded(index_path, MAX_SHELF_BYTES)?;
        index_lock.publish(&bytes, &repo.git_dir.join("index"))?;
        Ok(())
    })();
    if let Err(error) = result {
        if index_lock.published {
            return Err((error, true));
        }
        let mut incomplete = false;
        for change in written.into_iter().rev() {
            let current = read_work(repo, &change.path).ok();
            if current.as_ref() == Some(&change.before) {
                continue;
            }
            if current
                .as_ref()
                .is_some_and(|actual| matches_written(actual, &change.working))
            {
                if write_work(repo, &change.path, &change.before).is_err() {
                    incomplete = true;
                }
            } else {
                incomplete = true;
            }
        }
        return Err((error, incomplete));
    }
    Ok(())
}

pub fn apply_shelf(
    repo: &Repository,
    prepared: &PreparedShelf,
    cancel: &AtomicBool,
) -> Result<ShelfReport, GitError> {
    cancelled(cancel)?;
    let root = store(repo)?;
    if preview_hash(&prepared.preview) != prepared.preview_hash
        || repo.snapshot(cancel)? != prepared.snapshot
        || tree_hash(&root, cancel)? != prepared.store_hash
    {
        return Err(stale());
    }
    ensure_directory(&root)?;
    let _store_lock = Lock::acquire(root.join(".lock"))?;
    if repo.snapshot(cancel)? != prepared.snapshot
        || tree_hash(&root, cancel)? != prepared.store_hash
    {
        return Err(stale());
    }
    let mut report = ShelfReport {
        success: true,
        summary: prepared.preview.summary.clone(),
        output: String::new(),
        outcome_unknown: false,
        shelf_id: None,
        recovery_path: None,
        conflicts: prepared.preview.conflicts.clone(),
    };
    match &prepared.plan {
        Plan::Lists(doc) => {
            let json = serde_json::to_vec_pretty(doc)
                .map_err(|_| invalid("Não foi possível serializar changelists"))?;
            if json.len() > MAX_METADATA {
                return Err(invalid("Changelists excede 1 MiB"));
            }
            cancelled(cancel)?;
            let mut temp = tempfile::Builder::new()
                .prefix(".tmp-")
                .tempfile_in(&root)?;
            temp.write_all(&json)?;
            temp.as_file().sync_all()?;
            temp.persist(root.join("changelists.json"))
                .map_err(|e| GitError::from(e.error))?;
            sync_dir(&root)?;
        }
        Plan::Delete(id) => {
            cancelled(cancel)?;
            move_deleted(&root, id, false)?;
            report.shelf_id = Some(id.clone());
        }
        Plan::RestoreDeleted(id) => {
            cancelled(cancel)?;
            move_deleted(&root, id, true)?;
            report.shelf_id = Some(id.clone());
        }
        Plan::DeletePermanently(id) => {
            cancelled(cancel)?;
            let deleted = root.join("deleted");
            check_directory(&deleted)?;
            let target = deleted.join(id);
            check_directory(&target)?;
            fs::remove_dir_all(target)?;
            sync_dir(&deleted)?;
            report.shelf_id = Some(id.clone());
        }
        Plan::Shelve(_, changes) | Plan::Unshelve(_, changes) => {
            let mut index_lock = Lock::acquire(repo.git_dir.join("index.lock"))?;
            let original = index_bytes(repo)?;
            let (_temp, index_path) = prepare_index(repo, changes, &original, cancel)?;
            if repo.snapshot(cancel)? != prepared.snapshot {
                return Err(stale());
            }
            let used = storage_size(&root)?;
            let backup = if let Plan::Unshelve(id, _) = &prepared.plan {
                let mut entries = Vec::new();
                let mut bytes = 0;
                for c in changes {
                    let index = read_index(repo, &c.path, cancel)?;
                    bytes += index.as_ref().map_or(0, |i| i.data.len())
                        + c.before.as_ref().map_or(0, |i| i.data.len());
                    if bytes > MAX_SHELF_BYTES {
                        return Err(invalid("Backup Unshelve excede 32 MiB"));
                    }
                    entries.push(SavedPath {
                        path: c.path.clone(),
                        base: None,
                        index,
                        working: c.before.clone(),
                    });
                }
                Some(Bundle {
                    info: ShelfInfo {
                        id: fresh_id(),
                        name: format!("Backup before Unshelve {id}"),
                        created_unix: SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs(),
                        base: prepared.snapshot.head.clone(),
                        paths: changes.iter().map(|c| c.path.clone()).collect(),
                        bytes: bytes as u64,
                        files: file_statuses(&entries, &prepared.snapshot),
                        deleted_unix: None,
                    },
                    entries,
                    staged_patch: vec![],
                    working_patch: vec![],
                })
            } else {
                None
            };
            let estimate = match &prepared.plan {
                Plan::Shelve(b, _) => {
                    b.info.bytes
                        + b.staged_patch.len() as u64
                        + b.working_patch.len() as u64
                        + original.len() as u64
                        + MAX_METADATA as u64
                }
                _ => {
                    backup.as_ref().unwrap().info.bytes
                        + original.len() as u64
                        + MAX_METADATA as u64
                }
            };
            let replacement = prepared
                .unshelve
                .as_ref()
                .filter(|options| options.remove_applied)
                .and_then(|options| options.remaining.as_ref())
                .map_or(0, |bundle| {
                    bundle.info.bytes
                        + bundle.staged_patch.len() as u64
                        + bundle.working_patch.len() as u64
                        + MAX_METADATA as u64
                });
            if used.saturating_add(estimate).saturating_add(replacement) > MAX_STORE_BYTES {
                return Err(invalid(
                    "Shelf/recovery excederia 256 MiB; preserve backups e libere espaço explicitamente",
                ));
            }
            match &prepared.plan {
                Plan::Shelve(b, _) => {
                    let shelves = root.join("shelves");
                    let saved = save_bundle(&shelves, b, Some(&original))?;
                    report.shelf_id = Some(b.info.id.clone());
                    report.recovery_path = Some(saved);
                }
                Plan::Unshelve(id, _) => {
                    report.recovery_path = Some(save_bundle(
                        &root.join("recovery"),
                        backup.as_ref().unwrap(),
                        Some(&original),
                    )?);
                    report.shelf_id = Some(id.clone());
                }
                _ => unreachable!(),
            }
            if let Err((error, unknown)) = apply_changes(
                repo,
                changes,
                &index_path,
                &mut index_lock,
                &original,
                &prepared.snapshot,
                cancel,
            ) {
                report.success = false;
                report.outcome_unknown = unknown;
                report.summary = if unknown {
                    "Shelf preservado; a operação não concluiu e alguns arquivos exigem recuperação"
                        .into()
                } else {
                    "Shelf preservado; a operação não concluiu e as alterações originais foram mantidas/restauradas".into()
                };
                report.output = format!(
                    "{error}\nBackup durável: {}",
                    report.recovery_path.as_ref().unwrap().display()
                );
            } else {
                report.output = if report.conflicts.is_empty() {
                    "Operação concluída. Índice e arquivos não selecionados preservados.".into()
                } else {
                    "Shelf aplicado com conflitos explícitos no índice; resolva Base/Ours/Theirs antes de criar commit. Shelf e backup mantidos.".into()
                };
                if let Some(options) = &prepared.unshelve {
                    let organization = (|| -> Result<(), GitError> {
                        if let Some(lists) = &options.lists {
                            save_lists(&root, lists, cancel)?;
                        }
                        if options.remove_applied {
                            if let Some(remaining) = &options.remaining {
                                let backup = replace_bundle(&root, remaining)?;
                                report.output.push_str(&format!(
                                    "\nShelf anterior preservado em {}",
                                    backup.display()
                                ));
                            } else if let Plan::Unshelve(id, _) = &prepared.plan {
                                move_deleted(&root, id, false)?;
                                report.output.push_str("\nShelf completo aplicado; original preservado em Recently Deleted.");
                            }
                        }
                        Ok(())
                    })();
                    if let Err(error) = organization {
                        report.success = false;
                        report.summary =
                            "Arquivos e índice aplicados; organização do Shelf não foi concluída"
                                .into();
                        report.output.push_str(&format!("\n{error}\nAs alterações já aplicadas e o backup permanecem disponíveis."));
                    }
                }
            }
        }
    }
    Ok(report)
}
