//! Shared code actions and Rust inlay request slots and opaque resolve cache; never execute commands.
use super::*;
use crate::{CodeActionEditResult, CodeActionsResult, InlayHintsResult, RustCapabilities};
use std::path::Path;
struct Context {
    id: u64,
    path: PathBuf,
    canonical: PathBuf,
    version: u64,
    text: String,
    range: RenameRange,
    started: Instant,
}
struct PendingAction {
    context: Context,
    resolve: Option<u64>,
}
struct Cached {
    action: crate::rust_actions::Action,
    path: PathBuf,
    canonical: PathBuf,
    version: u64,
}
#[derive(Default)]
pub(super) struct State {
    capabilities: RustCapabilities,
    resolve_supported: bool,
    actions: Option<PendingAction>,
    hints: Option<Context>,
    cache: Vec<Cached>,
    pub(super) diagnostics: HashMap<PathBuf, (u64, Vec<Value>)>,
}
impl Connection {
    pub(super) fn has_code_action_token(&self, token: u64) -> bool {
        self.rust_features
            .cache
            .iter()
            .any(|c| c.action.item.token == token)
    }
    pub(super) fn cancel_rust_actions(&mut self, clear: bool) {
        if let Some(pending) = self.rust_features.actions.take() {
            let prefix = if pending.resolve.is_some() {
                "code-action-resolve"
            } else {
                "code-actions"
            };
            let _=self.send(json!({"jsonrpc":"2.0","method":"$/cancelRequest","params":{"id":format!("{prefix}-{}",pending.context.id)}}));
        }
        if clear {
            self.rust_features.cache.clear();
        }
    }
    pub(super) fn cancel_rust_hints(&mut self) {
        if let Some(pending) = self.rust_features.hints.take() {
            let _=self.send(json!({"jsonrpc":"2.0","method":"$/cancelRequest","params":{"id":format!("inlay-hints-{}",pending.id)}}));
        }
    }
    pub(super) fn rust_capabilities(&mut self, value: &Value) -> Option<Event> {
        self.rust_features.capabilities = RustCapabilities {
            code_actions: crate::formatting::supported(
                value.pointer("/result/capabilities/codeActionProvider"),
            ),
            inlay_hints: matches!(self.language, Language::Rust | Language::Python)
                && crate::formatting::supported(
                    value.pointer("/result/capabilities/inlayHintProvider"),
                ),
        };
        self.rust_features.resolve_supported = value
            .pointer("/result/capabilities/codeActionProvider/resolveProvider")
            == Some(&Value::Bool(true));
        Some(Event::RustCapabilities {
            language: self.language,
            capabilities: self.rust_features.capabilities.clone(),
        })
    }
    pub(super) fn request_rust_feature(
        &mut self,
        id: u64,
        path: PathBuf,
        version: u64,
        range: RenameRange,
        document: &Document,
        hints: bool,
    ) -> Result<(), String> {
        if (hints && !matches!(self.language, Language::Rust | Language::Python))
            || document.language != self.language
            || self.state != ProviderState::Ready
        {
            return Err("Provider de linguagem não está pronto".into());
        }
        let supported = if hints {
            self.rust_features.capabilities.inlay_hints
        } else {
            self.rust_features.capabilities.code_actions
        };
        if !supported {
            return Err("Provider não oferece este recurso".into());
        }
        if document.version != version || self.opened.get(&document.path) != Some(&version) {
            return Err("Documento ainda não sincronizado na versão atual".into());
        }
        crate::rename::validate_range(&document.text, &range)?;
        if (range.start_line, range.start_character) > (range.end_line, range.end_character) {
            return Err("Faixa invertida".into());
        }
        if hints && range.end_line.saturating_sub(range.start_line) > 2000 {
            return Err("Faixa inlay limitada a2000linhas".into());
        }
        let canonical = document.path.clone();
        let mut params = json!({"textDocument":{"uri":protocol::file_uri(&canonical)?},"range":{"start":{"line":range.start_line,"character":range.start_character},"end":{"line":range.end_line,"character":range.end_character}}});
        if !hints {
            let diagnostics = self
                .rust_features
                .diagnostics
                .get(&canonical)
                .filter(|(v, _)| *v == version)
                .map(|(_, ds)| {
                    ds.iter()
                        .filter(|d| {
                            let start = (
                                d["range"]["start"]["line"].as_u64().unwrap_or(u64::MAX),
                                d["range"]["start"]["character"]
                                    .as_u64()
                                    .unwrap_or(u64::MAX),
                            );
                            let end = (
                                d["range"]["end"]["line"].as_u64().unwrap_or(0),
                                d["range"]["end"]["character"].as_u64().unwrap_or(0),
                            );
                            start <= (u64::from(range.end_line), u64::from(range.end_character))
                                && end
                                    >= (
                                        u64::from(range.start_line),
                                        u64::from(range.start_character),
                                    )
                        })
                        .cloned()
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            params["context"] =
                json!({"diagnostics":diagnostics,"triggerKind":1,"only":["quickfix"]});
        }
        let prefix = if hints { "inlay-hints" } else { "code-actions" };
        self.send(json!({"jsonrpc":"2.0","id":format!("{prefix}-{id}"),"method":if hints{"textDocument/inlayHint"}else{"textDocument/codeAction"},"params":params}))?;
        let context = Context {
            id,
            path,
            canonical,
            version,
            text: document.text.clone(),
            range,
            started: Instant::now(),
        };
        if hints {
            self.rust_features.hints = Some(context);
        } else {
            self.rust_features.actions = Some(PendingAction {
                context,
                resolve: None,
            });
        }
        Ok(())
    }
    pub(super) fn request_rust_resolve(
        &mut self,
        id: u64,
        token: u64,
        documents: &HashMap<PathBuf, Document>,
        root: &Path,
    ) -> Result<Option<Event>, String> {
        let cached = self
            .rust_features
            .cache
            .iter()
            .find(|c| c.action.item.token == token)
            .ok_or("Ação obsoleta ou desconhecida")?;
        let doc = documents
            .get(&cached.canonical)
            .ok_or("Documento de ação não está sincronizado")?;
        if doc.version != cached.version
            || self.opened.get(&cached.canonical) != Some(&cached.version)
        {
            return Err("Ação de versão obsoleta".into());
        }
        if let Some(reason) = &cached.action.item.disabled_reason {
            return Err(reason.clone());
        }
        let original = cached.action.original.clone();
        let context = Context {
            id,
            path: cached.path.clone(),
            canonical: cached.canonical.clone(),
            version: cached.version,
            text: doc.text.clone(),
            range: RenameRange {
                start_line: 0,
                start_character: 0,
                end_line: 0,
                end_character: 0,
            },
            started: Instant::now(),
        };
        if original.get("edit").is_some_and(|v| !v.is_null()) {
            let result = decode_resolved(
                &original,
                token,
                &cached.action.item.title,
                &context,
                documents,
                root,
            );
            return Ok(Some(Event::CodeActionEdit { id, result }));
        }
        if !self.rust_features.resolve_supported {
            return Err("Provider não resolve ações de código".into());
        }
        self.send(json!({"jsonrpc":"2.0","id":format!("code-action-resolve-{id}"),"method":"codeAction/resolve","params":original}))?;
        self.rust_features.actions = Some(PendingAction {
            context,
            resolve: Some(token),
        });
        Ok(None)
    }
    /// Handle a feature response before the general initialize/notification logic.
    pub(super) fn rust_feature_response(
        &mut self,
        value: &Value,
        documents: &HashMap<PathBuf, Document>,
        root: Option<&Path>,
    ) -> Option<Event> {
        if value.get("method").is_some() {
            return None;
        }
        if self
            .rust_features
            .hints
            .as_ref()
            .is_some_and(|p| value.get("id") == Some(&json!(format!("inlay-hints-{}", p.id))))
        {
            let pending = self.rust_features.hints.take().unwrap();
            let id = pending.id;
            let result = response_result(value).and_then(|v| {
                current_document(&pending, documents)?;
                let (items, truncated) =
                    crate::rust_inlay::decode(Some(v), &pending.text, &pending.range)?;
                Ok(InlayHintsResult {
                    path: pending.path,
                    version: pending.version,
                    range: pending.range,
                    items,
                    truncated,
                })
            });
            return Some(Event::InlayHints { id, result });
        }
        let pending = self.rust_features.actions.as_ref()?;
        let prefix = if pending.resolve.is_some() {
            "code-action-resolve"
        } else {
            "code-actions"
        };
        if value.get("id") != Some(&json!(format!("{prefix}-{}", pending.context.id))) {
            return None;
        }
        let pending = self.rust_features.actions.take().unwrap();
        let id = pending.context.id;
        if let Some(token) = pending.resolve {
            let result = response_result(value).and_then(|v| {
                current_document(&pending.context, documents)?;
                let cached = self
                    .rust_features
                    .cache
                    .iter()
                    .find(|c| c.action.item.token == token)
                    .ok_or("Ação de código já invalidada")?;
                decode_resolved(
                    v,
                    token,
                    &cached.action.item.title,
                    &pending.context,
                    documents,
                    root.ok_or("Workspace ausente")?,
                )
            });
            Some(Event::CodeActionEdit { id, result })
        } else {
            let result = response_result(value).and_then(|v| {
                current_document(&pending.context, documents)?;
                let adapted;
                let v = if self.typescript_action_adapter {
                    adapted = Value::Array(
                        v.as_array()
                            .map(|items| {
                                items
                                    .iter()
                                    .map(crate::rust_actions::edit_only_typescript_action)
                                    .collect()
                            })
                            .unwrap_or_default(),
                    );
                    if v.is_null() || v.is_array() {
                        &adapted
                    } else {
                        v
                    }
                } else {
                    v
                };
                let contextual;
                let v = if self.phpactor_completion
                    && let Some(items) = v.as_array()
                {
                    contextual = Value::Array(
                        items
                            .iter()
                            .filter(|a| {
                                crate::rust_actions::phpactor_contextual(
                                    a,
                                    &pending.context.canonical,
                                    &pending.context.range,
                                )
                            })
                            .cloned()
                            .collect(),
                    );
                    &contextual
                } else {
                    v
                };
                let (actions, truncated) =
                    crate::rust_actions::list(Some(v), self.rust_features.resolve_supported)?;
                let items = actions.iter().map(|a| a.item.clone()).collect();
                self.rust_features.cache = actions
                    .into_iter()
                    .map(|action| Cached {
                        action,
                        path: pending.context.path.clone(),
                        canonical: pending.context.canonical.clone(),
                        version: pending.context.version,
                    })
                    .collect();
                Ok(CodeActionsResult {
                    path: pending.context.path,
                    version: pending.context.version,
                    range: pending.context.range,
                    items,
                    truncated,
                })
            });
            Some(Event::CodeActions { id, result })
        }
    }
    pub(super) fn rust_feature_deadlines(&mut self) -> Vec<Event> {
        let mut events = vec![];
        if self
            .rust_features
            .actions
            .as_ref()
            .is_some_and(|p| p.context.started.elapsed() > Duration::from_secs(10))
        {
            let p = self.rust_features.actions.as_ref().unwrap();
            let id = p.context.id;
            let resolve = p.resolve.is_some();
            self.cancel_rust_actions(false);
            events.push(if resolve {
                Event::CodeActionEdit {
                    id,
                    result: Err("Timeout de resolução de ação após10segundos".into()),
                }
            } else {
                Event::CodeActions {
                    id,
                    result: Err("Timeout de ações de código após10segundos".into()),
                }
            });
        }
        if self
            .rust_features
            .hints
            .as_ref()
            .is_some_and(|p| p.started.elapsed() > Duration::from_secs(10))
        {
            let id = self.rust_features.hints.as_ref().unwrap().id;
            self.cancel_rust_hints();
            events.push(Event::InlayHints {
                id,
                result: Err("Timeout de inlay hints após10segundos".into()),
            });
        }
        events
    }
}
fn response_result(value: &Value) -> Result<&Value, String> {
    if let Some(error) = value.get("error") {
        return Err(format!(
            "Recurso de linguagem falhou: {}",
            error.to_string().chars().take(2048).collect::<String>()
        ));
    }
    value
        .get("result")
        .ok_or("Resposta de linguagem sem result".into())
}
fn current_document(
    context: &Context,
    documents: &HashMap<PathBuf, Document>,
) -> Result<(), String> {
    if documents
        .get(&context.canonical)
        .is_none_or(|d| d.version != context.version || d.text != context.text)
    {
        Err("Resposta de linguagem obsoleta".into())
    } else {
        Ok(())
    }
}
fn decode_resolved(
    value: &Value,
    token: u64,
    title: &str,
    context: &Context,
    documents: &HashMap<PathBuf, Document>,
    root: &Path,
) -> Result<CodeActionEditResult, String> {
    if value.get("title").and_then(Value::as_str) != Some(title) {
        return Err("Resolução alterou a identidade da ação".into());
    }
    let language = Language::for_path(&context.path).ok_or("Linguagem não suportada")?;
    let changes = crate::rust_actions::edits_for_language(value, root, language)?;
    for file in &changes {
        if let Some(doc) = documents.get(&file.path) {
            if file.version.is_some_and(|v| v != doc.version) {
                return Err("Quick fix contém versão obsoleta de documento aberto".into());
            }
            for edit in &file.edits {
                crate::rename::validate_range(
                    &doc.text,
                    &RenameRange {
                        start_line: edit.start_line,
                        start_character: edit.start_character,
                        end_line: edit.end_line,
                        end_character: edit.end_character,
                    },
                )?;
            }
        } else if file.version.is_some_and(|v| v != 0) {
            return Err("Quick fix contém versão desconhecida de documento fechado".into());
        }
    }
    Ok(CodeActionEditResult {
        path: context.path.clone(),
        version: context.version,
        token,
        title: title.into(),
        changes,
    })
}
