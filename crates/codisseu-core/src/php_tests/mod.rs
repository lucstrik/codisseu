//! Explicit PHP test runs. Filesystem, reports and processes stay off the UI thread.
mod integration;
pub(crate) mod process;
mod report;
#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    time::Instant,
};

pub const MAX_CASES: usize = 10_000;
pub const MAX_OUTPUT: usize = 1024 * 1024;
pub const MAX_REPORT: u64 = 16 * 1024 * 1024;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Framework {
    #[default]
    Auto,
    PhpUnit,
    Pest,
}
impl Framework {
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Automático",
            Self::PhpUnit => "PHPUnit",
            Self::Pest => "Pest",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    pub root: PathBuf,
    pub framework: Framework,
    /// Empty selects the bundled PHP, then PHP from PATH in development.
    pub php: String,
    /// Empty selects vendor/bin/{pest,phpunit}.
    pub runner: String,
    /// Empty lets the framework discover phpunit.xml[.dist].
    pub xml: String,
    pub timeout_secs: u64,
}
impl Config {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            framework: Framework::Auto,
            php: String::new(),
            runner: String::new(),
            xml: String::new(),
            timeout_secs: 300,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Passed,
    Failed,
    Error,
    Skipped,
}
impl Outcome {
    pub fn label(self) -> &'static str {
        match self {
            Self::Passed => "Aprovado",
            Self::Failed => "Falha",
            Self::Error => "Erro",
            Self::Skipped => "Ignorado",
        }
    }
    pub fn is_failure(self) -> bool {
        matches!(self, Self::Failed | Self::Error)
    }
}
#[derive(Clone, Debug)]
pub struct TestCase {
    /// Explicit suite hierarchy and leaf title supplied by structured runners.
    pub ancestors: Vec<String>,
    pub title: String,
    pub name: String,
    pub class: String,
    pub path: Option<PathBuf>,
    pub line: Option<u32>,
    pub outcome: Outcome,
    pub seconds: f64,
    pub details: String,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum State {
    #[default]
    Idle,
    Preparing,
    Running,
    Passed,
    Failed,
    Cancelled,
    Error,
}
impl State {
    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "Nenhuma execução",
            Self::Preparing => "Configurando…",
            Self::Running => "Executando…",
            Self::Passed => "Testes aprovados",
            Self::Failed => "Execução com falhas",
            Self::Cancelled => "Execução cancelada",
            Self::Error => "Erro de execução",
        }
    }
}
#[derive(Clone, Debug, Default)]
pub struct RunReport {
    pub cases: Vec<TestCase>,
    pub output: String,
    pub output_truncated: bool,
    pub exit_codes: Vec<Option<i32>>,
    pub seconds: f64,
    pub error: Option<String>,
}
impl RunReport {
    pub fn counts(&self) -> [usize; 4] {
        let mut counts = [0; 4];
        for case in &self.cases {
            counts[match case.outcome {
                Outcome::Passed => 0,
                Outcome::Failed => 1,
                Outcome::Error => 2,
                Outcome::Skipped => 3,
            }] += 1;
        }
        counts
    }
}
#[derive(Clone, Debug)]
pub enum Scope {
    Suite,
    File(PathBuf),
}
#[derive(Debug)]
pub enum TestCommand {
    Inspect(PathBuf),
    Run { config: Config, scope: Scope },
    RunSelected(usize),
    RerunFailed,
    RerunLast,
    Cancel,
}
#[derive(Clone)]
struct Invocation {
    file: Option<PathBuf>,
    names: Vec<(String, String)>,
}
#[derive(Clone)]
struct LastRun {
    config: Config,
    invocations: Vec<Invocation>,
    cases: Vec<TestCase>,
}
enum Event {
    Config(Result<Config, String>),
    Started(String),
    Resolved(Config),
    Output(String),
    Finished(State, RunReport),
}
pub struct Service {
    pub config: Option<Config>,
    pub state: State,
    pub report: RunReport,
    pub command: String,
    pub revision: u64,
    receiver: Option<Receiver<Event>>,
    cancel: Arc<AtomicBool>,
    profile: PathBuf,
    last: Option<LastRun>,
    pending_run: Option<(Config, Vec<Invocation>)>,
}
impl Service {
    pub fn new(profile: PathBuf) -> Self {
        Self {
            config: None,
            state: State::Idle,
            report: RunReport::default(),
            command: String::new(),
            revision: 0,
            receiver: None,
            cancel: Arc::new(AtomicBool::new(false)),
            profile,
            last: None,
            pending_run: None,
        }
    }
    pub fn is_busy(&self) -> bool {
        self.receiver.is_some()
    }
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
    pub fn can_rerun_failed(&self) -> bool {
        !self.is_busy()
            && self.last.as_ref().is_some_and(|last| {
                last.cases
                    .iter()
                    .any(|c| c.outcome.is_failure() && c.path.is_some())
            })
    }
    pub fn last_root(&self) -> Option<&Path> {
        self.last.as_ref().map(|last| last.config.root.as_path())
    }
    pub fn dispatch(&mut self, command: TestCommand, allowed: Vec<PathBuf>) -> Result<(), String> {
        if matches!(command, TestCommand::Cancel) {
            self.cancel();
            return Ok(());
        }
        if self.is_busy() {
            return Err("Aguarde ou interrompa os testes PHP".into());
        }
        let (sender, receiver) = mpsc::sync_channel(32);
        let cancel = Arc::new(AtomicBool::new(false));
        let profile = self.profile.clone();
        match command {
            TestCommand::Inspect(root) => {
                if self.config.as_ref().is_none_or(|c| c.root != root) {
                    self.config = None;
                    self.report = RunReport::default();
                    self.command.clear();
                    self.last = None;
                }
                std::thread::Builder::new()
                    .name("codisseu-tests-config".into())
                    .spawn(move || {
                        let _ = sender.send(Event::Config(inspect(&profile, &root, &allowed)));
                    })
                    .map_err(|e| e.to_string())?;
                self.state = State::Preparing;
            }
            command => {
                let (config, invocations) = match command {
                    TestCommand::Run { config, scope } => (
                        config,
                        vec![Invocation {
                            file: match scope {
                                Scope::Suite => None,
                                Scope::File(path) => Some(path),
                            },
                            names: vec![],
                        }],
                    ),
                    TestCommand::RunSelected(index) => {
                        let last = self
                            .last
                            .as_ref()
                            .ok_or("Execute primeiro a suite ou o arquivo")?;
                        let case = last.cases.get(index).ok_or("Teste selecionado inválido")?;
                        let file = case.path.clone().ok_or(
                            "O relatório não informou um arquivo local válido para este teste",
                        )?;
                        (
                            last.config.clone(),
                            vec![Invocation {
                                file: Some(file),
                                names: vec![(case.class.clone(), case.name.clone())],
                            }],
                        )
                    }
                    TestCommand::RerunFailed => {
                        let last = self.last.as_ref().ok_or("Não há execução anterior")?;
                        let mut groups = BTreeMap::<PathBuf, Vec<(String, String)>>::new();
                        for case in last.cases.iter().filter(|c| c.outcome.is_failure()) {
                            groups.entry(case.path.clone().ok_or("Um teste falho não tem arquivo válido; execute a suite novamente")?).or_default().push((case.class.clone(), case.name.clone()));
                        }
                        if groups.is_empty() {
                            return Err("Não há testes falhos para reexecutar".into());
                        }
                        (
                            last.config.clone(),
                            groups
                                .into_iter()
                                .map(|(file, names)| Invocation {
                                    file: Some(file),
                                    names,
                                })
                                .collect(),
                        )
                    }
                    TestCommand::RerunLast => {
                        let last = self.last.as_ref().ok_or("Não há execução anterior")?;
                        (last.config.clone(), last.invocations.clone())
                    }
                    _ => unreachable!(),
                };
                self.config = Some(config.clone());
                self.pending_run = Some((config.clone(), invocations.clone()));
                let cancelled = cancel.clone();
                std::thread::Builder::new()
                    .name("codisseu-php-tests".into())
                    .spawn(move || {
                        let (state, report) = execute(
                            &profile,
                            &config,
                            &invocations,
                            &allowed,
                            &cancelled,
                            &sender,
                        );
                        let _ = sender.send(Event::Finished(state, report));
                    })
                    .map_err(|e| e.to_string())?;
                self.state = State::Running;
                self.report = RunReport::default();
                self.command.clear();
            }
        }
        self.receiver = Some(receiver);
        self.cancel = cancel;
        self.revision = self.revision.wrapping_add(1);
        Ok(())
    }
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        for _ in 0..64 {
            let Some(receiver) = self.receiver.as_ref() else {
                break;
            };
            let event = match receiver.try_recv() {
                Ok(event) => event,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.receiver = None;
                    self.state = State::Error;
                    self.last = None;
                    self.pending_run = None;
                    self.report.error = Some("Worker de testes encerrado sem resultado".into());
                    self.revision = self.revision.wrapping_add(1);
                    changed = true;
                    break;
                }
            };
            changed = true;
            match event {
                Event::Config(result) => {
                    self.receiver = None;
                    match result {
                        Ok(config) => {
                            self.config = Some(config);
                            self.state = State::Idle;
                        }
                        Err(e) => {
                            self.report.error = Some(e);
                            self.state = State::Error;
                        }
                    }
                    self.revision = self.revision.wrapping_add(1);
                }
                Event::Resolved(config) => {
                    if let Some((pending, _)) = &mut self.pending_run {
                        *pending = config;
                    }
                }
                Event::Started(command) => self.command = command,
                Event::Output(output) => self.report.output = output,
                Event::Finished(state, report) => {
                    self.state = state;
                    self.report = report;
                    self.receiver = None;
                    if let Some((config, invocations)) = self.pending_run.take() {
                        self.config = Some(config.clone());
                        self.last = Some(LastRun {
                            config,
                            invocations,
                            cases: self.report.cases.clone(),
                        });
                    }
                    self.revision = self.revision.wrapping_add(1);
                }
            }
        }
        changed
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        self.cancel();
    }
}
fn append_output(report: &mut RunReport, text: &str) {
    let available = MAX_OUTPUT.saturating_sub(report.output.len());
    let mut end = text.len().min(available);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    report.output.push_str(&text[..end]);
    report.output_truncated |= end < text.len();
}
fn root_allowed(root: &Path, allowed: &[PathBuf]) -> Result<PathBuf, String> {
    let root = root
        .canonicalize()
        .map_err(|e| format!("Raiz de testes indisponível: {e}"))?;
    if !root.is_dir()
        || !allowed
            .iter()
            .any(|p| p.canonicalize().is_ok_and(|p| root.starts_with(p)))
    {
        return Err("Raiz dos testes fora das pastas abertas".into());
    }
    Ok(root)
}
fn inspect(profile: &Path, root: &Path, allowed: &[PathBuf]) -> Result<Config, String> {
    let root = root_allowed(root, allowed)?;
    let mut config = Config::new(root.clone());
    if profile.exists() {
        let bytes = read_bounded(profile, 1024 * 1024)?;
        let profiles: Vec<Config> = serde_json::from_slice(&bytes)
            .map_err(|e| format!("Configurações de testes inválidas: {e}"))?;
        if let Some(saved) = profiles.into_iter().find(|c| c.root == root) {
            config = saved;
        }
    }
    Ok(config)
}
fn save_config(profile: &Path, config: &Config) -> Result<(), String> {
    let mut profiles: Vec<Config> = if profile.exists() {
        serde_json::from_slice(&read_bounded(profile, 1024 * 1024)?)
            .map_err(|e| format!("Configurações de testes inválidas: {e}"))?
    } else {
        vec![]
    };
    profiles.retain(|c| c.root != config.root);
    if profiles.len() >= 128 {
        return Err("Limite de128 configurações de testes".into());
    }
    profiles.push(config.clone());
    let parent = profile.parent().ok_or("Perfil de testes sem pasta")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    use std::io::Write;
    serde_json::to_writer(&mut temp, &profiles).map_err(|e| e.to_string())?;
    temp.flush().map_err(|e| e.to_string())?;
    temp.as_file().sync_all().map_err(|e| e.to_string())?;
    temp.persist(profile).map_err(|e| e.to_string())?;
    Ok(())
}
fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut bytes = vec![];
    std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("Arquivo de testes excede o limite".into());
    }
    Ok(bytes)
}
fn local_file(root: &Path, path: &Path) -> Result<PathBuf, String> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let path = path
        .canonicalize()
        .map_err(|e| format!("Arquivo de testes indisponível: {e}"))?;
    if !path.starts_with(root) || !path.is_file() {
        return Err("Arquivo de testes fora do projeto".into());
    }
    Ok(path)
}
fn literal_filter(names: &[(String, String)], framework: Framework) -> Result<String, String> {
    if names.len() > MAX_CASES
        || names.iter().any(|(class, name)| {
            [class, name]
                .iter()
                .any(|s| s.len() > 4096 || s.contains(['\0', '\n', '\r']))
        })
    {
        return Err("Filtro de testes excede os limites".into());
    }
    let escape = |s: &str| {
        let mut out = String::new();
        for c in s.chars() {
            if "\\.^$|?*+()[]{}~".contains(c) {
                out.push('\\');
            }
            out.push(c);
        }
        out
    };
    let alternatives = names
        .iter()
        .map(|(class, name)| {
            if framework == Framework::Pest {
                escape(name)
            } else {
                escape(&format!("{class}::{name}"))
            }
        })
        .collect::<Vec<_>>()
        .join("|");
    let filter = if framework == Framework::Pest {
        format!("~::(?:{alternatives})$~u")
    } else {
        format!("~^(?:{alternatives})$~u")
    };
    if filter.len() > 64 * 1024 {
        return Err("Filtro de falhos excede64KiB; execute por arquivo".into());
    }
    Ok(filter)
}

fn execute(
    profile: &Path,
    config: &Config,
    invocations: &[Invocation],
    allowed: &[PathBuf],
    cancel: &AtomicBool,
    sender: &SyncSender<Event>,
) -> (State, RunReport) {
    let started = Instant::now();
    let mut report = RunReport::default();
    let result = (|| {
        let mut config = config.clone();
        config.root = root_allowed(&config.root, allowed)?;
        if !(1..=3600).contains(&config.timeout_secs) {
            return Err("Prazo de testes deve estar entre1 e3600segundos".into());
        }
        for value in [&config.php, &config.runner, &config.xml] {
            if value.len() > 4096 || value.contains(['\0', '\n', '\r']) {
                return Err("Caminho de testes inválido".into());
            }
        }
        let framework = match config.framework {
            Framework::Auto => {
                if config.root.join("vendor/bin/pest").is_file() {
                    Framework::Pest
                } else {
                    Framework::PhpUnit
                }
            }
            value => value,
        };
        let runner = local_file(
            &config.root,
            Path::new(if config.runner.is_empty() {
                if framework == Framework::Pest {
                    "vendor/bin/pest"
                } else {
                    "vendor/bin/phpunit"
                }
            } else {
                &config.runner
            }),
        )?;
        let php = if config.php.is_empty() {
            codisseu_runtime::discover()?
                .map(|r| r.tool("php"))
                .transpose()?
                .unwrap_or_else(|| "php".into())
        } else {
            PathBuf::from(&config.php)
        };
        let xml = if config.xml.is_empty() {
            None
        } else {
            Some(local_file(&config.root, Path::new(&config.xml))?)
        };
        // Validate every rerun target before executing any part of the batch.
        let invocations = invocations
            .iter()
            .map(|i| {
                Ok(Invocation {
                    file: i
                        .file
                        .as_ref()
                        .map(|f| local_file(&config.root, f))
                        .transpose()?,
                    names: i.names.clone(),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        save_config(profile, &config)?;
        let mut resolved = config.clone();
        resolved.framework = framework;
        resolved.runner = runner.to_string_lossy().into_owned();
        resolved.php = php.to_string_lossy().into_owned();
        resolved.xml = xml
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let _ = sender.send(Event::Resolved(resolved));
        let directory = tempfile::Builder::new()
            .prefix("codisseu-php-tests-")
            .tempdir()
            .map_err(|e| e.to_string())?;
        for (index, invocation) in invocations.iter().enumerate() {
            if cancel.load(Ordering::SeqCst) {
                return Ok(State::Cancelled);
            }
            let junit = directory.path().join(format!("{index}.xml"));
            let mut args = vec![
                runner.to_string_lossy().into_owned(),
                "--colors=never".into(),
            ];
            if let Some(xml) = &xml {
                args.extend(["--configuration".into(), xml.to_string_lossy().into_owned()]);
            }
            args.extend(["--log-junit".into(), junit.to_string_lossy().into_owned()]);
            if !invocation.names.is_empty() {
                args.extend([
                    "--filter".into(),
                    literal_filter(&invocation.names, framework)?,
                ]);
            }
            if let Some(file) = &invocation.file {
                args.push(file.to_string_lossy().into_owned());
            }
            let display = format!(
                "{} {}",
                php.display(),
                args.iter()
                    .map(|s| format!("{s:?}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            );
            let _ = sender.try_send(Event::Started(display.clone()));
            append_output(&mut report, &format!("\n{display}\n"));
            let output = process::run(
                &php,
                &args,
                &config.root,
                std::time::Duration::from_secs(config.timeout_secs)
                    .saturating_sub(started.elapsed()),
                cancel,
                sender,
            )?;
            append_output(&mut report, &output.text);
            report.output_truncated |= output.truncated;
            report.exit_codes.push(output.code);
            if output.cancelled {
                return Ok(State::Cancelled);
            }
            if output.timeout {
                return Err("Execução interrompida: prazo dos testes excedido".into());
            }
            let bytes = read_bounded(&junit, MAX_REPORT).map_err(|e| {
                format!(
                    "Runner terminou com código {:?}, sem relatório JUnit válido: {e}",
                    output.code
                )
            })?;
            let cases = report::parse(&bytes, &config.root)?;
            if cases.is_empty() {
                return Err(
                    "Nenhum teste executado; confira configuração, arquivo e filtro".into(),
                );
            }
            if report.cases.len() + cases.len() > MAX_CASES {
                return Err("Relatório excede10000 testes".into());
            }
            let failed = cases.iter().any(|c| c.outcome.is_failure());
            report.cases.extend(cases);
            if !output.success && !failed {
                return Err(format!(
                    "Runner terminou com código {:?}; confira avisos e console",
                    output.code
                ));
            }
        }
        Ok(if report.cases.iter().any(|c| c.outcome.is_failure()) {
            State::Failed
        } else {
            State::Passed
        })
    })();
    report.seconds = started.elapsed().as_secs_f64();
    match result {
        Ok(state) => (state, report),
        Err(error) => {
            report.error = Some(error);
            (
                if cancel.load(Ordering::SeqCst) {
                    State::Cancelled
                } else {
                    State::Error
                },
                report,
            )
        }
    }
}
