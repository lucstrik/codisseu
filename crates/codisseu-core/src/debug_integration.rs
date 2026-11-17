//! Debug commands and profile persistence. Opening a profile never starts code.
use super::*;
use std::{collections::BTreeSet, fs, io::Write, path::Path};

const MAX_PROFILE_BYTES: u64 = 2 * 1024 * 1024;
#[derive(Clone, Default, Serialize, Deserialize)]
struct WorkspaceDebug {
    configurations: Vec<debugger::DebugConfiguration>,
    breakpoints: BTreeMap<PathBuf, Vec<u32>>,
    #[serde(default)]
    breakpoint_settings: Vec<debugger::Breakpoint>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Profile {
    schema_version: u32,
    workspaces: BTreeMap<PathBuf, WorkspaceDebug>,
}
impl Default for Profile {
    fn default() -> Self {
        Self {
            schema_version: 2,
            workspaces: BTreeMap::new(),
        }
    }
}
enum Request {
    Save(Profile),
}
enum Response {
    Loaded(Result<Profile, String>),
    Saved(Result<(), String>),
}
pub(super) struct DebugProfiles {
    sender: Option<SyncSender<Request>>,
    receiver: Receiver<Response>,
    profile: Option<Profile>,
    root: Option<PathBuf>,
    pending: usize,
}
impl DebugProfiles {
    pub(super) fn pending(&self) -> bool {
        self.pending > 0
    }
    pub(super) fn new(path: PathBuf) -> Self {
        let (sender, requests) = mpsc::sync_channel(16);
        let (replies, receiver) = mpsc::sync_channel(16);
        let started = std::thread::Builder::new()
            .name("codisseu-debug-profiles".into())
            .spawn(move || {
                if replies.send(Response::Loaded(load(&path))).is_err() {
                    return;
                }
                while let Ok(Request::Save(profile)) = requests.recv() {
                    if replies
                        .send(Response::Saved(save(&path, &profile)))
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .is_ok();
        Self {
            sender: started.then_some(sender),
            receiver,
            profile: None,
            root: None,
            pending: usize::from(started),
        }
    }
    fn update(
        &mut self,
        root: &Path,
        change: impl FnOnce(&mut WorkspaceDebug),
    ) -> Result<(), String> {
        let mut next = self
            .profile
            .clone()
            .ok_or("Aguarde o carregamento das configurações de debug")?;
        if next.workspaces.len() >= 128 && !next.workspaces.contains_key(root) {
            return Err("Limite de 128 workspaces nas configurações de debug".into());
        }
        change(next.workspaces.entry(root.to_path_buf()).or_default());
        // Older releases must refuse an integrated-browser profile rather than
        // silently interpreting it as an external Chrome attachment.
        if next
            .workspaces
            .values()
            .any(|w| w.configurations.iter().any(|c| c.integrated_browser))
        {
            next.schema_version = 3;
        }
        self.sender
            .as_ref()
            .ok_or("Worker de configurações de debug indisponível")?
            .try_send(Request::Save(next.clone()))
            .map_err(|_| "Fila de configurações de debug cheia")?;
        self.profile = Some(next);
        self.pending += 1;
        Ok(())
    }
}
fn load(path: &Path) -> Result<Profile, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Profile::default()),
        Err(error) => return Err(format!("Não foi possível ler o perfil de debug: {error}")),
    };
    if !metadata.is_file() || metadata.len() > MAX_PROFILE_BYTES {
        return Err("Perfil de debug inválido ou acima de 2 MiB; arquivo preservado".into());
    }
    let mut profile: Profile = serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| format!("Perfil de debug inválido; arquivo preservado: {e}"))?;
    if !matches!(profile.schema_version, 1..=3) || profile.workspaces.len() > 128 {
        return Err(
            "Versão ou tamanho do perfil de debug não suportado; arquivo preservado".into(),
        );
    }
    for workspace in profile.workspaces.values_mut() {
        if workspace.configurations.len() > 128
            || workspace.breakpoints.values().map(Vec::len).sum::<usize>() > 512
            || workspace.breakpoint_settings.len() > 512
        {
            return Err("Perfil de debug excede os limites de configurações/breakpoints".into());
        }
        for config in &mut workspace.configurations {
            config.trusted = false;
            config.env.clear();
        }
        for lines in workspace.breakpoints.values_mut() {
            lines.retain(|line| *line > 0 && *line <= 1_000_000);
            lines.sort_unstable();
            lines.dedup();
        }
        let mut seen = BTreeSet::new();
        for point in &workspace.breakpoint_settings {
            if point.line == 0
                || point.line > 1_000_000
                || !seen.insert((point.path.clone(), point.line))
            {
                return Err("Invalid saved breakpoint settings; profile preserved".into());
            }
            point.options.validate()?;
        }
    }
    profile.schema_version = profile.schema_version.max(2);
    Ok(profile)
}
fn save(path: &Path, profile: &Profile) -> Result<(), String> {
    let parent = path.parent().ok_or("Perfil de debug sem diretório")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    if fs::symlink_metadata(path).is_ok_and(|m| !m.is_file()) {
        return Err("Destino do perfil de debug não é arquivo regular".into());
    }
    let bytes = serde_json::to_vec_pretty(profile).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_PROFILE_BYTES {
        return Err("Perfil de debug excede 2 MiB".into());
    }
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    file.write_all(&bytes)
        .and_then(|()| file.as_file().sync_all())
        .map_err(|e| e.to_string())?;
    file.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}
impl AppService {
    pub fn debugger_has_sessions(&self) -> bool {
        self.browser_debug_pending()
            || self
                .debugger
                .sessions
                .iter()
                .any(|session| session.state.is_active())
    }
    pub fn stop_debugger(&mut self) {
        self.stop_browser_debug();
        let ids: Vec<_> = self
            .debugger
            .sessions
            .iter()
            .filter(|s| s.state.is_active() && s.state != debugger::DebugState::Terminating)
            .map(|s| s.id)
            .collect();
        for id in ids {
            let _ = self.debugger.execute(debugger::DebugCommand::Stop(id));
        }
    }
    pub(super) fn dispatch_debug(
        &mut self,
        mut command: debugger::DebugCommand,
    ) -> Result<(), String> {
        if let debugger::DebugCommand::Start(config) = &command
            && config.integrated_browser
        {
            return self.start_browser_debug(config.clone());
        }
        if let debugger::DebugCommand::Stop(id) = &command {
            self.browser_debug_stop_session(*id);
        }
        if let debugger::DebugCommand::Start(config) = &mut command
            && config.runtime == debugger::DebugRuntime::Python
            && matches!(config.executable.to_str(), Some("" | "python" | "python3"))
        {
            if self.python.is_busy() {
                return Err("Aguarde a inspeção do interpretador Python antes de depurar".into());
            }
            let scope = config.program.as_deref().unwrap_or(&config.cwd);
            if self
                .python
                .config
                .as_ref()
                .is_some_and(|c| scope.starts_with(&c.root))
            {
                config.executable = self.python_interpreter_for(scope).ok_or(
                    "Aguarde ou corrija a inspeção do interpretador Python selecionado antes de depurar; nenhum outro ambiente será usado automaticamente",
                )?;
            }
        }
        if let debugger::DebugCommand::Start(config) = &command {
            if config.request == debugger::DebugRequest::Launch
                && self.documents.iter().any(Document::is_dirty)
            {
                return Err("Salve os arquivos alterados antes de iniciar o debug; o runtime executa os arquivos do disco".into());
            }
            if !config.trusted {
                return Err("Autorize a execução desta configuração no diálogo de debug".into());
            }
        }
        let breakpoint_change = matches!(
            command,
            debugger::DebugCommand::ToggleBreakpoint { .. }
                | debugger::DebugCommand::SetBreakpoints { .. }
                | debugger::DebugCommand::SetBreakpoint(_)
                | debugger::DebugCommand::SetSourceBreakpoints { .. }
        );
        if breakpoint_change && self.debug_profiles.profile.is_none() {
            return Err("Aguarde o carregamento dos breakpoints".into());
        }
        self.debugger.execute(command)?;
        if breakpoint_change && let Some(root) = self.workspace.clone() {
            let mut breakpoints = BTreeMap::<PathBuf, Vec<u32>>::new();
            for point in &self.debugger.breakpoints {
                breakpoints
                    .entry(point.path.clone())
                    .or_default()
                    .push(point.line);
            }
            let settings = self.debugger.breakpoints.clone();
            self.debug_profiles.update(&root, |profile| {
                profile.breakpoints = breakpoints;
                profile.breakpoint_settings = settings;
            })?;
        }
        Ok(())
    }
    pub(super) fn save_debug_configuration(
        &mut self,
        mut config: debugger::DebugConfiguration,
    ) -> Result<(), String> {
        let root = self
            .workspace
            .clone()
            .ok_or("Abra uma pasta para salvar configurações de debug")?;
        if config.id.is_empty()
            || config.id.len() > 1024
            || config.name.trim().is_empty()
            || config.name.len() > 256
        {
            return Err("Nome ou identificador da configuração de debug inválido".into());
        }
        if self.debug_configurations.len() >= 128
            && !self.debug_configurations.iter().any(|c| c.id == config.id)
        {
            return Err("Limite de 128 configurações de debug por workspace".into());
        }
        // Execution authorization and runtime secrets belong to this session.
        config.trusted = false;
        config.env.clear();
        if config.integrated_browser {
            config.browser_tab = None;
            config.browser_inspect_uri = None;
            config.port = 0;
            config.url = config.url.split(['?', '#']).next().unwrap_or("").to_owned();
        }
        self.debug_profiles.update(&root, |profile| {
            profile
                .configurations
                .retain(|existing| existing.id != config.id);
            profile.configurations.push(config);
        })?;
        self.refresh_debug_configurations();
        Ok(())
    }
    pub(super) fn delete_debug_configuration(&mut self, id: String) -> Result<(), String> {
        let root = self
            .workspace
            .clone()
            .ok_or("Abra uma pasta para excluir a configuração de debug")?;
        self.debug_profiles.update(&root, |profile| {
            profile.configurations.retain(|c| c.id != id)
        })?;
        self.refresh_debug_configurations();
        Ok(())
    }
    fn refresh_debug_configurations(&mut self) {
        self.debug_configurations = self
            .workspace
            .as_ref()
            .and_then(|root| self.debug_profiles.profile.as_ref()?.workspaces.get(root))
            .map(|p| p.configurations.clone())
            .unwrap_or_default();
    }
    pub(super) fn poll_debugger(&mut self) -> bool {
        let mut changed = self.debugger.poll();
        let mut loaded = false;
        while let Ok(response) = self.debug_profiles.receiver.try_recv() {
            changed = true;
            self.debug_profiles.pending = self.debug_profiles.pending.saturating_sub(1);
            match response {
                Response::Loaded(Ok(profile)) => {
                    self.debug_profiles.profile = Some(profile);
                    loaded = true;
                }
                Response::Loaded(Err(error)) | Response::Saved(Err(error)) => {
                    self.debug_profile_error = Some(error)
                }
                Response::Saved(Ok(())) => self.debug_profile_error = None,
            }
        }
        if self.debug_profiles.profile.is_some()
            && (loaded || self.debug_profiles.root != self.workspace)
        {
            self.debug_profiles.root = self.workspace.clone();
            self.refresh_debug_configurations();
            let mut paths: BTreeMap<PathBuf, Vec<u32>> = self
                .debugger
                .breakpoints
                .iter()
                .map(|p| (p.path.clone(), Vec::new()))
                .collect();
            if let Some(profile) = self
                .workspace
                .as_ref()
                .and_then(|root| self.debug_profiles.profile.as_ref()?.workspaces.get(root))
            {
                paths.extend(profile.breakpoints.clone());
            }
            let settings = self
                .workspace
                .as_ref()
                .and_then(|root| self.debug_profiles.profile.as_ref()?.workspaces.get(root))
                .map(|p| p.breakpoint_settings.clone())
                .unwrap_or_default();
            for point in &settings {
                paths.entry(point.path.clone()).or_default();
            }
            for (path, lines) in paths {
                let mut points: Vec<_> = lines
                    .into_iter()
                    .map(|line| debugger::Breakpoint::new(path.clone(), line))
                    .collect();
                for point in settings.iter().filter(|b| b.path == path) {
                    points.retain(|b| b.line != point.line);
                    points.push(point.clone());
                }
                if let Err(error) =
                    self.debugger
                        .execute(debugger::DebugCommand::SetSourceBreakpoints {
                            path,
                            breakpoints: points,
                        })
                {
                    self.debug_profile_error = Some(error);
                }
            }
            changed = true;
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn idle(service: &mut AppService) {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            service.poll();
            if !service.is_busy() {
                break;
            }
            assert!(std::time::Instant::now() < until, "{}", service.status);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    #[test]
    fn configurations_and_breakpoints_restore_without_running_code() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        fs::create_dir(&workspace).unwrap();
        let source = workspace.join("fixture.php");
        fs::write(&source, "<?php\n$value = 42;\necho $value;\n").unwrap();
        let session = temp.path().join("profile/session.json");
        {
            let mut service = AppService::new(session.clone());
            service
                .dispatch(Command::OpenWorkspace(workspace.clone()))
                .unwrap();
            idle(&mut service);
            let mut config = debugger::DebugConfiguration::for_file(&source, &workspace);
            config.trusted = true;
            config
                .env
                .insert("PRIVATE_TOKEN".into(), "never-write-this".into());
            service
                .dispatch(Command::SaveDebugConfiguration(config))
                .unwrap();
            service
                .dispatch(Command::Debug(debugger::DebugCommand::ToggleBreakpoint {
                    path: source.clone(),
                    line: 2,
                }))
                .unwrap();
            let mut point = service.debugger.breakpoints[0].clone();
            point.enabled = false;
            point.options.condition = "$i === 3".into();
            point.options.log_message = "i={$i}".into();
            service
                .dispatch(Command::Debug(debugger::DebugCommand::SetBreakpoint(point)))
                .unwrap();
            service
                .dispatch(Command::Debug(
                    debugger::DebugCommand::SetBreakpointsEnabled(false),
                ))
                .unwrap();
            idle(&mut service);
            assert!(service.debugger.sessions.is_empty());
            let profile = fs::read_to_string(session.with_extension("debugger.json")).unwrap();
            assert!(!profile.contains("never-write-this"));
            assert!(!profile.contains("PRIVATE_TOKEN"));
        }
        let mut service = AppService::new(session);
        service.dispatch(Command::OpenWorkspace(workspace)).unwrap();
        idle(&mut service);
        assert_eq!(service.debug_configurations.len(), 1);
        assert!(!service.debug_configurations[0].trusted);
        assert_eq!(service.debugger.breakpoints.len(), 1);
        assert_eq!(service.debugger.breakpoints[0].line, 2);
        assert!(!service.debugger.breakpoints[0].enabled);
        assert_eq!(
            service.debugger.breakpoints[0].options.condition,
            "$i === 3"
        );
        assert_eq!(
            service.debugger.breakpoints[0].options.log_message,
            "i={$i}"
        );
        assert!(!service.debugger.breakpoints[0].verified);
        assert!(
            service.debugger.breakpoints_enabled(),
            "Master mute is not persisted"
        );
        assert!(service.debugger.sessions.is_empty());
        assert!(
            service
                .dispatch(Command::Debug(debugger::DebugCommand::Start(
                    service.debug_configurations[0].clone()
                )))
                .is_err()
        );
        assert!(service.debugger.sessions.is_empty());
    }
    #[test]
    fn profile_roundtrip_drops_execution_approval_and_secrets() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("debug.json");
        let mut profile = Profile::default();
        let mut config = debugger::DebugConfiguration {
            trusted: true,
            ..Default::default()
        };
        config.env.insert("TOKEN".into(), "secret".into());
        profile.workspaces.insert(
            temp.path().into(),
            WorkspaceDebug {
                configurations: vec![config],
                breakpoints: BTreeMap::from([(temp.path().join("test.php"), vec![3, 3, 0])]),
                breakpoint_settings: vec![],
            },
        );
        // A crafted profile cannot authorize execution on load.
        save(&path, &profile).unwrap();
        let restored = load(&path).unwrap();
        let workspace = restored.workspaces.get(temp.path()).unwrap();
        assert!(!workspace.configurations[0].trusted);
        assert!(workspace.configurations[0].env.is_empty());
        assert_eq!(workspace.breakpoints.values().next().unwrap(), &[3]);
    }
    #[test]
    fn python_module_interpreter_roundtrip_never_restores_execution_authority() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("debug.json");
        let config = debugger::DebugConfiguration {
            runtime: debugger::DebugRuntime::Python,
            executable: temp.path().join("venv/bin/python"),
            module: Some("unittest".into()),
            args: vec!["test_weather.Weather.test_pass".into()],
            trusted: true,
            ..Default::default()
        };
        let mut profile = Profile::default();
        profile.workspaces.insert(
            temp.path().into(),
            WorkspaceDebug {
                configurations: vec![config.clone()],
                ..Default::default()
            },
        );
        save(&path, &profile).unwrap();
        let restored = load(&path).unwrap();
        let actual = &restored.workspaces[temp.path()].configurations[0];
        assert_eq!(actual.runtime, debugger::DebugRuntime::Python);
        assert_eq!(actual.executable, config.executable);
        assert_eq!(actual.module, config.module);
        assert_eq!(actual.args, config.args);
        assert!(!actual.trusted);
    }

    #[test]
    fn automatic_python_debug_never_falls_back_during_unresolved_selection() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("never.py");
        fs::write(&source, "raise RuntimeError('must not run')\n").unwrap();
        let mut app = AppService::new(temp.path().join("profile/session.json"));
        let mut selection = crate::python_project::Config::new(temp.path().into());
        selection.interpreter = "/unresolved/venv/bin/python".into();
        app.python.config = Some(selection);
        app.python.interpreter = None;
        let mut config = debugger::DebugConfiguration::for_file(source, temp.path());
        config.trusted = true;
        let error = app
            .dispatch(Command::Debug(debugger::DebugCommand::Start(config)))
            .unwrap_err();
        assert!(error.contains("inspeção"));
        assert!(app.debugger.sessions.is_empty());
    }
    #[test]
    fn legacy_breakpoints_migrate_without_starting_a_session() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("debug.json");
        let original = br#"{"schema_version":1,"workspaces":{"/fixture":{"configurations":[],"breakpoints":{"/fixture/a.php":[2]}}}}"#;
        fs::write(&path, original).unwrap();
        let profile = load(&path).unwrap();
        assert_eq!(profile.schema_version, 2);
        let workspace = &profile.workspaces[Path::new("/fixture")];
        assert_eq!(workspace.breakpoints[Path::new("/fixture/a.php")], vec![2]);
        assert!(workspace.breakpoint_settings.is_empty());
        assert_eq!(fs::read(&path).unwrap(), original);
    }
    #[test]
    fn invalid_profile_is_preserved_and_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("debug.json");
        fs::write(&path, br#"{"schema_version":99,"workspaces":{}}"#).unwrap();
        assert!(load(&path).is_err());
        assert!(fs::read_to_string(path).unwrap().contains("99"));
    }
}
