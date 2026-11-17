//! Bounded, versioned view metadata. No text, caches or GUI types.
use super::*;
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct EditorSplit {
    pub horizontal: bool,
    pub ratio: f32,
}
impl Default for EditorSplit {
    fn default() -> Self {
        Self {
            horizontal: false,
            ratio: 0.5,
        }
    }
}
impl EditorSplit {
    pub(super) fn validate(self) -> Result<(), String> {
        if self.ratio.is_finite() && (0.2..=0.8).contains(&self.ratio) {
            Ok(())
        } else {
            Err("Proporção de grupos inválida".into())
        }
    }
}
pub(super) fn valid_scroll(scroll: [f32; 2]) -> bool {
    scroll
        .iter()
        .all(|x| x.is_finite() && (0.0..=100_000_000.0).contains(x))
}
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct EditorSession {
    schema_version: u32,
    split: EditorSplit,
    active_group: usize,
    groups: Vec<SavedGroup>,
}
#[derive(Debug, Serialize, Deserialize)]
struct SavedGroup {
    document: PathBuf,
    #[serde(default)]
    tabs: Option<Vec<PathBuf>>,
    positions: Vec<SavedPosition>,
}
#[derive(Debug, Serialize, Deserialize)]
struct SavedPosition {
    path: PathBuf,
    selection: [usize; 2],
    scroll: [f32; 2],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_anchor: Option<SourceScrollAnchor>,
}
impl EditorSession {
    pub(super) fn validate(&self, files: &[OpenedFile]) -> Result<(), String> {
        self.validate_paths(&files.iter().map(|f| f.path.clone()).collect::<Vec<_>>())
    }
    pub(super) fn validate_paths(&self, paths: &[PathBuf]) -> Result<(), String> {
        self.split.validate()?;
        if !matches!(self.schema_version, 1 | 2)
            || self.groups.is_empty()
            || self.groups.len() > MAX_EDITOR_VIEWS
            || self.active_group >= self.groups.len()
        {
            return Err("Composição de grupos inválida".into());
        }
        let mut union = std::collections::HashSet::new();
        for group in &self.groups {
            let known = |path: &PathBuf| paths.contains(path);
            let mut seen = std::collections::HashSet::new();
            if !known(&group.document)
                || group.positions.is_empty()
                || group.positions.len() > MAX_DOCUMENTS
            {
                return Err("Documento/posições de grupo inválidos".into());
            }
            let members = if self.schema_version == 2 {
                let tabs = group.tabs.as_ref().ok_or("Grupo sem lista de abas")?;
                if tabs.is_empty() || tabs.len() > MAX_DOCUMENTS {
                    return Err("Lista de abas inválida".into());
                }
                let mut members = std::collections::HashSet::new();
                for path in tabs {
                    if !known(path) || !members.insert(path) {
                        return Err("Aba duplicada ou desconhecida".into());
                    }
                    union.insert(path);
                }
                if !members.contains(&group.document) {
                    return Err("Aba ativa fora do grupo".into());
                }
                members
            } else {
                if group.tabs.is_some() {
                    return Err("Lista de abas exige editor_state2".into());
                }
                paths.iter().collect()
            };
            for p in &group.positions {
                if !members.contains(&p.path) || !seen.insert(&p.path) || !valid_scroll(p.scroll) {
                    return Err("Posição de grupo duplicada, desconhecida ou inválida".into());
                }
                if p.source_anchor.is_some_and(|anchor| !anchor.is_valid()) {
                    return Err("Âncora de leitura do grupo inválida".into());
                }
            }
            if !seen.contains(&group.document) {
                return Err("Grupo sem posição do documento ativo".into());
            }
        }
        if self.schema_version == 2 && union.len() != paths.len() {
            return Err("Documento carregado sem aba em nenhum grupo".into());
        }
        Ok(())
    }
}
impl AppService {
    pub fn editor_split(&self) -> EditorSplit {
        self.editor_split
    }
    pub(super) fn save_editor_state(&self) -> Option<EditorSession> {
        let mut groups = Vec::new();
        let mut active_group = 0;
        for view in &self.views {
            let documents: Vec<_> = view
                .tabs
                .iter()
                .filter_map(|id| {
                    self.documents
                        .iter()
                        .find(|d| d.id == *id && !d.is_untitled() && !d.is_read_only())
                })
                .collect();
            let Some(first) = documents.first() else {
                continue;
            };
            let document = documents
                .iter()
                .find(|d| d.id == view.document)
                .unwrap_or(first);
            if Some(view.id) == self.active_view {
                active_group = groups.len();
            }
            groups.push(SavedGroup {
                document: document.path.clone(),
                tabs: Some(documents.iter().map(|d| d.path.clone()).collect()),
                positions: view
                    .positions
                    .iter()
                    .filter_map(|(id, p)| {
                        documents
                            .iter()
                            .find(|d| d.id == *id)
                            .map(|d| SavedPosition {
                                path: d.path.clone(),
                                selection: [p.selection.anchor, p.selection.head],
                                scroll: p.scroll,
                                source_anchor: p.source_anchor,
                            })
                    })
                    .collect(),
            });
        }
        if groups.is_empty() {
            return None;
        }
        Some(EditorSession {
            schema_version: 2,
            split: self.editor_split,
            active_group,
            groups,
        })
    }
    pub(super) fn restore_editor_state(&mut self, state: EditorSession) {
        self.views.clear();
        self.editor_split = state.split;
        for group in state.groups {
            self.next_view += 1;
            let document = self
                .documents
                .iter()
                .find(|d| d.path == group.document)
                .unwrap()
                .id;
            let tabs: Vec<DocumentId> = if let Some(tabs) = group.tabs {
                tabs.iter()
                    .map(|path| self.documents.iter().find(|d| &d.path == path).unwrap().id)
                    .collect()
            } else {
                self.documents.iter().map(|d| d.id).collect()
            };
            let positions = group
                .positions
                .into_iter()
                .map(|p| {
                    let d = self.documents.iter().find(|d| d.path == p.path).unwrap();
                    (
                        d.id,
                        views::Position {
                            selection: Selection {
                                anchor: p.selection[0].min(d.len_chars()),
                                head: p.selection[1].min(d.len_chars()),
                            },
                            revision: d.buffer.revision(),
                            scroll: p.scroll,
                            restored: true,
                            source_anchor: p.source_anchor.map(|anchor| SourceScrollAnchor {
                                character: anchor.character.min(d.len_chars()),
                                offset: anchor.offset,
                            }),
                        },
                    )
                })
                .collect();
            self.views.push(DocumentView {
                id: self.next_view,
                document,
                tabs,
                positions,
            });
        }
        let v = &self.views[state.active_group];
        self.active_view = Some(v.id);
        self.active = Some(v.document);
        let selection = v.selection();
        self.document_mut(v.document)
            .unwrap()
            .buffer
            .set_selection(selection)
            .unwrap();
    }
}
