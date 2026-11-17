use super::{Config, TestCommand};
use crate::AppService;
use std::path::PathBuf;
impl AppService {
    pub fn php_test_root(&self) -> Option<PathBuf> {
        self.active_document()
            .filter(|d| !d.is_read_only())
            .map(|d| d.project_root().to_path_buf())
            .or_else(|| self.workspace_roots().first().cloned())
    }
    pub(crate) fn dispatch_php_tests(&mut self, command: TestCommand) -> Result<(), String> {
        let root = match &command {
            TestCommand::Run {
                config: Config { root, .. },
                ..
            }
            | TestCommand::Inspect(root) => Some(root.as_path()),
            TestCommand::RunSelected(_) | TestCommand::RerunFailed | TestCommand::RerunLast => {
                self.php_tests.last_root()
            }
            TestCommand::Cancel => None,
        };
        if let Some(root) = root {
            let known = self.workspace_roots().iter().any(|p| p == root)
                || self.documents.iter().any(|d| d.project_root() == root)
                || self.project_roots().any(|p| p == root);
            if !known {
                return Err("Abra o projeto dos testes antes de executar esta configuração".into());
            }
            if !matches!(command, TestCommand::Inspect(_))
                && self
                    .documents
                    .iter()
                    .any(|d| d.is_dirty() && d.path.starts_with(root))
            {
                return Err("Salve os arquivos alterados deste projeto antes de executar os testes; nenhum arquivo foi salvo automaticamente".into());
            }
        }
        self.php_tests.dispatch(command, self.workspace_roots())
    }
}
