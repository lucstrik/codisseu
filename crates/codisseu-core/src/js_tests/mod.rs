//! Vitest/Jest processes and JSON results; no project code runs during inspection.
mod integration;
mod report;
#[cfg(test)]
mod tests;
pub use crate::php_tests::{Outcome, RunReport, Scope, State, TestCase};
use crate::{debugger, php_tests::process};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Framework {
    #[default]
    Auto,
    Vitest,
    Jest,
}
impl Framework {
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Automático",
            Self::Vitest => "Vitest",
            Self::Jest => "Jest",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    pub root: PathBuf,
    pub framework: Framework,
    pub node: String,
    pub runner: String,
    pub configuration: String,
    pub timeout_secs: u64,
}
impl Config {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            framework: Framework::Auto,
            node: String::new(),
            runner: String::new(),
            configuration: String::new(),
            timeout_secs: 300,
        }
    }
}
#[derive(Clone, Debug)]
pub enum TestCommand {
    Inspect(PathBuf),
    Run { config: Config, scope: Scope },
    RunSelected(usize),
    DebugSelected(usize),
    Debug { config: Config, scope: Scope },
    RerunFailed,
    RerunLast,
    Watch(bool),
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
    invocations: Vec<Invocation>,
    cases: Vec<TestCase>,
}
struct Prepared {
    config: Config,
    program: PathBuf,
    args: Vec<String>,
    directory: tempfile::TempDir,
    report: PathBuf,
    names: Vec<String>,
}
enum Event {
    Config(Result<Config, String>),
    Output(String),
    Resolved(Config),
    Finished(State, RunReport),
    Debug(Result<Prepared, String>),
}
pub struct Service {
    pub config: Option<Config>,
    pub state: State,
    pub report: RunReport,
    pub revision: u64,
    pub command: String,
    pub watching: bool,
    pub watch_pending: bool,
    pub debug_session: Option<u64>,
    receiver: Option<Receiver<Event>>,
    cancel: Arc<AtomicBool>,
    profile: PathBuf,
    last: Option<LastRun>,
    pending: Option<(Config, Vec<Invocation>)>,
    prepared: Option<Prepared>,
    debug_sent: bool,
    debug_started: Option<Instant>,
    watch_due: Option<Instant>,
}
impl Service {
    pub fn new(profile: PathBuf) -> Self {
        Self {
            config: None,
            state: State::Idle,
            report: RunReport::default(),
            revision: 0,
            command: String::new(),
            watching: false,
            watch_pending: false,
            debug_session: None,
            receiver: None,
            cancel: Arc::new(AtomicBool::new(false)),
            profile,
            last: None,
            pending: None,
            prepared: None,
            debug_sent: false,
            debug_started: None,
            watch_due: None,
        }
    }
    pub fn is_busy(&self) -> bool {
        self.receiver.is_some() || self.prepared.is_some() || self.debug_session.is_some()
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
    pub fn cancel(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
        self.watching = false;
        self.watch_pending = false;
        self.watch_due = None;
    }
    pub fn saved(&mut self, path: &Path) {
        if self.watching && self.last_root().is_some_and(|root| path.starts_with(root)) {
            self.watch_pending = true;
            self.watch_due = Some(Instant::now() + Duration::from_millis(300));
        }
    }
    pub fn watch_ready(&self) -> bool {
        self.watching
            && self.watch_pending
            && !self.is_busy()
            && self.watch_due.is_some_and(|t| Instant::now() >= t)
    }
    pub fn dispatch(&mut self, command: TestCommand, allowed: Vec<PathBuf>) -> Result<(), String> {
        if let TestCommand::Watch(enabled) = command {
            if enabled && self.last.is_none() {
                return Err("Execute os testes antes de ativar Rerun Automatically".into());
            }
            self.watching = enabled;
            if !enabled {
                self.watch_pending = false;
                self.watch_due = None;
            }
            return Ok(());
        }
        if matches!(command, TestCommand::Cancel) {
            self.cancel();
            return Ok(());
        }
        if self.is_busy() {
            return Err("Aguarde ou interrompa os testes JavaScript/TypeScript".into());
        }
        let (tx, rx) = mpsc::sync_channel(32);
        if let TestCommand::Inspect(root) = command {
            if self.config.as_ref().is_none_or(|c| c.root != root) {
                self.cancel();
                self.config = None;
                self.last = None;
                self.report = RunReport::default();
                self.command.clear();
            }
            let profile = self.profile.clone();
            std::thread::Builder::new()
                .name("codisseu-js-tests-config".into())
                .spawn(move || {
                    let _ = tx.send(Event::Config(inspect(&profile, &root, &allowed)));
                })
                .map_err(|e| e.to_string())?;
            self.state = State::Preparing;
        } else {
            let debug = matches!(
                command,
                TestCommand::DebugSelected(_) | TestCommand::Debug { .. }
            );
            let (config, invocations) = match command {
                TestCommand::Run { config, scope } | TestCommand::Debug { config, scope } => (
                    config,
                    vec![Invocation {
                        file: match scope {
                            Scope::Suite => None,
                            Scope::File(p) => Some(p),
                        },
                        names: vec![],
                    }],
                ),
                TestCommand::RunSelected(i) | TestCommand::DebugSelected(i) => {
                    let l = self
                        .last
                        .as_ref()
                        .ok_or("Execute primeiro a suite ou um arquivo")?;
                    let c = l.cases.get(i).ok_or("Teste selecionado inválido")?;
                    if l.cases
                        .iter()
                        .filter(|other| other.path == c.path && other.name == c.name)
                        .count()
                        != 1
                    {
                        return Err("Nome de teste ambíguo; execute o arquivo inteiro".into());
                    }
                    (
                        l.config.clone(),
                        vec![Invocation {
                            file: Some(c.path.clone().ok_or("Teste sem arquivo local válido")?),
                            names: vec![c.name.clone()],
                        }],
                    )
                }
                TestCommand::RerunLast => {
                    let l = self.last.as_ref().ok_or("Nenhuma execução anterior")?;
                    (l.config.clone(), l.invocations.clone())
                }
                TestCommand::RerunFailed => {
                    let l = self.last.as_ref().ok_or("Nenhuma execução anterior")?;
                    let mut groups = BTreeMap::<PathBuf, Vec<String>>::new();
                    for c in l.cases.iter().filter(|c| c.outcome.is_failure()) {
                        groups
                            .entry(
                                c.path
                                    .clone()
                                    .ok_or("Falha sem arquivo local válido; execute a suite")?,
                            )
                            .or_default()
                            .push(c.name.clone());
                    }
                    if groups.is_empty() {
                        return Err("Não há testes falhos".into());
                    }
                    (
                        l.config.clone(),
                        groups
                            .into_iter()
                            .map(|(file, names)| Invocation {
                                file: Some(file),
                                names,
                            })
                            .collect(),
                    )
                }
                _ => unreachable!(),
            };
            let profile = self.profile.clone();
            let cancel = Arc::new(AtomicBool::new(false));
            self.cancel = cancel.clone();
            self.pending = Some((config.clone(), invocations.clone()));
            self.config = Some(config.clone());
            if debug {
                self.watching = false;
                self.watch_pending = false;
            }
            std::thread::Builder::new()
                .name("codisseu-js-tests".into())
                .spawn(move || {
                    if debug {
                        let result = prepare(&profile, &config, &invocations[0], &allowed, true);
                        let _ = tx.send(Event::Debug(result));
                    } else {
                        let (state, report) =
                            execute(&profile, &config, &invocations, &allowed, &cancel, &tx);
                        let _ = tx.send(Event::Finished(state, report));
                    }
                })
                .map_err(|e| e.to_string())?;
            self.state = State::Running;
            self.report = RunReport::default();
            self.command.clear();
        }
        self.receiver = Some(rx);
        self.revision = self.revision.wrapping_add(1);
        Ok(())
    }
    fn finished(&mut self, state: State, report: RunReport) {
        self.state = state;
        self.report = report;
        self.receiver = None;
        if let Some((config, invocations)) = self.pending.take() {
            self.config = Some(config.clone());
            self.last = Some(LastRun {
                config,
                invocations,
                cases: self.report.cases.clone(),
            });
        }
        self.revision = self.revision.wrapping_add(1);
    }
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        for _ in 0..64 {
            let Some(rx) = &self.receiver else {
                break;
            };
            let event = match rx.try_recv() {
                Ok(e) => e,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(_) => {
                    self.finished(
                        State::Error,
                        RunReport {
                            error: Some("Worker de testes JS encerrado sem resultado".into()),
                            ..Default::default()
                        },
                    );
                    break;
                }
            };
            changed = true;
            match event {
                Event::Config(result) => {
                    self.receiver = None;
                    match result {
                        Ok(c) => {
                            self.config = Some(c);
                            self.state = State::Idle;
                        }
                        Err(e) => {
                            self.state = State::Error;
                            self.report.error = Some(e);
                        }
                    }
                    self.revision = self.revision.wrapping_add(1);
                }
                Event::Output(s) => self.report.output = s,
                Event::Resolved(c) => {
                    if let Some((config, _)) = &mut self.pending {
                        *config = c;
                    }
                }
                Event::Finished(state, report) => self.finished(state, report),
                Event::Debug(result) => {
                    self.receiver = None;
                    match result {
                        Ok(prepared) => {
                            if let Some((c, _)) = &mut self.pending {
                                *c = prepared.config.clone();
                            }
                            self.prepared = Some(prepared);
                            self.debug_sent = false;
                            self.debug_started = Some(Instant::now());
                        }
                        Err(e) => self.finished(
                            State::Error,
                            RunReport {
                                error: Some(e),
                                ..Default::default()
                            },
                        ),
                    }
                }
            }
        }
        changed
    }
    pub(crate) fn debug_launch(&mut self) -> Option<debugger::DebugConfiguration> {
        if self.debug_sent || self.cancel.load(Ordering::SeqCst) {
            return None;
        }
        let p = self.prepared.as_ref()?;
        self.debug_sent = true;
        Some(debugger::DebugConfiguration {
            id: "js-tests".into(),
            name: format!("{} Tests", p.config.framework.label()),
            runtime: debugger::DebugRuntime::Node,
            program: Some(p.program.clone()),
            cwd: p.config.root.clone(),
            executable: p.config.node.clone().into(),
            args: p.args.clone(),
            auto_attach_child_processes: true,
            trusted: true,
            ..Default::default()
        })
    }
    pub(crate) fn debug_failed(&mut self, error: String) {
        self.prepared = None;
        self.debug_session = None;
        self.finished(
            State::Error,
            RunReport {
                error: Some(error),
                ..Default::default()
            },
        );
    }
    pub(crate) fn poll_debug(&mut self, sessions: &[debugger::DebugSession]) -> Option<u64> {
        if self.prepared.is_some()
            && self.debug_session.is_none()
            && self.cancel.load(Ordering::SeqCst)
        {
            self.prepared = None;
            self.finished(State::Cancelled, RunReport::default());
            return None;
        }
        let id = self.debug_session?;
        let session = sessions.iter().find(|s| s.id == id)?;
        let timeout = self.prepared.as_ref().is_some_and(|p| {
            self.debug_started
                .is_some_and(|t| t.elapsed() > Duration::from_secs(p.config.timeout_secs))
        });
        if session.state.is_active() {
            return (self.cancel.load(Ordering::SeqCst) || timeout).then_some(id);
        }
        self.debug_session = None;
        let prepared = self.prepared.take()?;
        let cancelled = self.cancel.load(Ordering::SeqCst);
        let output = session
            .output
            .iter()
            .map(|o| o.text.as_str())
            .collect::<String>();
        let code = session.exit_code.and_then(|c| i32::try_from(c).ok());
        let error = session
            .last_error
            .clone()
            .or_else(|| timeout.then(|| "Prazo de debug dos testes excedido".into()));
        let seconds = self
            .debug_started
            .take()
            .map_or(0., |t| t.elapsed().as_secs_f64());
        let (tx, rx) = mpsc::sync_channel(2);
        self.receiver = Some(rx);
        let spawn = std::thread::Builder::new()
            .name("codisseu-js-debug-results".into())
            .spawn(move || {
                let receipt = bounded(&prepared.directory.path().join("exit.json"), 1024)
                    .ok()
                    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                    .and_then(|j| j["code"].as_i64())
                    .and_then(|c| i32::try_from(c).ok());
                let code = receipt.or(code);
                let mut report = RunReport {
                    output: process::inert(output.as_bytes()),
                    exit_codes: vec![code],
                    seconds,
                    ..Default::default()
                };
                let state = if cancelled {
                    State::Cancelled
                } else if let Some(e) = error {
                    report.error = Some(e);
                    State::Error
                } else {
                    finish_report(&prepared, &mut report, code)
                };
                let _ = tx.send(Event::Finished(state, report));
            });
        if let Err(e) = spawn {
            self.debug_failed(e.to_string());
        }
        None
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        self.cancel();
    }
}
fn bounded(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut bytes = vec![];
    std::fs::File::open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("Arquivo de testes excede limite".into());
    }
    Ok(bytes)
}
fn allowed_root(root: &Path, allowed: &[PathBuf]) -> Result<PathBuf, String> {
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    if !root.is_dir()
        || !allowed
            .iter()
            .any(|p| p.canonicalize().is_ok_and(|p| root.starts_with(p)))
    {
        return Err("Raiz dos testes fora das pastas abertas".into());
    }
    Ok(root)
}
fn local_file(root: &Path, path: &Path) -> Result<PathBuf, String> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
    .canonicalize()
    .map_err(|e| e.to_string())?;
    if !path.starts_with(root) || !path.is_file() {
        return Err("Arquivo de testes fora do projeto".into());
    }
    Ok(path)
}
fn inspect(profile: &Path, root: &Path, allowed: &[PathBuf]) -> Result<Config, String> {
    let root = allowed_root(root, allowed)?;
    if profile.exists() {
        let configs: Vec<Config> =
            serde_json::from_slice(&bounded(profile, 1024 * 1024)?).map_err(|e| e.to_string())?;
        if let Some(c) = configs.into_iter().find(|c| c.root == root) {
            return Ok(c);
        }
    }
    Ok(Config::new(root))
}
fn save(profile: &Path, config: &Config) -> Result<(), String> {
    use std::io::Write;
    let mut configs: Vec<Config> = if profile.exists() {
        serde_json::from_slice(&bounded(profile, 1024 * 1024)?).map_err(|e| e.to_string())?
    } else {
        vec![]
    };
    configs.retain(|c| c.root != config.root);
    if configs.len() >= 128 {
        return Err("Limite de128 configurações de testes".into());
    }
    configs.push(config.clone());
    let parent = profile.parent().ok_or("Perfil inválido")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut f = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    serde_json::to_writer(&mut f, &configs).map_err(|e| e.to_string())?;
    f.flush()
        .and_then(|()| f.as_file().sync_all())
        .map_err(|e| e.to_string())?;
    f.persist(profile).map_err(|e| e.to_string())?;
    Ok(())
}
fn filter(names: &[String]) -> Result<String, String> {
    let escape = |s: &str| {
        let mut out = String::new();
        for c in s.chars() {
            if "\\.^$|?*+()[]{}".contains(c) {
                out.push('\\');
            }
            out.push(c);
        }
        out
    };
    let result = format!(
        "^(?:{})$",
        names
            .iter()
            .map(|s| escape(s))
            .collect::<Vec<_>>()
            .join("|")
    );
    if result.len() > 65536 || names.iter().any(|s| s.contains('\0')) {
        return Err("Filtro de testes excede limite".into());
    }
    Ok(result)
}
fn prepare(
    profile: &Path,
    config: &Config,
    invocation: &Invocation,
    allowed: &[PathBuf],
    debug: bool,
) -> Result<Prepared, String> {
    let mut config = config.clone();
    config.root = allowed_root(&config.root, allowed)?;
    if !(1..=3600).contains(&config.timeout_secs)
        || [&config.node, &config.runner, &config.configuration]
            .iter()
            .any(|v| v.len() > 4096 || v.contains(['\0', '\n', '\r']))
    {
        return Err("Configuração de testes inválida".into());
    }
    if config.framework == Framework::Auto {
        config.framework = if config.root.join("node_modules/vitest/vitest.mjs").is_file() {
            Framework::Vitest
        } else {
            Framework::Jest
        };
    }
    let runner = if config.runner.is_empty() {
        if config.framework == Framework::Vitest {
            "node_modules/vitest/vitest.mjs"
        } else {
            "node_modules/jest/bin/jest.js"
        }
    } else {
        &config.runner
    };
    let program = local_file(&config.root, Path::new(runner))?;
    config.runner = program.display().to_string();
    if config.node.is_empty() {
        config.node = codisseu_runtime::node()?.display().to_string();
    }
    let configuration = if config.configuration.is_empty() {
        None
    } else {
        Some(local_file(&config.root, Path::new(&config.configuration))?)
    };
    let file = invocation
        .file
        .as_ref()
        .map(|p| local_file(&config.root, p))
        .transpose()?;
    let directory = tempfile::Builder::new()
        .prefix("codisseu-js-tests-")
        .tempdir()
        .map_err(|e| e.to_string())?;
    let report = directory.path().join("results.json");
    let mut args = if config.framework == Framework::Vitest {
        vec![
            "run".into(),
            "--reporter=json".into(),
            "--includeTaskLocation".into(),
            format!("--outputFile={}", report.display()),
            "--no-color".into(),
        ]
    } else {
        vec![
            "--runInBand".into(),
            "--json".into(),
            format!("--outputFile={}", report.display()),
            "--testLocationInResults".into(),
            "--watch=false".into(),
            "--colors=false".into(),
        ]
    };
    if debug && config.framework == Framework::Vitest {
        args.extend([
            "--no-file-parallelism".into(),
            "--maxWorkers=1".into(),
            "--pool=forks".into(),
            "--testTimeout=0".into(),
            "--hookTimeout=0".into(),
        ]);
    }
    if let Some(p) = configuration {
        args.extend(["--config".into(), p.display().to_string()]);
    }
    if !invocation.names.is_empty() {
        args.extend(["--testNamePattern".into(), filter(&invocation.names)?]);
    }
    if let Some(p) = file {
        if config.framework == Framework::Jest {
            args.push("--runTestsByPath".into());
        } else {
            args.push("--".into());
        }
        args.push(p.display().to_string());
    }
    save(profile, &config)?;
    let program = if debug {
        let wrapper = directory.path().join("debug-runner.cjs");
        let exit = directory.path().join("exit.json");
        let source = format!(
            "const fs=require('node:fs');const {{pathToFileURL}}=require('node:url');process.on('exit', code=>fs.writeFileSync({},JSON.stringify({{code}})));process.argv[1]={};import(pathToFileURL(process.argv[1]).href).catch(e=>{{console.error(e);process.exitCode=1}});",
            serde_json::to_string(&exit).map_err(|e| e.to_string())?,
            serde_json::to_string(&program).map_err(|e| e.to_string())?
        );
        std::fs::write(&wrapper, source).map_err(|e| e.to_string())?;
        wrapper
    } else {
        program
    };
    Ok(Prepared {
        config,
        program,
        args,
        directory,
        report,
        names: invocation.names.clone(),
    })
}
fn finish_report(prepared: &Prepared, report: &mut RunReport, code: Option<i32>) -> State {
    // Keep the private directory alive through report decoding.
    let _ = &prepared.directory;
    match bounded(&prepared.report, 16 * 1024 * 1024)
        .and_then(|b| report::parse(&b, &prepared.config.root, prepared.config.framework))
    {
        Ok(mut cases) if !cases.is_empty() => {
            if !prepared.names.is_empty() {
                cases.retain(|c| prepared.names.contains(&c.name));
                if cases.is_empty() {
                    report.error = Some("Filtro não corresponde a nenhum teste".into());
                    return State::Error;
                }
            }
            let failed = cases.iter().any(|c| c.outcome.is_failure());
            if report.cases.len() + cases.len() > 10_000 {
                report.error = Some("Relatório excede10000 testes".into());
                return State::Error;
            }
            report.cases.extend(cases);
            if code != Some(0) && !failed {
                report.error = Some(format!(
                    "Runner terminou com código {code:?}; confira console"
                ));
                State::Error
            } else if failed {
                State::Failed
            } else {
                State::Passed
            }
        }
        Ok(_) => {
            report.error =
                Some("Nenhum teste executado; confira arquivo/configuração/filtro".into());
            State::Error
        }
        Err(e) => {
            report.error = Some(format!(
                "Runner terminou com código {code:?}, sem relatório válido: {e}"
            ));
            State::Error
        }
    }
}
fn execute(
    profile: &Path,
    config: &Config,
    invocations: &[Invocation],
    allowed: &[PathBuf],
    cancel: &AtomicBool,
    tx: &SyncSender<Event>,
) -> (State, RunReport) {
    let started = Instant::now();
    let mut report = RunReport::default();
    let result = (|| {
        for invocation in invocations {
            if cancel.load(Ordering::SeqCst) {
                return Ok(State::Cancelled);
            }
            let p = prepare(profile, config, invocation, allowed, false)?;
            let _ = tx.send(Event::Resolved(p.config.clone()));
            let mut args = vec![p.program.display().to_string()];
            args.extend(p.args.clone());
            let output = process::run_with_output(
                Path::new(&p.config.node),
                &args,
                &p.config.root,
                Duration::from_secs(config.timeout_secs).saturating_sub(started.elapsed()),
                cancel,
                &|text| {
                    let _ = tx.try_send(Event::Output(text));
                },
            )?;
            let space = (1024 * 1024usize).saturating_sub(report.output.len());
            let text = output
                .text
                .chars()
                .scan(0, |size, c| {
                    *size += c.len_utf8();
                    (*size <= space).then_some(c)
                })
                .collect::<String>();
            report.output_truncated |= output.truncated || text.len() < output.text.len();
            report.output.push_str(&text);
            report.exit_codes.push(output.code);
            if output.cancelled {
                return Ok(State::Cancelled);
            }
            if output.timeout {
                return Err("Prazo dos testes excedido".into());
            }
            if finish_report(&p, &mut report, output.code) == State::Error {
                return Ok(State::Error);
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
        Err(e) => {
            report.error = Some(e);
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
