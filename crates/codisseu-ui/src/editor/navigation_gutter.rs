//! Latest-only PHP declaration worker. No parsing, I/O or full-buffer conversion
//! occurs while rendering. One service belongs to the application window, not each view.
use codisseu_core::{
    TextChange, TextSnapshot,
    navigation_symbols::{
        MAX_BYTES, NavigationAnchor, NavigationEdit, NavigationInspection, NavigationStats,
        PhpNavigationCache,
    },
};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
#[derive(Clone, Debug, Eq, PartialEq)]
struct Key {
    document: u64,
    path: PathBuf,
    revision: u64,
}
struct Request {
    key: Key,
    snapshot: TextSnapshot,
    change: Option<TextChange>,
    generation: u64,
    due: Instant,
}
struct Reply {
    key: Key,
    generation: u64,
    result: Result<NavigationInspection, String>,
    stats: NavigationStats,
}
#[derive(Default)]
struct Mailbox {
    request: Mutex<Option<Request>>,
    ready: Condvar,
    stopped: AtomicBool,
    generation: AtomicU64,
    reply: Mutex<Option<Reply>>,
}
#[derive(Default)]
pub(crate) struct NavigationGutter {
    mailbox: Arc<Mailbox>,
    started: bool,
    key: Option<Key>,
    generation: u64,
    pending: bool,
    anchors: Arc<[NavigationAnchor]>,
    empty: Arc<[NavigationAnchor]>,
    pub truncated: bool,
    pub error: Option<String>,
    stats: NavigationStats,
    scheduled: u64,
}
impl NavigationGutter {
    /// Returns true only when a new document revision was scheduled.
    pub fn schedule(
        &mut self,
        document: u64,
        path: &Path,
        revision: u64,
        snapshot: TextSnapshot,
        change: Option<&TextChange>,
    ) -> bool {
        if self.key.as_ref().is_some_and(|key| {
            key.document == document && key.path == path && key.revision == revision
        }) {
            return false;
        }
        let key = Key {
            document,
            path: path.into(),
            revision,
        };
        self.cancel();
        self.key = Some(key.clone());
        self.pending = true;
        self.scheduled += 1;
        if !self.started {
            match start(self.mailbox.clone()) {
                Ok(()) => self.started = true,
                Err(error) => {
                    self.error = Some(error);
                    self.pending = false;
                    return false;
                }
            }
        }
        self.generation = self.mailbox.generation.fetch_add(1, Ordering::AcqRel) + 1;
        let request = Request {
            key,
            snapshot,
            change: change.cloned(),
            generation: self.generation,
            due: Instant::now() + Duration::from_millis(150),
        };
        match self.mailbox.request.lock() {
            Ok(mut slot) => {
                *slot = Some(request);
                self.mailbox.ready.notify_one();
            }
            Err(_) => {
                self.pending = false;
                self.error = Some("Fila de declarações indisponível".into());
            }
        }
        true
    }
    pub fn cancel(&mut self) {
        if self.key.is_none() && !self.pending && self.anchors.is_empty() && self.error.is_none() {
            return;
        }
        self.generation = self.mailbox.generation.fetch_add(1, Ordering::AcqRel) + 1;
        if let Ok(mut slot) = self.mailbox.request.lock() {
            *slot = None;
        }
        if let Ok(mut slot) = self.mailbox.reply.lock() {
            *slot = None;
        }
        self.mailbox.ready.notify_one();
        self.key = None;
        self.anchors = self.empty.clone();
        self.pending = false;
        self.truncated = false;
        self.error = None;
    }
    pub fn poll(&mut self) -> bool {
        let reply = self
            .mailbox
            .reply
            .lock()
            .ok()
            .and_then(|mut slot| slot.take());
        let Some(reply) = reply else {
            return false;
        };
        if reply.generation != self.generation || self.key.as_ref() != Some(&reply.key) {
            return false;
        }
        self.pending = false;
        self.stats = reply.stats;
        match reply.result {
            Ok(result) => {
                self.anchors = result.anchors.into();
                self.truncated = result.truncated;
                self.error = None;
            }
            Err(error) => {
                self.anchors = self.empty.clone();
                self.error = Some(error);
            }
        }
        true
    }
    pub fn anchors(&self, document: u64, revision: u64) -> Arc<[NavigationAnchor]> {
        if self
            .key
            .as_ref()
            .is_some_and(|key| key.document == document && key.revision == revision)
        {
            self.anchors.clone()
        } else {
            self.empty.clone()
        }
    }
    pub fn pending(&self) -> bool {
        self.pending
    }
    pub fn stats(&self) -> NavigationStats {
        self.stats
    }
    #[cfg(test)]
    pub fn scheduled(&self) -> u64 {
        self.scheduled
    }
}
impl Drop for NavigationGutter {
    fn drop(&mut self) {
        self.mailbox.stopped.store(true, Ordering::Release);
        self.mailbox.generation.fetch_add(1, Ordering::AcqRel);
        self.mailbox.ready.notify_one();
    }
}
fn start(mailbox: Arc<Mailbox>) -> Result<(), String> {
    std::thread::Builder::new()
        .name("codisseu-php-declarations".into())
        .spawn(move || {
            let mut cache = PhpNavigationCache::default();
            let mut total_stats = NavigationStats::default();
            let mut previous: Option<(Key, TextSnapshot)> = None;
            loop {
                let request = {
                    let Ok(mut slot) = mailbox.request.lock() else {
                        return;
                    };
                    loop {
                        if mailbox.stopped.load(Ordering::Acquire) {
                            return;
                        }
                        if let Some(request) = slot.as_ref() {
                            let now = Instant::now();
                            let due = request.due;
                            if now >= due {
                                break;
                            }
                            slot = match mailbox.ready.wait_timeout(slot, due - now) {
                                Ok((slot, _)) => slot,
                                Err(_) => return,
                            };
                        } else {
                            slot = match mailbox.ready.wait(slot) {
                                Ok(slot) => slot,
                                Err(_) => return,
                            };
                        }
                    }
                    slot.take().unwrap()
                };
                let cancelled = || {
                    mailbox.stopped.load(Ordering::Acquire)
                        || mailbox.generation.load(Ordering::Acquire) != request.generation
                };
                if cancelled() {
                    continue;
                }
                let same_document = previous.as_ref().is_some_and(|(key, _)| {
                    key.document == request.key.document && key.path == request.key.path
                });
                if !same_document {
                    cache = PhpNavigationCache::default();
                }
                let edit = if same_document {
                    previous.as_ref().and_then(|(_, old)| {
                        let change = request.change.as_ref()?;
                        Some(NavigationEdit {
                            old: old.try_char_to_byte(change.old_chars.start).ok()?
                                ..old.try_char_to_byte(change.old_chars.end).ok()?,
                            new: request
                                .snapshot
                                .try_char_to_byte(change.new_chars.start)
                                .ok()?
                                ..request
                                    .snapshot
                                    .try_char_to_byte(change.new_chars.end)
                                    .ok()?,
                        })
                    })
                } else {
                    None
                };
                let before_stats = cache.stats();
                let result = if request.snapshot.len_bytes() > MAX_BYTES {
                    cache.clear();
                    Ok(NavigationInspection {
                        truncated: true,
                        ..Default::default()
                    })
                } else {
                    // This bounded conversion happens once per accepted revision,
                    // after debounce and exclusively on this worker.
                    cache.inspect(&request.snapshot.to_string(), edit, &cancelled)
                };
                let after_stats = cache.stats();
                total_stats.parses += after_stats.parses - before_stats.parses;
                total_stats.incremental_parses +=
                    after_stats.incremental_parses - before_stats.incremental_parses;
                if cancelled() {
                    continue;
                }
                if result.is_ok() {
                    previous = Some((request.key.clone(), request.snapshot));
                }
                if let Ok(mut slot) = mailbox.reply.lock() {
                    *slot = Some(Reply {
                        key: request.key,
                        generation: request.generation,
                        result,
                        stats: total_stats,
                    });
                }
            }
        })
        .map(|_| ())
        .map_err(|e| format!("Não foi possível iniciar análise de declarações: {e}"))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn wait(service: &mut NavigationGutter) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while service.pending() {
            service.poll();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(3));
        }
    }
    #[test]
    fn stable_frames_do_not_parse_again_and_latest_revision_wins() {
        let mut service = NavigationGutter::default();
        let path = Path::new("/fixture/service.php");
        let source = TextSnapshot::from_str("<?php function first(){}\r\n");
        assert!(service.schedule(1, path, 1, source.clone(), None));
        wait(&mut service);
        assert_eq!(service.anchors(1, 1)[0].name, "first");
        let stats = service.stats();
        let stable = service.anchors(1, 1);
        for _ in 0..1000 {
            assert!(!service.schedule(1, path, 1, source.clone(), None));
            assert!(!service.poll());
            assert!(Arc::ptr_eq(&stable, &service.anchors(1, 1)));
        }
        assert_eq!(stats, service.stats());
        assert_eq!(service.scheduled(), 1);
        service.schedule(
            1,
            path,
            2,
            TextSnapshot::from_str("<?php function stale(){}"),
            None,
        );
        service.schedule(
            1,
            path,
            3,
            TextSnapshot::from_str("<?php function newest(){}"),
            None,
        );
        assert!(service.anchors(1, 1).is_empty());
        wait(&mut service);
        assert!(service.anchors(1, 2).is_empty());
        assert_eq!(service.anchors(1, 3)[0].name, "newest");
    }
    #[test]
    fn incremental_rope_edit_and_cancel_preserve_exact_scalar_coordinates() {
        let mut service = NavigationGutter::default();
        let path = Path::new("/fixture/service.php");
        let before = "<?php /* 😀 */\r\nfunction before(){}";
        let source = TextSnapshot::from_str(before);
        service.schedule(1, path, 1, source.clone(), None);
        wait(&mut service);
        let index = before[..before.find("before").unwrap()].chars().count();
        let after = TextSnapshot::from_str(&before.replace("before", "after"));
        let change = TextChange {
            revision: 2,
            old_chars: index..index + 6,
            new_chars: index..index + 5,
            old_lines: 1..2,
            new_lines: 1..2,
        };
        service.schedule(1, path, 2, after, Some(&change));
        wait(&mut service);
        assert_eq!(service.stats().incremental_parses, 1);
        assert_eq!(service.anchors(1, 2)[0].scalar_range, index..index + 5);
        service.schedule(
            1,
            path,
            3,
            TextSnapshot::from_str("<?php function ignored(){}"),
            None,
        );
        service.cancel();
        std::thread::sleep(Duration::from_millis(180));
        assert!(!service.poll());
        assert!(service.anchors(1, 3).is_empty());
        assert!(!service.pending());
    }
    #[test]
    fn old_reply_cannot_clear_new_pending_or_restore_old_symbols() {
        let mut service = NavigationGutter::default();
        let path = Path::new("/fixture/one.php");
        service.schedule(
            7,
            path,
            1,
            TextSnapshot::from_str("<?php function old(){}"),
            None,
        );
        wait(&mut service);
        let key = service.key.clone().unwrap();
        let generation = service.generation;
        let old = service.anchors(7, 1).to_vec();
        service.schedule(
            7,
            path,
            2,
            TextSnapshot::from_str("<?php function current(){}"),
            None,
        );
        *service.mailbox.reply.lock().unwrap() = Some(Reply {
            key,
            generation,
            result: Ok(NavigationInspection {
                anchors: old,
                ..Default::default()
            }),
            stats: NavigationStats::default(),
        });
        assert!(!service.poll());
        assert!(service.pending());
        assert!(service.anchors(7, 1).is_empty());
        assert!(service.anchors(7, 2).is_empty());
        wait(&mut service);
        assert_eq!(service.anchors(7, 2)[0].name, "current");
    }
    #[test]
    fn oversized_document_does_not_parse_and_drop_releases_worker_mailbox() {
        let mut service = NavigationGutter::default();
        service.schedule(
            1,
            Path::new("/fixture/large.php"),
            1,
            TextSnapshot::from_str(&" ".repeat(MAX_BYTES + 1)),
            None,
        );
        wait(&mut service);
        assert!(service.truncated);
        assert!(service.anchors(1, 1).is_empty());
        assert_eq!(service.stats().parses, 0);
        let weak = Arc::downgrade(&service.mailbox);
        drop(service);
        let deadline = Instant::now() + Duration::from_secs(2);
        while weak.upgrade().is_some() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(3));
        }
    }
}
