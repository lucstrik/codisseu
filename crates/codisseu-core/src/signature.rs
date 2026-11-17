//! Read-only signature requests over synchronized open buffers.
use super::*;

#[derive(Default)]
pub struct SignatureHelpState {
    pub result: Option<SignatureHelpResult>,
    pub error: Option<String>,
    context: Option<Context>,
}
struct Context {
    id: DocumentId,
    version: u64,
    caret: usize,
    line: u32,
    character: u32,
    versions: Vec<(DocumentId, u64)>,
    pending: bool,
}
impl SignatureHelpState {
    pub fn is_pending(&self) -> bool {
        self.context.as_ref().is_some_and(|c| c.pending)
    }
    pub fn is_open(&self) -> bool {
        self.context.is_some()
    }
    pub fn anchor(&self) -> Option<(DocumentId, u64, usize)> {
        self.context.as_ref().map(|c| (c.id, c.version, c.caret))
    }
}
impl AppService {
    pub(super) fn cancel_signature_help(&mut self) {
        if self.signature_help.context.take().is_some() {
            let _ = self.language.dispatch(LanguageCommand::CancelSignatureHelp);
        }
        self.signature_help.result = None;
        self.signature_help.error = None;
    }
    pub(super) fn signature_help_failed(&mut self, error: String) {
        self.cancel_signature_help();
        self.signature_help.error = Some(error.clone());
        self.last_error = Some(error.clone());
        self.status = error;
    }
    pub(super) fn request_signature_help(
        &mut self,
        id: DocumentId,
        version: u64,
        caret: usize,
        trigger: SignatureHelpTrigger,
        is_retrigger: bool,
    ) -> Result<(), String> {
        let doc = self
            .active_document()
            .filter(|d| d.id == id && d.version == version)
            .ok_or("Assinatura: documento ativo ou versão mudou")?;
        doc.ensure_language_size()?;
        let (line, character) =
            scalar_lsp_position(doc.text(), caret).ok_or("Assinatura: cursor inválido")?;
        let language = Language::for_path(&doc.path).ok_or("Sem provider para este documento")?;
        if !self
            .language
            .statuses
            .iter()
            .any(|s| s.language == language && s.state == ProviderState::Ready)
        {
            return Err("Inicie o provider deste documento antes de consultar assinatura".into());
        }
        let path = doc.path.clone();
        self.cancel_navigation();
        if !is_retrigger {
            self.cancel_completion();
        }
        self.cancel_references();
        self.cancel_rename();
        self.cancel_signature_help();
        self.language.dispatch(LanguageCommand::CancelHover)?;
        self.sync_documents();
        if self
            .documents
            .iter()
            .filter(|d| {
                !d.is_read_only()
                    && self
                        .language_project_root()
                        .is_none_or(|root| d.project_root() == root)
            })
            .any(|d| {
                Language::for_path(&d.path).is_some_and(|lang| {
                    self.language
                        .statuses
                        .iter()
                        .any(|s| s.language == lang && s.state == ProviderState::Ready)
                        && self.language_versions.get(&d.path) != Some(&d.version)
                })
            })
        {
            return Err("Aguarde a sincronização dos buffers antes de consultar assinatura".into());
        }
        self.language.dispatch(LanguageCommand::SignatureHelp {
            path,
            version,
            line,
            character,
            trigger,
            is_retrigger,
        })?;
        self.signature_help.context = Some(Context {
            id,
            version,
            caret,
            line,
            character,
            versions: self.documents.iter().map(|d| (d.id, d.version)).collect(),
            pending: true,
        });
        self.status = "Consultando assinatura…".into();
        Ok(())
    }
    pub(super) fn poll_signature_help(&mut self) -> bool {
        let Some(c) = self.signature_help.context.as_ref() else {
            return false;
        };
        if self.active != Some(c.id)
            || c.versions.iter().any(|(id, version)| {
                !self
                    .documents
                    .iter()
                    .any(|d| d.id == *id && d.version == *version)
            })
        {
            self.cancel_signature_help();
            return true;
        }
        if self
            .active_document()
            .and_then(|d| Language::for_path(&d.path))
            .is_some_and(|language| {
                self.language.statuses.iter().any(|s| {
                    s.language == language
                        && matches!(s.state, ProviderState::Failed | ProviderState::Stopped)
                })
            })
        {
            self.signature_help_failed("Assinatura cancelada: provider indisponível".into());
            return true;
        }
        if let Some(error) = self.language.signature_help_error.take() {
            self.signature_help_failed(error);
            return true;
        }
        if !c.pending {
            return false;
        }
        // A provider that becomes ready can synchronize another already-open
        // buffer without changing any document version. LanguageService then
        // invalidates its pending request (and the wire timeout) locally. Mirror
        // that cancellation instead of waiting forever for a discarded reply.
        if self.language.signature_help.is_none() && !self.language.signature_help_pending() {
            self.cancel_signature_help();
            return true;
        }
        let Some(result) = self.language.signature_help.take() else {
            return false;
        };
        if self.active_document().is_none_or(|d| d.path != result.path)
            || result.version != c.version
            || result.line != c.line
            || result.character != c.character
        {
            self.cancel_signature_help();
            return true;
        }
        self.status = if result.signatures.is_empty() {
            "Nenhuma assinatura nesta posição".into()
        } else {
            format!("{} assinatura(s)", result.signatures.len())
        };
        self.signature_help.result = Some(result);
        self.signature_help.context.as_mut().unwrap().pending = false;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs, thread,
        time::{Duration, Instant},
    };
    fn fixture() -> (tempfile::TempDir, AppService, DocumentId) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("signature.ts");
        fs::write(&path, "// 🙂 e\u{301}\r\nf(1, 2)\r\n").unwrap();
        let mut app = AppService::new(dir.path().join("session.json"));
        app.dispatch(Command::OpenFile(path)).unwrap();
        let end = Instant::now() + Duration::from_secs(5);
        while app.is_busy() {
            app.poll();
            assert!(Instant::now() < end);
            thread::sleep(Duration::from_millis(1));
        }
        let id = app.active.unwrap();
        (dir, app, id)
    }
    fn incoming(app: &mut AppService, caret: usize, wrong_position: bool) {
        let d = app.active_document().unwrap();
        let (line, character) = scalar_lsp_position(d.text(), caret).unwrap();
        let (id, version, path) = (d.id, d.version, d.path.clone());
        app.signature_help.context = Some(Context {
            id,
            version,
            caret,
            line,
            character,
            versions: app.documents.iter().map(|d| (d.id, d.version)).collect(),
            pending: true,
        });
        app.language.signature_help = Some(SignatureHelpResult {
            path,
            version,
            line,
            character: character + u32::from(wrong_position),
            signatures: vec![SignatureInformation {
                label: "f(x: number, y: number)".into(),
                parameters: vec![
                    SignatureParameter {
                        label_range: 2..11,
                        documentation: None,
                    },
                    SignatureParameter {
                        label_range: 13..22,
                        documentation: None,
                    },
                ],
                documentation: None,
                active_parameter: None,
            }],
            active_signature: Some(0),
            active_parameter: Some(1),
            truncated: false,
        });
        app.poll_signature_help();
    }
    #[test]
    fn signature_is_read_only_correlated_and_invalidated_by_edit_selection_view_and_provider() {
        let (_dir, mut app, id) = fixture();
        let original = app.active_document().unwrap().text().to_owned();
        let caret = original[..original.find('2').unwrap()].chars().count();
        incoming(&mut app, caret, true);
        assert!(!app.signature_help.is_open());
        incoming(&mut app, caret, false);
        assert!(app.signature_help.result.is_some());
        app.dispatch(Command::SetSelection {
            id,
            expected_version: 1,
            selection: Selection::caret(caret),
        })
        .unwrap();
        assert!(!app.signature_help.is_open());
        incoming(&mut app, caret, false);
        app.dispatch(Command::Edit {
            id,
            expected_version: 1,
            text: original.replace('2', "3"),
        })
        .unwrap();
        assert!(!app.signature_help.is_open());
        app.dispatch(Command::Undo { id }).unwrap();
        assert_eq!(app.active_document().unwrap().text(), original);
        incoming(&mut app, caret, false);
        app.dispatch(Command::CreateView).unwrap();
        assert!(!app.signature_help.is_open());
        incoming(&mut app, caret, false);
        app.language
            .statuses
            .push(codisseu_language::ProviderStatus {
                language: Language::TypeScript,
                state: ProviderState::Failed,
                detail: "controlled crash".into(),
            });
        app.poll_signature_help();
        assert!(!app.signature_help.is_open());
        assert!(
            app.signature_help
                .error
                .as_ref()
                .unwrap()
                .contains("indisponível")
        );
        assert_eq!(
            fs::read_to_string(&app.active_document().unwrap().path).unwrap(),
            original
        );
    }
    #[test]
    fn signature_preflight_rejects_stale_crlf_invalid_cursor_and_missing_provider_without_editing()
    {
        let (_dir, mut app, id) = fixture();
        let original = app.active_document().unwrap().text().to_owned();
        let crlf = original[..original.find('\n').unwrap()].chars().count();
        for (version, caret) in [(2, 0), (1, usize::MAX), (1, crlf), (1, 0)] {
            assert!(
                app.dispatch(Command::SignatureHelp {
                    id,
                    expected_version: version,
                    character_index: caret,
                    trigger: SignatureHelpTrigger::Invoked,
                    is_retrigger: false
                })
                .is_err()
            );
            assert!(!app.signature_help.is_open());
            assert!(app.signature_help.error.is_some());
            assert_eq!(app.active_document().unwrap().text(), original);
        }
        app.dispatch(Command::CancelSignatureHelp).unwrap();
        assert!(app.signature_help.error.is_none());
    }
    #[test]
    fn late_provider_sync_without_version_changes_cancels_the_application_request() {
        let (dir, mut app, id) = fixture();
        let php = dir.path().join("dependency.php");
        fs::write(&php, "<?php function dependency(): void {}\n").unwrap();
        app.dispatch(Command::OpenFile(php.clone())).unwrap();
        let end = Instant::now() + Duration::from_secs(5);
        while app.is_busy() {
            app.poll();
            assert!(Instant::now() < end);
            thread::sleep(Duration::from_millis(1));
        }
        app.dispatch(Command::Activate { id }).unwrap();
        // Isolated state-machine test: actual provider transport/replies are
        // covered separately. Do not poll worker events before the assertion.
        app.language.statuses = vec![
            codisseu_language::ProviderStatus {
                language: Language::TypeScript,
                state: ProviderState::Ready,
                detail: "controlled ready".into(),
            },
            codisseu_language::ProviderStatus {
                language: Language::Php,
                state: ProviderState::Starting,
                detail: "controlled initial index".into(),
            },
        ];
        let versions: Vec<_> = app.documents.iter().map(|d| (d.id, d.version)).collect();
        app.dispatch(Command::SignatureHelp {
            id,
            expected_version: 1,
            character_index: 0,
            trigger: SignatureHelpTrigger::Invoked,
            is_retrigger: false,
        })
        .unwrap();
        assert!(app.signature_help.is_pending());
        assert!(app.language.signature_help_pending());
        assert!(!app.language_versions.contains_key(&php));
        app.language.statuses[1].state = ProviderState::Ready;
        app.sync_documents();
        assert_eq!(
            app.documents
                .iter()
                .map(|d| (d.id, d.version))
                .collect::<Vec<_>>(),
            versions
        );
        assert_eq!(app.language_versions.get(&php), Some(&1));
        assert!(!app.language.signature_help_pending());
        assert!(app.signature_help.is_pending());
        assert!(app.poll_signature_help());
        assert!(!app.signature_help.is_open());
        assert!(!app.signature_help.is_pending());
        assert!(app.signature_help.result.is_none());
        assert!(app.signature_help.error.is_none());
    }
}
