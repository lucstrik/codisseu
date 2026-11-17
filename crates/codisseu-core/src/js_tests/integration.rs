use super::*;
use crate::AppService;
impl AppService {
    pub(crate) fn dispatch_js_tests(&mut self, command: TestCommand) -> Result<(), String> {
        let root = match &command {
            TestCommand::Run { config, .. } | TestCommand::Debug { config, .. } => {
                Some(config.root.as_path())
            }
            TestCommand::Inspect(root) => Some(root.as_path()),
            _ => self.js_tests.last_root(),
        };
        if !matches!(command, TestCommand::Cancel | TestCommand::Watch(false))
            && let Some(root) = root
        {
            if !self.workspace_roots().iter().any(|p| root.starts_with(p)) {
                return Err("Abra o projeto dos testes".into());
            }
            if !matches!(command, TestCommand::Inspect(_) | TestCommand::Watch(_))
                && self
                    .documents
                    .iter()
                    .any(|d| d.is_dirty() && d.path.starts_with(root))
            {
                return Err("Salve os arquivos alterados antes de executar testes JS/TS; nenhum Save automático".into());
            }
        }
        if matches!(command, TestCommand::Cancel)
            && let Some(id) = self.js_tests.debug_session
        {
            let _ = self.debugger.execute(debugger::DebugCommand::Stop(id));
        }
        self.js_tests.dispatch(command, self.workspace_roots())
    }
    pub(crate) fn poll_js_tests(&mut self) -> bool {
        let mut changed = self.js_tests.poll();
        if let Some(config) = self.js_tests.debug_launch() {
            if self
                .documents
                .iter()
                .any(|d| d.is_dirty() && d.path.starts_with(&config.cwd))
            {
                self.js_tests.debug_failed(
                    "Debug cancelado: arquivos alterados durante a preparação".into(),
                );
            } else {
                match self.debugger.execute(debugger::DebugCommand::Start(config)) {
                    Ok(Some(id)) => self.js_tests.debug_session = Some(id),
                    Ok(None) => self
                        .js_tests
                        .debug_failed("Depurador não criou sessão".into()),
                    Err(e) => self.js_tests.debug_failed(e),
                }
            }
            changed = true;
        }
        if let Some(id) = self.js_tests.poll_debug(&self.debugger.sessions) {
            let _ = self.debugger.execute(debugger::DebugCommand::Stop(id));
            changed = true;
        }
        if self.js_tests.watch_ready()
            && self.js_tests.last_root().is_some_and(|r| {
                !self
                    .documents
                    .iter()
                    .any(|d| d.is_dirty() && d.path.starts_with(r))
            })
        {
            self.js_tests.watch_pending = false;
            if let Err(e) = self.dispatch_js_tests(TestCommand::RerunLast) {
                self.js_tests.report.error = Some(e);
                self.js_tests.watching = false;
            }
            changed = true;
        }
        changed
    }
}
