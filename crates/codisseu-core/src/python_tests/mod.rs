//! Explicit Python test runs through a private structured helper and shared process limits.
use crate::php_tests::{MAX_CASES, MAX_REPORT, process};
pub use crate::php_tests::{Outcome, RunReport, Scope, State, TestCase};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Framework {
    #[default]
    Auto,
    Pytest,
    Unittest,
}
impl Framework {
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Automático",
            Self::Pytest => "pytest",
            Self::Unittest => "unittest",
        }
    }
    fn argument(self) -> &'static str {
        match self {
            Self::Unittest => "unittest",
            _ => "pytest",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub root: PathBuf,
    pub framework: Framework,
    pub interpreter: String,
    pub timeout_secs: u64,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            root: PathBuf::new(),
            framework: Framework::Auto,
            interpreter: String::new(),
            timeout_secs: 300,
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
    names: Vec<String>,
}
#[derive(Clone)]
struct LastRun {
    config: Config,
    interpreter: PathBuf,
    invocation: Invocation,
    cases: Vec<TestCase>,
}
enum Event {
    Config(Result<Config, String>),
    Started(String, PathBuf),
    Output(String),
    Finished(State, RunReport),
}
pub struct Service {
    pub config: Option<Config>,
    pub state: State,
    pub report: RunReport,
    pub command: String,
    pub revision: u64,
    profile: PathBuf,
    receiver: Option<Receiver<Event>>,
    cancel: Arc<AtomicBool>,
    last: Option<LastRun>,
    pending: Option<(Config, Invocation)>,
    project_interpreters: BTreeMap<PathBuf, PathBuf>,
}
impl Service {
    pub fn new(profile: PathBuf) -> Self {
        Self {
            config: None,
            state: State::Idle,
            report: RunReport::default(),
            command: String::new(),
            revision: 0,
            profile,
            receiver: None,
            cancel: Arc::new(AtomicBool::new(false)),
            last: None,
            pending: None,
            project_interpreters: BTreeMap::new(),
        }
    }
    pub fn is_busy(&self) -> bool {
        self.receiver.is_some()
    }
    /// Selection resolved by the shared project worker; never persisted as a
    /// test override and never read from the filesystem on the GUI thread.
    pub fn set_project_interpreter(&mut self, root: PathBuf, interpreter: Option<PathBuf>) {
        if let Some(interpreter) = interpreter {
            if self.project_interpreters.len() >= 128
                && !self.project_interpreters.contains_key(&root)
            {
                self.project_interpreters.pop_first();
            }
            self.project_interpreters.insert(root, interpreter);
        } else {
            self.project_interpreters.remove(&root);
        }
    }
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
    pub fn last_root(&self) -> Option<&Path> {
        self.last.as_ref().map(|l| l.config.root.as_path())
    }
    pub fn can_rerun_failed(&self) -> bool {
        !self.is_busy()
            && self.last.as_ref().is_some_and(|l| {
                l.cases
                    .iter()
                    .any(|c| c.outcome.is_failure() && c.path.is_some())
            })
    }
    pub fn debug_selected(
        &self,
        index: usize,
    ) -> Result<crate::debugger::DebugConfiguration, String> {
        if self.is_busy() {
            return Err("Aguarde a execução Python".into());
        }
        let last = self
            .last
            .as_ref()
            .ok_or("Execute os testes Python primeiro")?;
        let case = last.cases.get(index).ok_or("Caso Python inválido")?;
        if case.path.is_none() {
            return Err("Caso sem fonte navegável não pode ser depurado".into());
        }
        Ok(crate::debugger::DebugConfiguration {
            runtime: crate::debugger::DebugRuntime::Python,
            name: format!("Debug {}", case.title),
            executable: if last.config.interpreter.is_empty() {
                self.project_interpreters
                    .get(&last.config.root)
                    .cloned()
                    .unwrap_or_else(|| last.interpreter.clone())
            } else {
                last.interpreter.clone()
            },
            program: None,
            module: Some(last.config.framework.argument().into()),
            args: if last.config.framework == Framework::Unittest {
                vec![case.name.clone()]
            } else {
                vec!["-q".into(), case.name.clone()]
            },
            cwd: last.config.root.clone(),
            trusted: false,
            ..Default::default()
        })
    }
    pub fn dispatch(&mut self, command: TestCommand, allowed: Vec<PathBuf>) -> Result<(), String> {
        if matches!(command, TestCommand::Cancel) {
            self.cancel();
            return Ok(());
        }
        if self.is_busy() {
            return Err("Aguarde ou interrompa os testes Python".into());
        }
        let (requested, invocation) = match command {
            TestCommand::Inspect(root) => {
                let profile = self.profile.clone();
                let (tx, rx) = mpsc::sync_channel(16);
                std::thread::Builder::new()
                    .name("codisseu-python-tests-inspect".into())
                    .spawn(move || {
                        let result = (|| {
                            let root = checked_root(&root, &allowed)?;
                            Ok(load_profile(&profile)?
                                .remove(&root)
                                .unwrap_or_else(|| Config::new(root)))
                        })();
                        let _ = tx.send(Event::Config(result));
                    })
                    .map_err(|e| e.to_string())?;
                self.receiver = Some(rx);
                self.state = State::Preparing;
                return Ok(());
            }
            TestCommand::Run { config, scope } => (
                config,
                Invocation {
                    file: match scope {
                        Scope::Suite => None,
                        Scope::File(path) => Some(path),
                    },
                    names: vec![],
                },
            ),
            TestCommand::RunSelected(index) => {
                let last = self
                    .last
                    .as_ref()
                    .ok_or("Execute uma suíte Python primeiro")?;
                let case = last.cases.get(index).ok_or("Caso Python inválido")?;
                if case.path.is_none() {
                    return Err(
                        "Caso Python fora do projeto ou sem fonte não pode ser repetido".into(),
                    );
                }
                (
                    last.config.clone(),
                    Invocation {
                        file: None,
                        names: vec![case.name.clone()],
                    },
                )
            }
            TestCommand::RerunFailed => {
                let last = self
                    .last
                    .as_ref()
                    .ok_or("Nenhuma execução Python anterior")?;
                let names = last
                    .cases
                    .iter()
                    .filter(|c| c.outcome.is_failure() && c.path.is_some())
                    .map(|c| c.name.clone())
                    .collect::<Vec<_>>();
                if names.is_empty() {
                    return Err("Nenhum teste Python com falha para repetir".into());
                }
                (last.config.clone(), Invocation { file: None, names })
            }
            TestCommand::RerunLast => {
                let last = self
                    .last
                    .as_ref()
                    .ok_or("Nenhuma execução Python anterior")?;
                (last.config.clone(), last.invocation.clone())
            }
            TestCommand::Cancel => unreachable!(),
        };
        let config = requested.clone();
        let operation = invocation.clone();
        let profile = self.profile.clone();
        let project_interpreter = self.project_interpreters.get(&requested.root).cloned();
        self.cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.cancel.clone();
        let (tx, rx) = mpsc::sync_channel(16);
        std::thread::Builder::new()
            .name("codisseu-python-tests".into())
            .spawn(move || {
                let started = Instant::now();
                let result = (|| {
                    let root = checked_root(&config.root, &allowed)?;
                    validate_config(&config)?;
                    let mut project = crate::python_project::Config::new(root.clone());
                    project.interpreter = if config.interpreter.is_empty() {
                        project_interpreter
                            .as_ref()
                            .map(|p| p.display().to_string())
                            .unwrap_or_default()
                    } else {
                        config.interpreter.clone()
                    };
                    let interpreter = crate::python_project::resolve_interpreter(&project)?;
                    let helper = codisseu_runtime::python_test_runner()?;
                    let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
                    let report_path = temp.path().join("results.json");
                    let mut args = vec![
                        "-I".into(),
                        helper.display().to_string(),
                        "--framework".into(),
                        config.framework.argument().into(),
                        "--root".into(),
                        root.display().to_string(),
                        "--report".into(),
                        report_path.display().to_string(),
                    ];
                    if let Some(file) = &operation.file {
                        let file = checked_file(file, &root)?;
                        args.extend(["--file".into(), file.display().to_string()]);
                    }
                    if operation.names.len() > MAX_CASES
                        || operation.names.iter().any(|n| {
                            n.is_empty() || n.len() > 4096 || n.contains(['\0', '\r', '\n'])
                        })
                    {
                        return Err("Seleção Python excede os limites".into());
                    }
                    for name in &operation.names {
                        if name.starts_with('-') {
                            return Err(
                                "Identificador Python não pode ser uma opção de comando".into()
                            );
                        }
                        if config.framework != Framework::Unittest {
                            checked_file(Path::new(name.split("::").next().unwrap_or("")), &root)?;
                        }
                        args.extend(["--selected".into(), name.clone()]);
                    }
                    persist(&profile, &config)?;
                    let _ = tx.send(Event::Started(
                        format!(
                            "{} {}",
                            interpreter.display(),
                            args.iter()
                                .map(|a| format!("{a:?}"))
                                .collect::<Vec<_>>()
                                .join(" ")
                        ),
                        interpreter.clone(),
                    ));
                    let output = process::run_with_output(
                        &interpreter,
                        &args,
                        &root,
                        Duration::from_secs(config.timeout_secs),
                        &cancel,
                        &|text| {
                            let _ = tx.try_send(Event::Output(text));
                        },
                    )?;
                    let mut report = RunReport {
                        output: output.text,
                        output_truncated: output.truncated,
                        exit_codes: vec![output.code],
                        seconds: started.elapsed().as_secs_f64(),
                        ..Default::default()
                    };
                    if output.cancelled {
                        return Ok((State::Cancelled, report));
                    }
                    if output.timeout {
                        report.error =
                            Some("Execução Python excedeu o tempo limite e foi encerrada".into());
                        return Ok((State::Error, report));
                    }
                    let bytes = read_report(&report_path)?;
                    let (cases, error, wire_code) = parse_report(&bytes, &root)?;
                    report.cases = cases;
                    report.error = error;
                    if wire_code != output.code {
                        report.error =
                            Some("Exit code do relatório Python difere do processo real".into());
                    }
                    if report.cases.is_empty() && report.error.is_none() {
                        report.error = Some("Nenhum teste Python foi coletado".into());
                    }
                    let state = if report.error.is_some() {
                        State::Error
                    } else if !output.success || report.cases.iter().any(|c| c.outcome.is_failure())
                    {
                        State::Failed
                    } else {
                        State::Passed
                    };
                    Ok((state, report))
                })();
                let (state, report) = result.unwrap_or_else(|e| {
                    (
                        State::Error,
                        RunReport {
                            error: Some(e),
                            seconds: started.elapsed().as_secs_f64(),
                            ..Default::default()
                        },
                    )
                });
                let _ = tx.send(Event::Finished(state, report));
            })
            .map_err(|e| e.to_string())?;
        self.pending = Some((requested, invocation));
        self.receiver = Some(rx);
        self.state = State::Preparing;
        self.report = RunReport::default();
        self.command.clear();
        self.revision = self.revision.wrapping_add(1);
        Ok(())
    }
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        let mut done = false;
        while let Some(event) = self.receiver.as_ref().and_then(|rx| match rx.try_recv() {
            Ok(event) => Some(event),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(_) => Some(Event::Finished(
                State::Error,
                RunReport {
                    error: Some("Worker Python encerrado sem resultado".into()),
                    ..Default::default()
                },
            )),
        }) {
            changed = true;
            match event {
                Event::Config(result) => {
                    match result {
                        Ok(config) => {
                            self.config = Some(config);
                            self.state = State::Idle;
                        }
                        Err(e) => {
                            self.state = State::Error;
                            self.report.error = Some(e);
                        }
                    }
                    done = true;
                }
                Event::Started(command, interpreter) => {
                    self.command = command;
                    self.state = State::Running;
                    if let Some((config, invocation)) = &self.pending {
                        self.config = Some(config.clone());
                        let mut config = config.clone();
                        if config.framework == Framework::Auto {
                            config.framework = Framework::Pytest;
                        }
                        self.last = Some(LastRun {
                            config,
                            interpreter,
                            invocation: invocation.clone(),
                            cases: vec![],
                        });
                    }
                }
                Event::Output(output) => self.report.output = output,
                Event::Finished(state, report) => {
                    let started = self.state == State::Running;
                    self.state = state;
                    if started && let Some(last) = self.last.as_mut() {
                        last.cases = report.cases.clone();
                    }
                    self.report = report;
                    self.pending = None;
                    done = true;
                }
            }
            if done {
                break;
            }
        }
        if done {
            self.receiver = None;
        }
        if changed {
            self.revision = self.revision.wrapping_add(1);
        }
        changed
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        self.cancel();
    }
}

fn checked_root(path: &Path, allowed: &[PathBuf]) -> Result<PathBuf, String> {
    let root = path.canonicalize().map_err(|e| e.to_string())?;
    if !root.is_dir()
        || !allowed
            .iter()
            .filter_map(|p| p.canonicalize().ok())
            .any(|p| root.starts_with(p))
    {
        return Err("Testes Python fora dos projetos abertos".into());
    }
    Ok(root)
}
fn checked_file(path: &Path, root: &Path) -> Result<PathBuf, String> {
    let path = if path.is_absolute() {
        path.into()
    } else {
        root.join(path)
    };
    let path = path.canonicalize().map_err(|e| e.to_string())?;
    if !path.starts_with(root)
        || !path.is_file()
        || path.extension().is_none_or(|e| e != "py" && e != "pyw")
    {
        return Err("Arquivo de testes Python fora do projeto ou inválido".into());
    }
    Ok(path)
}
fn validate_config(config: &Config) -> Result<(), String> {
    if !(1..=3600).contains(&config.timeout_secs)
        || config.interpreter.len() > 4096
        || config.interpreter.contains('\0')
    {
        return Err("Timeout/interpretador dos testes Python inválido".into());
    }
    Ok(())
}
fn load_profile(path: &Path) -> Result<BTreeMap<PathBuf, Config>, String> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(e) => Err(e.to_string()),
        Ok(meta) => {
            if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > 1024 * 1024 {
                return Err("Perfil Python Tests inválido; arquivo preservado".into());
            }
            let value: serde_json::Value =
                serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            if value["schema"] != 1 {
                return Err(
                    "Versão do perfil Python Tests desconhecida; arquivo preservado".into(),
                );
            }
            let configs: BTreeMap<PathBuf, Config> =
                serde_json::from_value(value["projects"].clone()).map_err(|e| e.to_string())?;
            if configs.len() > 128 {
                return Err("Perfil Python Tests excede128projetos".into());
            }
            Ok(configs)
        }
    }
}
fn persist(path: &Path, config: &Config) -> Result<(), String> {
    let mut configs = load_profile(path)?;
    configs.insert(config.root.clone(), config.clone());
    if configs.len() > 128 {
        return Err("Perfil Python Tests excede128projetos".into());
    }
    let parent = path.parent().ok_or("Perfil sem diretório")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    temp.write_all(
        &serde_json::to_vec_pretty(&serde_json::json!({"schema":1,"projects":configs}))
            .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    temp.as_file().sync_all().map_err(|e| e.to_string())?;
    temp.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}
fn read_report(path: &Path) -> Result<Vec<u8>, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| "Runner Python não produziu relatório estruturado".to_string())?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_REPORT {
        return Err("Relatório Python inválido ou excede16MiB".into());
    }
    fs::read(path).map_err(|e| e.to_string())
}
fn bounded(value: &serde_json::Value, key: &str, max: usize) -> Result<String, String> {
    let text = value[key]
        .as_str()
        .ok_or("Campo do relatório Python inválido")?;
    if text.len() > max || text.contains('\0') {
        return Err("Campo do relatório Python excede limite".into());
    }
    Ok(text.into())
}
type ParsedReport = (Vec<TestCase>, Option<String>, Option<i32>);
fn parse_report(bytes: &[u8], root: &Path) -> Result<ParsedReport, String> {
    let report: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|e| format!("Relatório Python JSON inválido: {e}"))?;
    if report["schema"] != 1 {
        return Err("Schema do relatório Python inválido".into());
    }
    let values = report["cases"]
        .as_array()
        .ok_or("Relatório Python sem casos")?;
    if values.len() > MAX_CASES {
        return Err("Relatório Python excede10000casos".into());
    }
    let mut cases = Vec::new();
    for value in values {
        let name = bounded(value, "name", 4096)?;
        let title = bounded(value, "title", 4096)?;
        let class = bounded(value, "class", 4096)?;
        let details = bounded(value, "details", 65536)?;
        let path = value["path"]
            .as_str()
            .and_then(|p| Path::new(p).canonicalize().ok())
            .filter(|p| p.starts_with(root) && p.is_file());
        let line = value["line"]
            .as_u64()
            .filter(|n| *n > 0 && *n <= 1_000_000)
            .map(|n| n as u32);
        let ancestors = value["ancestors"]
            .as_array()
            .ok_or("Hierarquia Python inválida")?;
        if ancestors.len() > 64 {
            return Err("Hierarquia Python excessiva".into());
        }
        let ancestors = ancestors
            .iter()
            .map(|v| {
                v.as_str()
                    .filter(|s| s.len() <= 4096)
                    .map(str::to_string)
                    .ok_or("Hierarquia Python inválida")
            })
            .collect::<Result<Vec<_>, _>>()?;
        let outcome = match value["outcome"].as_str() {
            Some("Passed") => Outcome::Passed,
            Some("Failed") => Outcome::Failed,
            Some("Error") => Outcome::Error,
            Some("Skipped") => Outcome::Skipped,
            _ => return Err("Resultado Python desconhecido".into()),
        };
        let seconds = value["seconds"]
            .as_f64()
            .filter(|v| v.is_finite() && *v >= 0.)
            .ok_or("Duração Python inválida")?;
        cases.push(TestCase {
            ancestors,
            title,
            name,
            class,
            path,
            line,
            outcome,
            seconds,
            details,
        });
    }
    let error = if report["error"].is_null() {
        None
    } else {
        Some(bounded(&report, "error", 65536)?)
    };
    let code = report["exit_code"]
        .as_i64()
        .and_then(|v| i32::try_from(v).ok())
        .ok_or("Exit code Python inválido")?;
    Ok((cases, error, Some(code)))
}
