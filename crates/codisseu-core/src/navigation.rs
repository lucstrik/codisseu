use super::*;

const HISTORY_LIMIT: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct NavigationPosition {
    pub document_id: DocumentId,
    pub version: u64,
    pub character_index: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct NavigationSelection {
    pub document_id: DocumentId,
    pub version: u64,
    pub range: Range<usize>,
}

#[derive(Clone, Debug)]
pub struct NavigationPreview {
    pub document_id: DocumentId,
    pub version: u64,
    pub character_index: usize,
    pub targets: Vec<DefinitionTarget>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Intent {
    Definition,
    Implementation,
    SuperDefinition,
    TypeDefinition,
    Preview,
}

#[derive(Default)]
pub struct NavigationState {
    pub choices: Vec<DefinitionTarget>,
    pub preview: Option<NavigationPreview>,
    pub presentation: Option<codisseu_language::DefinitionPresentation>,
    pub selection: Option<NavigationSelection>,
    pub error: Option<String>,
    /// The target file is known but this provider's legacy byte/UTF-16 range
    /// cannot be certified. In that case no symbol selection is manufactured.
    pub warning: Option<String>,
    context: Option<Context>,
    back: Vec<NavigationPosition>,
    forward: Vec<NavigationPosition>,
    next_token: u64,
}

struct Context {
    intent: Intent,
    token: u64,
    origin: NavigationPosition,
    versions: Vec<(DocumentId, u64)>,
    phase: Phase,
}

enum Phase {
    Requesting,
    Choosing,
    Opening,
    PreviewReady,
}

impl NavigationState {
    pub(super) fn context_token(&self) -> Option<u64> {
        self.context.as_ref().map(|c| c.token)
    }
    pub fn is_pending(&self) -> bool {
        self.context
            .as_ref()
            .is_some_and(|c| matches!(c.phase, Phase::Requesting | Phase::Opening))
    }
    pub fn is_preview(&self) -> bool {
        self.context
            .as_ref()
            .is_some_and(|c| c.intent == Intent::Preview)
    }
    pub fn is_implementation(&self) -> bool {
        self.context
            .as_ref()
            .is_some_and(|c| c.intent == Intent::Implementation)
    }
    pub fn is_super_definition(&self) -> bool {
        self.context
            .as_ref()
            .is_some_and(|c| c.intent == Intent::SuperDefinition)
    }
    pub fn is_type_definition(&self) -> bool {
        self.context
            .as_ref()
            .is_some_and(|c| c.intent == Intent::TypeDefinition)
    }
    pub fn can_back(&self) -> bool {
        !self.back.is_empty()
    }
    pub fn can_forward(&self) -> bool {
        !self.forward.is_empty()
    }
}

impl AppService {
    pub(super) fn navigate_to_range(
        &mut self,
        id: DocumentId,
        version: u64,
        range: Range<usize>,
    ) -> Result<(), String> {
        let caret = self
            .active_view()
            .and_then(|v| self.view(v))
            .map_or(0, |v| v.selection().head);
        let origin = self.navigation_position(id, version, caret)?;
        self.navigation_position(id, version, range.start)?;
        self.navigation_position(id, version, range.end)?;
        if range.start > range.end {
            return Err("Intervalo de navegação invertido".into());
        }
        self.cancel_navigation();
        if caret != range.start {
            push_bounded(&mut self.navigation.back, origin);
            self.navigation.forward.clear();
        }
        self.clear_search();
        self.navigation.selection = Some(NavigationSelection {
            document_id: id,
            version,
            range,
        });
        Ok(())
    }
    pub(super) fn request_documentation_hover(
        &mut self,
        id: DocumentId,
        expected_version: u64,
        character_index: usize,
        passive: bool,
    ) -> Result<(), String> {
        if !passive {
            self.cancel_completion();
            self.cancel_signature_help();
            self.cancel_references();
            self.cancel_rename();
        }
        let doc = self
            .documents
            .iter()
            .find(|doc| doc.id == id)
            .ok_or("Documento não encontrado")?;
        if doc.version != expected_version {
            return Err("Documentação: versão obsoleta".into());
        }
        doc.ensure_language_size()?;
        let (line, character) = scalar_lsp_position(doc.text(), character_index)
            .ok_or("Documentação: posição inválida")?;
        let path = doc.path.clone();
        let language = Language::for_path(&path).ok_or("Sem provider para este documento")?;
        if !self
            .language
            .statuses
            .iter()
            .any(|status| status.language == language && status.state == ProviderState::Ready)
        {
            return Err("Inicie o provider deste documento antes de consultar documentação".into());
        }
        self.sync_documents();
        self.language.dispatch(LanguageCommand::Hover {
            path,
            version: expected_version,
            line,
            character,
        })
    }

    pub(super) fn cancel_definition_preview(&mut self) {
        if self.navigation.is_preview() {
            self.cancel_navigation();
        }
    }

    pub(super) fn cancel_navigation(&mut self) {
        let had_request = self.navigation.context.take().is_some();
        self.navigation.choices.clear();
        self.navigation.preview = None;
        self.navigation.presentation = None;
        self.navigation.selection = None;
        self.navigation.warning = None;
        if had_request {
            // Local invalidation is unconditional, including when the transport
            // queue is full. Replies cannot activate a document without context.
            let _ = self.language.dispatch(LanguageCommand::CancelDefinition);
        }
    }

    pub(super) fn navigation_position(
        &self,
        id: DocumentId,
        version: u64,
        index: usize,
    ) -> Result<NavigationPosition, String> {
        let doc = self
            .documents
            .iter()
            .find(|d| d.id == id)
            .ok_or("Documento não encontrado")?;
        if self.active != Some(id) || doc.version != version {
            return Err("Navegação obsoleta: documento ativo ou versão mudou".into());
        }
        doc.ensure_language_size()?;
        let snapshot = doc.snapshot();
        if index > snapshot.len_chars()
            || (index > 0
                && snapshot.char(index - 1) == '\r'
                && (index == snapshot.len_chars() || snapshot.char(index) == '\n'))
        {
            return Err("Posição de navegação inválida".into());
        }
        Ok(NavigationPosition {
            document_id: id,
            version,
            character_index: index,
        })
    }

    pub(super) fn request_definition(
        &mut self,
        id: DocumentId,
        version: u64,
        index: usize,
    ) -> Result<(), String> {
        self.request_navigation(id, version, index, Intent::Definition)
    }
    pub(super) fn request_implementation(
        &mut self,
        id: DocumentId,
        version: u64,
        index: usize,
    ) -> Result<(), String> {
        self.request_navigation(id, version, index, Intent::Implementation)
    }
    pub(super) fn request_super_definition(
        &mut self,
        id: DocumentId,
        version: u64,
        index: usize,
    ) -> Result<(), String> {
        self.request_navigation(id, version, index, Intent::SuperDefinition)
    }
    pub(super) fn request_type_definition(
        &mut self,
        id: DocumentId,
        version: u64,
        index: usize,
    ) -> Result<(), String> {
        self.request_navigation(id, version, index, Intent::TypeDefinition)
    }
    pub(super) fn preview_definition(&mut self, id: DocumentId, version: u64, index: usize) {
        if self.navigation.context.is_some() && !self.navigation.is_preview() {
            return;
        }
        // Hover is speculative: unsupported/absent providers do not publish an
        // editor error, change focus, or replace an explicit navigation request.
        if self
            .request_navigation(id, version, index, Intent::Preview)
            .is_err()
        {
            self.cancel_definition_preview();
        }
    }
    fn request_navigation(
        &mut self,
        id: DocumentId,
        version: u64,
        index: usize,
        intent: Intent,
    ) -> Result<(), String> {
        let origin = self.navigation_position(id, version, index)?;
        if intent != Intent::Preview {
            self.cancel_references();
            self.cancel_rename();
        }
        let doc = self.active_document().ok_or("Abra um documento")?;
        if doc.is_read_only() {
            return Err("Biblioteca Rust somente leitura: volte ao projeto para consultar navegação semântica".into());
        }
        let path = doc.path.clone();
        let language = Language::for_path(&path).ok_or("Sem provider para este documento")?;
        // Rope line lookup avoids walking/materializing the whole file for each
        // speculative hover. Only the prefix of the actual source line is read.
        let snapshot = doc.snapshot();
        let line = snapshot.char_to_line(index);
        let character: usize = snapshot
            .slice(snapshot.line_to_char(line)..index)
            .chars()
            .map(char::len_utf16)
            .sum();
        let (line, character) = (
            u32::try_from(line).map_err(|_| "Linha LSP excessiva")?,
            u32::try_from(character).map_err(|_| "Coluna LSP excessiva")?,
        );
        if !self
            .language
            .statuses
            .iter()
            .any(|s| s.language == language && s.state == ProviderState::Ready)
        {
            return Err("Inicie o provider deste documento antes de ir à definição".into());
        }
        self.cancel_navigation();
        if intent != Intent::Preview {
            self.cancel_completion();
            self.cancel_signature_help();
        }
        self.sync_documents();
        if self
            .documents
            .iter()
            .filter(|doc| {
                !doc.is_read_only()
                    && self
                        .language_project_root()
                        .is_none_or(|root| doc.project_root() == root)
            })
            .any(|doc| {
                Language::for_path(&doc.path).is_some_and(|language| {
                    self.language.statuses.iter().any(|status| {
                        status.language == language && status.state == ProviderState::Ready
                    }) && self.language_versions.get(&doc.path) != Some(&doc.version)
                })
            })
        {
            return Err("Aguarde a sincronização dos buffers antes de ir à definição".into());
        }
        self.language
            .dispatch(if intent == Intent::Implementation {
                LanguageCommand::Implementation {
                    path,
                    version,
                    line,
                    character,
                }
            } else if intent == Intent::SuperDefinition {
                LanguageCommand::SuperDefinition {
                    path,
                    version,
                    line,
                    character,
                }
            } else if intent == Intent::TypeDefinition {
                LanguageCommand::TypeDefinition {
                    path,
                    version,
                    line,
                    character,
                }
            } else {
                LanguageCommand::Definition {
                    path,
                    version,
                    line,
                    character,
                }
            })?;
        self.navigation.next_token += 1;
        self.navigation.context = Some(Context {
            intent,
            token: self.navigation.next_token,
            origin,
            versions: self.documents.iter().map(|d| (d.id, d.version)).collect(),
            phase: Phase::Requesting,
        });
        self.navigation.error = None;
        if intent != Intent::Preview {
            self.status = if intent == Intent::Implementation {
                "Consultando implementações…"
            } else if intent == Intent::SuperDefinition {
                "Consultando supermétodo/supertipo…"
            } else if intent == Intent::TypeDefinition {
                "Consultando declaração de tipo…"
            } else {
                "Consultando definição…"
            }
            .into();
        }
        Ok(())
    }

    fn navigation_current(&self) -> bool {
        self.navigation.context.as_ref().is_some_and(|c| {
            self.active == Some(c.origin.document_id)
                && c.versions.iter().all(|(id, version)| {
                    self.documents
                        .iter()
                        .any(|d| d.id == *id && d.version == *version)
                })
        })
    }

    pub(super) fn poll_definition(&mut self) -> bool {
        let requesting = self
            .navigation
            .context
            .as_ref()
            .is_some_and(|c| matches!(c.phase, Phase::Requesting));
        if !requesting {
            return false;
        }
        if !self.navigation_current() {
            self.cancel_navigation();
            self.cancel_completion();
            self.cancel_signature_help();
            return true;
        }
        if let Some(error) = self.language.definition_error.take() {
            if self.navigation.is_preview() {
                self.cancel_definition_preview();
            } else {
                self.navigation_failed(error);
            }
            return true;
        }
        let Some(result) = self.language.definition.take() else {
            return false;
        };
        let origin = &self.navigation.context.as_ref().unwrap().origin;
        let doc = self.active_document().unwrap();
        if result.path != doc.path || result.version != origin.version {
            self.cancel_navigation();
            self.cancel_completion();
            self.cancel_signature_help();
            return true;
        }
        if self.navigation.is_preview() {
            self.navigation.preview = Some(NavigationPreview {
                document_id: origin.document_id,
                version: origin.version,
                character_index: origin.character_index,
                targets: result.targets,
            });
            self.navigation.context.as_mut().unwrap().phase = Phase::PreviewReady;
            return true;
        }
        self.navigation.context.as_mut().unwrap().phase = Phase::Choosing;
        self.navigation.presentation = result.presentation;
        self.navigation.choices = result.targets;
        match self.navigation.choices.len() {
            0 => {
                let implementation = self.navigation.is_implementation();
                let super_definition = self.navigation.is_super_definition();
                let type_definition = self.navigation.is_type_definition();
                self.cancel_navigation();
                self.cancel_completion();
                self.cancel_signature_help();
                self.status = if implementation {
                    "Nenhuma implementação encontrada nesta posição"
                } else if super_definition {
                    "Nenhum supermétodo/supertipo encontrado nesta posição"
                } else if type_definition {
                    "Nenhuma declaração de tipo encontrada nesta posição"
                } else {
                    "Nenhuma definição encontrada nesta posição"
                }
                .into();
            }
            1 => {
                if let Err(error) = self.choose_definition(0) {
                    self.navigation_failed(error);
                }
            }
            _ => {
                self.status = if self.navigation.is_implementation() {
                    "Escolha uma implementação"
                } else if self.navigation.is_super_definition() {
                    "Escolha um supermétodo/supertipo"
                } else if self.navigation.is_type_definition() {
                    "Escolha uma declaração de tipo"
                } else {
                    "Escolha uma definição"
                }
                .into()
            }
        }
        true
    }

    pub(super) fn show_definition_targets(&mut self) -> Result<(), String> {
        if !self.navigation_current() {
            return Err("Destinos obsoletos".into());
        }
        let context = self
            .navigation
            .context
            .as_ref()
            .ok_or("Consulte uma definição primeiro")?;
        if !matches!(context.phase, Phase::Choosing) {
            return Err("Aguarde os destinos".into());
        }
        let origin = context.origin.clone();
        let versions = context.versions.clone();
        let targets = self.navigation.choices.clone();
        self.cancel_navigation();
        self.present_navigation_targets(origin, versions, targets)
    }

    pub(super) fn choose_definition(&mut self, index: usize) -> Result<(), String> {
        if !self.navigation_current() {
            return Err("Definição obsoleta: documento mudou".into());
        }
        let context = self
            .navigation
            .context
            .as_ref()
            .ok_or("Nenhuma consulta de definição")?;
        if !matches!(context.phase, Phase::Choosing) {
            return Err("Aguarde a consulta de definição".into());
        }
        let target = self
            .navigation
            .choices
            .get(index)
            .cloned()
            .ok_or("Destino de definição inválido")?;
        let token = context.token;
        let workspace = self.language_project_root().ok_or("Abra um workspace")?;
        let allow_rust_libraries = self.documents.iter().any(|d| {
            d.id == context.origin.document_id
                && Language::for_path(&d.path) == Some(Language::Rust)
        });
        self.enqueue(
            Work::Navigation {
                token,
                target,
                workspace,
                open_paths: self.documents.iter().map(|d| d.path.clone()).collect(),
                allow_rust_libraries,
                python_interpreter: self
                    .documents
                    .iter()
                    .find(|d| {
                        d.id == context.origin.document_id
                            && Language::for_path(&d.path) == Some(Language::Python)
                    })
                    .and_then(|d| self.python_interpreter_for(&d.path)),
            },
            Pending::Navigation(token),
        )?;
        self.navigation.context.as_mut().unwrap().phase = Phase::Opening;
        self.navigation.choices.clear();
        self.status = "Abrindo definição…".into();
        Ok(())
    }

    pub(super) fn navigation_failed(&mut self, error: String) {
        self.cancel_navigation();
        self.cancel_completion();
        self.cancel_signature_help();
        self.navigation.error = Some(error.clone());
        self.last_error = Some(error.clone());
        self.status = error;
    }

    pub(super) fn finish_navigation(
        &mut self,
        token: u64,
        target: DefinitionTarget,
        file: Option<OpenedFile>,
        read_only: bool,
    ) -> Result<(), String> {
        if self
            .navigation
            .context
            .as_ref()
            .is_none_or(|c| c.token != token)
            || !self.navigation_current()
        {
            return Ok(());
        }
        let origin = self.navigation.context.as_ref().unwrap().origin.clone();
        self.activate_navigation_target(origin, target, file, read_only)?;
        self.navigation.context = None;
        if self.navigation.warning.is_none() {
            self.status = "Definição aberta".into();
        }
        Ok(())
    }

    pub(super) fn activate_navigation_target(
        &mut self,
        origin: NavigationPosition,
        target: DefinitionTarget,
        file: Option<OpenedFile>,
        read_only: bool,
    ) -> Result<(), String> {
        let loaded = self.documents.iter().find(|d| d.path == target.path);
        if read_only && loaded.is_some_and(Document::is_dirty) {
            return Err("Biblioteca já aberta contém alterações; preserve-as antes de navegar em modo somente leitura".into());
        }
        let text = loaded
            .map(Document::text)
            .or_else(|| file.as_ref().map(|f| f.text.as_str()))
            .ok_or("Destino não carregado")?;
        let encoding = loaded
            .map(Document::encoding)
            .or_else(|| file.as_ref().map(|f| f.encoding))
            .ok_or("Destino sem codificação")?;
        if Language::for_path(&target.path) == Some(Language::Php)
            && encoding != super::FileEncoding::Utf8
            && utf16_byte_offset(text, target.end_line, target.end_character)
                .is_none_or(|end| !text[..end].is_ascii())
        {
            // The mixed-encoding fixture observed Phpactor return column 38
            // for a closed Latin1 class, then 36 after the target was opened as
            // Unicode. Indexed byte offsets and workspace text can have different
            // layouts. Open the verified file without claiming an unproved range;
            // do not guess a coordinate or rewrite the source encoding.
            let id = if let Some(doc) = loaded {
                doc.id
            } else {
                self.insert_document(file.ok_or("Destino não carregado")?)?
            };
            push_bounded(&mut self.navigation.back, origin);
            self.navigation.forward.clear();
            self.clear_search();
            self.active = Some(id);
            self.navigation.selection = None;
            let warning = "Arquivo de definição aberto; posição do símbolo não certificada pelo provider PHP para fontes legadas com acentos".to_owned();
            self.status = warning.clone();
            self.navigation.warning = Some(warning);
            return Ok(());
        }
        self.navigation.warning = None;
        let start = utf16_byte_offset(text, target.start_line, target.start_character)
            .ok_or("Posição inicial da definição inválida")?;
        let end = utf16_byte_offset(text, target.end_line, target.end_character)
            .ok_or("Posição final da definição inválida")?;
        if end < start {
            return Err("Intervalo de definição invertido".into());
        }
        let range = text[..start].chars().count()..text[..end].chars().count();
        // Validate the entire range and capacity before opening or activating a tab.
        let id = if let Some(doc) = loaded {
            doc.id
        } else {
            self.insert_document(file.ok_or("Destino não carregado")?)?
        };
        let version = self.documents.iter().find(|d| d.id == id).unwrap().version;
        if read_only {
            self.document_mut(id)?.read_only = true;
        }
        push_bounded(&mut self.navigation.back, origin);
        self.navigation.forward.clear();
        self.clear_search();
        self.active = Some(id);
        self.navigation.selection = Some(NavigationSelection {
            document_id: id,
            version,
            range,
        });
        self.status = "Definição aberta".into();
        Ok(())
    }

    pub(super) fn navigate_history(
        &mut self,
        id: DocumentId,
        version: u64,
        index: usize,
        forward: bool,
    ) -> Result<(), String> {
        let current = self.navigation_position(id, version, index)?;
        self.cancel_references();
        self.cancel_rename();
        let stack = if forward {
            &self.navigation.forward
        } else {
            &self.navigation.back
        };
        let target = stack
            .last()
            .cloned()
            .ok_or("Sem posição no histórico de navegação")?;
        let doc = self
            .documents
            .iter()
            .find(|d| d.id == target.document_id && d.version == target.version)
            .ok_or("Posição histórica obsoleta: documento editado ou fechado")?;
        scalar_lsp_position(doc.text(), target.character_index)
            .ok_or("Posição histórica inválida")?;
        self.cancel_navigation();
        self.cancel_completion();
        self.cancel_signature_help();
        if forward {
            self.navigation.forward.pop();
            push_bounded(&mut self.navigation.back, current);
        } else {
            self.navigation.back.pop();
            push_bounded(&mut self.navigation.forward, current);
        }
        self.clear_search();
        self.active = Some(target.document_id);
        self.navigation.selection = Some(NavigationSelection {
            document_id: target.document_id,
            version: target.version,
            range: target.character_index..target.character_index,
        });
        self.navigation.error = None;
        self.status = if forward {
            "Navegação avançou"
        } else {
            "Navegação voltou"
        }
        .into();
        Ok(())
    }
}

fn push_bounded(history: &mut Vec<NavigationPosition>, position: NavigationPosition) {
    if history.len() == HISTORY_LIMIT {
        history.remove(0);
    }
    history.push(position);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs, thread,
        time::{Duration, Instant},
    };

    fn idle(app: &mut AppService) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            app.poll();
            if !app.is_busy() {
                return;
            }
            assert!(Instant::now() < deadline, "{}", app.status);
            thread::sleep(Duration::from_millis(1));
        }
    }
    fn open(app: &mut AppService, path: &std::path::Path) -> DocumentId {
        app.dispatch(Command::OpenFile(path.into())).unwrap();
        idle(app);
        app.active.unwrap()
    }
    fn target(path: PathBuf) -> DefinitionTarget {
        DefinitionTarget {
            path,
            start_line: 0,
            start_character: 20,
            end_line: 0,
            end_character: 26,
        }
    }
    // A controlled incoming LSP result tests the application boundary separately
    // from real-provider tests. Filesystem/navigation/history remain real.
    fn incoming(app: &mut AppService, target: DefinitionTarget, index: usize) {
        app.navigation.next_token += 1;
        let doc = app.active_document().unwrap();
        app.navigation.context = Some(Context {
            intent: Intent::Definition,
            token: app.navigation.next_token,
            origin: NavigationPosition {
                document_id: doc.id,
                version: doc.version,
                character_index: index,
            },
            versions: app.documents.iter().map(|d| (d.id, d.version)).collect(),
            phase: Phase::Choosing,
        });
        app.navigation.choices = vec![target];
    }
    fn fixture() -> (tempfile::TempDir, AppService, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let source = root.join("main.ts");
        let destination = root.join("ação.ts");
        fs::write(&source, "/* 😀 */ target();\r\n").unwrap();
        fs::write(&destination, "/* 😀 */ export const target = 42;\r\n").unwrap();
        let mut app = AppService::new(root.join("session.json"));
        app.dispatch(Command::OpenWorkspace(root)).unwrap();
        idle(&mut app);
        open(&mut app, &source);
        (dir, app, source, destination)
    }
    #[test]
    fn structural_navigation_rejects_stale_ranges_and_preserves_history_and_source() {
        let (_dir, mut app, source, _) = fixture();
        app.dispatch(Command::SetAutoSave {
            enabled: false,
            delay_ms: 1000,
        })
        .unwrap();
        let id = app.active.unwrap();
        let before = app.active_document().unwrap().text().to_owned();
        assert!(
            app.dispatch(Command::NavigateToRange {
                id,
                expected_version: 99,
                range: 0..1
            })
            .is_err()
        );
        assert!(
            app.dispatch(Command::NavigateToRange {
                id,
                expected_version: 1,
                range: 0..999
            })
            .is_err()
        );
        assert!(!app.navigation.can_back());
        app.dispatch(Command::SetSelection {
            id,
            expected_version: 1,
            selection: Selection::caret(12),
        })
        .unwrap();
        app.dispatch(Command::NavigateToRange {
            id,
            expected_version: 1,
            range: 8..14,
        })
        .unwrap();
        assert_eq!(app.navigation.selection.as_ref().unwrap().range, 8..14);
        assert!(app.navigation.can_back());
        app.dispatch(Command::NavigateBack {
            id,
            expected_version: 1,
            character_index: 8,
        })
        .unwrap();
        assert_eq!(app.navigation.selection.as_ref().unwrap().range, 12..12);
        assert_eq!(app.active_document().unwrap().text(), before);
        assert_eq!(fs::read_to_string(source).unwrap(), before);
        assert!(!app.active_document().unwrap().is_dirty());
    }
    #[test]
    fn navigation_preserves_both_dirty_buffers_history_and_disk() {
        let (_dir, mut app, source, destination) = fixture();
        let source_id = app.active.unwrap();
        app.dispatch(Command::Edit {
            id: source_id,
            expected_version: 1,
            text: "/* 😀 e\u{301} */ target();\r\n// dirty source".into(),
        })
        .unwrap();
        let target_id = open(&mut app, &destination);
        let dirty_target = "/* 😀 e\u{301} */ export const target = 99;\r\n// dirty target";
        app.dispatch(Command::Edit {
            id: target_id,
            expected_version: 1,
            text: dirty_target.into(),
        })
        .unwrap();
        app.dispatch(Command::Activate { id: source_id }).unwrap();
        // Target is already open: even invalid bytes on disk must not replace it.
        fs::write(&destination, [0, 255]).unwrap();
        let prefix = dirty_target[..dirty_target.find("target").unwrap()]
            .encode_utf16()
            .count() as u32;
        let mut found = target(destination.clone());
        found.start_character = prefix;
        found.end_character = prefix + 6;
        incoming(&mut app, found, 12);
        app.dispatch(Command::ChooseDefinition { index: 0 })
            .unwrap();
        idle(&mut app);
        assert_eq!(app.active, Some(target_id));
        assert_eq!(app.active_document().unwrap().text(), dirty_target);
        let selection = app.navigation.selection.as_ref().unwrap();
        let selected: String = dirty_target
            .chars()
            .skip(selection.range.start)
            .take(selection.range.len())
            .collect();
        assert_eq!(selected, "target");
        let index = selection.range.start;
        app.dispatch(Command::NavigateBack {
            id: target_id,
            expected_version: 2,
            character_index: index,
        })
        .unwrap();
        assert_eq!(app.active, Some(source_id));
        assert_eq!(app.navigation.selection.as_ref().unwrap().range, 12..12);
        app.dispatch(Command::NavigateForward {
            id: source_id,
            expected_version: 2,
            character_index: 12,
        })
        .unwrap();
        assert_eq!(app.active, Some(target_id));
        assert!(app.documents.iter().all(Document::is_dirty));
        assert_eq!(fs::read(&destination).unwrap(), [0, 255]);
        assert_eq!(
            fs::read_to_string(&source).unwrap(),
            "/* 😀 */ target();\r\n"
        );
    }
    #[test]
    fn invalid_range_never_opens_or_activates_a_tab() {
        let (_dir, mut app, _source, destination) = fixture();
        let id = app.active;
        let mut found = target(destination);
        found.start_character = 4;
        found.end_character = 5; // surrogate interior
        incoming(&mut app, found, 10);
        app.dispatch(Command::ChooseDefinition { index: 0 })
            .unwrap();
        idle(&mut app);
        assert_eq!(app.active, id);
        assert_eq!(app.documents.len(), 1);
        assert!(
            app.navigation
                .error
                .as_deref()
                .unwrap()
                .contains("inválida")
        );
    }
    #[test]
    fn edits_cancel_pending_destination_reads_and_discard_old_failures() {
        let (_dir, mut app, _source, destination) = fixture();
        let id = app.active.unwrap();
        incoming(&mut app, target(destination), 10);
        app.dispatch(Command::ChooseDefinition { index: 0 })
            .unwrap();
        app.dispatch(Command::Edit {
            id,
            expected_version: 1,
            text: "const edited = true;".into(),
        })
        .unwrap();
        idle(&mut app);
        assert_eq!(app.active, Some(id));
        assert_eq!(app.documents.len(), 1);
        let absent = app.workspace.as_ref().unwrap().join("absent.ts");
        incoming(&mut app, target(absent), 10);
        app.dispatch(Command::ChooseDefinition { index: 0 })
            .unwrap();
        app.dispatch(Command::CancelDefinition).unwrap();
        idle(&mut app);
        assert!(app.navigation.error.is_none());
        assert!(app.last_error.is_none());
    }
    #[test]
    fn outside_workspace_and_symlink_targets_are_rejected() {
        let (_dir, mut app, _source, _destination) = fixture();
        let outside = tempfile::tempdir().unwrap();
        let secret = outside.path().join("outside.ts");
        fs::write(&secret, "export const target = 1;").unwrap();
        let mut paths = vec![secret.clone()];
        #[cfg(unix)]
        {
            let link = app.workspace.as_ref().unwrap().join("linked.ts");
            std::os::unix::fs::symlink(&secret, &link).unwrap();
            paths.push(link);
        }
        for path in paths {
            incoming(&mut app, target(path), 10);
            app.dispatch(Command::ChooseDefinition { index: 0 })
                .unwrap();
            idle(&mut app);
            assert_eq!(app.documents.len(), 1);
            assert!(
                app.navigation
                    .error
                    .as_deref()
                    .unwrap()
                    .contains("fora do workspace")
            );
        }
    }
    #[test]
    fn read_only_navigation_cannot_hide_an_existing_dirty_buffer() {
        let (_dir, mut app, _source, destination) = fixture();
        let source = app.active.unwrap();
        let id = open(&mut app, &destination);
        let changed = "/* unsaved */ export const target = 84;\r\n";
        app.dispatch(Command::Edit {
            id,
            expected_version: 1,
            text: changed.into(),
        })
        .unwrap();
        app.dispatch(Command::Activate { id: source }).unwrap();
        let origin = app.navigation_position(source, 1, 0).unwrap();
        let error = app
            .activate_navigation_target(origin, target(destination), None, true)
            .unwrap_err();
        assert!(error.contains("alterações"));
        assert_eq!(app.active, Some(source));
        let doc = app.documents.iter().find(|d| d.id == id).unwrap();
        assert_eq!(doc.text(), changed);
        assert!(doc.is_dirty());
        assert!(!doc.is_read_only());
    }
    #[test]
    fn stale_history_does_not_apply_an_old_caret_or_reload() {
        let (_dir, mut app, _source, destination) = fixture();
        let source_id = app.active.unwrap();
        incoming(&mut app, target(destination), 10);
        app.dispatch(Command::ChooseDefinition { index: 0 })
            .unwrap();
        idle(&mut app);
        let target_id = app.active.unwrap();
        app.dispatch(Command::Edit {
            id: source_id,
            expected_version: 1,
            text: "const changed = 1;".into(),
        })
        .unwrap();
        assert!(
            app.dispatch(Command::NavigateBack {
                id: target_id,
                expected_version: 1,
                character_index: 0
            })
            .unwrap_err()
            .contains("obsoleta")
        );
        assert_eq!(app.active, Some(target_id));
        assert_eq!(
            app.documents
                .iter()
                .find(|d| d.id == source_id)
                .unwrap()
                .text(),
            "const changed = 1;"
        );
    }
    #[test]
    fn failed_buffer_sync_cancels_definition_choices_without_activating_target() {
        let (_dir, mut app, source, destination) = fixture();
        let id = open(&mut app, &source);
        incoming(&mut app, target(destination), 0);
        app.language.sync_failures.insert(
            source,
            codisseu_language::SyncFailure {
                version: 1,
                message: "controlled transport failure".into(),
            },
        );
        app.poll();
        assert_eq!(app.active, Some(id));
        assert!(app.navigation.choices.is_empty());
        assert!(!app.navigation.is_pending());
        assert!(
            app.navigation
                .error
                .as_ref()
                .unwrap()
                .contains("sincronização")
        );
        assert_eq!(app.documents.len(), 1);
    }
    #[test]
    fn speculative_results_never_open_a_target_or_replace_an_explicit_navigation() {
        let (_dir, mut app, source, destination) = fixture();
        let id = app.active.unwrap();
        let before = app.active_document().unwrap().selection();
        incoming(&mut app, target(destination.clone()), 10);
        let ctx = app.navigation.context.as_mut().unwrap();
        ctx.intent = Intent::Preview;
        ctx.phase = Phase::Requesting;
        app.navigation.choices.clear();
        app.language.definition = Some(codisseu_language::DefinitionResult {
            presentation: None,
            path: source.clone(),
            version: 1,
            line: 0,
            character: 11,
            targets: vec![target(destination.clone())],
        });
        assert!(app.poll_definition());
        assert!(!app.navigation.is_pending());
        assert!(app.navigation.preview.is_some());
        assert!(app.navigation.choices.is_empty());
        assert_eq!(app.active, Some(id));
        assert_eq!(app.documents.len(), 1);
        assert_eq!(app.active_document().unwrap().selection(), before);
        assert!(!app.navigation.can_back());
        app.dispatch(Command::CancelDefinitionPreview).unwrap();
        app.language.definition = Some(codisseu_language::DefinitionResult {
            presentation: None,
            path: source,
            version: 1,
            line: 0,
            character: 11,
            targets: vec![target(destination.clone())],
        });
        assert!(!app.poll_definition());
        assert!(app.navigation.preview.is_none());
        assert_eq!(app.active, Some(id));
        incoming(&mut app, target(destination), 10);
        app.dispatch(Command::CancelDefinitionPreview).unwrap();
        assert_eq!(
            app.navigation.choices.len(),
            1,
            "leaving a hover cannot cancel an explicit choice"
        );
    }

    #[test]
    fn preview_errors_are_silent_and_source_edits_invalidate_pending_and_resolved_results() {
        let (_dir, mut app, _source, destination) = fixture();
        incoming(&mut app, target(destination.clone()), 10);
        let ctx = app.navigation.context.as_mut().unwrap();
        ctx.intent = Intent::Preview;
        ctx.phase = Phase::Requesting;
        app.navigation.choices.clear();
        app.language.definition_error = Some("unsupported provider".into());
        let status = app.status.clone();
        assert!(app.poll_definition());
        assert!(app.navigation.error.is_none());
        assert!(app.last_error.is_none());
        assert_eq!(app.status, status);
        incoming(&mut app, target(destination), 10);
        let ctx = app.navigation.context.as_mut().unwrap();
        ctx.intent = Intent::Preview;
        ctx.phase = Phase::Requesting;
        app.navigation.choices.clear();
        let d = app.active_document().unwrap();
        app.dispatch(Command::Edit {
            id: d.id,
            expected_version: d.version,
            text: "/* changed */ target();".into(),
        })
        .unwrap();
        assert!(!app.navigation.is_preview());
        assert!(app.navigation.preview.is_none());
        assert!(!app.poll_definition());
    }
    #[test]
    fn chooser_targets_can_be_pinned_to_the_existing_versioned_preview_panel() {
        let (_dir, mut app, source, destination) = fixture();
        let origin = app.active.unwrap();
        incoming(&mut app, target(destination.clone()), 10);
        app.dispatch(Command::ShowDefinitionTargets).unwrap();
        assert!(app.navigation.choices.is_empty());
        idle(&mut app);
        assert_eq!(app.active, Some(origin));
        assert_eq!(app.references.items.len(), 1);
        assert_eq!(app.references.items[0].target.path, destination);
        app.dispatch(Command::ChooseReference {
            index: 0,
            id: origin,
            expected_version: 1,
            character_index: 10,
        })
        .unwrap();
        idle(&mut app);
        assert_eq!(app.active_document().unwrap().path, destination);
        assert_eq!(
            fs::read_to_string(source).unwrap(),
            "/* 😀 */ target();\r\n"
        );
    }
}
