//! Two simultaneous document views; one canonical buffer/history per document.
use super::*;
pub type ViewId = u64;
pub const MAX_EDITOR_VIEWS: usize = 2;
/// Source identity of the first visible visual row, independent of lazy heights.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SourceScrollAnchor {
    pub character: usize,
    pub offset: f32,
}
impl SourceScrollAnchor {
    pub(super) fn is_valid(self) -> bool {
        self.offset.is_finite() && (0.0..=4096.0).contains(&self.offset)
    }
}
#[derive(Clone, Debug)]
pub(super) struct Position {
    pub(super) selection: Selection,
    pub(super) revision: u64,
    pub(super) scroll: [f32; 2],
    pub(super) restored: bool,
    pub(super) source_anchor: Option<SourceScrollAnchor>,
}
#[derive(Clone, Debug)]
pub struct DocumentView {
    pub id: ViewId,
    pub document: DocumentId,
    pub(super) tabs: Vec<DocumentId>,
    pub(super) positions: BTreeMap<DocumentId, Position>,
}
impl DocumentView {
    pub fn tabs(&self) -> &[DocumentId] {
        &self.tabs
    }
    pub fn has_restored_viewport(&self) -> bool {
        self.positions
            .get(&self.document)
            .is_some_and(|p| p.restored)
    }
    pub fn scroll(&self) -> [f32; 2] {
        self.positions
            .get(&self.document)
            .map_or([0.; 2], |p| p.scroll)
    }
    pub fn source_scroll_anchor(&self) -> Option<SourceScrollAnchor> {
        self.positions
            .get(&self.document)
            .and_then(|position| position.source_anchor)
    }
    pub fn selection(&self) -> Selection {
        self.positions
            .get(&self.document)
            .map_or(Selection::default(), |p| p.selection)
    }
}
impl AppService {
    pub fn document_views(&self) -> &[DocumentView] {
        &self.views
    }
    pub fn active_view(&self) -> Option<ViewId> {
        self.active_view
    }
    pub fn view(&self, id: ViewId) -> Option<&DocumentView> {
        self.views.iter().find(|v| v.id == id)
    }
    pub fn view_document(&self, id: ViewId) -> Option<&Document> {
        let view = self.view(id)?;
        self.documents.iter().find(|d| d.id == view.document)
    }
    pub(super) fn remap_views(&mut self, id: DocumentId, deltas: &[TextDelta]) {
        let Some(doc) = self.documents.iter().find(|d| d.id == id) else {
            return;
        };
        let revision = doc.buffer.revision();
        for view in &mut self.views {
            if let Some(p) = view.positions.get_mut(&id) {
                for delta in deltas {
                    p.selection = p.selection.mapped(delta);
                    if let Some(anchor) = &mut p.source_anchor {
                        anchor.character = Selection::caret(anchor.character).mapped(delta).head;
                    }
                }
                p.revision = revision;
            }
        }
    }
    pub(super) fn sync_views(&mut self) {
        if self.documents.is_empty() {
            self.views.clear();
            self.active_view = None;
            self.active = None;
            return;
        }
        for view in &mut self.views {
            view.tabs
                .retain(|id| self.documents.iter().any(|d| d.id == *id));
            if !view.tabs.contains(&view.document)
                && let Some(&id) = view.tabs.first()
            {
                view.document = id;
            }
        }
        self.views.retain(|v| !v.tabs.is_empty());
        if self.views.is_empty() {
            let document = self
                .active
                .filter(|id| self.documents.iter().any(|d| d.id == *id))
                .unwrap_or(self.documents[0].id);
            self.next_view += 1;
            self.views.push(DocumentView {
                id: self.next_view,
                document,
                tabs: self.documents.iter().map(|d| d.id).collect(),
                positions: BTreeMap::new(),
            });
            self.active_view = Some(self.next_view);
            self.active = Some(document);
        }
        if !self.views.iter().any(|v| Some(v.id) == self.active_view) {
            self.active_view = Some(self.views[0].id);
            self.active = Some(self.views[0].document);
        }
        let mut restore = None;
        let active = self
            .views
            .iter()
            .position(|v| Some(v.id) == self.active_view)
            .unwrap();
        if let Some(id) = self
            .active
            .filter(|id| self.documents.iter().any(|d| d.id == *id))
        {
            let view = &mut self.views[active];
            if !view.tabs.contains(&id) {
                view.tabs.push(id);
            }
            if view.document != id {
                view.document = id;
                restore = Some((view.id, id));
            }
        } else {
            self.active = Some(self.views[active].document);
        }
        // Semantic operations may load participants without displaying them. Keep
        // every canonical document reachable in one group; never hide dirty buffers.
        for doc in &self.documents {
            if !self.views.iter().any(|v| v.tabs.contains(&doc.id)) {
                self.views[active].tabs.push(doc.id);
            }
        }
        for view in &mut self.views {
            view.positions.retain(|id, _| view.tabs.contains(id));
            for &id in &view.tabs {
                if let Some(doc) = self.documents.iter().find(|d| d.id == id) {
                    view.positions.entry(id).or_insert(Position {
                        selection: doc.selection(),
                        revision: doc.buffer.revision(),
                        scroll: [0.; 2],
                        restored: false,
                        source_anchor: None,
                    });
                }
            }
            for (&id, position) in &mut view.positions {
                let Some(doc) = self.documents.iter().find(|d| d.id == id) else {
                    continue;
                };
                let revision = doc.buffer.revision();
                if position.revision != revision {
                    if let Some(change) = doc.last_change()
                        && position.revision + 1 == change.revision
                    {
                        if let Some(deltas) = doc.buffer.last_selection_map() {
                            for (range, inserted) in deltas {
                                let map = |p: usize| {
                                    if p < range.start {
                                        p
                                    } else if p >= range.end {
                                        p - range.len() + inserted
                                    } else {
                                        range.start + inserted
                                    }
                                };
                                position.selection = Selection {
                                    anchor: map(position.selection.anchor),
                                    head: map(position.selection.head),
                                };
                                if let Some(anchor) = &mut position.source_anchor {
                                    anchor.character = map(anchor.character);
                                }
                            }
                        } else {
                            let map = |p: usize| {
                                if p < change.old_chars.start {
                                    p
                                } else if p >= change.old_chars.end {
                                    p - change.old_chars.len() + change.new_chars.len()
                                } else {
                                    change.new_chars.end
                                }
                            };
                            position.selection = Selection {
                                anchor: map(position.selection.anchor),
                                head: map(position.selection.head),
                            };
                            if let Some(anchor) = &mut position.source_anchor {
                                anchor.character = map(anchor.character);
                            }
                        }
                    }
                    position.selection.anchor = position.selection.anchor.min(doc.len_chars());
                    position.selection.head = position.selection.head.min(doc.len_chars());
                    if let Some(anchor) = &mut position.source_anchor {
                        anchor.character = anchor.character.min(doc.len_chars());
                    }
                    position.revision = revision;
                }
            }
        }
        if let Some((owner, id)) = restore
            && let Some(selection) = self.view(owner).map(DocumentView::selection)
            && let Ok(doc) = self.document_mut(id)
        {
            let _ = doc.buffer.set_selection(selection);
        }
        if let Some(view) = self
            .views
            .iter_mut()
            .find(|v| Some(v.id) == self.active_view)
            && let Some(doc) = self.documents.iter().find(|d| d.id == view.document)
        {
            let scroll = view.positions.get(&doc.id).map_or([0.; 2], |p| p.scroll);
            let restored = view.positions.get(&doc.id).is_some_and(|p| p.restored);
            let source_anchor = view.positions.get(&doc.id).and_then(|p| p.source_anchor);
            view.positions.insert(
                doc.id,
                Position {
                    selection: doc.selection(),
                    revision: doc.buffer.revision(),
                    scroll,
                    restored,
                    source_anchor,
                },
            );
        }
    }
    pub(super) fn create_view(&mut self) -> Result<(), String> {
        self.sync_views();
        if self.views.len() >= MAX_EDITOR_VIEWS {
            return Err("Limite de duas visualizações simultâneas".into());
        }
        let document = self
            .active
            .ok_or("Abra um documento antes de dividir o editor")?;
        let mut positions = BTreeMap::new();
        let doc = self.active_document().unwrap();
        positions.insert(
            document,
            Position {
                selection: doc.selection(),
                revision: doc.buffer.revision(),
                scroll: [0.; 2],
                restored: false,
                source_anchor: None,
            },
        );
        self.next_view += 1;
        self.views.push(DocumentView {
            id: self.next_view,
            document,
            tabs: vec![document],
            positions,
        });
        self.activate_view(self.next_view)
    }
    pub(super) fn activate_view(&mut self, id: ViewId) -> Result<(), String> {
        self.sync_views();
        let view = self.view(id).ok_or("Visualização não encontrada")?;
        let document = view.document;
        let selection = view.selection();
        self.execute(Command::Activate { id: document })?;
        self.active_view = Some(id);
        self.document_mut(document)?
            .buffer
            .set_selection(selection)?;
        Ok(())
    }
    pub fn tab_close_needs_confirmation(&self, view: ViewId, id: DocumentId) -> bool {
        self.view(view).is_some_and(|v| v.tabs.contains(&id))
            && self.views.iter().filter(|v| v.tabs.contains(&id)).count() == 1
            && self.documents.iter().any(|d| d.id == id && d.is_dirty())
    }
    pub(super) fn activate_tab(&mut self, view: ViewId, id: DocumentId) -> Result<(), String> {
        if !self.view(view).is_some_and(|v| v.tabs.contains(&id)) {
            return Err("Aba não pertence ao grupo".into());
        }
        self.activate_view(view)?;
        self.execute(Command::Activate { id })
    }
    pub(super) fn show_document_in_view(
        &mut self,
        view: ViewId,
        id: DocumentId,
    ) -> Result<(), String> {
        // Validate before changing membership. This selects another view of the
        // canonical buffer; no filesystem request or duplicate document is made.
        if !self.documents.iter().any(|document| document.id == id) {
            return Err("Documento não encontrado".into());
        }
        let group = self
            .views
            .iter_mut()
            .find(|group| group.id == view)
            .ok_or("Visualização não encontrada")?;
        if !group.tabs.contains(&id) {
            group.tabs.push(id);
        }
        self.activate_tab(view, id)
    }
    pub(super) fn reorder_tab(
        &mut self,
        view: ViewId,
        id: DocumentId,
        index: usize,
    ) -> Result<(), String> {
        if self.pending.values().any(|p| *p == Pending::Restore) {
            return Err("Aguarde a restauração da sessão antes de mover abas".into());
        }
        let group = self
            .views
            .iter_mut()
            .find(|v| v.id == view)
            .ok_or("Grupo não encontrado")?;
        let source = group
            .tabs
            .iter()
            .position(|&tab| tab == id)
            .ok_or("Aba não pertence ao grupo")?;
        if index >= group.tabs.len() {
            return Err("Posição de aba inválida".into());
        }
        // Rotate IDs in place. The canonical document, caches and per-tab
        // positions keep their identity, even for passive groups or dirty files.
        if source < index {
            group.tabs[source..=index].rotate_left(1);
        } else if index < source {
            group.tabs[index..=source].rotate_right(1);
        }
        Ok(())
    }
    pub(super) fn close_tab(
        &mut self,
        view: ViewId,
        id: DocumentId,
        discard: bool,
    ) -> Result<(), String> {
        let group = self
            .views
            .iter()
            .position(|v| v.id == view)
            .ok_or("Grupo não encontrado")?;
        let index = self.views[group]
            .tabs
            .iter()
            .position(|&d| d == id)
            .ok_or("Aba não pertence ao grupo")?;
        let was_selected = self.views[group].document == id;
        let last = self.views.iter().filter(|v| v.tabs.contains(&id)).count() == 1;
        // Validate/close the canonical document before any membership mutation.
        if last {
            self.execute(Command::Close { id, discard })?;
        }
        let v = &mut self.views[group];
        v.tabs.remove(index);
        v.positions.remove(&id);
        if !v.tabs.is_empty() && v.document == id {
            v.document = v.tabs[index.min(v.tabs.len() - 1)];
        }
        let changed_active = self.active_view == Some(view) && was_selected;
        if v.tabs.is_empty() {
            self.views.remove(group);
        }
        if self.views.is_empty() {
            self.active_view = None;
            self.active = None;
        } else if changed_active {
            let v = self
                .views
                .iter()
                .find(|v| v.id == view)
                .unwrap_or(&self.views[0]);
            let (owner, document, selection) = (v.id, v.document, v.selection());
            self.active_view = Some(owner);
            self.execute(Command::Activate { id: document })?;
            self.document_mut(document)?
                .buffer
                .set_selection(selection)?;
        }
        self.status = "Aba fechada no grupo".into();
        Ok(())
    }
    pub(super) fn close_view(&mut self, id: ViewId) -> Result<(), String> {
        if self.views.len() <= 1 {
            return Err("A última visualização não pode ser fechada".into());
        }
        let index = self
            .views
            .iter()
            .position(|v| v.id == id)
            .ok_or("Visualização não encontrada")?;
        self.sync_views();
        let removed = self.views.remove(index);
        let remaining = &mut self.views[0];
        for document in removed.tabs {
            if !remaining.tabs.contains(&document) {
                remaining.tabs.push(document);
                if let Some(p) = removed.positions.get(&document) {
                    remaining.positions.insert(document, p.clone());
                }
            }
        }
        if self.active_view == Some(id) {
            let (owner, document, selection) =
                (remaining.id, remaining.document, remaining.selection());
            self.active_view = Some(owner);
            self.execute(Command::Activate { id: document })?;
            self.document_mut(document)?
                .buffer
                .set_selection(selection)?;
        }
        Ok(())
    }
}
