//! Profile metadata and explicit OS-vault operations, all I/O off the GUI.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    time::{Duration, Instant},
};
use zeroize::Zeroizing;
const MAX_FILE: usize = 1024 * 1024;
const VAULT: &str = "io.codisseu.Codisseu.browser.v1";
pub struct Secret(Zeroizing<String>);
impl Default for Secret {
    fn default() -> Self {
        Self::new(String::new())
    }
}
impl Secret {
    pub fn new(s: String) -> Self {
        Self(Zeroizing::new(s))
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
    pub fn edit(&mut self) -> &mut String {
        &mut self.0
    }
}
impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret([redacted])")
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Bookmark {
    pub id: u64,
    pub name: String,
    pub url: String,
    pub project: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Login {
    pub id: String,
    pub origin: String,
    pub username: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Library {
    pub version: u32,
    pub bookmarks: Vec<Bookmark>,
    pub logins: Vec<Login>,
}
impl Default for Library {
    fn default() -> Self {
        Self {
            version: 1,
            bookmarks: vec![],
            logins: vec![],
        }
    }
}
pub fn origin(value: &str) -> Result<String, String> {
    let value = crate::browser::validate_url(value)?;
    let u = url::Url::parse(&value).map_err(|_| "Endereço inválido")?;
    if !matches!(u.scheme(), "http" | "https") {
        return Err("Abra uma página HTTP/HTTPS para salvar um acesso".into());
    }
    Ok(u.origin().ascii_serialization())
}
fn text(s: &str, max: usize) -> bool {
    !s.trim().is_empty() && s.len() <= max && !s.chars().any(char::is_control)
}
fn login_id(origin: &str, username: &str) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(origin, username)).unwrap())
    )
}
impl Library {
    fn validate(&self) -> Result<(), String> {
        if self.version != 1 || self.bookmarks.len() > 512 || self.logins.len() > 256 {
            return Err("Biblioteca inválida ou acima do limite".into());
        }
        let mut ids = std::collections::BTreeSet::new();
        for b in &self.bookmarks {
            if b.id == 0
                || !ids.insert(b.id)
                || !text(&b.name, 256)
                || b.project.len() > 4096
                || b.project.chars().any(char::is_control)
                || origin(&b.url).is_err()
                || crate::browser::validate_url(&b.url)? != b.url
            {
                return Err("Favorito inválido".into());
            }
        }
        let mut ids = std::collections::BTreeSet::new();
        for l in &self.logins {
            if !text(&l.username, 1024)
                || origin(&l.origin)? != l.origin
                || login_id(&l.origin, &l.username) != l.id
                || !ids.insert(&l.id)
            {
                return Err("Acesso salvo inválido".into());
            }
        }
        Ok(())
    }
}
pub enum Action {
    SaveBookmark {
        id: Option<u64>,
        name: String,
        url: String,
        project: String,
    },
    DeleteBookmark(u64),
    SaveLogin {
        origin: String,
        username: String,
        password: Secret,
    },
    DeleteLogin(String),
    LoadLogin(String),
}
pub struct LoadedLogin {
    pub login: Login,
    pub password: Secret,
}
trait Vault: Send + Sync + 'static {
    fn get(&self, account: &str) -> Result<Option<Secret>, String>;
    fn set(&self, account: &str, password: &str) -> Result<(), String>;
    fn delete(&self, account: &str) -> Result<(), String>;
}
struct OsVault;
fn vault_error(_: keyring::Error) -> String {
    "Não foi possível acessar o cofre do sistema. Desbloqueie o cofre e tente novamente; nenhuma senha é gravada em arquivo.".into()
}
fn entry(account: &str) -> Result<keyring::Entry, String> {
    if !cfg!(any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows"
    )) {
        return Err("Cofre do sistema indisponível nesta plataforma".into());
    }
    keyring::Entry::new(VAULT, account).map_err(vault_error)
}
impl Vault for OsVault {
    fn get(&self, a: &str) -> Result<Option<Secret>, String> {
        match entry(a)?.get_password() {
            Ok(s) if s.len() <= 16384 => Ok(Some(Secret::new(s))),
            Ok(s) => {
                drop(Secret::new(s));
                Err("Entrada do cofre excede o limite".into())
            }
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(vault_error(e)),
        }
    }
    fn set(&self, a: &str, p: &str) -> Result<(), String> {
        entry(a)?.set_password(p).map_err(vault_error)
    }
    fn delete(&self, a: &str) -> Result<(), String> {
        match entry(a)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(vault_error(e)),
        }
    }
}
struct Reply {
    notice: &'static str,
    result: Result<(Library, Option<LoadedLogin>), String>,
}
type Waker = Arc<dyn Fn() + Send + Sync>;
pub struct BrowserLibrary {
    pub state: Library,
    pub revision: u64,
    pub error: Option<String>,
    pub notice: String,
    send: SyncSender<Action>,
    receive: Receiver<Reply>,
    running: Arc<AtomicBool>,
    pending: Option<Instant>,
    waker: Arc<Mutex<Option<Waker>>>,
    loaded: Option<LoadedLogin>,
}
impl BrowserLibrary {
    pub fn new(path: PathBuf) -> Self {
        let path = std::path::absolute(&path).unwrap_or(path);
        Self::with_vault(path, Arc::new(OsVault))
    }
    fn with_vault(path: PathBuf, vault: Arc<dyn Vault>) -> Self {
        let (send, rx) = mpsc::sync_channel(1);
        let (tx, receive) = mpsc::sync_channel(1);
        let running = Arc::new(AtomicBool::new(true));
        let signal = running.clone();
        let waker: Arc<Mutex<Option<Waker>>> = Arc::new(Mutex::new(None));
        let wake = waker.clone();
        let spawned = std::thread::Builder::new()
            .name("codisseu-browser-library".into())
            .spawn(move || {
                let initial = read_file(&path)
                    .and_then(|bytes| decode(bytes.as_deref()).map(|state| (state, bytes)));
                let mut state = Library::default();
                let mut expected = None;
                let mut writable = false;
                let result = match initial {
                    Ok((s, bytes)) => {
                        state = s.clone();
                        expected = bytes;
                        writable = true;
                        Ok((s, None))
                    }
                    Err(e) => Err(e),
                };
                let _ = tx.send(Reply { result, notice: "" });
                signal.store(false, Ordering::Release);
                notify(&wake);
                while let Ok(action) = rx.recv() {
                    let notice = match &action {
                        Action::SaveBookmark { .. } => "Favorito salvo",
                        Action::DeleteBookmark(_) => "Favorito excluído",
                        Action::SaveLogin { .. } => "Acesso salvo no cofre",
                        Action::DeleteLogin(_) => "Acesso excluído",
                        Action::LoadLogin(_) => "",
                    };
                    let result = if !writable {
                        Err("Biblioteca anterior inválida: arquivo preservado".into())
                    } else {
                        apply(&path, &mut state, &mut expected, vault.as_ref(), action)
                    };
                    let _ = tx.send(Reply { result, notice });
                    signal.store(false, Ordering::Release);
                    notify(&wake);
                }
            });
        let error = spawned
            .err()
            .map(|_| "Não foi possível iniciar a biblioteca do navegador".into());
        if error.is_some() {
            running.store(false, Ordering::Release);
        }
        let pending = error.is_none().then(Instant::now);
        Self {
            state: Library::default(),
            revision: 0,
            error,
            notice: String::new(),
            send,
            receive,
            running,
            pending,
            waker,
            loaded: None,
        }
    }
    pub fn set_waker(&mut self, w: Waker) {
        if let Ok(mut v) = self.waker.lock() {
            *v = Some(w)
        }
    }
    pub fn is_busy(&self) -> bool {
        self.pending.is_some()
    }
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Acquire) || self.pending.is_some()
    }
    pub fn dispatch(&mut self, action: Action) -> Result<(), String> {
        self.poll();
        if self.is_running() {
            return Err("Aguarde a operação da biblioteca/cofre".into());
        }
        self.running.store(true, Ordering::Release);
        if self.send.try_send(action).is_err() {
            self.running.store(false, Ordering::Release);
            return Err("Biblioteca indisponível".into());
        }
        self.error = None;
        self.notice.clear();
        self.loaded = None;
        self.pending = Some(Instant::now());
        Ok(())
    }
    pub fn take_login(&mut self) -> Option<LoadedLogin> {
        self.loaded.take()
    }
    pub fn poll(&mut self) -> bool {
        if let Ok(reply) = self.receive.try_recv() {
            self.pending = None;
            self.revision += 1;
            match reply.result {
                Ok((state, loaded)) => {
                    self.state = state;
                    self.loaded = loaded;
                    self.error = None;
                    self.notice = reply.notice.into()
                }
                Err(e) => {
                    self.error = Some(e);
                    self.notice.clear()
                }
            };
            return true;
        }
        if self
            .pending
            .is_some_and(|t| t.elapsed() > Duration::from_secs(5))
        {
            self.pending = None;
            self.revision += 1;
            self.error=Some("O cofre não respondeu em 5 segundos. A operação pode concluir depois; aguarde ou desbloqueie o cofre antes de tentar novamente.".into());
            return true;
        }
        false
    }
}
fn notify(w: &Mutex<Option<Waker>>) {
    let callback = w.lock().ok().and_then(|v| v.clone());
    if let Some(w) = callback {
        w()
    }
}
fn read_file(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Ok(m) if m.is_file() && !m.file_type().is_symlink() && m.len() <= MAX_FILE as u64 => (),
        _ => return Err("Biblioteca inválida ou inacessível; arquivo preservado".into()),
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .and_then(|f| f.take(MAX_FILE as u64 + 1).read_to_end(&mut bytes))
        .map_err(|_| "Não foi possível ler a biblioteca")?;
    if bytes.len() > MAX_FILE {
        return Err("Biblioteca excede 1 MiB".into());
    }
    Ok(Some(bytes))
}
fn decode(bytes: Option<&[u8]>) -> Result<Library, String> {
    let state = bytes
        .map(serde_json::from_slice)
        .transpose()
        .map_err(|_| "Biblioteca inválida; arquivo preservado")?
        .unwrap_or_default();
    Library::validate(&state)?;
    Ok(state)
}
fn persist(path: &Path, state: &Library, expected: &mut Option<Vec<u8>>) -> Result<(), String> {
    state.validate()?;
    let bytes =
        serde_json::to_vec_pretty(state).map_err(|_| "Não foi possível serializar a biblioteca")?;
    if bytes.len() > MAX_FILE {
        return Err("Biblioteca excede 1 MiB".into());
    }
    let parent = path.parent().ok_or("Perfil inválido")?;
    fs::create_dir_all(parent).map_err(|_| "Não foi possível criar o perfil")?;
    let lock_path = path.with_extension("browser-library.lock");
    if fs::symlink_metadata(&lock_path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err("Trava de biblioteca inválida".into());
    }
    let lock = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)
        .map_err(|_| "Não foi possível bloquear a biblioteca")?;
    lock.try_lock()
        .map_err(|_| "Biblioteca em uso por outra instância")?;
    if &read_file(path)? != expected {
        return Err("Biblioteca alterada por outra instância. Reabra a IDE antes de gravar".into());
    }
    let mut file = tempfile::NamedTempFile::new_in(parent)
        .map_err(|_| "Não foi possível preparar a biblioteca")?;
    file.write_all(&bytes)
        .and_then(|_| file.as_file().sync_all())
        .map_err(|_| "Não foi possível gravar a biblioteca")?;
    file.persist(path)
        .map_err(|_| "Não foi possível publicar a biblioteca")?;
    *expected = Some(bytes);
    Ok(())
}
fn apply(
    path: &Path,
    state: &mut Library,
    expected: &mut Option<Vec<u8>>,
    vault: &dyn Vault,
    action: Action,
) -> Result<(Library, Option<LoadedLogin>), String> {
    let mut next = state.clone();
    let namespace = format!("{:x}", Sha256::digest(path.as_os_str().as_encoded_bytes()));
    let account = |id: &str| format!("{namespace}:{id}");
    match action {
        Action::SaveBookmark {
            id,
            name,
            url,
            project,
        } => {
            let url = crate::browser::normalize_url(&url)?;
            origin(&url)?;
            let id = id.or_else(|| {
                next.bookmarks
                    .iter()
                    .find(|b| b.url == url && b.project == project)
                    .map(|b| b.id)
            });
            let id = match id {
                Some(id) if next.bookmarks.iter().any(|b| b.id == id) => id,
                Some(_) => return Err("Favorito não encontrado".into()),
                None => next
                    .bookmarks
                    .iter()
                    .map(|b| b.id)
                    .max()
                    .unwrap_or(0)
                    .checked_add(1)
                    .ok_or("IDs esgotados")?,
            };
            next.bookmarks.retain(|b| b.id != id);
            next.bookmarks.push(Bookmark {
                id,
                name,
                url,
                project,
            });
        }
        Action::DeleteBookmark(id) => next.bookmarks.retain(|b| b.id != id),
        Action::SaveLogin {
            origin: url,
            username,
            password,
        } => {
            let origin = origin(&url)?;
            if !text(&username, 1024)
                || password.expose().is_empty()
                || password.expose().len() > 16384
            {
                return Err("Informe usuário e senha válidos (até 16 KiB)".into());
            }
            let id = login_id(&origin, &username);
            next.logins.retain(|l| l.id != id);
            next.logins.push(Login {
                id: id.clone(),
                origin,
                username,
            });
            next.validate()?;
            if &read_file(path)? != expected {
                return Err("Biblioteca alterada por outra instância".into());
            }
            vault.set(&account(&id), password.expose())?;
        }
        Action::DeleteLogin(id) => {
            if !next.logins.iter().any(|l| l.id == id) {
                return Err("Acesso não encontrado".into());
            }
            vault.delete(&account(&id))?;
            next.logins.retain(|l| l.id != id);
        }
        Action::LoadLogin(id) => {
            let login = next
                .logins
                .iter()
                .find(|l| l.id == id)
                .cloned()
                .ok_or("Acesso não encontrado")?;
            let password = vault
                .get(&account(&id))?
                .ok_or("Senha não encontrada no cofre; salve novamente o acesso")?;
            return Ok((next, Some(LoadedLogin { login, password })));
        }
    }
    persist(path,&next,expected).map_err(|e|format!("{e}. Se a operação envolveu o cofre, ela pode ter concluído; confira e tente novamente."))?;
    *state = next.clone();
    Ok((next, None))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    #[derive(Default)]
    struct Memory(Mutex<BTreeMap<String, String>>);
    impl Vault for Memory {
        fn get(&self, a: &str) -> Result<Option<Secret>, String> {
            Ok(self.0.lock().unwrap().get(a).cloned().map(Secret::new))
        }
        fn set(&self, a: &str, p: &str) -> Result<(), String> {
            self.0.lock().unwrap().insert(a.into(), p.into());
            Ok(())
        }
        fn delete(&self, a: &str) -> Result<(), String> {
            self.0.lock().unwrap().remove(a);
            Ok(())
        }
    }
    fn settle(s: &mut BrowserLibrary) {
        let end = Instant::now() + Duration::from_secs(10);
        while s.is_busy() || s.is_running() {
            s.poll();
            assert!(Instant::now() < end, "{:?}", s.error);
            std::thread::sleep(Duration::from_millis(2));
        }
        s.poll();
    }
    fn send(s: &mut BrowserLibrary, a: Action) {
        s.dispatch(a).unwrap();
        settle(s);
        assert!(s.error.is_none(), "{:?}", s.error);
    }
    #[test]
    fn bookmarks_persist_edit_delete_and_conflicts_preserve_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profile.json");
        let vault = Arc::new(Memory::default());
        let mut a = BrowserLibrary::with_vault(path.clone(), vault.clone());
        settle(&mut a);
        send(
            &mut a,
            Action::SaveBookmark {
                id: None,
                name: "Frontend ação".into(),
                url: "localhost:5173".into(),
                project: "workspace A".into(),
            },
        );
        let id = a.state.bookmarks[0].id;
        assert_eq!(a.state.bookmarks[0].url, "http://localhost:5173/");
        send(
            &mut a,
            Action::SaveBookmark {
                id: None,
                name: "Mesmo frontend".into(),
                url: "http://localhost:5173/".into(),
                project: "workspace A".into(),
            },
        );
        assert_eq!(a.state.bookmarks.len(), 1);
        assert_eq!(a.state.bookmarks[0].id, id);
        assert_eq!(a.state.bookmarks[0].name, "Mesmo frontend");
        let mut b = BrowserLibrary::with_vault(path.clone(), vault);
        settle(&mut b);
        assert_eq!(b.state.bookmarks, a.state.bookmarks);
        send(
            &mut a,
            Action::SaveBookmark {
                id: Some(id),
                name: "Editado".into(),
                url: "https://example.invalid/path".into(),
                project: "workspace B".into(),
            },
        );
        let original = fs::read(&path).unwrap();
        b.dispatch(Action::DeleteBookmark(id)).unwrap();
        settle(&mut b);
        assert!(b.error.as_deref().unwrap().contains("outra instância"));
        assert_eq!(fs::read(&path).unwrap(), original);
        send(&mut a, Action::DeleteBookmark(id));
        assert!(a.state.bookmarks.is_empty());
        a.dispatch(Action::SaveBookmark {
            id: None,
            name: "Invalid".into(),
            url: "javascript:42".into(),
            project: String::new(),
        })
        .unwrap();
        settle(&mut a);
        assert!(a.error.is_some());
        assert!(a.state.bookmarks.is_empty());
    }
    #[test]
    fn saved_login_roundtrip_is_profile_and_origin_scoped_without_secret_in_json_or_debug() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profile.json");
        let vault = Arc::new(Memory::default());
        let secret = "owned-test-secret-unique";
        assert!(!format!("{:?}", Secret::new(secret.into())).contains(secret));
        let mut a = BrowserLibrary::with_vault(path.clone(), vault.clone());
        settle(&mut a);
        for origin in [
            "http://localhost:5173",
            "http://localhost:5174",
            "https://localhost:5173",
        ] {
            send(
                &mut a,
                Action::SaveLogin {
                    origin: origin.into(),
                    username: "fixture@example.invalid".into(),
                    password: Secret::new(secret.into()),
                },
            );
        }
        assert_eq!(a.state.logins.len(), 3);
        assert_eq!(vault.0.lock().unwrap().len(), 3);
        assert!(
            !String::from_utf8(fs::read(&path).unwrap())
                .unwrap()
                .contains(secret)
        );
        drop(a);
        let mut a = BrowserLibrary::with_vault(path, vault.clone());
        settle(&mut a);
        let id = a.state.logins[0].id.clone();
        send(&mut a, Action::LoadLogin(id.clone()));
        assert_eq!(a.take_login().unwrap().password.expose(), secret);
        let mut other = BrowserLibrary::with_vault(dir.path().join("other.json"), vault.clone());
        settle(&mut other);
        send(
            &mut other,
            Action::SaveLogin {
                origin: "http://localhost:5173".into(),
                username: "fixture@example.invalid".into(),
                password: Secret::new("other-profile".into()),
            },
        );
        assert_eq!(vault.0.lock().unwrap().len(), 4);
        send(&mut a, Action::DeleteLogin(id));
        assert_eq!(vault.0.lock().unwrap().len(), 3);
    }
    #[test]
    fn invalid_profile_and_locked_vault_never_fall_back_to_plaintext() {
        struct Locked;
        impl Vault for Locked {
            fn get(&self, _: &str) -> Result<Option<Secret>, String> {
                Err("Cofre bloqueado".into())
            }
            fn set(&self, _: &str, _: &str) -> Result<(), String> {
                Err("Cofre bloqueado".into())
            }
            fn delete(&self, _: &str) -> Result<(), String> {
                Err("Cofre bloqueado".into())
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profile.json");
        let mut s = BrowserLibrary::with_vault(path.clone(), Arc::new(Locked));
        settle(&mut s);
        s.dispatch(Action::SaveLogin {
            origin: "https://example.invalid".into(),
            username: "fixture".into(),
            password: Secret::new("never-on-disk".into()),
        })
        .unwrap();
        settle(&mut s);
        assert!(s.error.is_some());
        assert!(!path.exists());
        fs::write(&path, b"{broken").unwrap();
        let mut s = BrowserLibrary::with_vault(path.clone(), Arc::new(Memory::default()));
        settle(&mut s);
        assert!(s.error.is_some());
        s.dispatch(Action::DeleteBookmark(1)).unwrap();
        settle(&mut s);
        assert_eq!(fs::read(&path).unwrap(), b"{broken");
    }
    #[test]
    #[ignore = "explicit own system-vault entry; no personal entries enumerated"]
    fn own_os_vault_roundtrip_reopen_delete() {
        assert_eq!(
            std::env::var("CODISSEU_BROWSER_VAULT_TEST").as_deref(),
            Ok("1")
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profile.json");
        let mut s = BrowserLibrary::new(path.clone());
        settle(&mut s);
        send(
            &mut s,
            Action::SaveLogin {
                origin: "https://browser-vault-fixture.invalid".into(),
                username: "fixture-only".into(),
                password: Secret::new("own-vault-test-value".into()),
            },
        );
        let id = s.state.logins[0].id.clone();
        drop(s);
        let mut s = BrowserLibrary::new(path.clone());
        settle(&mut s);
        send(&mut s, Action::LoadLogin(id.clone()));
        assert_eq!(
            s.take_login().unwrap().password.expose(),
            "own-vault-test-value"
        );
        send(&mut s, Action::DeleteLogin(id));
        assert!(s.state.logins.is_empty());
        assert!(
            !fs::read_to_string(path)
                .unwrap()
                .contains("own-vault-test-value")
        );
    }
}

#[cfg(test)]
mod timeout_tests {
    use super::*;
    #[test]
    fn timeout_keeps_one_worker_and_reports_late_completion_without_plaintext() {
        struct Slow {
            gate: Arc<(Mutex<bool>, std::sync::Condvar)>,
            entered: Arc<AtomicBool>,
        }
        impl Vault for Slow {
            fn get(&self, _: &str) -> Result<Option<Secret>, String> {
                Ok(None)
            }
            fn set(&self, _: &str, _: &str) -> Result<(), String> {
                self.entered.store(true, Ordering::Release);
                let (lock, cv) = &*self.gate;
                let mut value = lock.lock().unwrap();
                while !*value {
                    value = cv.wait(value).unwrap();
                }
                Ok(())
            }
            fn delete(&self, _: &str) -> Result<(), String> {
                Ok(())
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.json");
        let gate = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
        let entered = Arc::new(AtomicBool::new(false));
        let mut s = BrowserLibrary::with_vault(
            path.clone(),
            Arc::new(Slow {
                gate: gate.clone(),
                entered: entered.clone(),
            }),
        );
        let end = Instant::now() + Duration::from_secs(3);
        while s.is_running() {
            s.poll();
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(2));
        }
        s.dispatch(Action::SaveLogin {
            origin: "https://example.invalid".into(),
            username: "fixture".into(),
            password: Secret::new("own-delayed-secret".into()),
        })
        .unwrap();
        while !entered.load(Ordering::Acquire) {
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(2));
        }
        s.pending = Some(Instant::now() - Duration::from_secs(6));
        s.poll();
        assert!(!s.is_busy());
        assert!(s.is_running());
        assert!(s.error.is_some());
        assert!(s.dispatch(Action::DeleteBookmark(1)).is_err());
        assert!(!path.exists());
        let (lock, cv) = &*gate;
        *lock.lock().unwrap() = true;
        cv.notify_one();
        while s.is_running() {
            s.poll();
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(2));
        }
        s.poll();
        assert_eq!(s.state.logins.len(), 1);
        assert!(
            !fs::read_to_string(path)
                .unwrap()
                .contains("own-delayed-secret")
        );
    }
}
