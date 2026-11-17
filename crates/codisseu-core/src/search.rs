//! Bounded background computation for the initial M3 literal-search contract.
use super::{DocumentId, SearchResult};
use codisseu_editor::search::{Replacement, find_literal, replace_literal};
use std::{
    collections::BTreeMap,
    ops::Range,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
};

const QUEUE_SIZE: usize = 8;

pub(super) enum Operation {
    Find,
    Replace {
        replacement: String,
        selection: Option<Range<usize>>,
    },
}

pub(super) struct Request {
    pub document_id: DocumentId,
    pub version: u64,
    pub text: String,
    pub query: String,
    pub operation: Operation,
}

struct Job {
    id: u64,
    generation: u64,
    find_generation: Option<u64>,
    request: Request,
}

pub(super) enum Completion {
    Found(SearchResult),
    Replaced(Replacement),
}

pub(super) struct Reply {
    id: u64,
    generation: u64,
    find_generation: Option<u64>,
    pub document_id: DocumentId,
    pub version: u64,
    pub result: Result<Completion, String>,
}

impl Reply {
    pub fn is_replacement(&self) -> bool {
        self.find_generation.is_none()
    }
}

pub(super) struct Worker {
    sender: SyncSender<Job>,
    receiver: Receiver<Reply>,
    generation: Arc<AtomicU64>,
    find_generation: Arc<AtomicU64>,
    pending: BTreeMap<u64, bool>,
    current_find: Option<u64>,
    next_id: u64,
}

impl Worker {
    pub fn new(generation: Arc<AtomicU64>) -> Self {
        let (sender, jobs) = mpsc::sync_channel::<Job>(QUEUE_SIZE);
        let (replies, receiver) = mpsc::sync_channel(QUEUE_SIZE);
        let find_generation = Arc::new(AtomicU64::new(0));
        let worker_generation = Arc::clone(&generation);
        let worker_find_generation = Arc::clone(&find_generation);
        std::thread::Builder::new()
            .name("codisseu-search".into())
            .spawn(move || {
                while let Ok(job) = jobs.recv() {
                    let check = || {
                        if worker_generation.load(Ordering::SeqCst) != job.generation
                            || job.find_generation.is_some_and(|expected| {
                                worker_find_generation.load(Ordering::SeqCst) != expected
                            })
                        {
                            Err("Busca/substituição cancelada".into())
                        } else {
                            Ok(())
                        }
                    };
                    let request = job.request;
                    let result = check().and_then(|()| match request.operation {
                        Operation::Find => {
                            find_literal(&request.text, &request.query, check).map(|found| {
                                Completion::Found(SearchResult {
                                    document_id: request.document_id,
                                    version: request.version,
                                    query: request.query,
                                    ranges: found.ranges,
                                    truncated: found.truncated,
                                })
                            })
                        }
                        Operation::Replace {
                            replacement,
                            selection,
                        } => replace_literal(
                            &request.text,
                            &request.query,
                            &replacement,
                            selection,
                            check,
                        )
                        .map(Completion::Replaced),
                    });
                    if replies
                        .send(Reply {
                            id: job.id,
                            generation: job.generation,
                            find_generation: job.find_generation,
                            document_id: request.document_id,
                            version: request.version,
                            result,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .expect("Não foi possível criar o worker de busca");
        Self {
            sender,
            receiver,
            generation,
            find_generation,
            pending: BTreeMap::new(),
            current_find: None,
            next_id: 1,
        }
    }

    pub fn enqueue(&mut self, request: Request) -> Result<(), String> {
        // Count unacknowledged jobs, including cancelled jobs, to bound snapshots
        // and the reply queue even if the UI stops polling.
        if self.pending.len() >= QUEUE_SIZE {
            return Err("Fila de busca cheia; aguarde as operações pendentes".into());
        }
        let is_find = matches!(request.operation, Operation::Find);
        let find_generation = if is_find {
            Some(self.find_generation.fetch_add(1, Ordering::SeqCst) + 1)
        } else {
            None
        };
        let id = self.next_id;
        self.sender
            .try_send(Job {
                id,
                generation: self.generation.load(Ordering::SeqCst),
                find_generation,
                request,
            })
            .map_err(|_| "Worker de busca indisponível ou fila cheia".to_owned())?;
        self.pending.insert(id, is_find);
        if is_find {
            self.current_find = Some(id);
        }
        self.next_id += 1;
        Ok(())
    }

    pub fn cancel_find(&mut self) {
        self.find_generation.fetch_add(1, Ordering::SeqCst);
        self.current_find = None;
    }

    pub fn is_busy(&self) -> bool {
        !self.pending.is_empty()
    }

    pub fn search_pending(&self) -> bool {
        self.current_find.is_some() || self.pending.values().any(|find| !find)
    }

    /// Returns both valid completions and an indication that stale work drained.
    pub fn poll_one(&mut self) -> Option<Option<Reply>> {
        let reply = self.receiver.try_recv().ok()?;
        self.pending.remove(&reply.id);
        if self.current_find == Some(reply.id) {
            self.current_find = None;
        }
        if reply.generation != self.generation.load(Ordering::SeqCst)
            || reply
                .find_generation
                .is_some_and(|expected| self.find_generation.load(Ordering::SeqCst) != expected)
        {
            return Some(None);
        }
        Some(Some(reply))
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel_find();
    }
}
