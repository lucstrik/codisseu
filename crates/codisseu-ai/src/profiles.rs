//! Private, bounded chat history. This file contains text and non-secret CLI
//! configuration only; attachments, credentials and running approvals are transient.
//! The caller chooses a profile path outside every project directory.
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
};

pub const MAX_PROFILE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_WORKSPACES: usize = 64;
pub const MAX_CHATS: usize = 16;
pub const MAX_ENTRIES: usize = 128;
pub const MAX_CHAT_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub schema_version: u32,
    pub workspaces: BTreeMap<PathBuf, StoredWorkspace>,
}
impl Default for Profile {
    fn default() -> Self {
        Self {
            schema_version: 1,
            workspaces: BTreeMap::new(),
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredWorkspace {
    pub chats: Vec<StoredChat>,
    pub active: Option<String>,
    /// Explicit choices for future conversations, oldest provider first. Old
    /// profiles omit this field; historical transcripts do not grant new access.
    #[serde(default)]
    pub defaults: Vec<crate::Config>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredChat {
    pub id: String,
    pub title: String,
    pub provider: String,
    pub program: String,
    pub model: String,
    pub effort: String,
    pub mode: String,
    pub session: Option<String>,
    #[serde(default)]
    pub usage: Option<crate::usage::Tokens>,
    pub entries: Vec<StoredEntry>,
    #[serde(default)]
    pub draft: String,
    /// Last transcript activity, Unix milliseconds. Zero means an older profile
    /// did not record a timestamp; loading it must not invent one.
    #[serde(default)]
    pub updated_at: u64,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredEntry {
    pub id: String,
    pub role: String,
    pub text: String,
    pub detail: String,
}

#[derive(Debug)]
pub enum Reply {
    Loaded(Result<Profile, String>),
    Saved { id: u64, result: Result<(), String> },
}
struct Save {
    id: u64,
    profile: Profile,
}
/// Filesystem and serialization run only on this worker. Requests and replies
/// both have bounded queues. Keep polling through the final save ACK on exit.
pub struct Worker {
    requests: SyncSender<Save>,
    replies: Receiver<Reply>,
    next_id: u64,
}
impl Worker {
    pub fn new(path: PathBuf) -> Result<Self, String> {
        validate_path(&path)?;
        let (requests, input) = mpsc::sync_channel::<Save>(2);
        let (output, replies) = mpsc::sync_channel(4);
        std::thread::Builder::new().name("codisseu-ai-profile".into()).spawn(move || {
            let loaded = load(&path);
            let (initial, mut previous, writable) = match loaded {
                Ok((profile, bytes)) => (Ok(profile), bytes, true),
                Err(error) => (Err(error), None, false),
            };
            if output.send(Reply::Loaded(initial)).is_err() { return; }
            while let Ok(request) = input.recv() {
                let result = if writable {
                    save(&path, &request.profile, previous.as_deref()).map(|bytes| { previous = Some(bytes); })
                } else {
                    Err("Perfil de AI Chat inválido: o arquivo original foi preservado; corrija-o antes de salvar".into())
                };
                if output.send(Reply::Saved { id: request.id, result }).is_err() { break; }
            }
        }).map_err(|error| format!("Não foi possível iniciar o perfil de AI Chat: {error}"))?;
        Ok(Self {
            requests,
            replies,
            next_id: 1,
        })
    }
    pub fn poll(&self) -> Option<Reply> {
        self.replies.try_recv().ok()
    }
    /// Queue a snapshot without waiting for I/O. A full queue leaves the caller
    /// responsible for retrying the latest snapshot; it never silently loses one.
    pub fn save(&mut self, profile: Profile) -> Result<u64, String> {
        let id = self.next_id;
        let next = id
            .checked_add(1)
            .ok_or("Identificador do perfil esgotado")?;
        self.requests
            .try_send(Save { id, profile })
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => "Aguarde a gravação anterior do AI Chat".to_owned(),
                mpsc::TrySendError::Disconnected(_) => {
                    "Worker do perfil de AI Chat encerrado".to_owned()
                }
            })?;
        self.next_id = next;
        Ok(id)
    }
}

fn validate_path(path: &Path) -> Result<(), String> {
    if !path.is_absolute()
        || path.file_name().is_none()
        || path.components().any(|c| matches!(c, Component::ParentDir))
    {
        return Err("O perfil de AI Chat exige caminho absoluto sem '..'".into());
    }
    Ok(())
}
fn validate(profile: &Profile) -> Result<(), String> {
    if profile.schema_version != 1 {
        return Err("Versão de perfil de AI Chat não suportada".into());
    }
    if profile.workspaces.len() > MAX_WORKSPACES {
        return Err("Limite de projetos do AI Chat excedido".into());
    }
    for (root, workspace) in &profile.workspaces {
        validate_path(root)?;
        if root.as_os_str().len() > 4096 || workspace.chats.len() > MAX_CHATS {
            return Err("Limite de conversas ou caminho do AI Chat excedido".into());
        }
        if workspace.defaults.len() > 2
            || workspace
                .defaults
                .iter()
                .enumerate()
                .any(|(index, config)| {
                    config.program.trim().is_empty()
                        || config.program.len() > 4096
                        || config.model.len() > 256
                        || config.effort.len() > 32
                        || workspace.defaults[..index]
                            .iter()
                            .any(|previous| previous.provider == config.provider)
                })
        {
            return Err("Preferências de AI Chat inválidas ou excessivas".into());
        }
        let mut ids = BTreeSet::new();
        for chat in &workspace.chats {
            if chat.id.is_empty()
                || !ids.insert(chat.id.as_str())
                || chat.id.len() > 256
                || chat.title.len() > 1024
                || chat.program.len() > 4096
                || chat.model.len() > 256
                || chat.provider.len() > 64
                || chat.effort.len() > 64
                || chat.mode.len() > 64
                || chat.session.as_ref().is_some_and(|s| s.len() > 1024)
                || chat.entries.len() > MAX_ENTRIES
                || chat.draft.len() > crate::MAX_PROMPT
            {
                return Err("Conversa de AI Chat inválida ou excessiva".into());
            }
            let mut body_bytes = chat.draft.len();
            let mut entries = BTreeSet::new();
            for entry in &chat.entries {
                if entry.id.is_empty()
                    || !entries.insert(entry.id.as_str())
                    || entry.id.len() > 1024
                    || entry.role.len() > 128
                    || entry.text.len() > 256 * 1024
                    || entry.detail.len() > 256 * 1024
                {
                    return Err("Mensagem de AI Chat inválida ou excessiva".into());
                }
                body_bytes = body_bytes
                    .saturating_add(entry.text.len())
                    .saturating_add(entry.detail.len());
            }
            if body_bytes > MAX_CHAT_BYTES {
                return Err("Histórico da conversa excede2MiB".into());
            }
        }
        if workspace
            .active
            .as_ref()
            .is_some_and(|id| !ids.contains(id.as_str()))
        {
            return Err("Conversa ativa não encontrada no perfil de AI Chat".into());
        }
    }
    Ok(())
}
fn check_parents(path: &Path) -> Result<(), String> {
    for parent in path.ancestors().skip(1) {
        match fs::symlink_metadata(parent) {
            Ok(meta) if !meta.is_dir() || meta.file_type().is_symlink() => {
                return Err("Diretório do perfil de AI Chat não pode ser link simbólico".into());
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("Falha ao inspecionar diretório do perfil: {error}")),
        }
    }
    Ok(())
}
fn read_bytes(path: &Path) -> Result<Option<Vec<u8>>, String> {
    check_parents(path)?;
    let metadata = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("Falha ao inspecionar perfil de AI Chat: {error}")),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("Perfil de AI Chat deve ser arquivo regular, sem link simbólico".into());
    }
    if metadata.len() > MAX_PROFILE_BYTES as u64 {
        return Err("Perfil de AI Chat excede16MiB".into());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK);
    }
    let file = options
        .open(path)
        .map_err(|error| format!("Falha ao abrir perfil de AI Chat: {error}"))?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Perfil de AI Chat deixou de ser arquivo regular".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_PROFILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Falha ao ler perfil de AI Chat: {error}"))?;
    if bytes.len() > MAX_PROFILE_BYTES {
        return Err("Perfil de AI Chat excede16MiB".into());
    }
    Ok(Some(bytes))
}
fn load(path: &Path) -> Result<(Profile, Option<Vec<u8>>), String> {
    let Some(bytes) = read_bytes(path)? else {
        return Ok((Profile::default(), None));
    };
    let profile: Profile = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Perfil de AI Chat inválido: {error}"))?;
    validate(&profile)?;
    Ok((profile, Some(bytes)))
}
fn save(path: &Path, profile: &Profile, previous: Option<&[u8]>) -> Result<Vec<u8>, String> {
    validate(profile)?;
    let bytes = serde_json::to_vec(profile)
        .map_err(|error| format!("Falha ao serializar perfil: {error}"))?;
    if bytes.len() > MAX_PROFILE_BYTES {
        return Err("Perfil de AI Chat excede16MiB".into());
    }
    if read_bytes(path)?.as_deref() != previous {
        return Err("Perfil de AI Chat alterado externamente; gravação recusada".into());
    }
    let parent = path.parent().ok_or("Diretório de perfil ausente")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)
            .map_err(|error| format!("Falha ao criar diretório de perfil: {error}"))?;
    }
    #[cfg(not(unix))]
    fs::create_dir_all(parent)
        .map_err(|error| format!("Falha ao criar diretório de perfil: {error}"))?;
    check_parents(path)?;
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let serial = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let temporary = parent.join(format!(
        ".ai-chat-{}-{now}-{serial}.tmp",
        std::process::id()
    ));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(nix::libc::O_NOFOLLOW);
    }
    let mut created = false;
    let result = (|| {
        let mut file = options
            .open(&temporary)
            .map_err(|error| format!("Falha ao criar perfil temporário: {error}"))?;
        created = true;
        file.write_all(&bytes)
            .and_then(|()| file.sync_all())
            .map_err(|error| format!("Falha ao gravar perfil temporário: {error}"))?;
        if read_bytes(path)?.as_deref() != previous {
            return Err(
                "Perfil de AI Chat alterado durante a gravação; original preservado".into(),
            );
        }
        fs::rename(&temporary, path)
            .map_err(|error| format!("Falha ao substituir perfil de AI Chat: {error}"))?;
        #[cfg(unix)]
        File::open(parent)
            .and_then(|file| file.sync_all())
            .map_err(|error| format!("Falha ao sincronizar diretório do perfil: {error}"))?;
        Ok(bytes)
    })();
    if created {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    fn profile() -> Profile {
        let mut profile = Profile::default();
        for root in ["/projetos/ação", "/projetos/outro"] {
            profile.workspaces.insert(
                root.into(),
                StoredWorkspace {
                    defaults: Vec::new(),
                    active: Some("chat-1".into()),
                    chats: vec![StoredChat {
                        id: "chat-1".into(),
                        title: "Correção 🦀".into(),
                        provider: "Codex".into(),
                        program: "codex".into(),
                        model: "modelo".into(),
                        effort: "high".into(),
                        mode: "Agent".into(),
                        session: Some(format!("session-{root}")),
                        usage: None,
                        draft: String::new(),
                        updated_at: 0,
                        entries: vec![StoredEntry {
                            id: "user-1".into(),
                            role: "Você".into(),
                            text: format!("Revise {root}\nUnicode ç 🦀"),
                            detail: String::new(),
                        }],
                    }],
                },
            );
        }
        profile
    }
    fn reply(worker: &Worker) -> Reply {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(reply) = worker.poll() {
                return reply;
            }
            assert!(Instant::now() < deadline, "worker did not reply");
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    #[test]
    fn worker_round_trip_keeps_unicode_and_workspaces_separate() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("private/chats.json");
        let mut worker = Worker::new(path.clone()).unwrap();
        assert!(matches!(reply(&worker), Reply::Loaded(Ok(p)) if p == Profile::default()));
        let expected = profile();
        let id = worker.save(expected.clone()).unwrap();
        assert!(
            matches!(reply(&worker), Reply::Saved { id: saved, result: Ok(()) } if saved == id)
        );
        drop(worker);
        let reopened = Worker::new(path.clone()).unwrap();
        assert!(matches!(reply(&reopened), Reply::Loaded(Ok(p)) if p == expected));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
    #[test]
    fn schema_one_without_activity_times_loads_unknown_and_preserves_them_on_save() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("chat.json");
        let mut legacy = serde_json::to_value(profile()).unwrap();
        for workspace in legacy["workspaces"].as_object_mut().unwrap().values_mut() {
            for chat in workspace["chats"].as_array_mut().unwrap() {
                chat.as_object_mut().unwrap().remove("updated_at");
            }
        }
        fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
        let mut worker = Worker::new(path.clone()).unwrap();
        let Reply::Loaded(Ok(loaded)) = reply(&worker) else {
            panic!("legacy profile did not load")
        };
        assert_eq!(loaded.schema_version, 1);
        assert!(
            loaded
                .workspaces
                .values()
                .flat_map(|workspace| &workspace.chats)
                .all(|chat| chat.updated_at == 0)
        );
        worker.save(loaded.clone()).unwrap();
        assert!(matches!(
            reply(&worker),
            Reply::Saved { result: Ok(()), .. }
        ));
        let saved: Profile = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(saved, loaded);
    }
    #[test]
    fn invalid_load_cannot_be_overwritten_by_a_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("chats.json");
        let original = b"{broken user history";
        fs::write(&path, original).unwrap();
        let mut worker = Worker::new(path.clone()).unwrap();
        assert!(matches!(reply(&worker), Reply::Loaded(Err(_))));
        worker.save(profile()).unwrap();
        assert!(matches!(
            reply(&worker),
            Reply::Saved { result: Err(_), .. }
        ));
        assert_eq!(fs::read(path).unwrap(), original);
    }
    #[test]
    fn limits_and_external_changes_preserve_previous_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("chats.json");
        let initial = profile();
        let bytes = save(&path, &initial, None).unwrap();
        let mut excessive = initial.clone();
        excessive.workspaces.values_mut().next().unwrap().chats[0].entries[0].text =
            "é".repeat(256 * 1024);
        assert!(save(&path, &excessive, Some(&bytes)).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        fs::write(&path, b"external update").unwrap();
        assert!(save(&path, &initial, Some(&bytes)).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"external update");
    }
    #[test]
    fn rejects_excessive_collections_missing_active_and_unsupported_schema() {
        let mut p = profile();
        p.schema_version = 2;
        assert!(validate(&p).is_err());
        p.schema_version = 1;
        let workspace = p.workspaces.values_mut().next().unwrap();
        workspace.active = Some("missing".into());
        assert!(validate(&p).is_err());
        let workspace = p.workspaces.values_mut().next().unwrap();
        workspace.active = None;
        workspace.chats[0].entries = vec![workspace.chats[0].entries[0].clone(); MAX_ENTRIES + 1];
        assert!(validate(&p).is_err());
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("huge.json");
        let file = File::create(&path).unwrap();
        file.set_len(MAX_PROFILE_BYTES as u64 + 1).unwrap();
        assert!(load(&path).is_err());
    }
    #[test]
    fn old_profile_without_draft_loads_empty_and_draft_obeys_shared_byte_budget() {
        let mut value = serde_json::to_value(profile()).unwrap();
        for workspace in value["workspaces"].as_object_mut().unwrap().values_mut() {
            for chat in workspace["chats"].as_array_mut().unwrap() {
                chat.as_object_mut().unwrap().remove("draft");
            }
        }
        let mut decoded: Profile = serde_json::from_value(value).unwrap();
        assert!(
            decoded
                .workspaces
                .values()
                .all(|workspace| workspace.chats.iter().all(|chat| chat.draft.is_empty()))
        );
        validate(&decoded).unwrap();
        let chat = &mut decoded.workspaces.values_mut().next().unwrap().chats[0];
        chat.draft = "x".repeat(crate::MAX_PROMPT + 1);
        assert!(validate(&decoded).is_err());
        let chat = &mut decoded.workspaces.values_mut().next().unwrap().chats[0];
        chat.draft = "🌧 ação".into();
        chat.entries = (0..8)
            .map(|n| StoredEntry {
                id: n.to_string(),
                role: "Assistente".into(),
                text: "x".repeat(256 * 1024),
                detail: String::new(),
            })
            .collect();
        assert!(validate(&decoded).is_err());
        decoded.workspaces.values_mut().next().unwrap().chats[0]
            .entries
            .pop();
        validate(&decoded).unwrap();
    }
    #[test]
    fn provider_defaults_are_bounded_unique_and_validated_before_persistence() {
        let mut profile = profile();
        let root = profile.workspaces.keys().next().unwrap().clone();
        profile.workspaces.get_mut(&root).unwrap().defaults = vec![crate::Config::default()];
        validate(&profile).unwrap();
        profile
            .workspaces
            .get_mut(&root)
            .unwrap()
            .defaults
            .push(crate::Config::default());
        assert!(validate(&profile).unwrap_err().contains("Preferências"));
        profile.workspaces.get_mut(&root).unwrap().defaults[1].provider = crate::Provider::Claude;
        validate(&profile).unwrap();
        profile.workspaces.get_mut(&root).unwrap().defaults[0].program = " ".into();
        assert!(validate(&profile).unwrap_err().contains("Preferências"));
        profile.workspaces.get_mut(&root).unwrap().defaults[0].program = "codex".into();
        profile.workspaces.get_mut(&root).unwrap().defaults[0].effort = "x".repeat(33);
        assert!(validate(&profile).unwrap_err().contains("Preferências"));
    }
    #[cfg(unix)]
    #[test]
    fn symlink_and_directory_targets_preserve_original() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let original = temp.path().join("original.json");
        fs::write(&original, b"do not replace").unwrap();
        let path = temp.path().join("chats.json");
        symlink(&original, &path).unwrap();
        assert!(load(&path).is_err());
        assert!(save(&path, &profile(), None).is_err());
        assert_eq!(fs::read(original).unwrap(), b"do not replace");
        assert!(save(temp.path(), &profile(), None).is_err());
        let linked_parent = temp.path().join("linked");
        symlink(temp.path(), &linked_parent).unwrap();
        assert!(load(&linked_parent.join("other.json")).is_err());
    }
}
