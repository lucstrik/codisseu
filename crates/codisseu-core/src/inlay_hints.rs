//! Background, versioned Rust adornments. They never edit a document or occupy a foreground job.
use super::*;
use codisseu_language::InlayHint;

#[derive(Default)]
pub struct InlayHintState {
    pub items: Vec<InlayHint>,
    pub error: Option<String>,
    pub truncated: bool,
    pub revision: u64,
    context: Option<Context>,
}
struct Context {
    id: DocumentId,
    path: PathBuf,
    version: u64,
    range: RenameRange,
    versions: Vec<(DocumentId, u64)>,
    pending: bool,
    language: Language,
}
impl InlayHintState {
    pub fn anchor(&self) -> Option<(DocumentId, u64)> {
        self.context.as_ref().map(|c| (c.id, c.version))
    }
    pub fn is_pending(&self) -> bool {
        self.context.as_ref().is_some_and(|c| c.pending)
    }
    pub fn requested_range(&self) -> Option<&RenameRange> {
        self.context.as_ref().map(|c| &c.range)
    }
}
impl AppService {
    pub(super) fn cancel_inlay_hints(&mut self) {
        if self.inlay_hints.context.take().is_some() {
            let _ = self.language.dispatch(LanguageCommand::CancelInlayHints);
        }
        if !self.inlay_hints.items.is_empty() {
            self.inlay_hints.revision = self.inlay_hints.revision.wrapping_add(1);
        }
        self.inlay_hints.items.clear();
        self.inlay_hints.truncated = false;
    }
    pub(super) fn request_inlay_hints(
        &mut self,
        id: DocumentId,
        version: u64,
        lines: Range<usize>,
    ) -> Result<(), String> {
        let doc = self
            .active_document()
            .filter(|d| d.id == id && d.version == version && !d.is_read_only())
            .ok_or("Inlay hints: documento mudou ou é somente leitura")?;
        doc.ensure_language_size()?;
        let language = Language::for_path(&doc.path)
            .filter(|l| matches!(l, Language::Rust | Language::Python))
            .ok_or("Dicas inline indisponíveis nesta linguagem")?;
        if !self.inlay_hints_enabled(id) {
            return Err("Dicas inline desativadas nas preferências".into());
        }
        let snapshot = doc.snapshot();
        if lines.start >= lines.end || lines.end > snapshot.len_lines() || lines.len() > 2000 {
            return Err("Faixa de inlay hints inválida ou excessiva".into());
        }
        let end = lines.end - 1;
        let last = snapshot.line(end).to_string();
        let range = RenameRange {
            start_line: lines.start as u32,
            start_character: 0,
            end_line: end as u32,
            end_character: last.trim_end_matches(['\r', '\n']).encode_utf16().count() as u32,
        };
        let path = doc.path.clone();
        self.sync_documents();
        if self.language_versions.get(&path) != Some(&version) {
            return Err("Aguardando sincronização para inlay hints".into());
        }
        self.language.dispatch(LanguageCommand::InlayHints {
            path: path.clone(),
            version,
            range: range.clone(),
        })?;
        self.inlay_hints.context = Some(Context {
            id,
            path,
            version,
            range,
            versions: self.documents.iter().map(|d| (d.id, d.version)).collect(),
            pending: true,
            language,
        });
        self.inlay_hints.error = None;
        self.inlay_hints.items.clear();
        self.inlay_hints.revision = self.inlay_hints.revision.wrapping_add(1);
        Ok(())
    }
    pub(super) fn poll_inlay_hints(&mut self) -> bool {
        let Some(c) = &self.inlay_hints.context else {
            return false;
        };
        if !self.inlay_hints_enabled(c.id)
            || self.active_document().is_none_or(|d| {
                d.id != c.id || d.version != c.version || d.path != c.path || d.is_read_only()
            })
            || c.versions
                != self
                    .documents
                    .iter()
                    .map(|d| (d.id, d.version))
                    .collect::<Vec<_>>()
            || !self
                .language
                .statuses
                .iter()
                .any(|s| s.language == c.language && s.state == ProviderState::Ready)
        {
            self.cancel_inlay_hints();
            return true;
        }
        if let Some(error) = self.language.inlay_hints_error.take() {
            self.cancel_inlay_hints();
            self.inlay_hints.error = Some(error);
            return true;
        }
        if !c.pending {
            return false;
        }
        if let Some(result) = self.language.inlay_hints.take() {
            let c = self.inlay_hints.context.as_ref().unwrap();
            if result.path != c.path || result.version != c.version || result.range != c.range {
                self.cancel_inlay_hints();
                return true;
            }
            self.inlay_hints.items = result.items;
            self.inlay_hints.truncated = result.truncated;
            self.inlay_hints.revision = self.inlay_hints.revision.wrapping_add(1);
            self.inlay_hints.context.as_mut().unwrap().pending = false;
            return true;
        }
        if !self.language.inlay_hints_pending() {
            self.cancel_inlay_hints();
            return true;
        }
        false
    }
}
