//! Versioned completion and atomic, worker-built edits. Never writes a document.
use super::*;

#[derive(Clone, Debug, Serialize)]
pub struct CompletionSelection {
    pub document_id: DocumentId,
    pub version: u64,
    pub character_index: usize,
    pub trigger_signature: bool,
}

#[derive(Default)]
pub struct CompletionState {
    pub items: Vec<CompletionItem>,
    pub is_incomplete: bool,
    pub truncated: bool,
    pub error: Option<String>,
    pub selection: Option<CompletionSelection>,
    context: Option<Context>,
    next_token: u64,
}
struct Context {
    token: u64,
    id: DocumentId,
    version: u64,
    caret: usize,
    prefix_start: usize,
    line: u32,
    character: u32,
    versions: Vec<(DocumentId, u64)>,
    phase: Phase,
    php_callable: bool,
}
#[derive(PartialEq)]
enum Phase {
    Requesting,
    Choosing,
    Applying,
}
impl CompletionState {
    pub fn is_pending(&self) -> bool {
        self.context
            .as_ref()
            .is_some_and(|c| c.phase != Phase::Choosing)
    }
    pub fn is_applying(&self) -> bool {
        self.context
            .as_ref()
            .is_some_and(|c| c.phase == Phase::Applying)
    }
    pub fn is_open(&self) -> bool {
        self.context.is_some()
    }
    pub fn anchor(&self) -> Option<(DocumentId, u64, usize)> {
        self.context
            .as_ref()
            .map(|c| (c.id, c.version, c.prefix_start))
    }
    pub(super) fn context_token(&self) -> Option<u64> {
        self.context.as_ref().map(|c| c.token)
    }
}
impl AppService {
    pub(super) fn cancel_completion(&mut self) {
        let requested = self.completion.context.take().is_some();
        self.completion.items.clear();
        self.completion.selection = None;
        self.completion.is_incomplete = false;
        self.completion.truncated = false;
        if requested {
            let _ = self.language.dispatch(LanguageCommand::CancelCompletion);
        }
    }
    pub(super) fn completion_failed(&mut self, error: String) {
        self.cancel_completion();
        self.completion.error = Some(error.clone());
        self.last_error = Some(error.clone());
        self.status = error;
    }
    fn completion_current(&self) -> bool {
        self.completion.context.as_ref().is_some_and(|c| {
            self.active == Some(c.id)
                && c.versions.iter().all(|(id, version)| {
                    self.documents
                        .iter()
                        .any(|d| d.id == *id && d.version == *version)
                })
        })
    }
    pub(super) fn request_completion(
        &mut self,
        id: DocumentId,
        version: u64,
        caret: usize,
    ) -> Result<(), String> {
        let doc = self
            .active_document()
            .filter(|d| d.id == id && d.version == version)
            .ok_or("Sugestões: documento ativo ou versão mudou")?;
        doc.ensure_language_size()?;
        let (line, character) =
            scalar_lsp_position(doc.text(), caret).ok_or("Sugestões: cursor inválido")?;
        let language = Language::for_path(&doc.path).ok_or("Sem provider para este documento")?;
        if !self
            .language
            .statuses
            .iter()
            .any(|s| s.language == language && s.state == ProviderState::Ready)
        {
            return Err("Inicie o provider deste documento antes de solicitar sugestões".into());
        }
        let path = doc.path.clone();
        let prefix_start = prefix_start(doc.text(), caret);
        self.cancel_navigation();
        self.cancel_completion();
        self.cancel_references();
        self.cancel_rename();
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
            return Err("Aguarde a sincronização dos buffers antes de solicitar sugestões".into());
        }
        self.language.dispatch(LanguageCommand::Completion {
            path,
            version,
            line,
            character,
        })?;
        self.completion.next_token += 1;
        self.completion.context = Some(Context {
            token: self.completion.next_token,
            id,
            version,
            caret,
            prefix_start,
            line,
            character,
            versions: self.documents.iter().map(|d| (d.id, d.version)).collect(),
            phase: Phase::Requesting,
            php_callable: false,
        });
        self.completion.error = None;
        self.status = "Consultando sugestões…".into();
        Ok(())
    }
    pub(super) fn poll_completion(&mut self) -> bool {
        let Some(c) = self.completion.context.as_ref() else {
            return false;
        };
        if !self.completion_current() {
            self.cancel_completion();
            return true;
        }
        if c.phase != Phase::Requesting {
            return false;
        }
        if let Some(error) = self.language.completion_error.take() {
            self.completion_failed(error);
            return true;
        }
        let Some(result) = self.language.completion.take() else {
            return false;
        };
        let c = self.completion.context.as_ref().unwrap();
        let doc = self.active_document().unwrap();
        if result.path != doc.path
            || result.version != c.version
            || result.line != c.line
            || result.character != c.character
        {
            self.cancel_completion();
            return true;
        }
        let prefix: String = doc
            .text()
            .chars()
            .skip(c.prefix_start)
            .take(c.caret - c.prefix_start)
            .collect();
        let prefix = prefix.to_lowercase();
        let mut items: Vec<_> = result
            .items
            .into_iter()
            .filter(|item| {
                let filter = item
                    .filter_text
                    .as_deref()
                    .unwrap_or(&item.label)
                    .to_lowercase();
                // Resource names and qualified identifiers contain separators.
                // An explicit provider range can begin before the last word.
                let edit_prefix = item.text_edit.as_ref().and_then(|edit| {
                    let start =
                        utf16_byte_offset(doc.text(), edit.start_line, edit.start_character)?;
                    let end = utf16_byte_offset(doc.text(), c.line, c.character)?;
                    doc.text().get(start..end).map(str::to_lowercase)
                });
                edit_prefix.as_ref().is_some_and(|p| filter.starts_with(p))
                    || filter.starts_with(&prefix)
                    || (prefix.starts_with('$')
                        && filter.starts_with(prefix.trim_start_matches('$')))
            })
            .collect();
        items.sort_by(|a, b| {
            a.sort_text
                .as_deref()
                .unwrap_or(&a.label)
                .cmp(b.sort_text.as_deref().unwrap_or(&b.label))
        });
        self.completion.items = items;
        self.completion.is_incomplete = result.is_incomplete;
        self.completion.truncated = result.truncated;
        if self.completion.items.is_empty() {
            self.cancel_completion();
            self.status = "Nenhuma sugestão nesta posição".into();
        } else {
            self.completion.context.as_mut().unwrap().phase = Phase::Choosing;
            self.status = format!("{} sugestão(ões)", self.completion.items.len());
        }
        true
    }
    pub(super) fn accept_completion(&mut self, index: usize, replace: bool) -> Result<(), String> {
        if !self.completion_current() {
            return Err("Sugestão obsoleta: documento mudou".into());
        }
        let c = self
            .completion
            .context
            .as_ref()
            .ok_or("Solicite sugestões primeiro")?;
        if c.phase != Phase::Choosing {
            return Err("Aguarde as sugestões".into());
        }
        let item = self
            .completion
            .items
            .get(index)
            .ok_or("Sugestão inválida")?;
        if let Some(reason) = &item.unsupported_reason {
            return Err(format!("Sugestão indisponível: {reason}"));
        }
        let doc = self.active_document().unwrap();
        let requested_edit = if replace {
            item.text_edit.clone()
        } else {
            item.insert_edit.clone().or_else(|| item.text_edit.clone())
        };
        let primary = if let Some(edit) = requested_edit {
            edit
        } else {
            let (start_line, start_character) =
                scalar_lsp_position(doc.text(), c.prefix_start).ok_or("Prefixo inválido")?;
            let suffix = if replace {
                doc.text()
                    .chars()
                    .skip(c.caret)
                    .take_while(|ch| prefix_character(*ch))
                    .count()
            } else {
                0
            };
            let (end_line, end_character) =
                scalar_lsp_position(doc.text(), c.caret + suffix).ok_or("Sufixo inválido")?;
            CompletionEdit {
                start_line,
                start_character,
                end_line,
                end_character,
                new_text: item.new_text.clone(),
            }
        };
        let token = c.token;
        let id = c.id;
        let version = c.version;
        let php_callable = Language::for_path(&doc.path) == Some(Language::Php)
            && matches!(item.kind, Some(2 | 3))
            && !primary.new_text.contains('$')
            && codisseu_syntax::validate_rename_candidate(&primary.new_text).is_ok();
        self.enqueue(
            Work::Completion {
                token,
                id,
                version,
                text: doc.text().into(),
                caret: c.caret,
                primary,
                additional: item.additional_edits.clone(),
                php_callable,
            },
            Pending::Completion(token),
        )?;
        self.completion.context.as_mut().unwrap().phase = Phase::Applying;
        self.completion.context.as_mut().unwrap().php_callable = php_callable;
        self.completion.items.clear();
        self.status = "Aplicando sugestão…".into();
        Ok(())
    }
    pub(super) fn finish_completion(
        &mut self,
        token: u64,
        id: DocumentId,
        version: u64,
        text: String,
        caret: usize,
    ) -> Result<(), String> {
        if self.completion.context_token() != Some(token) || !self.completion_current() {
            return Ok(());
        }
        let trigger_signature = self
            .completion
            .context
            .as_ref()
            .is_some_and(|c| c.php_callable)
            && text.chars().nth(caret.saturating_sub(1)) == Some('(');
        let doc = self
            .documents
            .iter()
            .find(|d| d.id == id)
            .ok_or("Documento fechado")?;
        let delta = TextDelta::between(doc.text(), &text);
        self.execute(Command::EditTransaction {
            id,
            expected_version: version,
            delta,
            kind: EditKind::Atomic,
            before: doc.selection(),
            after: Selection::caret(caret),
        })?;
        let version = self.active_document().ok_or("Documento fechado")?.version;
        self.cancel_completion();
        self.cancel_signature_help();
        self.completion.selection = Some(CompletionSelection {
            document_id: id,
            version,
            character_index: caret,
            trigger_signature,
        });
        self.completion.error = None;
        self.status = "Sugestão aplicada; salve para gravar".into();
        self.last_error = None;
        Ok(())
    }
}

// Only a fallback replacement prefix when LSP supplies no edit. This is not symbol analysis.
fn prefix_start(text: &str, caret: usize) -> usize {
    let mut start = caret;
    let end = text
        .char_indices()
        .nth(caret)
        .map_or(text.len(), |(i, _)| i);
    for ch in text[..end].chars().rev() {
        if prefix_character(ch) {
            start -= 1
        } else {
            break;
        }
    }
    start
}

fn prefix_character(ch: char) -> bool {
    ch.is_alphanumeric()
        || ch == '_'
        || ch == '$'
        || matches!(ch,'\u{0300}'..='\u{036f}'|'\u{1ab0}'..='\u{1aff}'|'\u{1dc0}'..='\u{1dff}'|'\u{20d0}'..='\u{20ff}'|'\u{fe20}'..='\u{fe2f}')
}
pub(super) fn build_edits(
    text: &str,
    caret: usize,
    primary: CompletionEdit,
    additional: Vec<CompletionEdit>,
) -> Result<(String, usize), String> {
    if additional.len() > 16 {
        return Err("Sugestão excede 16 edições adicionais".into());
    }
    let caret_byte = text
        .char_indices()
        .nth(caret)
        .map(|(i, _)| i)
        .or_else(|| (caret == text.chars().count()).then_some(text.len()))
        .ok_or("Cursor inválido")?;
    let mut edits = Vec::new();
    for (index, edit) in std::iter::once(primary).chain(additional).enumerate() {
        if edit.new_text.len() > 64 * 1024 || edit.new_text.contains('\0') {
            return Err("Texto da sugestão inválido ou excede64KiB".into());
        }
        let start = utf16_byte_offset(text, edit.start_line, edit.start_character)
            .ok_or("Início da edição UTF-16 inválido")?;
        let end = utf16_byte_offset(text, edit.end_line, edit.end_character)
            .ok_or("Fim da edição UTF-16 inválido")?;
        if start > end {
            return Err("Edição invertida".into());
        }
        if index == 0
            && (edit.start_line != edit.end_line || caret_byte < start || caret_byte > end)
        {
            return Err("Edição principal não contém o cursor na mesma linha".into());
        }
        edits.push((start, end, edit.new_text, index == 0));
    }
    edits.sort_by_key(|e| (e.0, e.1));
    for pair in edits.windows(2) {
        if pair[1].0 < pair[0].1 || pair[1].0 == pair[0].0 {
            return Err("Edições da sugestão sobrepostas".into());
        }
    }
    let mut size = text.len();
    for (start, end, new_text, _) in &edits {
        size = size
            .checked_sub(end - start)
            .and_then(|n| n.checked_add(new_text.len()))
            .ok_or("Tamanho de edição inválido")?;
    }
    if size > codisseu_editor::MAX_DOCUMENT_BYTES {
        return Err("Sugestão excederia o limite de1MiB do documento".into());
    }
    let mut result = String::with_capacity(size);
    let mut previous = 0;
    let mut cursor = 0;
    for (start, end, new_text, is_primary) in edits {
        result.push_str(&text[previous..start]);
        result.push_str(&new_text);
        if is_primary {
            cursor = result.chars().count()
        }
        previous = end;
    }
    result.push_str(&text[previous..]);
    Ok((result, cursor))
}

pub(super) fn build_call_edits(
    text: &str,
    caret: usize,
    mut primary: CompletionEdit,
    additional: Vec<CompletionEdit>,
    php_callable: bool,
) -> Result<(String, usize), String> {
    if !php_callable {
        return build_edits(text, caret, primary, additional);
    }
    let original = primary.clone();
    let end = utf16_byte_offset(text, primary.end_line, primary.end_character)
        .ok_or("Fim da edição UTF-16 inválido")?;
    let whitespace = text[end..]
        .chars()
        .take_while(|ch| matches!(ch, ' ' | '\t'))
        .count();
    let existing = text[end..].chars().nth(whitespace) == Some('(');
    let name_chars = primary.new_text.chars().count();
    if !existing {
        primary.new_text.push_str("()");
    }
    let (result, cursor) = build_edits(text, caret, primary, additional.clone())?;
    let name_end = if existing {
        cursor
    } else {
        cursor.saturating_sub(2)
    };
    if !codisseu_syntax::php_completion::is_call_name(&result, name_end, name_chars) {
        return build_edits(text, caret, original, additional);
    }
    let caret = if existing {
        let spaces = result
            .chars()
            .skip(cursor)
            .take_while(|ch| matches!(ch, ' ' | '\t'))
            .count();
        if result.chars().nth(cursor + spaces) == Some('(') {
            cursor + spaces + 1
        } else {
            cursor
        }
    } else {
        cursor - 1
    };
    Ok((result, caret))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs, thread,
        time::{Duration, Instant},
    };
    fn edit(sl: u32, sc: u32, el: u32, ec: u32, new_text: &str) -> CompletionEdit {
        CompletionEdit {
            start_line: sl,
            start_character: sc,
            end_line: el,
            end_character: ec,
            new_text: new_text.into(),
        }
    }
    fn item(label: &str) -> CompletionItem {
        CompletionItem {
            label: label.into(),
            detail: None,
            kind: Some(2),
            filter_text: None,
            sort_text: None,
            new_text: label.into(),
            text_edit: None,
            insert_edit: None,
            additional_edits: vec![],
            unsupported_reason: None,
        }
    }
    fn idle(app: &mut AppService) {
        let until = Instant::now() + Duration::from_secs(5);
        loop {
            app.poll();
            if !app.is_busy() {
                return;
            }
            assert!(Instant::now() < until, "{}", app.status);
            thread::sleep(Duration::from_millis(1));
        }
    }
    fn fixture() -> (tempfile::TempDir, AppService, DocumentId, String) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("weather.ts");
        let text = "// 🌧 e\u{301}\r\nweather.rain;\r\n".to_string();
        fs::write(&path, &text).unwrap();
        let mut app = AppService::new(dir.path().join("session.json"));
        app.dispatch(Command::OpenFile(path)).unwrap();
        idle(&mut app);
        let id = app.active.unwrap();
        (dir, app, id, text)
    }
    fn incoming(app: &mut AppService, caret: usize, items: Vec<CompletionItem>) {
        let doc = app.active_document().unwrap();
        let (line, character) = scalar_lsp_position(doc.text(), caret).unwrap();
        let c = Context {
            token: app.completion.next_token + 1,
            id: doc.id,
            version: doc.version,
            caret,
            prefix_start: prefix_start(doc.text(), caret),
            line,
            character,
            versions: app.documents.iter().map(|d| (d.id, d.version)).collect(),
            phase: Phase::Requesting,
            php_callable: false,
        };
        app.language.completion = Some(codisseu_language::CompletionResult {
            path: doc.path.clone(),
            version: doc.version,
            line,
            character,
            items,
            is_incomplete: false,
            truncated: false,
        });
        app.completion.next_token += 1;
        app.completion.context = Some(c);
        app.poll();
    }
    #[test]
    fn php_callable_completion_adds_or_reuses_parentheses_only_in_call_context() {
        for (source, expected, call) in [
            ("<?php\n$g->gre;", "<?php\n$g->greet();", true),
            ("<?php\n$g->gre();", "<?php\n$g->greet();", true),
            ("<?php\n$g?->gre;", "<?php\n$g?->greet();", true),
            ("<?php\nG::gre;", "<?php\nG::greet();", true),
            ("<?php\n$g->gre  (1);", "<?php\n$g->greet  (1);", true),
            ("<?php\n// gre", "<?php\n// greet", false),
            ("<?php\n$x = 'gre';", "<?php\n$x = 'greet';", false),
            (
                "<?php\nfunction gre() {}",
                "<?php\nfunction greet() {}",
                false,
            ),
            ("<?php\nuse A\\gre;", "<?php\nuse A\\greet;", false),
        ] {
            let start = source[..source.rfind("gre").unwrap()].chars().count();
            let end = start + 3;
            let (sl, sc) = scalar_lsp_position(source, start).unwrap();
            let (el, ec) = scalar_lsp_position(source, end).unwrap();
            let (actual, caret) =
                build_call_edits(source, end, edit(sl, sc, el, ec, "greet"), vec![], true).unwrap();
            assert_eq!(actual, expected, "{source}");
            assert_eq!(
                actual.chars().nth(caret.saturating_sub(1)) == Some('('),
                call,
                "{source}, caret={caret}"
            );
        }
        let (text, caret) =
            build_call_edits("<?php\nstrl;", 10, edit(1, 0, 1, 4, "strlen"), vec![], true).unwrap();
        assert_eq!(text, "<?php\nstrlen();");
        assert_eq!(text.chars().nth(caret - 1), Some('('));
    }

    #[test]
    fn php_completed_call_and_import_share_one_undo_and_place_caret_inside_arguments() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Greeting.php");
        let original = "<?php\r\n// 🌧 e\u{301}\r\n$greeter->gre;\r\n";
        fs::write(&path, original).unwrap();
        let mut app = AppService::new(dir.path().join("session.json"));
        app.dispatch(Command::OpenFile(path.clone())).unwrap();
        idle(&mut app);
        let doc = app.active_document().unwrap();
        let (id, version) = (doc.id, doc.version);
        let caret = original[..original.find(';').unwrap()].chars().count();
        let before = Selection::caret(caret);
        app.dispatch(Command::SetSelection {
            id,
            expected_version: version,
            selection: before,
        })
        .unwrap();
        let mut chosen = item("greet");
        chosen.additional_edits = vec![edit(1, 0, 1, 0, "use EditorAudit\\Greeting;\r\n")];
        incoming(&mut app, caret, vec![chosen]);
        app.dispatch(Command::AcceptCompletion {
            index: 0,
            replace: false,
        })
        .unwrap();
        idle(&mut app);
        let expected = original
            .replace("// 🌧", "use EditorAudit\\Greeting;\r\n// 🌧")
            .replace("->gre;", "->greet();");
        assert_eq!(app.active_document().unwrap().text(), expected);
        let after = Selection::caret(
            expected[..expected.find("greet()").unwrap()]
                .chars()
                .count()
                + "greet(".chars().count(),
        );
        assert_eq!(app.active_document().unwrap().selection(), after);
        assert!(app.completion.selection.as_ref().unwrap().trigger_signature);
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
        app.dispatch(Command::Undo { id }).unwrap();
        assert_eq!(app.active_document().unwrap().text(), original);
        assert_eq!(app.active_document().unwrap().selection(), before);
        app.dispatch(Command::Redo { id }).unwrap();
        assert_eq!(app.active_document().unwrap().text(), expected);
        assert_eq!(app.active_document().unwrap().selection(), after);
    }
    #[test]
    fn qualified_completion_filters_from_explicit_range_and_replaces_suffix_once() {
        let (_dir, mut app, id, original) = fixture();
        let caret = original[..original.find("weather.rain").unwrap()]
            .chars()
            .count()
            + 10;
        let mut suggestion = item("weather.rainstorm");
        suggestion.text_edit = Some(edit(1, 0, 1, 12, "weather.rainstorm"));
        incoming(&mut app, caret, vec![suggestion]);
        assert_eq!(app.completion.items.len(), 1);
        app.dispatch(Command::AcceptCompletion {
            index: 0,
            replace: false,
        })
        .unwrap();
        idle(&mut app);
        assert_eq!(
            app.active_document().unwrap().text(),
            original.replace("weather.rain", "weather.rainstorm")
        );
        app.dispatch(Command::Undo { id }).unwrap();
        assert_eq!(app.active_document().unwrap().text(), original);
        assert_eq!(
            fs::read_to_string(&app.active_document().unwrap().path).unwrap(),
            original
        );
    }
    #[test]
    fn completion_keeps_dirty_unicode_crlf_and_has_one_undo_without_saving() {
        let (_dir, mut app, id, disk) = fixture();
        let dirty = disk.replace("// 🌧", "// unsaved 🌧");
        app.dispatch(Command::Edit {
            id,
            expected_version: 1,
            text: dirty.clone(),
        })
        .unwrap();
        let caret = dirty[..dirty.find(';').unwrap()].chars().count();
        let mut chosen = item("rainfall");
        chosen.new_text = "rainfall()".into();
        chosen.additional_edits = vec![edit(0, 0, 0, 0, "// import\r\n")];
        let view_before = Selection {
            anchor: caret,
            head: caret.saturating_sub(3),
        };
        let version = app.active_document().unwrap().version;
        app.dispatch(Command::SetSelection {
            id,
            expected_version: version,
            selection: view_before,
        })
        .unwrap();
        incoming(&mut app, caret, vec![chosen]);
        app.dispatch(Command::AcceptCompletion {
            index: 0,
            replace: false,
        })
        .unwrap();
        idle(&mut app);
        let expected = format!(
            "// import\r\n{}",
            dirty.replace("weather.rain;", "weather.rainfall();")
        );
        let doc = app.active_document().unwrap();
        assert_eq!(doc.text(), expected);
        assert!(doc.is_dirty());
        assert_eq!(fs::read_to_string(&doc.path).unwrap(), disk);
        let selection = app.completion.selection.as_ref().unwrap();
        assert_eq!(
            selection.character_index,
            expected[..expected.find(';').unwrap()].chars().count()
        );
        assert_eq!(selection.version, doc.version);
        let view_after = Selection::caret(selection.character_index);
        assert_eq!(doc.selection(), view_after);
        app.dispatch(Command::Undo { id }).unwrap();
        assert_eq!(app.active_document().unwrap().selection(), view_before);
        assert_eq!(app.active_document().unwrap().text(), dirty);
        app.dispatch(Command::Redo { id }).unwrap();
        assert_eq!(app.active_document().unwrap().text(), expected);
        assert_eq!(app.active_document().unwrap().selection(), view_after);
    }
    #[test]
    fn edits_validate_all_surrogates_overlap_and_limits_before_mutating() {
        let text = "🌧 e\u{301} rain;";
        assert!(
            build_edits(
                text,
                9,
                edit(0, 6, 0, 10, "x"),
                vec![edit(0, 1, 0, 1, "bad")]
            )
            .is_err()
        );
        assert!(
            build_edits(
                text,
                9,
                edit(0, 6, 0, 10, "x"),
                vec![edit(0, 6, 0, 6, "overlap")]
            )
            .is_err()
        );
        assert!(
            build_edits(
                text,
                9,
                edit(0, 6, 0, 10, "x"),
                vec![edit(0, 0, 0, 0, "\0")]
            )
            .is_err()
        );
        let large = "x".repeat(codisseu_editor::MAX_DOCUMENT_BYTES);
        assert!(
            build_edits(
                &large,
                large.len(),
                edit(0, large.len() as u32, 0, large.len() as u32, "more"),
                vec![]
            )
            .is_err()
        );
    }
    #[test]
    fn cancelled_worker_reply_and_new_request_do_not_apply_old_completion() {
        let (_dir, mut app, id, text) = fixture();
        let caret = text[..text.find(';').unwrap()].chars().count();
        incoming(&mut app, caret, vec![item("rainfall")]);
        app.dispatch(Command::AcceptCompletion {
            index: 0,
            replace: false,
        })
        .unwrap();
        let newer = text.replace("rain;", "rainc;");
        app.dispatch(Command::Edit {
            id,
            expected_version: 1,
            text: newer.clone(),
        })
        .unwrap();
        let caret = newer[..newer.find(';').unwrap()].chars().count();
        incoming(&mut app, caret, vec![item("raincoat")]);
        idle(&mut app);
        assert_eq!(app.active_document().unwrap().text(), newer);
        assert_eq!(app.completion.items[0].label, "raincoat");
        app.dispatch(Command::CancelCompletion).unwrap();
        assert!(!app.completion.is_open());
    }
    #[test]
    fn unsupported_items_and_stale_versions_leave_text_and_disk_intact() {
        let (_dir, mut app, id, text) = fixture();
        let caret = text[..text.find(';').unwrap()].chars().count();
        let mut unsupported = item("rainfall");
        unsupported.unsupported_reason = Some("snippet".into());
        incoming(&mut app, caret, vec![unsupported]);
        assert!(
            app.dispatch(Command::AcceptCompletion {
                index: 0,
                replace: false
            })
            .unwrap_err()
            .contains("snippet")
        );
        assert_eq!(app.active_document().unwrap().text(), text);
        incoming(&mut app, caret, vec![item("rainfall")]);
        app.dispatch(Command::Activate { id }).unwrap();
        assert!(
            app.dispatch(Command::AcceptCompletion {
                index: 0,
                replace: false
            })
            .is_err()
        );
        assert_eq!(
            fs::read_to_string(&app.active_document().unwrap().path).unwrap(),
            text
        );
    }
    #[test]
    fn failed_sync_cancels_ready_choices_and_preserves_dirty_text() {
        let (_dir, mut app, _id, text) = fixture();
        let caret = text[..text.find(';').unwrap()].chars().count();
        incoming(&mut app, caret, vec![item("rainfall")]);
        let doc = app.active_document().unwrap();
        app.language.sync_failures.insert(
            doc.path.clone(),
            codisseu_language::SyncFailure {
                version: doc.version,
                message: "controlled error".into(),
            },
        );
        app.poll();
        assert!(!app.completion.is_open());
        assert!(
            app.completion
                .error
                .as_ref()
                .unwrap()
                .contains("sincronização")
        );
        assert_eq!(app.active_document().unwrap().text(), text);
    }
    #[test]
    fn fallback_prefix_is_not_byte_count_and_never_consumes_suffix() {
        let text = "🌧 e\u{301} weather.raiSuffix";
        let caret = text[..text.find("Suffix").unwrap()].chars().count();
        assert_eq!(
            text.chars()
                .skip(prefix_start(text, caret))
                .take(caret - prefix_start(text, caret))
                .collect::<String>(),
            "rai"
        );
        let (line, ch) = scalar_lsp_position(text, caret).unwrap();
        let (_, start) = scalar_lsp_position(text, prefix_start(text, caret)).unwrap();
        let (result, _) =
            build_edits(text, caret, edit(line, start, line, ch, "rainfall"), vec![]).unwrap();
        assert_eq!(result, "🌧 e\u{301} weather.rainfallSuffix");
    }
    #[test]
    fn repeated_accept_preserves_already_accepted_worker_edit() {
        let (_dir, mut app, _id, text) = fixture();
        let caret = text[..text.find(';').unwrap()].chars().count();
        incoming(&mut app, caret, vec![item("rainfall")]);
        app.dispatch(Command::AcceptCompletion {
            index: 0,
            replace: false,
        })
        .unwrap();
        assert!(
            app.dispatch(Command::AcceptCompletion {
                index: 0,
                replace: false
            })
            .is_err()
        );
        assert!(app.completion.is_applying());
        idle(&mut app);
        assert_eq!(
            app.active_document().unwrap().text(),
            text.replace("rain;", "rainfall;")
        );
        assert!(app.last_error.is_none());
    }
    #[test]
    fn fallback_enter_inserts_and_tab_replaces_suffix() {
        for replace in [false, true] {
            let (_dir, mut app, id, text) = fixture();
            let text = text.replace("rain;", "rainXYZ;");
            app.dispatch(Command::Edit {
                id,
                expected_version: 1,
                text: text.clone(),
            })
            .unwrap();
            let caret = text[..text.find("XYZ").unwrap()].chars().count();
            incoming(&mut app, caret, vec![item("rainfall")]);
            app.dispatch(Command::AcceptCompletion { index: 0, replace })
                .unwrap();
            idle(&mut app);
            let expected = if replace {
                text.replace("rainXYZ", "rainfall")
            } else {
                text.replace("rainXYZ", "rainfallXYZ")
            };
            assert_eq!(app.active_document().unwrap().text(), expected);
            app.dispatch(Command::Undo { id }).unwrap();
            assert_eq!(app.active_document().unwrap().text(), text);
        }
    }
}
