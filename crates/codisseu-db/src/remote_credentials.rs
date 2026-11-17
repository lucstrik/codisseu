//! OS credential storage. Never stores passwords in datasource/session JSON.
//!
//! One bounded worker isolates blocking platform keyring calls from both the DB
//! runtime and GUI. A timed-out OS operation may finish later; the busy gate stays
//! held until it actually completes, preventing accumulating blocked threads.
use crate::remote::ConnectionConfig;
use sha2::{Digest, Sha256};
use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender},
    },
    time::Duration,
};
use tokio::sync::oneshot;
use zeroize::Zeroizing;

const SERVICE: &str = "io.codisseu.Codisseu.database.v1";
const DEADLINE: Duration = Duration::from_secs(5);
const MAX_SECRET: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CredentialError {
    Unavailable,
    Locked,
    InvalidIdentity,
    TooLarge,
    Busy,
    TimedOut,
    Corrupt,
    Ambiguous,
}
impl fmt::Display for CredentialError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unavailable => "Cofre do sistema indisponível. Inicie o Secret Service/Keychain/Gerenciador de Credenciais e tente novamente; a senha não foi gravada em arquivo.",
            Self::Locked => "Cofre do sistema bloqueado ou acesso negado. Desbloqueie-o na sessão do desktop e tente novamente.",
            Self::InvalidIdentity => "Cadastro inválido para o cofre. Confira identificação, host, porta e usuário do datasource.",
            Self::TooLarge => "Senha excede o limite de 64 KiB do cofre do Codisseu.",
            Self::Busy => "O cofre ainda atende uma operação anterior. Aguarde sua conclusão ou desbloqueie-o no desktop antes de tentar novamente.",
            Self::TimedOut => "O cofre não respondeu em 5 segundos. A operação do sistema pode concluir depois; desbloqueie-o e confira o cadastro antes de tentar novamente.",
            Self::Corrupt => "A entrada do Codisseu no cofre tem formato inválido. Remova a senha lembrada deste datasource e informe-a novamente.",
            Self::Ambiguous => "Há entradas duplicadas deste datasource no cofre. Resolva a duplicidade no gerenciador de senhas do sistema antes de tentar novamente.",
        })
    }
}
impl std::error::Error for CredentialError {}

/// Explicit exposure only; Debug never reveals the credential.
pub struct CredentialSecret(Zeroizing<String>);
impl CredentialSecret {
    pub fn expose(&self) -> &str {
        self.0.as_str()
    }
}
impl fmt::Debug for CredentialSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CredentialSecret([redacted])")
    }
}

trait Backend: Send + Sync + 'static {
    fn load(&self, account: &str) -> Result<Option<String>, CredentialError>;
    fn store(&self, account: &str, password: &str) -> Result<(), CredentialError>;
    fn delete(&self, account: &str) -> Result<(), CredentialError>;
}
struct OsBackend;
impl Backend for OsBackend {
    fn load(&self, account: &str) -> Result<Option<String>, CredentialError> {
        match entry(account)?.get_password() {
            Ok(password) => Ok(Some(password)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(map_error(error)),
        }
    }
    fn store(&self, account: &str, password: &str) -> Result<(), CredentialError> {
        entry(account)?.set_password(password).map_err(map_error)
    }
    fn delete(&self, account: &str) -> Result<(), CredentialError> {
        match entry(account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(map_error(error)),
        }
    }
}
fn entry(account: &str) -> Result<keyring::Entry, CredentialError> {
    // Never accept keyring's mock fallback as persistent storage on other OSes.
    if !cfg!(any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows"
    )) {
        return Err(CredentialError::Unavailable);
    }
    keyring::Entry::new(SERVICE, account).map_err(map_error)
}
fn map_error(error: keyring::Error) -> CredentialError {
    match error {
        keyring::Error::NoStorageAccess(_) => CredentialError::Locked,
        keyring::Error::BadEncoding(_) => CredentialError::Corrupt,
        keyring::Error::TooLong(_, _) | keyring::Error::Invalid(_, _) => {
            CredentialError::InvalidIdentity
        }
        keyring::Error::Ambiguous(_) => CredentialError::Ambiguous,
        _ => CredentialError::Unavailable,
    }
}

/// Stable, non-secret key. Canonical tuple serialization avoids separator
/// collisions. Changing endpoint/user/database never loads an unrelated secret.
fn account(config: &ConnectionConfig) -> Result<String, CredentialError> {
    if config.id.is_empty()
        || config.host.is_empty()
        || config.port == 0
        || [
            &config.id,
            &config.host,
            &config.username,
            &config.database,
            &config.auth_database,
        ]
        .iter()
        .any(|field| field.len() > 4096 || field.contains('\0'))
    {
        return Err(CredentialError::InvalidIdentity);
    }
    let legacy_identity = (
        "codisseu-database-credential-v1",
        &config.id,
        config.engine,
        config.host.to_ascii_lowercase(),
        config.port,
        &config.username,
        &config.database,
        &config.auth_database,
    );
    let mongo_scope = config.engine == crate::remote::Engine::Mongo
        && (!config.mongo.replica_set.is_empty()
            || config.mongo.authentication != crate::remote::MongoAuthentication::Automatic);
    if mongo_scope {
        config
            .mongo
            .validate()
            .map_err(|_| CredentialError::InvalidIdentity)?;
    }
    let identity = if mongo_scope {
        serde_json::to_vec(&(
            legacy_identity,
            &config.mongo.replica_set,
            config.mongo.authentication,
        ))
    } else {
        serde_json::to_vec(&legacy_identity)
    }
    .map_err(|_| CredentialError::InvalidIdentity)?;
    Ok(format!("datasource-{:x}", Sha256::digest(identity)))
}

enum Operation {
    Load,
    Store(Zeroizing<String>),
    Delete,
}
struct Request {
    account: String,
    operation: Operation,
    reply: oneshot::Sender<Result<Option<CredentialSecret>, CredentialError>>,
}
pub struct CredentialStore {
    sender: Option<SyncSender<Request>>,
    busy: Arc<AtomicBool>,
    deadline: Duration,
}
impl Default for CredentialStore {
    fn default() -> Self {
        Self::new()
    }
}
impl CredentialStore {
    /// Instantiation does not access the keyring or load existing credentials.
    pub fn new() -> Self {
        Self::with_backend(Arc::new(OsBackend), DEADLINE)
    }
    fn with_backend(backend: Arc<dyn Backend>, deadline: Duration) -> Self {
        let (sender, receiver) = mpsc::sync_channel::<Request>(1);
        let busy = Arc::new(AtomicBool::new(false));
        let worker_busy = Arc::clone(&busy);
        let started = std::thread::Builder::new()
            .name("codisseu-credential-store".into())
            .spawn(move || {
                while let Ok(request) = receiver.recv() {
                    let result =
                        match request.operation {
                            Operation::Load => {
                                backend
                                    .load(&request.account)
                                    .and_then(|value| match value {
                                        Some(value) if value.len() > MAX_SECRET => {
                                            Err(CredentialError::TooLarge)
                                        }
                                        value => Ok(value
                                            .map(|value| CredentialSecret(Zeroizing::new(value)))),
                                    })
                            }
                            Operation::Store(password) => backend
                                .store(&request.account, password.as_str())
                                .map(|_| None),
                            Operation::Delete => backend.delete(&request.account).map(|_| None),
                        };
                    worker_busy.store(false, Ordering::SeqCst);
                    let _ = request.reply.send(result);
                }
            });
        Self {
            sender: started.ok().map(|_| sender),
            busy,
            deadline,
        }
    }
    pub async fn load(
        &self,
        config: &ConnectionConfig,
    ) -> Result<Option<CredentialSecret>, CredentialError> {
        self.request(config, Operation::Load).await
    }
    /// The application service decides explicit remember/Apply/authentication policy.
    pub async fn store(
        &self,
        config: &ConnectionConfig,
        password: String,
    ) -> Result<(), CredentialError> {
        if password.len() > MAX_SECRET {
            return Err(CredentialError::TooLarge);
        }
        self.request(config, Operation::Store(Zeroizing::new(password)))
            .await
            .map(|_| ())
    }
    pub async fn delete(&self, config: &ConnectionConfig) -> Result<(), CredentialError> {
        self.request(config, Operation::Delete).await.map(|_| ())
    }
    async fn request(
        &self,
        config: &ConnectionConfig,
        operation: Operation,
    ) -> Result<Option<CredentialSecret>, CredentialError> {
        let account = account(config)?;
        let sender = self.sender.as_ref().ok_or(CredentialError::Unavailable)?;
        if self
            .busy
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(CredentialError::Busy);
        }
        let (reply, receiver) = oneshot::channel();
        if sender
            .try_send(Request {
                account,
                operation,
                reply,
            })
            .is_err()
        {
            self.busy.store(false, Ordering::SeqCst);
            return Err(CredentialError::Unavailable);
        }
        match tokio::time::timeout(self.deadline, receiver).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => {
                self.busy.store(false, Ordering::SeqCst);
                Err(CredentialError::Unavailable)
            }
            Err(_) => Err(CredentialError::TimedOut),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::BTreeMap,
        sync::{Mutex, atomic::AtomicUsize},
    };
    #[derive(Default)]
    struct MemoryBackend {
        values: Mutex<BTreeMap<String, String>>,
        calls: AtomicUsize,
    }
    impl Backend for MemoryBackend {
        fn load(&self, account: &str) -> Result<Option<String>, CredentialError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.values.lock().unwrap().get(account).cloned())
        }
        fn store(&self, account: &str, password: &str) -> Result<(), CredentialError> {
            self.values
                .lock()
                .unwrap()
                .insert(account.into(), password.into());
            Ok(())
        }
        fn delete(&self, account: &str) -> Result<(), CredentialError> {
            self.values.lock().unwrap().remove(account);
            Ok(())
        }
    }
    fn config() -> ConnectionConfig {
        ConnectionConfig {
            id: "owned-test".into(),
            host: "fixture.invalid".into(),
            ..Default::default()
        }
    }
    fn run<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }
    #[test]
    fn memory_roundtrip_is_explicit_bounded_redacted_and_endpoint_scoped() {
        run(async {
            let backend = Arc::new(MemoryBackend::default());
            let store = CredentialStore::with_backend(backend.clone(), DEADLINE);
            let config = config();
            assert_eq!(
                backend.calls.load(Ordering::SeqCst),
                0,
                "construction must not read keyring"
            );
            assert!(store.load(&config).await.unwrap().is_none());
            store
                .store(&config, "synthetic secret".into())
                .await
                .unwrap();
            let secret = store.load(&config).await.unwrap().unwrap();
            assert_eq!(secret.expose(), "synthetic secret");
            assert!(!format!("{secret:?}").contains("synthetic secret"));
            let mut changed = config.clone();
            changed.port += 1;
            assert!(store.load(&changed).await.unwrap().is_none());
            changed = config.clone();
            changed.username.push('x');
            assert!(store.load(&changed).await.unwrap().is_none());
            changed = config.clone();
            changed.database.push('x');
            assert!(store.load(&changed).await.unwrap().is_none());
            changed = config.clone();
            changed.id.push('x');
            assert!(store.load(&changed).await.unwrap().is_none());
            store.delete(&config).await.unwrap();
            store.delete(&config).await.unwrap();
            assert!(store.load(&config).await.unwrap().is_none());
            assert_eq!(
                store
                    .store(&config, "x".repeat(MAX_SECRET + 1))
                    .await
                    .unwrap_err(),
                CredentialError::TooLarge
            );
        });
    }
    #[test]
    fn identity_tuple_has_no_separator_collision_and_no_secret_content() {
        let mut one = config();
        one.id = "a:b".into();
        one.username = "c".into();
        let mut two = config();
        two.id = "a".into();
        two.username = "b:c".into();
        assert_ne!(account(&one), account(&two));
        assert!(!account(&one).unwrap().contains("a:b"));
        one.id.clear();
        assert_eq!(account(&one), Err(CredentialError::InvalidIdentity));
    }
    #[test]
    fn mongo_credential_scope_preserves_legacy_and_distinguishes_set_and_mechanism() {
        let mut config = config();
        config.engine = crate::remote::Engine::Mongo;
        let legacy = account(&config).unwrap();
        config.mongo.read_preference = crate::remote::MongoReadPreference::SecondaryPreferred;
        config.mongo.direct_connection = false;
        assert_eq!(account(&config).unwrap(), legacy);
        config.mongo.replica_set = "fixture-rs".into();
        let set = account(&config).unwrap();
        assert_ne!(set, legacy);
        config.mongo.authentication = crate::remote::MongoAuthentication::ScramSha1;
        assert_ne!(account(&config).unwrap(), set);
    }
    struct SlowBackend;
    impl Backend for SlowBackend {
        fn load(&self, _: &str) -> Result<Option<String>, CredentialError> {
            std::thread::sleep(Duration::from_millis(100));
            Ok(None)
        }
        fn store(&self, _: &str, _: &str) -> Result<(), CredentialError> {
            Ok(())
        }
        fn delete(&self, _: &str) -> Result<(), CredentialError> {
            Ok(())
        }
    }
    #[test]
    fn timeout_retains_busy_gate_until_os_operation_actually_finishes() {
        run(async {
            let store =
                CredentialStore::with_backend(Arc::new(SlowBackend), Duration::from_millis(10));
            assert_eq!(
                store.load(&config()).await.unwrap_err(),
                CredentialError::TimedOut
            );
            assert_eq!(
                store.load(&config()).await.unwrap_err(),
                CredentialError::Busy
            );
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            while store.busy.load(Ordering::SeqCst) {
                assert!(std::time::Instant::now() < deadline);
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            store.delete(&config()).await.unwrap();
        });
    }
    struct FailedBackend(CredentialError);
    impl Backend for FailedBackend {
        fn load(&self, _: &str) -> Result<Option<String>, CredentialError> {
            Err(self.0)
        }
        fn store(&self, _: &str, _: &str) -> Result<(), CredentialError> {
            Err(self.0)
        }
        fn delete(&self, _: &str) -> Result<(), CredentialError> {
            Err(self.0)
        }
    }
    #[test]
    fn locked_or_unavailable_store_has_no_plaintext_or_memory_fallback() {
        run(async {
            for error in [CredentialError::Locked, CredentialError::Unavailable] {
                let store = CredentialStore::with_backend(Arc::new(FailedBackend(error)), DEADLINE);
                assert_eq!(
                    store
                        .store(&config(), "synthetic secret".into())
                        .await
                        .unwrap_err(),
                    error
                );
                assert_eq!(store.load(&config()).await.unwrap_err(), error);
                assert_eq!(store.delete(&config()).await.unwrap_err(), error);
            }
        });
    }
    #[test]
    fn platform_errors_are_sanitized_without_publishing_their_payload() {
        let raw = std::io::Error::other("synthetic secret must never escape");
        let error = map_error(keyring::Error::NoStorageAccess(Box::new(raw)));
        assert_eq!(error, CredentialError::Locked);
        assert!(!error.to_string().contains("synthetic secret"));
        assert!(!format!("{error:?}").contains("synthetic secret"));
    }
}
