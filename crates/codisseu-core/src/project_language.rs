//! Lazy, bounded language sessions. A workspace window may own independent
//! Composer/TypeScript projects without sharing an LSP root or open buffers.
use super::{AppService, LanguageCommand, LanguageService, ProviderConfig, ProviderState};
use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
};

const MAX_CACHED_PROJECTS: usize = 7; // plus the public, currently active service

struct CachedProject {
    root: PathBuf,
    configs: Vec<ProviderConfig>,
    service: LanguageService,
    versions: HashMap<PathBuf, u64>,
    skipped_versions: HashMap<PathBuf, u64>,
    retries: HashMap<PathBuf, (u64, u8)>,
}

#[derive(Default)]
pub(super) struct ProjectLanguages {
    root: Option<PathBuf>,
    configs: Vec<ProviderConfig>,
    enabled: bool,
    stopping: bool,
    cached: VecDeque<CachedProject>,
}

impl ProjectLanguages {
    pub(super) fn defer(&mut self) {
        self.enabled = true;
        self.root = None;
        self.configs.clear();
        self.stopping = false;
    }
    pub(super) fn started(&mut self, root: PathBuf, configs: Vec<ProviderConfig>) {
        self.root = Some(root);
        self.configs = configs;
        self.enabled = true;
        self.stopping = false;
    }
    pub(super) fn is_busy(&self) -> bool {
        self.cached.iter().any(|p| p.service.is_busy())
    }
    pub(super) fn any_running(&self) -> bool {
        self.cached.iter().any(|p| {
            p.service
                .statuses
                .iter()
                .any(|s| matches!(s.state, ProviderState::Starting | ProviderState::Ready))
        })
    }
}

impl AppService {
    /// The active semantic root is independent of the folder displayed by the
    /// explorer. Resolution was performed by the file worker, never by the GUI.
    pub fn language_project_root(&self) -> Option<PathBuf> {
        self.active_document()
            .filter(|d| !d.is_read_only())
            .map(|d| d.project_root().to_path_buf())
            .or_else(|| {
                // Stop keeps session metadata while its processes finish. After
                // switching workspaces that cached root must not route Trust/Start
                // back to a project outside the newly opened workspace.
                self.project_languages
                    .root
                    .as_ref()
                    .filter(|root| self.workspace_root_for(root).is_some())
                    .cloned()
            })
            .or_else(|| self.workspace_default_root().map(PathBuf::from))
    }
    pub fn active_language_project(&self) -> Option<&Path> {
        self.project_languages.root.as_deref()
    }
    pub(super) fn project_language_busy(&self) -> bool {
        self.project_languages.is_busy()
    }
    pub fn any_project_providers_running(&self) -> bool {
        self.providers_running() || self.project_languages.any_running()
    }
    /// Close also reaches cached sessions: an inactive document must not
    /// remain open in its server after the final editor view is closed.
    pub(super) fn close_language_document(&mut self, path: &Path) -> Result<(), String> {
        let root = self
            .documents
            .iter()
            .find(|d| d.path == path)
            .map(|d| d.project_root().to_path_buf())
            .or_else(|| {
                self.project_languages
                    .cached
                    .iter()
                    .filter(|p| path.starts_with(&p.root))
                    .max_by_key(|p| p.root.components().count())
                    .map(|p| p.root.clone())
            });
        if let Some(project) = self
            .project_languages
            .cached
            .iter_mut()
            .find(|p| Some(&p.root) == root.as_ref())
        {
            project.versions.remove(path);
            project.skipped_versions.remove(path);
            project.retries.remove(path);
            project.service.dispatch(LanguageCommand::Close {
                path: path.to_path_buf(),
            })
        } else {
            self.language.dispatch(LanguageCommand::Close {
                path: path.to_path_buf(),
            })
        }
    }
    pub(super) fn stop_project_languages(&mut self) -> Result<(), String> {
        self.project_languages.enabled = false;
        self.project_languages.stopping = true;
        for project in &mut self.project_languages.cached {
            project.service.dispatch(LanguageCommand::Stop)?;
        }
        Ok(())
    }
    pub(super) fn poll_project_languages(&mut self) -> bool {
        let mut changed = false;
        for project in &mut self.project_languages.cached {
            changed |= project.service.poll();
        }
        if self.project_languages.stopping {
            self.project_languages
                .cached
                .retain(|p| p.service.is_busy());
            if self.project_languages.cached.is_empty() {
                self.project_languages.stopping = false;
            }
            return changed;
        }
        if !self.project_languages.enabled {
            return changed;
        }
        if self
            .active_document()
            .is_none_or(|doc| doc.is_read_only() || super::Language::for_path(&doc.path).is_none())
        {
            return changed;
        }
        let Some(root) = self.language_project_root() else {
            return changed;
        };
        if self.project_languages.root.as_ref() == Some(&root) {
            return changed;
        }
        // Changing tabs must not let a result from another project's server
        // become the active hover/completion/rename proposal.
        self.cancel_formatting();
        self.cancel_navigation();
        self.cancel_completion();
        self.cancel_signature_help();
        self.cancel_references();
        self.cancel_rename();
        let _ = self.language.dispatch(LanguageCommand::CancelHover);
        let configs = self.project_provider_configs(&root);
        let cached = self
            .project_languages
            .cached
            .iter()
            .position(|p| p.root == root && p.configs == configs)
            .and_then(|index| self.project_languages.cached.remove(index));
        let reused = cached.is_some();
        let (next, versions, skipped_versions, retries) = cached.map_or_else(
            || {
                (
                    LanguageService::new(),
                    HashMap::new(),
                    HashMap::new(),
                    HashMap::new(),
                )
            },
            |p| (p.service, p.versions, p.skipped_versions, p.retries),
        );
        let previous = std::mem::replace(&mut self.language, next);
        let previous_versions = std::mem::replace(&mut self.language_versions, versions);
        let previous_skipped =
            std::mem::replace(&mut self.language_skipped_versions, skipped_versions);
        let previous_retries = std::mem::replace(&mut self.language_retries, retries);
        if let Some(previous_root) = self.project_languages.root.take() {
            self.project_languages.cached.push_back(CachedProject {
                root: previous_root,
                configs: std::mem::take(&mut self.project_languages.configs),
                service: previous,
                versions: previous_versions,
                skipped_versions: previous_skipped,
                retries: previous_retries,
            });
        }
        if reused {
            // Restored worker and version maps are reused without spawning any
            // replacement worker merely for swapping the active session.
        } else if configs.is_empty() {
            self.status = format!("Análise de linguagem desativada para {}", root.display());
        } else if self.project_provider_configuration_authorized(&root, &configs) {
            if let Err(error) = self.language.dispatch(LanguageCommand::Start {
                workspace: root.clone(),
                configs: configs.clone(),
            }) {
                self.language.last_error = Some(error.clone());
                self.last_error = Some(error);
            }
        } else {
            self.status = format!(
                "Confie e configure os providers de {} antes de ativar",
                root.display()
            );
        }
        self.project_languages.root = Some(root);
        self.project_languages.configs = configs;
        while self.project_languages.cached.len() > MAX_CACHED_PROJECTS {
            // Drop delegates cleanup to the language service's process reaper;
            // no shutdown wait blocks the interactive thread.
            self.project_languages.cached.pop_front();
        }
        // Cached version maps avoid duplicate didChange versions on tab switches.
        // Edits made while inactive have a newer document version and are synced
        // normally; evicted/new sessions have empty maps and receive didOpen.
        true
    }
}
