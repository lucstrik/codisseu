//! Shutdown diagnostics and cancellation of speculative reads only.
use super::*;

impl AppService {
    /// Stable categories only: diagnostics must not disclose code, paths or credentials.
    pub fn shutdown_blockers(&self) -> Vec<&'static str> {
        let mut pending = Vec::new();
        macro_rules! busy {
            ($condition:expr, $label:literal) => {
                if $condition {
                    pending.push($label);
                }
            };
        }
        busy!(self.local_history.is_busy(), "histórico local");
        busy!(self.python.is_busy(), "projeto Python");
        busy!(self.code_actions.is_pending(), "correções de código");
        busy!(self.formatting.is_pending(), "formatação");
        busy!(self.rename.is_pending(), "refatoração");
        busy!(self.references.is_pending(), "referências");
        busy!(self.completion.is_pending(), "autocomplete");
        busy!(self.signature_help.is_pending(), "assinaturas");
        busy!(self.navigation.is_pending(), "navegação");
        for operation in self.pending.values() {
            let label = match operation {
                Pending::Save(_) => "gravação de arquivos",
                Pending::Persist => "gravação da sessão",
                Pending::Restore => "restauração da sessão",
                Pending::ProjectFile | Pending::RenameResources => "alterações de arquivos",
                Pending::Encoding => "codificação de arquivos",
                Pending::Workspace => "abertura de projeto",
                Pending::Directory => "árvore de arquivos",
                Pending::Read | Pending::GitRead(_) | Pending::GitTreeRead => "leitura de arquivos",
                Pending::Rename(_) => "refatoração",
                Pending::References(_) => "referências",
                Pending::Completion(_) => "autocomplete",
                Pending::Navigation(_) => "navegação",
            };
            if !pending.contains(&label) {
                pending.push(label);
            }
        }
        busy!(self.search_worker.is_busy(), "busca no documento");
        busy!(self.db.is_busy(), "SQLite");
        busy!(self.remote_db.is_busy(), "banco de dados remoto");
        busy!(self.language.is_busy(), "providers de linguagem");
        busy!(self.project_language_busy(), "providers de outros projetos");
        busy!(
            self.automatic_providers.is_busy(),
            "configurações de linguagem"
        );
        busy!(self.jira.is_busy(), "Jira");
        busy!(self.browser_library.is_busy(), "biblioteca do navegador");
        busy!(self.git.is_busy(), "Git");
        busy!(
            self.git_sessions.values().any(|s| s.service.is_busy()),
            "Git de outros projetos"
        );
        busy!(self.debug_profiles.pending(), "configurações de debug");
        busy!(self.ai.profile_pending(), "histórico e configurações AI");
        busy!(self.api.is_busy(), "coleções de API");
        busy!(self.drawings.is_busy(), "catálogo de desenhos");
        pending
    }

    /// Stop expensive, cancellable reads in every repository, including parked
    /// aggregate-workspace services. Accepted Git mutations and file writes retain
    /// their own acknowledgement and are never discarded by this method.
    pub fn cancel_shutdown_reads(&mut self) {
        self.git_commit_messages.cancel(None);
        self.clear_search();
        if self.git.cancellable_read_pending() && !self.git_applying {
            let _ = self.git.dispatch(git::GitCommand::Cancel);
        }
        for session in self.git_sessions.values_mut() {
            if session.service.cancellable_read_pending() && !session.applying {
                let _ = session.service.dispatch(git::GitCommand::Cancel);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs, thread,
        time::{Duration, Instant},
    };

    fn settle(app: &mut AppService) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.is_busy() {
            app.poll();
            assert!(Instant::now() < deadline, "{:?}", app.shutdown_blockers());
            thread::sleep(Duration::from_millis(2));
        }
        assert!(app.last_error.is_none(), "{:?}", app.last_error);
        assert!(app.shutdown_blockers().is_empty());
    }

    #[test]
    fn shutdown_cancel_reads_preserves_accepted_save_and_session_acknowledgements() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("sensitive-project-source.php");
        let session = dir.path().join("private-session.json");
        fs::write(&source, "<?php\n$weather = 'saved';\n").unwrap();
        let mut app = AppService::new(session.clone());
        app.dispatch(Command::SetAutoSave {
            enabled: false,
            delay_ms: 1000,
        })
        .unwrap();
        app.dispatch(Command::OpenFile(source.clone())).unwrap();
        settle(&mut app);
        let d = app.active_document().unwrap();
        let (id, expected_version) = (d.id, d.version);
        let saved = "<?php\n$weather = 'ação🌧';\n";
        app.dispatch(Command::Edit {
            id,
            expected_version,
            text: saved.into(),
        })
        .unwrap();
        app.dispatch(Command::Save { id }).unwrap();
        let generation = app.generation.load(Ordering::SeqCst);
        assert!(app.shutdown_blockers().contains(&"gravação de arquivos"));
        assert!(!app.shutdown_blockers().join(",").contains("sensitive"));
        app.cancel_shutdown_reads();
        assert_eq!(app.generation.load(Ordering::SeqCst), generation);
        assert!(app.pending.values().any(|p| matches!(p, Pending::Save(_))));
        settle(&mut app);
        assert_eq!(fs::read_to_string(&source).unwrap(), saved);
        assert!(!app.active_document().unwrap().is_dirty());
        app.dispatch(Command::PersistSession).unwrap();
        app.cancel_shutdown_reads();
        assert!(app.shutdown_blockers().contains(&"gravação da sessão"));
        settle(&mut app);
        assert!(session.is_file());
    }
}
