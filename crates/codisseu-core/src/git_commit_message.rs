//! Repository-bound, cancellable AI generation with read-only diff revalidation.
use super::*;
use ai::commit_message::CommitMessageService;
use git::commit_message::{Context, collect};
use std::{collections::VecDeque, path::Path, sync::atomic::AtomicBool, thread::JoinHandle};

pub struct ResultEvent {
    pub root: PathBuf,
    pub workspace: PathBuf,
    pub draft: String,
    pub result: Result<String, String>,
}
struct Reader {
    cancel: Arc<AtomicBool>,
    receiver: Receiver<Result<Context, String>>,
    worker: JoinHandle<()>,
    answer: Option<Result<Context, String>>,
}
impl Reader {
    fn start(root: PathBuf) -> Result<Self, String> {
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        let (sender, receiver) = mpsc::sync_channel(1);
        let worker = std::thread::Builder::new()
            .name("codisseu-commit-context".into())
            .spawn(move || {
                let result = collect(&root, &stop).map_err(|e| e.to_string());
                let _ = sender.try_send(result);
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            cancel,
            receiver,
            worker,
            answer: None,
        })
    }
    fn poll(&mut self) -> Option<Result<Context, String>> {
        if self.answer.is_none() {
            match self.receiver.try_recv() {
                Ok(result) => self.answer = Some(result),
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.answer = Some(Err("Leitura do diff encerrou sem resultado".into()))
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        self.worker
            .is_finished()
            .then(|| self.answer.take())
            .flatten()
    }
}
impl Drop for Reader {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}
struct Job {
    workspace: PathBuf,
    config: ai::Config,
    draft: String,
    reader: Option<Reader>,
    source: Option<Context>,
    candidate: Option<String>,
    agent: CommitMessageService,
    cancelled: bool,
}
#[derive(Default)]
pub struct Service {
    jobs: HashMap<PathBuf, Job>,
    results: VecDeque<ResultEvent>,
}
impl Service {
    pub fn is_running(&self, root: &Path) -> bool {
        self.jobs.contains_key(root)
    }
    pub fn any_running(&self) -> bool {
        !self.jobs.is_empty()
    }
    pub fn take_result(&mut self) -> Option<ResultEvent> {
        self.results.pop_front()
    }
    pub fn cancel(&mut self, root: Option<&Path>) {
        for (path, job) in &mut self.jobs {
            if root.is_none_or(|root| root == path) {
                job.cancelled = true;
                if let Some(reader) = &job.reader {
                    reader.cancel.store(true, Ordering::Release);
                }
                job.agent.cancel();
            }
        }
    }
    fn start(
        &mut self,
        workspace: PathBuf,
        root: PathBuf,
        config: ai::Config,
        draft: String,
    ) -> Result<(), String> {
        if self.is_running(&root) {
            return Err("Uma mensagem já está sendo gerada neste repositório".into());
        }
        if self.jobs.len() >= 4 {
            return Err("Aguarde uma das quatro gerações de mensagem em andamento".into());
        }
        let reader = Reader::start(root.clone())?;
        self.jobs.insert(
            root,
            Job {
                workspace,
                config,
                draft,
                reader: Some(reader),
                source: None,
                candidate: None,
                agent: CommitMessageService::default(),
                cancelled: false,
            },
        );
        Ok(())
    }
    pub(super) fn poll(&mut self, workspace: Option<&Path>) -> bool {
        let mut finished = Vec::new();
        let mut changed = false;
        for (root, job) in &mut self.jobs {
            if workspace != Some(job.workspace.as_path()) && !job.cancelled {
                job.cancelled = true;
                if let Some(reader) = &job.reader {
                    reader.cancel.store(true, Ordering::Release);
                }
                job.agent.cancel();
                changed = true;
            }
            changed |= job.agent.poll();
            if let Some(reader) = job.reader.as_mut()
                && let Some(result) = reader.poll()
            {
                job.reader = None;
                changed = true;
                if job.cancelled {
                    finished.push((root.clone(), Err("Geração de mensagem cancelada".into())));
                    continue;
                }
                match result {
                    Err(error) => {
                        finished.push((root.clone(), Err(error)));
                        continue;
                    }
                    Ok(context) => {
                        if let Some(source) = &job.source {
                            let result = if source == &context {
                                Ok(job.candidate.take().unwrap_or_default())
                            } else {
                                Err("As alterações Git mudaram durante a geração; sua mensagem foi preservada. Gere novamente.".into())
                            };
                            finished.push((root.clone(), result));
                            continue;
                        }
                        match job
                            .agent
                            .start(root.clone(), job.config.clone(), &context.text)
                        {
                            Ok(()) => job.source = Some(context),
                            Err(error) => {
                                finished.push((root.clone(), Err(error)));
                                continue;
                            }
                        }
                    }
                }
            }
            if let Some(result) = job.agent.take_result() {
                changed = true;
                if job.cancelled {
                    finished.push((root.clone(), Err("Geração de mensagem cancelada".into())));
                    continue;
                }
                match result {
                    Err(error) => finished.push((root.clone(), Err(error))),
                    Ok(message) => match Reader::start(root.clone()) {
                        Ok(reader) => {
                            job.candidate = Some(message);
                            job.reader = Some(reader);
                        }
                        Err(error) => finished.push((root.clone(), Err(error))),
                    },
                }
            } else if job.cancelled && job.reader.is_none() && !job.agent.is_running() {
                finished.push((root.clone(), Err("Geração de mensagem cancelada".into())));
            }
        }
        for (root, result) in finished {
            if let Some(job) = self.jobs.remove(&root) {
                if self.results.len() >= 32 {
                    self.results.pop_front();
                }
                if workspace == Some(job.workspace.as_path()) {
                    self.results.push_back(ResultEvent {
                        root,
                        workspace: job.workspace,
                        draft: job.draft,
                        result,
                    });
                }
                changed = true;
            }
        }
        changed
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        self.cancel(None);
    }
}
impl AppService {
    pub(super) fn generate_git_commit_message(&mut self, root: PathBuf) -> Result<(), String> {
        if self.ai.profile_pending() {
            return Err("Aguarde o carregamento da configuração de IA".into());
        }
        let workspace = self
            .workspace_path()
            .ok_or("Abra um projeto antes de gerar a mensagem")?
            .to_path_buf();
        if self
            .git_repository_service(&root)
            .and_then(|git| git.repo.as_ref())
            .is_none_or(|repo| repo.root != root)
        {
            return Err("Aguarde a detecção do repositório antes de gerar a mensagem".into());
        }
        let draft = self.git_draft(&root).unwrap_or("").to_owned();
        let config = self
            .ai
            .preferred_config(self.ai.config.provider)
            .cloned()
            .unwrap_or_else(|| self.ai.config.clone());
        self.git_commit_messages
            .start(workspace, root, config, draft)
    }
}
