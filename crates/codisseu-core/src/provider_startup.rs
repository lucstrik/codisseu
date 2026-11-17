//! Profile-owned provider policy. Reading/writing a policy never executes a provider.
//! Only the application service decides when a trusted configuration may start.
use super::{AppService, Language, ProviderConfig};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, SyncSender},
};

const MAX_PROFILE_BYTES: usize = 128 * 1024;
const MAX_APPROVALS: usize = 128;

#[derive(Clone, Copy)]
pub(super) enum ProviderOperation {
    Starting,
    Stopping { through_file_job: u64 },
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ProviderStartupStatus {
    /// False for services created by tests/CLI until explicitly enabled by the desktop.
    pub configured: bool,
    pub enabled: bool,
    pub trusted: bool,
    pub needs_trust: bool,
    pub suspended: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Approval {
    workspace: PathBuf,
    configs: Vec<ProviderConfig>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Profile {
    schema_version: u32,
    automatic: bool,
    configs: Vec<ProviderConfig>,
    approvals: Vec<Approval>,
    #[serde(default)]
    project_configs: Vec<Approval>,
    #[serde(default)]
    project_approvals: Vec<Approval>,
}
impl Default for Profile {
    fn default() -> Self {
        Self {
            schema_version: 1,
            automatic: true,
            configs: Vec::new(),
            approvals: Vec::new(),
            project_configs: Vec::new(),
            project_approvals: Vec::new(),
        }
    }
}

enum Request {
    Load(Option<PathBuf>),
    Save(Profile),
}
enum Response {
    Loaded {
        path: PathBuf,
        profile: Profile,
        managed: Option<Vec<ProviderConfig>>,
    },
    Saved,
}
struct Worker {
    send: SyncSender<Request>,
    receive: Receiver<Result<Response, String>>,
    pending: usize,
}
impl Worker {
    fn new(path: PathBuf, fallback: Option<PathBuf>) -> Result<Self, String> {
        let (send, requests) = mpsc::sync_channel(8);
        let (replies, receive) = mpsc::sync_channel(8);
        std::thread::Builder::new()
            .name("codisseu-provider-policy".into())
            .spawn(move || {
                let mut path = path;
                while let Ok(request) = requests.recv() {
                    let result = match request {
                        Request::Load(fallback) => load(&path, fallback.as_deref()),
                        Request::Save(profile) => save(&path, &profile).map(|()| Response::Saved),
                    };
                    if let Ok(Response::Loaded { path: resolved, .. }) = &result {
                        path = resolved.clone();
                    }
                    if replies.send(result).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| format!("Não foi possível carregar a política de providers: {e}"))?;
        let mut worker = Self {
            send,
            receive,
            pending: 0,
        };
        worker.request(Request::Load(fallback))?;
        Ok(worker)
    }
    fn request(&mut self, request: Request) -> Result<(), String> {
        self.send
            .try_send(request)
            .map_err(|_| "Fila da política de providers cheia".to_owned())?;
        self.pending += 1;
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct AutomaticProviders {
    state: ProviderStartupStatus,
    worker: Option<Worker>,
    profile: Option<Profile>,
    profile_path: Option<PathBuf>,
    managed_configs: Option<Vec<ProviderConfig>>,
    key: Option<Approval>,
    attempted: bool,
}
impl AutomaticProviders {
    pub(super) fn is_busy(&self) -> bool {
        self.worker.as_ref().is_some_and(|w| w.pending > 0)
    }
    pub(super) fn opened_workspace(&mut self) {
        self.key = None;
        self.attempted = false;
        self.state.trusted = false;
        self.state.needs_trust = false;
        self.state.suspended = false;
    }
    pub(super) fn suspend(&mut self) {
        self.state.suspended = true;
        self.state.needs_trust = false;
    }
    fn valid_profile_location(&self, workspace: &Path) -> bool {
        self.profile_path
            .as_ref()
            .is_some_and(|path| !path.starts_with(workspace))
    }
    fn persist(&mut self) -> Result<(), String> {
        let profile = self
            .profile
            .clone()
            .ok_or("Política de providers ainda não carregada")?;
        self.worker
            .as_mut()
            .ok_or("Inicialização automática não configurada")?
            .request(Request::Save(profile))
    }
    fn refresh(&mut self, workspace: Option<&Path>, configs: &[ProviderConfig]) {
        let Some(workspace) = workspace else { return };
        if self.profile.is_none() {
            return;
        }
        if !self.valid_profile_location(workspace) {
            self.state.trusted = false;
            self.state.needs_trust = false;
            self.state.error = Some("O perfil de confiança deve ficar fora do projeto; escolha uma sessão em seu perfil de usuário ou em uma pasta temporária externa.".into());
            return;
        }
        let key = Approval {
            workspace: workspace.to_path_buf(),
            configs: configs.to_vec(),
        };
        if self.key.as_ref() != Some(&key) {
            self.attempted = false;
            self.key = Some(key.clone());
        }
        self.state.trusted = self
            .profile
            .as_ref()
            .is_some_and(|p| p.approvals.contains(&key));
        self.state.needs_trust = self.state.enabled
            && !self.state.trusted
            && !self.state.suspended
            && !configs.is_empty();
    }
    pub(super) fn authorize(&mut self, workspace: PathBuf, configs: Vec<ProviderConfig>) {
        self.state.suspended = false;
        self.attempted = true;
        if !self.state.configured {
            return;
        }
        if !self.valid_profile_location(&workspace) {
            self.state.error = Some("Providers iniciados por sua ação; confiança não salva porque o perfil fica dentro do projeto.".into());
            return;
        }
        let key = Approval { workspace, configs };
        let Some(profile) = &mut self.profile else {
            self.state.error = Some(
                "Providers iniciados por sua ação; política indisponível para guardar a confiança."
                    .into(),
            );
            return;
        };
        profile.approvals.retain(|a| a.workspace != key.workspace);
        if profile.approvals.len() >= MAX_APPROVALS {
            self.state.error =
                Some("Limite de 128 projetos confiáveis; confiança não salva.".into());
            return;
        }
        profile.configs = key.configs.clone();
        profile.approvals.push(key.clone());
        self.key = Some(key);
        self.state.trusted = true;
        self.state.needs_trust = false;
        self.state.error = self.persist().err();
    }
}

impl AppService {
    /// A typed pending operation owns its acknowledgement. The general status
    /// bar may be updated by a concurrent file reply without changing which
    /// provider operation is actually completing.
    pub(super) fn poll_provider_operation(&mut self) -> bool {
        let older_files_settled = match self.provider_operation {
            Some(ProviderOperation::Stopping { through_file_job }) => {
                !self.pending.keys().any(|id| *id <= through_file_job)
            }
            _ => true,
        };
        if !self.language.is_busy()
            && !self.project_language_busy()
            && older_files_settled
            && let Some(operation) = self.provider_operation.take()
        {
            self.status = match operation {
                ProviderOperation::Stopping { .. } if self.language.last_error.is_some() => {
                    "Falha ao parar providers; consulte o painel Problemas"
                }
                ProviderOperation::Stopping { .. } if self.providers_running() => {
                    "Providers ainda ativos; não foi possível concluir a parada"
                }
                ProviderOperation::Stopping { .. } => {
                    "Providers parados; análise pausada até iniciar ou reabrir"
                }
                ProviderOperation::Starting => self.provider_summary(),
            }
            .into();
            return true;
        }
        // Keep an active-provider report honest if a provider later exits.
        // A Save, rename or other foreground status keeps its own message.
        if self.provider_operation.is_none()
            && matches!(
                self.status.as_str(),
                "Providers ativos; diagnósticos no painel Problemas"
                    | "Providers parcialmente ativos; consulte o painel Problemas"
            )
        {
            let summary = self.provider_summary();
            if self.status != summary {
                self.status = summary.into();
                return true;
            }
        }
        false
    }
    fn provider_summary(&self) -> &'static str {
        let ready = self
            .language
            .statuses
            .iter()
            .any(|s| s.state == super::ProviderState::Ready);
        let failed = self
            .language
            .statuses
            .iter()
            .any(|s| s.state == super::ProviderState::Failed);
        let stopped = self
            .language
            .statuses
            .iter()
            .any(|s| s.state == super::ProviderState::Stopped);
        match (ready, failed, stopped) {
            (true, true, _) | (true, _, true) => {
                "Providers parcialmente ativos; consulte o painel Problemas"
            }
            (true, false, false) => "Providers ativos; diagnósticos no painel Problemas",
            (false, true, _) => "Falha ao iniciar providers; consulte o painel Problemas",
            _ => "Providers parados; análise pausada até iniciar ou reabrir",
        }
    }
    /// Overrides are stored outside source trees and are effective only for
    /// this canonical project root. Calling this method is an explicit action;
    /// restored configurations alone never grant execution permission.
    pub fn configure_project_providers(
        &mut self,
        root: PathBuf,
        configs: Vec<ProviderConfig>,
    ) -> Result<(), String> {
        validate_configs(&configs)?;
        if self.any_project_providers_running()
            || self.language.is_busy()
            || self.project_language_busy()
        {
            return Err(
                "Pare os providers de todos os projetos antes de alterar a configuração".into(),
            );
        }
        if self.workspace_root_for(&root).is_none() {
            return Err("O projeto deve pertencer ao workspace aberto".into());
        }
        if self.workspace.as_ref() != Some(&root)
            && !self.documents.iter().any(|d| d.project_root() == root)
        {
            return Err("Abra um arquivo do projeto antes de configurar seus providers".into());
        }
        let automatic = &mut self.automatic_providers;
        if !automatic.valid_profile_location(&root) {
            return Err("O perfil de providers deve ficar fora do workspace".into());
        }
        let previous_profile = automatic.profile.clone();
        let profile = automatic
            .profile
            .as_mut()
            .ok_or("Aguarde a política de providers carregar")?;
        if profile.project_configs.len() >= MAX_APPROVALS
            && !profile.project_configs.iter().any(|a| a.workspace == root)
        {
            return Err("Limite de 128 configurações de projetos".into());
        }
        let approval = Approval {
            workspace: root,
            configs,
        };
        if profile.project_approvals.len() >= MAX_APPROVALS
            && !profile
                .project_approvals
                .iter()
                .any(|a| a.workspace == approval.workspace)
        {
            return Err("Limite de 128 aprovações de providers por projeto".into());
        }
        profile
            .project_configs
            .retain(|a| a.workspace != approval.workspace);
        profile.project_configs.push(approval.clone());
        profile
            .project_approvals
            .retain(|a| a.workspace != approval.workspace);
        profile.project_approvals.push(approval);
        let result = serde_json::to_vec(profile)
            .map_err(|e| e.to_string())
            .and_then(|bytes| {
                if bytes.len() > MAX_PROFILE_BYTES {
                    Err("Perfil de providers excede 128 KiB".into())
                } else {
                    Ok(())
                }
            })
            .and_then(|()| automatic.persist());
        if result.is_err() {
            automatic.profile = previous_profile;
        }
        result
    }
    pub fn project_provider_configs(&self, root: &Path) -> Vec<ProviderConfig> {
        self.automatic_providers
            .profile
            .as_ref()
            .and_then(|p| p.project_configs.iter().find(|a| a.workspace == root))
            .map_or_else(|| self.provider_configs.clone(), |a| a.configs.clone())
    }
    pub(super) fn project_provider_configuration_authorized(
        &self,
        root: &Path,
        configs: &[ProviderConfig],
    ) -> bool {
        if self.workspace_root_for(root).is_none() {
            return false;
        }
        let override_config = self
            .automatic_providers
            .profile
            .as_ref()
            .and_then(|p| p.project_configs.iter().find(|a| a.workspace == root));
        override_config.is_none()
            || (self.automatic_providers.state.error.is_none()
                && self.automatic_providers.profile.as_ref().is_some_and(|p| {
                    p.project_approvals.contains(&Approval {
                        workspace: root.to_path_buf(),
                        configs: configs.to_vec(),
                    })
                }))
    }
    pub fn provider_startup(&self) -> &ProviderStartupStatus {
        &self.automatic_providers.state
    }
    pub(super) fn enable_provider_startup(
        &mut self,
        profile: PathBuf,
        fallback: Option<PathBuf>,
    ) -> Result<(), String> {
        if self.automatic_providers.state.configured {
            return Ok(());
        }
        self.automatic_providers.worker = Some(Worker::new(profile, fallback)?);
        self.automatic_providers.state.configured = true;
        Ok(())
    }
    pub(super) fn set_provider_automatic(&mut self, enabled: bool) -> Result<(), String> {
        let automatic = &mut self.automatic_providers;
        if let Some(workspace) = &self.workspace
            && !automatic.valid_profile_location(workspace)
        {
            return Err("O perfil de confiança deve ficar fora do projeto".into());
        }
        let profile = automatic
            .profile
            .as_mut()
            .ok_or("Aguarde a política de providers carregar")?;
        profile.automatic = enabled;
        automatic.state.enabled = enabled;
        automatic.state.needs_trust = false;
        automatic.state.suspended = false;
        automatic.attempted = false;
        automatic.persist()
    }
    pub(super) fn poll_provider_startup(&mut self) -> bool {
        let mut changed = false;
        let automatic = &mut self.automatic_providers;
        if let Some(worker) = &mut automatic.worker {
            while let Ok(response) = worker.receive.try_recv() {
                worker.pending = worker.pending.saturating_sub(1);
                changed = true;
                match response {
                    Ok(Response::Loaded {
                        path,
                        profile,
                        managed,
                    }) => {
                        automatic.managed_configs = managed;
                        let rust_upgrade = !self.provider_configs.is_empty()
                            && (legacy_managed_configs(&self.provider_configs)
                                || managed_without_python(
                                    &self.provider_configs,
                                    &profile.configs,
                                ))
                            && profile
                                .configs
                                .iter()
                                .any(|c| matches!(c.language, Language::Rust | Language::Python))
                            && profile
                                .configs
                                .iter()
                                .filter(|c| {
                                    self.provider_configs
                                        .iter()
                                        .any(|previous| previous.language == c.language)
                                })
                                .eq(self.provider_configs.iter());
                        if !self.provider_configs_explicit
                            && (self.provider_configs.is_empty() || rust_upgrade)
                        {
                            self.provider_configs = profile.configs.clone();
                        }
                        automatic.state.enabled = profile.automatic;
                        automatic.profile_path = Some(path);
                        automatic.profile = Some(profile);
                    }
                    Ok(Response::Saved) => automatic.state.error = None,
                    Err(error) => {
                        self.status = format!("Inicialização automática indisponível: {error}");
                        automatic.state.error = Some(error);
                    }
                }
            }
        }
        if !self.provider_configs_explicit
            && legacy_managed_configs(&self.provider_configs)
            && let Some(managed) = &automatic.managed_configs
            && managed != &self.provider_configs
        {
            self.provider_configs = managed.clone();
        }
        let before = automatic.state.clone();
        automatic.refresh(self.workspace.as_deref(), &self.provider_configs);
        changed |= before != automatic.state;
        let should_start = automatic.state.enabled
            && automatic.state.trusted
            && !automatic.state.suspended
            && !automatic.attempted
            && self.pending.is_empty()
            && !self.language.is_busy()
            && !self.providers_running();
        if should_start {
            self.automatic_providers.attempted = true;
            if let Err(error) = self.start_configured_providers() {
                self.automatic_providers.state.error = Some(error);
            }
            changed = true;
        }
        changed
    }
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > MAX_PROFILE_BYTES as u64
    {
        return Err(
            "Configuração de providers deve ser arquivo regular de até 128 KiB, sem link simbólico"
                .into(),
        );
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(MAX_PROFILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_PROFILE_BYTES {
        return Err("Configuração excede 128 KiB".into());
    }
    serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))
}
fn validate_configs(configs: &[ProviderConfig]) -> Result<(), String> {
    if configs.len() > 4 {
        return Err("Política excede quatro providers".into());
    }
    for (i, c) in configs.iter().enumerate() {
        if configs[..i].iter().any(|a| a.language == c.language)
            || c.args.len() > 64
            || c.args.iter().map(String::len).sum::<usize>() > 32768
            || c.program.as_os_str().len() > 4096
        {
            return Err("Configuração de provider inválida ou acima dos limites".into());
        }
    }
    Ok(())
}
fn bundled_configs() -> Result<Option<Vec<ProviderConfig>>, String> {
    let Some(resources) = codisseu_runtime::discover()? else {
        return Ok(None);
    };
    resources
        .providers()?
        .into_iter()
        .map(|command| {
            let language = match command.language.as_str() {
                "Php" => Language::Php,
                "TypeScript" => Language::TypeScript,
                "Rust" => Language::Rust,
                "Python" => Language::Python,
                _ => return Err("Linguagem empacotada inválida".into()),
            };
            Ok(ProviderConfig {
                language,
                program: command.program,
                args: command.args,
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}
fn managed_without_python(configs: &[ProviderConfig], managed: &[ProviderConfig]) -> bool {
    configs.len() == 3
        && !configs.iter().any(|c| c.language == Language::Python)
        && managed
            .iter()
            .filter(|c| c.language != Language::Python)
            .eq(configs.iter())
}
fn upgrade_managed(profile: &mut Profile, managed: &[ProviderConfig]) {
    if profile.configs.is_empty()
        || legacy_managed_configs(&profile.configs)
        || managed_without_python(&profile.configs, managed)
    {
        profile.configs = managed.to_vec();
    }
    for project in &mut profile.project_configs {
        if legacy_managed_configs(&project.configs)
            || managed_without_python(&project.configs, managed)
        {
            project.configs = managed.to_vec();
        }
    }
    // Approvals stay byte-for-byte command vectors; adding Python or changing
    // an executable never transfers another provider's prior authorization.
}
fn legacy_managed_configs(configs: &[ProviderConfig]) -> bool {
    matches!(configs.len(),2|3) && configs.iter().any(|c|c.language==Language::Php) && configs.iter().any(|c|c.language==Language::TypeScript)
    && configs.iter().all(|c| match c.language {
        Language::Php => c.program==Path::new("/usr/bin/php") && c.args.len()==5 && c.args[0]=="-d" && c.args[1]=="xdebug.mode=off"
            && c.args[2].ends_with("/.tools/phpactor-2026.10.07.0/bin/phpactor") && c.args[3] == "--config-extra={\"language_server_worse_reflection.workspace_index.update_interval\":0,\"worse_reflection.enable_cache\":false,\"indexer.exclude_patterns\":[\"/vendor/**/Tests/**/*\",\"/vendor/**/tests/**/*\",\"/vendor/composer/**/*\",\"/vendor/rector/rector/stubs-rector\",\"/.tools/**/*\",\"/target/**/*\",\"/artifacts/**/*\",\"/node_modules/**/*\",\"/storage/framework/**/*\",\"/bootstrap/cache/**/*\"]}" && c.args[4]=="language-server",
        Language::Rust => c.args.is_empty() && default_rust_provider().is_some_and(|p|p.program==c.program),
        Language::Python => false,
        Language::TypeScript => c.program.to_string_lossy().ends_with("/.tools/node-v24.22.0-linux-x64/bin/node") && c.args.len()==2 && c.args[0].ends_with("/.tools/node_modules/typescript-language-server/lib/cli.mjs") && c.args[1]=="--stdio",
    })
}

fn load(path: &Path, fallback: Option<&Path>) -> Result<Response, String> {
    load_with_managed(path, fallback, bundled_configs()?)
}
fn load_with_managed(
    path: &Path,
    fallback: Option<&Path>,
    managed: Option<Vec<ProviderConfig>>,
) -> Result<Response, String> {
    let mut profile: Profile = match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Profile::default(),
        _ => read_json(path)?,
    };
    if profile.schema_version != 1
        || profile.approvals.len() > MAX_APPROVALS
        || profile.project_configs.len() > MAX_APPROVALS
        || profile.project_approvals.len() > MAX_APPROVALS
    {
        return Err("Versão ou limite de projetos da política de providers inválido".into());
    }
    if let Some(managed) = &managed {
        upgrade_managed(&mut profile, managed);
    }
    validate_configs(&profile.configs)?;
    for approval in profile
        .approvals
        .iter()
        .chain(&profile.project_configs)
        .chain(&profile.project_approvals)
    {
        validate_configs(&approval.configs)?;
    }
    // An installation manifest owns its complete tool list, including an empty
    // core package. Development fallbacks must never escape that boundary.
    if managed.is_none()
        && profile.configs.is_empty()
        && let Some(fallback) = fallback
    {
        match fs::symlink_metadata(fallback) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            _ => {
                profile.configs = read_json(fallback)?;
                validate_configs(&profile.configs)?;
            }
        }
    }
    if managed.is_none() {
        if legacy_managed_configs(&profile.configs) || profile.configs.is_empty() {
            add_rust_provider(&mut profile.configs);
            add_python_provider(&mut profile.configs);
        }
        for project in &mut profile.project_configs {
            if legacy_managed_configs(&project.configs) {
                add_rust_provider(&mut project.configs);
                add_python_provider(&mut project.configs);
            }
        }
    }
    // Resolve ancestors without creating files in an untrusted workspace.
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    let mut ancestor = normalized.as_path();
    let mut suffix = Vec::new();
    while !ancestor.exists() {
        suffix.push(
            ancestor
                .file_name()
                .ok_or("Caminho de perfil inválido")?
                .to_os_string(),
        );
        ancestor = ancestor.parent().ok_or("Caminho de perfil inválido")?;
    }
    let mut resolved = fs::canonicalize(ancestor).map_err(|e| e.to_string())?;
    for name in suffix.into_iter().rev() {
        resolved.push(name);
    }
    Ok(Response::Loaded {
        path: resolved,
        profile,
        managed,
    })
}
fn save(path: &Path, profile: &Profile) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    if let Ok(metadata) = fs::symlink_metadata(path)
        && (!metadata.is_file() || metadata.file_type().is_symlink())
    {
        return Err("Perfil de providers não pode sobrescrever link ou arquivo especial".into());
    }
    let bytes = serde_json::to_vec_pretty(profile).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_PROFILE_BYTES {
        return Err("Perfil de providers excede 128 KiB".into());
    }
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    temporary.write_all(&bytes).map_err(|e| e.to_string())?;
    temporary.as_file().sync_all().map_err(|e| e.to_string())?;
    temporary.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}

fn default_rust_provider() -> Option<ProviderConfig> {
    let name = if cfg!(windows) {
        "rust-analyzer.exe"
    } else {
        "rust-analyzer"
    };
    let cargo_home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".cargo")));
    let candidate = cargo_home
        .map(|home| home.join("bin").join(name))
        .filter(|p| p.is_file())
        .or_else(|| {
            std::env::var_os("PATH").and_then(|path| {
                std::env::split_paths(&path)
                    .map(|p| p.join(name))
                    .find(|p| p.is_file())
            })
        });
    candidate.map(|program| ProviderConfig {
        language: Language::Rust,
        program,
        args: vec![],
    })
}
fn add_rust_provider(configs: &mut Vec<ProviderConfig>) {
    if !configs.iter().any(|c| c.language == Language::Rust)
        && let Some(provider) = default_rust_provider()
    {
        configs.push(provider);
    }
}
fn add_python_provider(configs: &mut Vec<ProviderConfig>) {
    if configs.iter().any(|c| c.language == Language::Python) {
        return;
    }
    // Only the explicitly recognized application-managed command set is extended.
    // No executable from a project folder or user PATH is guessed as Python LSP.
    let tools = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.tools");
    let node = tools.join("node-v24.22.0-linux-x64/bin/node");
    let script = tools.join("python/node_modules/basedpyright/langserver.index.js");
    if let (Ok(node), Ok(script)) = (node.canonicalize(), script.canonicalize())
        && node.is_file()
        && script.is_file()
    {
        configs.push(ProviderConfig {
            language: Language::Python,
            program: node,
            args: vec![script.to_string_lossy().into(), "--stdio".into()],
        });
    }
}

#[cfg(test)]
mod python_tests {
    use super::*;
    fn configs() -> Vec<ProviderConfig> {
        [
            Language::Php,
            Language::TypeScript,
            Language::Rust,
            Language::Python,
        ]
        .into_iter()
        .map(|language| ProviderConfig {
            language,
            program: PathBuf::from(format!("/application/{language:?}")),
            args: vec![],
        })
        .collect()
    }
    #[test]
    fn installed_manifest_is_authoritative_even_without_optional_languages() {
        let temp = tempfile::tempdir().unwrap();
        let profile_path = temp.path().join("profile.json");
        let fallback = temp.path().join("developer-provider.json");
        fs::write(&fallback, "invalid development fallback must not be read").unwrap();
        for managed in [vec![], configs()[..2].to_vec()] {
            let Response::Loaded {
                profile,
                managed: loaded,
                ..
            } = load_with_managed(&profile_path, Some(&fallback), Some(managed.clone())).unwrap()
            else {
                panic!("policy was not loaded");
            };
            assert_eq!(profile.configs, managed);
            assert_eq!(loaded, Some(managed));
            assert!(!profile_path.exists(), "discovery must not write a profile");
        }
        assert!(load_with_managed(&profile_path, Some(&fallback), None).is_err());
    }
    #[test]
    fn core_package_keeps_explicit_user_providers_and_exact_approval() {
        let temp = tempfile::tempdir().unwrap();
        let profile_path = temp.path().join("profile.json");
        let custom = vec![ProviderConfig {
            language: Language::Php,
            program: "/user-selected/phpactor".into(),
            args: vec!["language-server".into()],
        }];
        let approval = Approval {
            workspace: "/user-selected/project".into(),
            configs: custom.clone(),
        };
        let original = Profile {
            configs: custom.clone(),
            approvals: vec![approval.clone()],
            ..Default::default()
        };
        fs::write(&profile_path, serde_json::to_vec(&original).unwrap()).unwrap();
        let Response::Loaded { profile, .. } =
            load_with_managed(&profile_path, None, Some(vec![])).unwrap()
        else {
            panic!("policy was not loaded");
        };
        assert_eq!(profile.configs, custom);
        assert_eq!(profile.approvals, vec![approval]);
    }
    #[test]
    fn managed_python_upgrade_never_transfers_approval_or_overwrites_explicit_vectors() {
        let managed = configs();
        let old = managed[..3].to_vec();
        let approval = Approval {
            workspace: "/project".into(),
            configs: old.clone(),
        };
        let explicit = Approval {
            workspace: "/custom".into(),
            configs: vec![ProviderConfig {
                language: Language::Python,
                program: "/custom/basedpyright".into(),
                args: vec!["--stdio".into()],
            }],
        };
        let mut profile = Profile {
            configs: old.clone(),
            approvals: vec![approval.clone()],
            project_configs: vec![approval.clone(), explicit.clone()],
            project_approvals: vec![approval.clone()],
            ..Default::default()
        };
        upgrade_managed(&mut profile, &managed);
        assert_eq!(profile.configs, managed);
        assert_eq!(profile.project_configs[0].configs, managed);
        assert_eq!(profile.approvals[0], approval);
        assert_eq!(profile.project_approvals[0], approval);
        assert_eq!(profile.project_configs[1], explicit);
        let mut custom = Profile {
            configs: explicit.configs.clone(),
            ..Default::default()
        };
        upgrade_managed(&mut custom, &managed);
        assert_eq!(custom.configs, explicit.configs);
        assert!(validate_configs(&managed).is_ok());
        let mut duplicate = managed.clone();
        duplicate.push(managed[0].clone());
        assert!(validate_configs(&duplicate).is_err());
    }
}
