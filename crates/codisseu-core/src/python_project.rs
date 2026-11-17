//! Versioned Python project configuration and shell-free plans; all filesystem work is off GUI.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub root: PathBuf,
    /// Empty selects .venv/venv, then the installation-owned interpreter.
    pub interpreter: String,
    pub cwd: String,
    pub module: String,
    pub arguments: Vec<String>,
    pub environment: BTreeMap<String, String>,
    pub venv: String,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            root: PathBuf::new(),
            interpreter: String::new(),
            cwd: String::new(),
            module: String::new(),
            arguments: vec![],
            environment: BTreeMap::new(),
            venv: ".venv".into(),
        }
    }
}
impl Config {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            ..Default::default()
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Action {
    #[default]
    RunFile,
    RunModule,
    Repl,
    Check,
    Lint,
    CreateVenv,
    InstallPackages,
    InstallRequirements,
}
impl Action {
    pub const ALL: [Self; 8] = [
        Self::RunFile,
        Self::RunModule,
        Self::Repl,
        Self::Check,
        Self::Lint,
        Self::CreateVenv,
        Self::InstallPackages,
        Self::InstallRequirements,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::RunFile => "Executar arquivo",
            Self::RunModule => "Executar módulo",
            Self::Repl => "Console Python",
            Self::Check => "Verificar sintaxe",
            Self::Lint => "Inspecionar com Ruff",
            Self::CreateVenv => "Criar ambiente virtual",
            Self::InstallPackages => "Instalar pacotes no ambiente",
            Self::InstallRequirements => "Instalar requirements",
        }
    }
    pub fn executes_project(self) -> bool {
        matches!(
            self,
            Self::RunFile | Self::RunModule | Self::Repl | Self::Check | Self::Lint
        )
    }
}
#[derive(Clone, Debug)]
pub struct Plan {
    pub root: PathBuf,
    pub program: PathBuf,
    pub cwd: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, Option<String>)>,
    pub action: Action,
}
#[derive(Clone, Debug)]
pub enum ProjectCommand {
    Inspect(PathBuf),
    Save(Config),
    Prepare {
        config: Config,
        action: Action,
        path: Option<PathBuf>,
        values: Vec<String>,
    },
    Cancel,
}
struct ResultEvent {
    config: Config,
    interpreter: PathBuf,
    plan: Option<Plan>,
}
pub struct Service {
    pub config: Option<Config>,
    pub interpreter: Option<PathBuf>,
    pub plan: Option<Plan>,
    pub error: Option<String>,
    pub revision: u64,
    profile: PathBuf,
    receiver: Option<Receiver<Result<ResultEvent, String>>>,
}
impl Service {
    pub fn new(profile: PathBuf) -> Self {
        Self {
            config: None,
            interpreter: None,
            plan: None,
            error: None,
            revision: 0,
            profile,
            receiver: None,
        }
    }
    pub fn is_busy(&self) -> bool {
        self.receiver.is_some()
    }
    pub fn cancel(&mut self) {
        self.receiver = None;
        self.plan = None;
        self.revision = self.revision.wrapping_add(1);
    }
    pub fn dispatch(&mut self, command: ProjectCommand, roots: &[PathBuf]) -> Result<(), String> {
        if matches!(command, ProjectCommand::Cancel) {
            self.cancel();
            return Ok(());
        }
        if self.is_busy() {
            return Err("Aguarde a preparação Python".into());
        }
        let root = match &command {
            ProjectCommand::Inspect(root) => root,
            _ => match &command {
                ProjectCommand::Save(c) | ProjectCommand::Prepare { config: c, .. } => &c.root,
                _ => unreachable!(),
            },
        };
        if !roots.iter().any(|r| root.starts_with(r)) {
            return Err("Abra o projeto Python desta configuração".into());
        }
        let profile = self.profile.clone();
        let (tx, rx) = mpsc::sync_channel(1);
        self.error = None;
        self.plan = None;
        std::thread::Builder::new()
            .name("codisseu-python-project".into())
            .spawn(move || {
                let result = (|| {
                    let (config, action, path, values, save) = match command {
                        ProjectCommand::Inspect(root) => {
                            (load(&profile, &root)?, None, None, vec![], false)
                        }
                        ProjectCommand::Save(config) => (config, None, None, vec![], true),
                        ProjectCommand::Prepare {
                            config,
                            action,
                            path,
                            values,
                        } => (config, Some(action), path, values, true),
                        ProjectCommand::Cancel => unreachable!(),
                    };
                    let interpreter = resolve_interpreter(&config)?;
                    let plan = action
                        .map(|action| {
                            plan(
                                &config,
                                interpreter.clone(),
                                action,
                                path.as_deref(),
                                &values,
                            )
                        })
                        .transpose()?;
                    if save {
                        persist(&profile, &config)?;
                    }
                    Ok(ResultEvent {
                        config,
                        interpreter,
                        plan,
                    })
                })();
                let _ = tx.send(result);
            })
            .map_err(|e| e.to_string())?;
        self.receiver = Some(rx);
        Ok(())
    }
    pub fn poll(&mut self) -> bool {
        let result = self.receiver.as_ref().and_then(|rx| match rx.try_recv() {
            Ok(v) => Some(v),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(_) => Some(Err("Preparação Python interrompida".into())),
        });
        let Some(result) = result else {
            return false;
        };
        self.receiver = None;
        match result {
            Ok(result) => {
                self.config = Some(result.config);
                self.interpreter = Some(result.interpreter);
                self.plan = result.plan;
                self.error = None;
            }
            Err(e) => self.error = Some(e),
        }
        self.revision = self.revision.wrapping_add(1);
        true
    }
}
fn config_map(profile: &Path) -> Result<BTreeMap<String, Config>, String> {
    match std::fs::symlink_metadata(profile) {
        Ok(m) if !m.is_file() || m.file_type().is_symlink() || m.len() > 1024 * 1024 => {
            Err("Perfil Python inválido ou excessivo".into())
        }
        Ok(_) => serde_json::from_slice(&std::fs::read(profile).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(e) => Err(e.to_string()),
    }
}
fn load(profile: &Path, root: &Path) -> Result<Config, String> {
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    Ok(config_map(profile)?
        .remove(&root.to_string_lossy().into_owned())
        .unwrap_or_else(|| Config::new(root)))
}
fn persist(profile: &Path, config: &Config) -> Result<(), String> {
    let mut configs = config_map(profile)?;
    if configs.len() >= 64 && !configs.contains_key(&config.root.to_string_lossy().into_owned()) {
        return Err("Limite de64projetos Python neste perfil".into());
    }
    configs.insert(config.root.to_string_lossy().into_owned(), config.clone());
    let bytes = serde_json::to_vec_pretty(&configs).map_err(|e| e.to_string())?;
    if bytes.len() > 1024 * 1024 {
        return Err("Perfil Python excede1MiB".into());
    }
    let parent = profile.parent().ok_or("Perfil Python sem pasta")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut stage = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    use std::io::Write;
    stage.write_all(&bytes).map_err(|e| e.to_string())?;
    stage.persist(profile).map_err(|e| e.to_string())?;
    Ok(())
}
pub fn resolve_interpreter(config: &Config) -> Result<PathBuf, String> {
    if !config.root.is_absolute()
        || config.root.canonicalize().map_err(|e| e.to_string())? != config.root
    {
        return Err("Raiz Python deve ser uma pasta canônica".into());
    }
    if !config.interpreter.is_empty() {
        if config.interpreter.len() > 4096 || config.interpreter.contains(['\0', '\n', '\r']) {
            return Err("Interpretador Python inválido".into());
        }
        let declared = Path::new(&config.interpreter);
        let path = if declared.is_absolute() {
            declared.to_path_buf()
        } else if declared.components().count() > 1 {
            config.root.join(declared)
        } else {
            find_path(&config.interpreter).ok_or("Interpretador Python não encontrado no PATH")?
        };
        executable(&path)?;
        return Ok(path);
    }
    for relative in [
        ".venv/bin/python",
        "venv/bin/python",
        ".venv/Scripts/python.exe",
        "venv/Scripts/python.exe",
    ] {
        let p = config.root.join(relative);
        if p.is_file() {
            executable(&p)?;
            return Ok(p);
        }
    }
    let p = codisseu_runtime::python()?;
    executable(&p)?;
    Ok(p)
}
fn find_path(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|p| p.join(name))
        .find(|p| p.is_absolute() && p.is_file())
}
fn executable(path: &Path) -> Result<(), String> {
    if !path.is_absolute() || !path.is_file() {
        return Err("Selecione um executável Python existente".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if std::fs::metadata(path)
            .map_err(|e| e.to_string())?
            .permissions()
            .mode()
            & 0o111
            == 0
        {
            return Err("Interpretador Python sem permissão de execução".into());
        }
    }
    Ok(())
}
pub fn is_venv(interpreter: &Path) -> bool {
    interpreter
        .parent()
        .and_then(Path::parent)
        .is_some_and(|root| root.join("pyvenv.cfg").is_file())
}
fn local_file(root: &Path, path: &Path) -> Result<PathBuf, String> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let resolved = path.canonicalize().map_err(|e| e.to_string())?;
    if !resolved.starts_with(root) || !resolved.is_file() {
        return Err("Arquivo Python fora do projeto".into());
    }
    Ok(resolved)
}
pub fn plan(
    config: &Config,
    interpreter: PathBuf,
    action: Action,
    path: Option<&Path>,
    values: &[String],
) -> Result<Plan, String> {
    executable(&interpreter)?;
    if config.arguments.len() > 128
        || config
            .arguments
            .iter()
            .any(|v| v.len() > 8192 || v.contains('\0'))
        || config.environment.len() > 64
    {
        return Err("Argumentos/ambiente Python excedem limites".into());
    }
    let cwd = if config.cwd.is_empty() {
        config.root.clone()
    } else {
        let p = Path::new(&config.cwd);
        if p.is_absolute() {
            p.into()
        } else {
            config.root.join(p)
        }
    }
    .canonicalize()
    .map_err(|e| e.to_string())?;
    if !cwd.starts_with(&config.root) || !cwd.is_dir() {
        return Err("Diretório de execução fora do projeto".into());
    }
    let mut args = Vec::new();
    let mut program = interpreter.clone();
    match action {
        Action::RunFile | Action::Check => {
            let p = local_file(&config.root, path.ok_or("Abra um arquivo Python")?)?;
            if p.extension().is_none_or(|e| e != "py" && e != "pyw") {
                return Err(
                    "Execução Python exige fonte .py ou .pyw; .pyi é somente análise".into(),
                );
            }
            if action == Action::Check {
                args.extend(["-I".into(),"-S".into(),"-c".into(),"import ast,sys,tokenize; p=sys.argv[1]; f=tokenize.open(p); ast.parse(f.read(),filename=p); f.close(); print('Sintaxe Python válida')".into()]);
            }
            args.push(p.display().to_string());
            if action == Action::RunFile {
                args.extend(config.arguments.clone());
            }
        }
        Action::Lint => {
            let p = local_file(&config.root, path.ok_or("Abra um arquivo Python")?)?;
            if !matches!(
                p.extension().and_then(|e| e.to_str()),
                Some("py" | "pyi" | "pyw")
            ) {
                return Err("Inspeção Ruff exige fonte Python .py/.pyi/.pyw".into());
            }
            program = codisseu_runtime::ruff()?;
            args.extend(["check".into(), p.display().to_string()]);
        }
        Action::RunModule => {
            if config.module.is_empty()
                || config.module.len() > 512
                || !config.module.split('.').all(|s| {
                    !s.is_empty()
                        && s.chars().all(|c| c.is_alphanumeric() || c == '_')
                        && !s.starts_with(|c: char| c.is_ascii_digit())
                })
            {
                return Err("Informe um módulo Python válido, como pacote.main".into());
            }
            args.extend(["-m".into(), config.module.clone()]);
            args.extend(config.arguments.clone());
        }
        Action::Repl => {
            args.push("-i".into());
        }
        Action::CreateVenv => {
            if config.venv.is_empty()
                || config.venv.len() > 128
                || config.venv == "."
                || config.venv == ".."
                || !config
                    .venv
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
            {
                return Err("Nome do ambiente deve ser uma pasta simples do projeto".into());
            }
            let target = config.root.join(&config.venv);
            if target.exists() || target.is_symlink() {
                return Err("Ambiente já existe; nada foi sobrescrito".into());
            }
            args.extend(["-m".into(), "venv".into(), target.display().to_string()]);
        }
        Action::InstallPackages | Action::InstallRequirements => {
            if !is_venv(&interpreter) {
                return Err("Selecione um ambiente virtual antes de instalar pacotes; Python global não será alterado".into());
            }
            args.extend(["-m".into(), "pip".into(), "install".into()]);
            if action == Action::InstallRequirements {
                let p = local_file(
                    &config.root,
                    Path::new(values.first().ok_or("Informe requirements.txt")?),
                )?;
                args.extend(["--requirement".into(), p.display().to_string()]);
            } else {
                if values.is_empty()
                    || values.len() > 64
                    || values.iter().any(|v| {
                        v.is_empty()
                            || v.starts_with('-')
                            || v.len() > 2048
                            || v.contains(['\0', '\n', '\r'])
                    })
                {
                    return Err(
                        "Informe um pacote/requisito por linha (sem opções de comando)".into(),
                    );
                }
                args.push("--".into());
                args.extend(values.iter().cloned());
            }
        }
    }
    let mut env = vec![("PYTHONUNBUFFERED".into(), Some("1".into()))];
    if matches!(
        action,
        Action::CreateVenv | Action::InstallPackages | Action::InstallRequirements
    ) {
        env.push(("PYTHONNOUSERSITE".into(), Some("1".into())));
        for name in [
            "PYTHONHOME",
            "PYTHONPATH",
            "PYTHONUSERBASE",
            "PYTHONSTARTUP",
            "PIP_TARGET",
            "PIP_PREFIX",
            "PIP_ROOT",
            "PIP_PYTHON",
            "PIP_USER",
            "PIP_REQUIRE_VIRTUALENV",
        ] {
            env.push((name.into(), None));
        }
        env.push((
            "PIP_CONFIG_FILE".into(),
            Some(if cfg!(windows) { "NUL" } else { "/dev/null" }.into()),
        ));
    }
    for (k, v) in &config.environment {
        if k.is_empty()
            || k.len() > 128
            || k.contains(['=', '\0', '\n', '\r'])
            || v.len() > 8192
            || v.contains('\0')
        {
            return Err("Variável de ambiente Python inválida".into());
        }
        if matches!(
            action,
            Action::InstallPackages | Action::InstallRequirements
        ) && matches!(
            k.as_str(),
            "PIP_TARGET"
                | "PIP_PREFIX"
                | "PIP_ROOT"
                | "PIP_PYTHON"
                | "PIP_USER"
                | "PIP_CONFIG_FILE"
                | "PYTHONHOME"
                | "PYTHONPATH"
        ) {
            return Err("Esta ação instala somente no ambiente selecionado; remova o redirecionamento pip/Python da configuração".into());
        }
        env.push((k.clone(), Some(v.clone())));
    }
    Ok(Plan {
        root: config.root.clone(),
        program,
        cwd,
        args,
        env,
        action,
    })
}
