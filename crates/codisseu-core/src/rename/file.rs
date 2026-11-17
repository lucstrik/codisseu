//! A file/module is an explicit Rename origin, independent of any declaration.
use super::*;
impl RenameState {
    pub fn is_file_rename(&self) -> bool {
        self.context.as_ref().is_some_and(|c| c.search.file_rename)
    }
}
impl AppService {
    pub(crate) fn prepare_file_rename(
        &mut self,
        id: DocumentId,
        version: u64,
    ) -> Result<(), String> {
        let origin = self.navigation_position(id, version, 0)?;
        let document = self.active_document().ok_or("Abra um arquivo")?;
        if Language::for_path(&document.path).is_none() || !document.language_supported() {
            return Err("Rename de arquivo exige PHP/JS/JSX/TS/TSX de até 1 MiB".into());
        }
        let path = document.path.clone();
        if !path.starts_with(self.workspace.as_ref().ok_or("Abra um workspace")?) {
            return Err("Arquivo fora do workspace".into());
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("Nome de arquivo não UTF-8")?
            .to_owned();
        self.cancel_rename();
        self.cancel_navigation();
        self.cancel_completion();
        self.cancel_signature_help();
        self.cancel_references();
        self.rename.next_token += 1;
        self.rename.context = Some(Context {
            search: RenameSearch {
                file_rename: true,
                ..Default::default()
            },
            token: self.rename.next_token,
            origin,
            path,
            line: 0,
            character: 0,
            versions: self.documents.iter().map(|d| (d.id, d.version)).collect(),
            snapshots: vec![],
            proposal: vec![],
            resources: vec![],
            validation: None,
            corpus: RenameCorpus::default(),
            phase: Phase::Prepared,
            methods: vec![],
            selected_methods: vec![],
            expansion: None,
            local_semantic: true,
            cancel: Arc::new(AtomicBool::new(false)),
        });
        let php = self
            .rename
            .context
            .as_ref()
            .unwrap()
            .path
            .extension()
            .is_some_and(|e| e == "php");
        self.rename.options = (if php {
            self.rename.php_file_options
        } else {
            self.rename.web_file_options
        })
        .unwrap_or(RenameOptions {
            search_file_references: true,
            search_comments_strings: true,
            search_text: !php,
            ..Default::default()
        });
        self.rename.error = None;
        self.rename.symbol_kind = codisseu_syntax::RenameSymbolKind::Unknown;
        self.rename.new_name = name.clone();
        self.rename.placeholder = Some(name);
        self.rename.prepared = true;
        self.rename.used_fallback = false;
        Ok(())
    }
    pub(super) fn preview_file_rename(&mut self, new_name: String) -> Result<(), String> {
        codisseu_syntax::validate_file_rename_name(&new_name).map_err(str::to_owned)?;
        if !self.rename_current() {
            return Err("Rename de arquivo obsoleto".into());
        }
        let c = self
            .rename
            .context
            .as_ref()
            .ok_or("Prepare rename primeiro")?;
        if !self.rename.prepared
            || !matches!(c.phase, Phase::Prepared | Phase::Preview)
            || !c.search.file_rename
        {
            return Err("Aguarde preparação de Rename de arquivo".into());
        }
        if c.path.extension() != std::path::Path::new(&new_name).extension() {
            return Err("Rename de arquivo conserva a extensão".into());
        }
        if c.path.file_name().and_then(|s| s.to_str()) == Some(&new_name) {
            return Err("Escolha um nome de arquivo diferente".into());
        }
        let mut buffers = vec![];
        let mut bytes = 0;
        for document in &self.documents {
            if self
                .language_project_root()
                .is_some_and(|root| document.project_root() != root)
            {
                continue;
            }
            if Language::for_path(&document.path).is_none()
                && !(self.rename.options.search_text
                    && matches!(
                        document.path.extension().and_then(|s| s.to_str()),
                        Some("txt" | "md")
                    ))
            {
                continue;
            }
            bytes += document.len_bytes() * 2;
            if !document.language_supported() || bytes > 8 * 1024 * 1024 {
                return Err("Buffers de Rename excedem 1 MiB/8 MiB de contexto".into());
            }
            buffers.push((document.path.clone(), Arc::from(document.text())));
        }
        let token = c.token;
        self.enqueue(
            Work::RenameFilePreview {
                token,
                workspace: self.language_project_root().ok_or("Abra um workspace")?,
                search: self.rename_search(),
                new_name: new_name.clone(),
                buffers,
                cancel: c.cancel.clone(),
            },
            Pending::Rename(token),
        )?;
        self.rename.context.as_mut().unwrap().phase = Phase::Building;
        self.rename.new_name = new_name;
        self.rename.files.clear();
        self.rename.resource_moves.clear();
        self.rename.validation = None;
        Ok(())
    }
}
