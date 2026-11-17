//! Formatting is one version-guarded transaction on the active dirty buffer.
use super::*;
use codisseu_language::{FormattingEdit, FormattingResult};
#[derive(Default)]
pub struct FormattingState {
    pub error: Option<String>,
    context: Option<Context>,
    local: Option<std::sync::mpsc::Receiver<Result<FormattingResult, String>>>,
    local_busy: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
struct Context {
    id: DocumentId,
    path: PathBuf,
    version: u64,
    range: Option<RenameRange>,
}
impl FormattingState {
    pub fn is_pending(&self) -> bool {
        self.context.is_some()
    }
}
impl AppService {
    pub(super) fn cancel_formatting(&mut self) {
        let local = self.formatting.local.take().is_some();
        if self.formatting.context.take().is_some() && !local {
            let _ = self.language.dispatch(LanguageCommand::CancelFormatting);
        }
        self.formatting.error = None;
    }
    pub(super) fn formatting_failed(&mut self, error: String) {
        self.cancel_formatting();
        self.formatting.error = Some(error.clone());
        self.last_error = Some(error.clone());
        self.status = error;
    }
    pub(super) fn request_formatting(
        &mut self,
        id: DocumentId,
        expected_version: u64,
        range: Option<Range<usize>>,
    ) -> Result<(), String> {
        let doc = self
            .active_document()
            .filter(|d| d.id == id && d.version == expected_version)
            .ok_or("Formatação: documento ativo ou versão mudou")?;
        doc.ensure_language_size()?;
        let scalar_range = range.clone().unwrap_or(0..doc.len_chars());
        let range = if let Some(r) = range {
            if r.start >= r.end {
                return Err("Selecione texto para formatar".into());
            }
            let (sl, sc) =
                scalar_lsp_position(doc.text(), r.start).ok_or("Início da seleção inválido")?;
            let (el, ec) =
                scalar_lsp_position(doc.text(), r.end).ok_or("Fim da seleção inválido")?;
            Some(RenameRange {
                start_line: sl,
                start_character: sc,
                end_line: el,
                end_character: ec,
            })
        } else {
            None
        };
        let php = doc
            .path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("php") || e.eq_ignore_ascii_case("phtml"));
        let python = Language::for_path(&doc.path) == Some(Language::Python);
        if python && range.is_some() {
            return Err("Formatação Python atua no arquivo inteiro; use Ctrl+Alt+L".into());
        }
        if php && range.is_some() {
            return Err("Formatação PHP atua no arquivo inteiro; use Ctrl+Alt+L".into());
        }
        if php
            || python
            || doc
                .path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("sql"))
        {
            if self.formatting.local.is_some()
                && self.formatting.context.as_ref().is_some_and(|c| {
                    c.id == id && c.version == expected_version && c.range == range
                })
            {
                return Ok(());
            }
            let snapshot = doc.snapshot();
            let crlf = doc.text().contains("\r\n");
            if snapshot.slice(scalar_range.clone()).len_bytes()
                > codisseu_db::sql_formatting::MAX_BYTES
            {
                return Err("Formatação local limitada a256KiB por operação".into());
            }
            if self.formatting.local_busy.swap(true, Ordering::SeqCst) {
                return Err("Aguarde o cálculo anterior de formatação".into());
            }
            let root = doc.project_root().to_path_buf();
            let path = doc.path.clone();
            let result_path = path.clone();
            let format_path = path.clone();
            let result_range = range.clone();
            let busy = self.formatting.local_busy.clone();
            let (tx, rx) = std::sync::mpsc::sync_channel(1);
            let spawn = std::thread::Builder::new()
                .name("codisseu-format".into())
                .spawn(move || {
                    let result = std::panic::catch_unwind(|| {
                        let source = snapshot.slice(scalar_range.clone()).to_string();
                        let formatted = if python {
                            crate::python_formatting::transform(
                                &source,
                                &format_path,
                                &root,
                                crate::python_formatting::Operation::Format,
                            )?
                        } else if php {
                            crate::php_formatting::format(&source)?
                        } else {
                            codisseu_db::sql_formatting::format_with_eol(&source, crlf)?
                        };
                        let prefix = source
                            .chars()
                            .zip(formatted.chars())
                            .take_while(|(a, b)| a == b)
                            .count();
                        let suffix = source
                            .chars()
                            .rev()
                            .take(source.chars().count() - prefix)
                            .zip(
                                formatted
                                    .chars()
                                    .rev()
                                    .take(formatted.chars().count() - prefix),
                            )
                            .take_while(|(a, b)| a == b)
                            .count();
                        let edit = FormattingEdit {
                            range: scalar_range.start + prefix..scalar_range.end - suffix,
                            new_text: formatted
                                .chars()
                                .skip(prefix)
                                .take(formatted.chars().count() - prefix - suffix)
                                .collect(),
                        };
                        Ok(FormattingResult {
                            path: result_path,
                            version: expected_version,
                            range: result_range,
                            edits: vec![edit],
                        })
                    })
                    .unwrap_or_else(|_| Err("Formatador interrompido".into()));
                    busy.store(false, Ordering::SeqCst);
                    let _ = tx.send(result);
                });
            if let Err(error) = spawn {
                self.formatting.local_busy.store(false, Ordering::SeqCst);
                return Err(format!("Não foi possível iniciar formatador: {error}"));
            }
            self.cancel_formatting();
            self.formatting.context = Some(Context {
                id,
                path,
                version: expected_version,
                range,
            });
            self.formatting.local = Some(rx);
            self.status = if python {
                "Formatando Python · Ruff…"
            } else if php {
                "Formatando PHP…"
            } else {
                "Formatando SQL…"
            }
            .into();
            self.last_error = None;
            return Ok(());
        }
        let language = Language::for_path(&doc.path)
            .ok_or("Sem provider de formatação para este documento")?;
        if !self
            .language
            .statuses
            .iter()
            .any(|s| s.language == language && s.state == ProviderState::Ready)
        {
            return Err("Inicie o provider antes de formatar".into());
        }
        let supported = self
            .language
            .formatting_capabilities
            .get(&language)
            .is_some_and(|c| if range.is_some() { c.range } else { c.document });
        if !supported {
            return Err("O provider não oferece esta modalidade de formatação".into());
        }
        let (tab_size, insert_spaces) =
            match codisseu_editor::commands::detect_indentation(&doc.snapshot()) {
                codisseu_editor::commands::Indentation::Spaces(width) => (width as u32, true),
                codisseu_editor::commands::Indentation::Tabs { tab_width } => {
                    (tab_width as u32, false)
                }
            };
        let path = doc.path.clone();
        self.cancel_formatting();
        self.cancel_navigation();
        self.cancel_completion();
        self.cancel_signature_help();
        self.cancel_references();
        self.cancel_rename();
        self.sync_documents();
        if self.language_versions.get(&path) != Some(&expected_version) {
            return Err("Aguarde a sincronização do documento antes de formatar".into());
        }
        self.language.dispatch(LanguageCommand::Formatting {
            path: path.clone(),
            version: expected_version,
            range: range.clone(),
            tab_size,
            insert_spaces,
        })?;
        self.formatting.context = Some(Context {
            id,
            path,
            version: expected_version,
            range,
        });
        self.status = "Formatando pelo servidor de linguagem…".into();
        self.last_error = None;
        Ok(())
    }
    pub(super) fn poll_formatting(&mut self) -> bool {
        let Some(c) = &self.formatting.context else {
            return false;
        };
        if self
            .active_document()
            .is_none_or(|d| d.id != c.id || d.path != c.path || d.version != c.version)
        {
            self.cancel_formatting();
            self.status = "Formatação cancelada: o documento mudou".into();
            return true;
        }
        if let Some(rx) = &self.formatting.local {
            match rx.try_recv() {
                Ok(Ok(result)) => {
                    if let Err(e) = self.apply_formatting_result(result) {
                        self.formatting_failed(e);
                    }
                }
                Ok(Err(e)) => self.formatting_failed(e),
                Err(std::sync::mpsc::TryRecvError::Empty) => return false,
                Err(_) => self.formatting_failed("Worker de formatação encerrado".into()),
            }
            return true;
        }
        if self.language.statuses.iter().any(|s| {
            Language::for_path(&c.path) == Some(s.language)
                && matches!(s.state, ProviderState::Stopped | ProviderState::Failed)
        }) {
            self.formatting_failed("Formatação cancelada: provider indisponível".into());
            return true;
        }
        if let Some(e) = self.language.formatting_error.take() {
            self.formatting_failed(e);
            return true;
        }
        let Some(result) = self.language.formatting.take() else {
            if !self.language.formatting_pending() {
                self.cancel_formatting();
                self.status = "Formatação cancelada: a sincronização mudou".into();
                return true;
            }
            return false;
        };
        if let Err(e) = self.apply_formatting_result(result) {
            self.formatting_failed(e);
        }
        true
    }
    fn apply_formatting_result(&mut self, result: FormattingResult) -> Result<(), String> {
        let c = self
            .formatting
            .context
            .as_ref()
            .ok_or("Formatação já cancelada")?;
        if result.path != c.path || result.version != c.version || result.range != c.range {
            return Err("Resultado de formatação fora do documento/versão solicitados".into());
        }
        let id = c.id;
        let version = c.version;
        let doc = self
            .active_document()
            .filter(|d| d.id == id && d.version == version && d.path == c.path)
            .ok_or("Formatação obsoleta")?;
        let before = doc.selection();
        let deltas = validated_deltas(doc.text(), result.edits)?;
        if deltas.is_empty() {
            self.cancel_formatting();
            self.status = "Documento já formatado".into();
            self.last_error = None;
            return Ok(());
        }
        let after = deltas
            .iter()
            .rev()
            .fold(before, |selection, delta| selection.mapped(delta));
        // execute performs the same version/mutation locks as ordinary editing;
        // the batch keeps real per-edit maps for passive views and Undo/Redo.
        self.execute(Command::EditBatchTransaction {
            id,
            expected_version: version,
            deltas,
            before,
            after,
        })?;
        self.cancel_formatting();
        self.status = "Formatação aplicada; salve para gravar".into();
        self.last_error = None;
        Ok(())
    }
}
fn validated_deltas(text: &str, edits: Vec<FormattingEdit>) -> Result<Vec<TextDelta>, String> {
    if edits.len() > 8192 {
        return Err("Formatação excede8192 edições".into());
    }
    let mut edits = edits;
    edits.sort_by_key(|e| (e.range.start, e.range.end));
    if edits
        .windows(2)
        .any(|p| p[1].range.start < p[0].range.end || p[1].range.start == p[0].range.start)
    {
        return Err("Edições de formatação sobrepostas".into());
    }
    let boundaries: Vec<_> = text
        .char_indices()
        .map(|(b, _)| b)
        .chain(std::iter::once(text.len()))
        .collect();
    let mut deltas = Vec::new();
    let mut bytes = text.len();
    let mut inserted = 0usize;
    for e in edits {
        if e.range.start > e.range.end
            || e.range.end >= boundaries.len()
            || e.new_text.contains('\0')
        {
            return Err("Edição de formatação fora do documento".into());
        }
        inserted = inserted
            .checked_add(e.new_text.len())
            .ok_or("Formatação excessiva")?;
        bytes = bytes
            .checked_sub(boundaries[e.range.end] - boundaries[e.range.start])
            .and_then(|n| n.checked_add(e.new_text.len()))
            .ok_or("Formatação excessiva")?;
        if inserted > 1024 * 1024 {
            return Err("Texto de formatação acima de1MiB".into());
        }
        if text[boundaries[e.range.start]..boundaries[e.range.end]] != e.new_text {
            deltas.push(TextDelta {
                range: e.range,
                insert: e.new_text,
            });
        }
    }
    if bytes > 1024 * 1024 {
        return Err("Formatação excederia1MiB".into());
    }
    deltas.sort_by_key(|d| (d.range.start, d.range.end));
    if deltas
        .windows(2)
        .any(|p| p[1].range.start < p[0].range.end || p[1].range.start == p[0].range.start)
    {
        return Err("Edições de formatação sobrepostas".into());
    }
    Ok(deltas)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs, thread,
        time::{Duration, Instant},
    };
    fn idle(app: &mut AppService) {
        let end = Instant::now() + Duration::from_secs(7);
        loop {
            app.poll();
            if !app.is_busy() {
                return;
            }
            assert!(
                Instant::now() < end,
                "{} {:?}",
                app.status,
                app.formatting.error
            );
            thread::sleep(Duration::from_millis(2));
        }
    }
    fn fixture(mode: &str) -> (tempfile::TempDir, AppService, DocumentId, String) {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        let text = "// 😀 e\u{301}\r\nlet x=1;\r\nlet y=2;\r\n".to_string();
        let path = root.join("format.ts");
        fs::write(&path, &text).unwrap();
        let mut app = AppService::new(root.join("session.json"));
        app.dispatch(Command::OpenWorkspace(root.clone())).unwrap();
        idle(&mut app);
        app.dispatch(Command::OpenFile(path)).unwrap();
        idle(&mut app);
        let id = app.active.unwrap();
        app.dispatch(Command::ConfigureProviders(vec![ProviderConfig {
            language: Language::TypeScript,
            program: "python3".into(),
            args: vec![
                format!(
                    "{}/../codisseu-language/tests/fixtures/formatting_server.py",
                    env!("CARGO_MANIFEST_DIR")
                ),
                mode.into(),
                root.join("wire.jsonl").to_str().unwrap().into(),
            ],
        }]))
        .unwrap();
        app.dispatch(Command::StartProviders).unwrap();
        idle(&mut app);
        (d, app, id, text)
    }
    #[test]
    fn formatting_transaction_preserves_dirty_unicode_crlf_selection_disk_and_one_undo() {
        let (_d, mut app, id, disk) = fixture("safe");
        let dirty = disk.replace("x=1", "x=3");
        app.dispatch(Command::Edit {
            id,
            expected_version: 1,
            text: dirty.clone(),
        })
        .unwrap();
        let before = Selection {
            anchor: dirty.chars().count() - 3,
            head: 7,
        };
        app.dispatch(Command::SetSelection {
            id,
            expected_version: 2,
            selection: before,
        })
        .unwrap();
        app.dispatch(Command::FormatDocument {
            id,
            expected_version: 2,
            range: None,
        })
        .unwrap();
        assert!(app.formatting.is_pending());
        idle(&mut app);
        assert!(app.formatting.error.is_none(), "{:?}", app.formatting.error);
        let expected = dirty.replace("x=3", "x = 3").replace("y=2", "y = 2");
        let after = app.active_document().unwrap().selection();
        assert_eq!(app.active_document().unwrap().text(), expected);
        assert_eq!(
            fs::read_to_string(&app.active_document().unwrap().path).unwrap(),
            disk
        );
        assert!(app.active_document().unwrap().is_dirty());
        app.dispatch(Command::Undo { id }).unwrap();
        assert_eq!(app.active_document().unwrap().text(), dirty);
        assert_eq!(app.active_document().unwrap().selection(), before);
        app.dispatch(Command::Redo { id }).unwrap();
        assert_eq!(app.active_document().unwrap().text(), expected);
        assert_eq!(app.active_document().unwrap().selection(), after);
        app.dispatch(Command::StopProviders).unwrap();
        idle(&mut app);
    }
    #[test]
    fn selection_only_formats_requested_range_and_rejects_stale_foreign_empty_requests() {
        let (_d, mut app, id, disk) = fixture("safe");
        let start = disk[..disk.find("let x").unwrap()].chars().count();
        let end = start + 8;
        assert!(
            app.dispatch(Command::FormatDocument {
                id,
                expected_version: 9,
                range: None
            })
            .is_err()
        );
        assert!(
            app.dispatch(Command::FormatDocument {
                id,
                expected_version: 1,
                range: Some(start..start)
            })
            .is_err()
        );
        app.dispatch(Command::FormatDocument {
            id,
            expected_version: 1,
            range: Some(start..end),
        })
        .unwrap();
        idle(&mut app);
        assert_eq!(
            app.active_document().unwrap().text(),
            disk.replace("x=1", "x = 1")
        );
        app.dispatch(Command::StopProviders).unwrap();
        idle(&mut app);
    }
    #[test]
    fn edit_and_cancel_during_slow_formatting_never_apply_late_response() {
        let (_d, mut app, id, disk) = fixture("delay");
        app.dispatch(Command::FormatDocument {
            id,
            expected_version: 1,
            range: None,
        })
        .unwrap();
        let newer = disk.replace("x=1", "x=9");
        app.dispatch(Command::Edit {
            id,
            expected_version: 1,
            text: newer.clone(),
        })
        .unwrap();
        idle(&mut app);
        assert_eq!(app.active_document().unwrap().text(), newer);
        app.dispatch(Command::FormatDocument {
            id,
            expected_version: 2,
            range: None,
        })
        .unwrap();
        app.dispatch(Command::CancelFormatting).unwrap();
        app.dispatch(Command::CancelFormatting).unwrap();
        idle(&mut app);
        assert_eq!(app.active_document().unwrap().text(), newer);
        app.dispatch(Command::StopProviders).unwrap();
        idle(&mut app);
    }
    #[test]
    fn invalid_or_unsupported_formatting_keeps_every_buffer_and_disk_unchanged() {
        for mode in [
            "unsupported",
            "surrogate",
            "overlap",
            "scope",
            "error",
            "null",
        ] {
            let (_d, mut app, id, disk) = fixture(mode);
            let started = app.dispatch(Command::FormatDocument {
                id,
                expected_version: 1,
                range: None,
            });
            if mode == "unsupported" {
                assert!(started.is_err());
            } else {
                started.unwrap();
                idle(&mut app);
            }
            assert_eq!(app.active_document().unwrap().text(), disk);
            assert_eq!(app.active_document().unwrap().version, 1);
            assert!(!app.active_document().unwrap().is_dirty());
            assert_eq!(
                fs::read_to_string(&app.active_document().unwrap().path).unwrap(),
                disk
            );
            app.dispatch(Command::StopProviders).unwrap();
            idle(&mut app);
        }
    }
    #[test]
    fn forged_foreign_results_and_overlapping_noops_never_reach_buffer() {
        let (_d, mut app, id, disk) = fixture("delay");
        app.dispatch(Command::FormatDocument {
            id,
            expected_version: 1,
            range: None,
        })
        .unwrap();
        let result = FormattingResult {
            path: app
                .active_document()
                .unwrap()
                .path
                .with_file_name("outside.ts"),
            version: 1,
            range: None,
            edits: vec![FormattingEdit {
                range: 0..1,
                new_text: "bad".into(),
            }],
        };
        assert!(app.apply_formatting_result(result).is_err());
        assert_eq!(app.active_document().unwrap().text(), disk);
        assert!(
            validated_deltas(
                "abc",
                vec![
                    FormattingEdit {
                        range: 0..2,
                        new_text: "ab".into()
                    },
                    FormattingEdit {
                        range: 1..1,
                        new_text: "x".into()
                    }
                ]
            )
            .is_err()
        );
        app.dispatch(Command::CancelFormatting).unwrap();
        app.dispatch(Command::StopProviders).unwrap();
        idle(&mut app);
    }
}
