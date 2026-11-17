//! Opt-in page CDP, private to the debugger channel, with counted byte budgets.
use crate::{
    Command, EngineState, protocol,
    service::{BrowserService, Mailbox, Request},
};
use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc::{self, Receiver, RecvError, RecvTimeoutError, SyncSender, TryRecvError},
};
use std::time::Duration;
const BYTE_BUDGET: usize = 8 * 1024 * 1024;
#[derive(Debug)]
pub enum DevToolsEvent {
    Message { json: String },
    Closed { message: Option<String> },
}
pub struct DevToolsChannel {
    pub sender: DevToolsSender,
    pub events: DevToolsReceiver,
}
#[derive(Clone)]
pub struct DevToolsSender {
    lease: Arc<Lease>,
}
pub struct DevToolsReceiver {
    receive: Receiver<DevToolsEvent>,
    bytes: Arc<AtomicUsize>,
    lease: Arc<Lease>,
}
struct Lease {
    id: u64,
    epoch: u64,
    browser_epoch: u64,
    send: SyncSender<Request>,
    mailbox: Weak<Mutex<Mailbox>>,
    live: AtomicBool,
    output: Arc<AtomicUsize>,
}
pub(crate) struct Subscription {
    pub epoch: u64,
    pub tx: SyncSender<DevToolsEvent>,
    pub bytes: Arc<AtomicUsize>,
}
pub(crate) struct Permit {
    bytes: usize,
    counter: Arc<AtomicUsize>,
}
impl Drop for Permit {
    fn drop(&mut self) {
        self.counter.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}
fn reserve(counter: &Arc<AtomicUsize>, bytes: usize) -> Result<Permit, String> {
    counter
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
            n.checked_add(bytes).filter(|n| *n <= BYTE_BUDGET)
        })
        .map_err(|_| "Fila CDP excede 8 MiB")?;
    Ok(Permit {
        bytes,
        counter: counter.clone(),
    })
}
impl DevToolsSender {
    pub fn send(&self, json: impl Into<String>) -> Result<(), String> {
        let json = json.into();
        protocol::validate_cdp(json.as_bytes())?;
        if !self.lease.live.load(Ordering::Acquire) {
            return Err("Canal DevTools encerrado".into());
        }
        let mailbox = self.lease.mailbox.upgrade().ok_or("Navegador encerrado")?;
        let valid = mailbox.lock().is_ok_and(|m| {
            m.epoch == self.lease.browser_epoch
                && m.wanted
                    .contains_key(&(self.lease.id, crate::Surface::Page))
                && m.subscriptions
                    .get(&self.lease.id)
                    .is_some_and(|s| s.epoch == self.lease.epoch)
        });
        if !valid {
            return Err("Assinatura DevTools obsoleta".into());
        }
        let permit = reserve(&self.lease.output, json.len())?;
        self.lease
            .send
            .try_send(Request {
                epoch: self.lease.browser_epoch,
                command: Command::DevToolsMessage {
                    id: self.lease.id,
                    epoch: self.lease.epoch,
                },
                payload: json.into_bytes(),
                permit: Some(permit),
                start: false,
                config: None,
                last_close: false,
                bootstrap: vec![],
            })
            .map_err(|_| "Fila CDP cheia ou navegador encerrado".into())
    }
}
impl DevToolsReceiver {
    fn received(&self, event: &DevToolsEvent) {
        if let DevToolsEvent::Message { json } = event {
            self.bytes.fetch_sub(json.len(), Ordering::AcqRel);
        }
    }
    pub fn recv(&self) -> Result<DevToolsEvent, RecvError> {
        let e = self.receive.recv()?;
        self.received(&e);
        Ok(e)
    }
    pub fn try_recv(&self) -> Result<DevToolsEvent, TryRecvError> {
        let e = self.receive.try_recv()?;
        self.received(&e);
        Ok(e)
    }
    pub fn recv_timeout(&self, timeout: Duration) -> Result<DevToolsEvent, RecvTimeoutError> {
        let e = self.receive.recv_timeout(timeout)?;
        self.received(&e);
        Ok(e)
    }
}
impl Lease {
    fn unsubscribe(&self) {
        if self.live.swap(false, Ordering::AcqRel) {
            if let Some(m) = self.mailbox.upgrade()
                && let Ok(mut m) = m.lock()
                && m.epoch == self.browser_epoch
                && m.subscriptions
                    .get(&self.id)
                    .is_some_and(|s| s.epoch == self.epoch)
            {
                close(&mut m, self.id, None);
            }
            let _ = self.send.try_send(Request {
                epoch: self.browser_epoch,
                command: Command::DevToolsUnsubscribe {
                    id: self.id,
                    epoch: self.epoch,
                },
                payload: vec![],
                permit: None,
                start: false,
                config: None,
                last_close: false,
                bootstrap: vec![],
            });
        }
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.unsubscribe();
    }
}
impl Drop for DevToolsReceiver {
    fn drop(&mut self) {
        self.lease.unsubscribe();
    }
}
impl BrowserService {
    /// Revoke an owned bridge immediately; engine cleanup uses the priority control lane.
    pub fn end_devtools(&mut self, id: u64) {
        if let Ok(mut m) = self.mailbox.lock() {
            close(
                &mut m,
                id,
                Some("Canal DevTools encerrado pelo aplicativo".into()),
            );
        }
    }
    pub fn devtools(&mut self, id: u64, epoch: u64) -> Result<DevToolsChannel, String> {
        if self.state != EngineState::Ready || !self.tabs.contains_key(&id) || epoch == 0 {
            return Err("DevTools exige aba/motor prontos e epoch positivo".into());
        }
        let (tx, receive) = mpsc::sync_channel(1024);
        let bytes = Arc::new(AtomicUsize::new(0));
        {
            let mut m = self.mailbox.lock().map_err(|_| "Estado CDP indisponível")?;
            if m.subscriptions.contains_key(&id)
                || m.last_cdp_epoch.get(&id).is_some_and(|old| epoch <= *old)
            {
                return Err("Aba já tem uma assinatura DevTools".into());
            }
            m.last_cdp_epoch.insert(id, epoch);
            m.subscriptions.insert(
                id,
                Subscription {
                    epoch,
                    tx,
                    bytes: bytes.clone(),
                },
            );
        }
        if self
            .send
            .try_send(Request {
                epoch: self.epoch,
                command: Command::DevToolsSubscribe { id, epoch },
                payload: vec![],
                permit: None,
                start: false,
                config: None,
                last_close: false,
                bootstrap: vec![],
            })
            .is_err()
        {
            if let Ok(mut m) = self.mailbox.lock() {
                close(&mut m, id, None);
            }
            return Err("Fila do navegador cheia".into());
        }
        let lease = Arc::new(Lease {
            id,
            epoch,
            browser_epoch: self.epoch,
            send: self.send.clone(),
            mailbox: Arc::downgrade(&self.mailbox),
            live: AtomicBool::new(true),
            output: Arc::new(AtomicUsize::new(0)),
        });
        Ok(DevToolsChannel {
            sender: DevToolsSender {
                lease: lease.clone(),
            },
            events: DevToolsReceiver {
                receive,
                bytes,
                lease,
            },
        })
    }
}
pub(crate) fn close(m: &mut Mailbox, id: u64, message: Option<String>) {
    if let Some(subscription) = m.subscriptions.remove(&id) {
        if m.wanted.contains_key(&(id, crate::Surface::Page)) {
            m.pending_cdp_cleanup
                .entry(id)
                .or_insert(subscription.epoch);
        }
        let _ = subscription.tx.try_send(DevToolsEvent::Closed { message });
    }
}
pub(crate) fn clear(m: &mut Mailbox, message: &str) {
    for id in m.subscriptions.keys().copied().collect::<Vec<_>>() {
        close(m, id, Some(message.into()));
    }
}
pub(crate) fn deliver(m: &mut Mailbox, id: u64, epoch: u64, json: String) -> bool {
    let Some(subscription) = m.subscriptions.get(&id).filter(|s| s.epoch == epoch) else {
        return false;
    };
    let Ok(mut permit) = reserve(&subscription.bytes, json.len()) else {
        close(m, id, Some("Fila de eventos CDP excedeu 8 MiB".into()));
        return false;
    };
    if subscription
        .tx
        .try_send(DevToolsEvent::Message { json })
        .is_err()
    {
        close(m, id, Some("Consumidor CDP lento ou encerrado".into()));
        return false;
    }
    permit.bytes = 0;
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unsubscribe_survives_a_completely_full_command_queue() {
        let (send, rx) = mpsc::sync_channel(1);
        let mailbox = Arc::new(Mutex::new(Mailbox::default()));
        {
            let mut m = mailbox.lock().unwrap();
            m.epoch = 1;
            m.wanted.insert(
                (1, crate::Surface::Page),
                crate::service::Desired::default(),
            );
            let (tx, _) = mpsc::sync_channel(1);
            m.subscriptions.insert(
                1,
                Subscription {
                    epoch: 9,
                    tx,
                    bytes: Arc::new(AtomicUsize::new(0)),
                },
            );
        }
        send.try_send(Request {
            epoch: 1,
            command: Command::Shutdown,
            payload: vec![],
            permit: None,
            start: false,
            config: None,
            last_close: false,
            bootstrap: vec![],
        })
        .unwrap_or_else(|_| panic!("owned queue"));
        let lease = Lease {
            id: 1,
            epoch: 9,
            browser_epoch: 1,
            send,
            mailbox: Arc::downgrade(&mailbox),
            live: AtomicBool::new(true),
            output: Arc::new(AtomicUsize::new(0)),
        };
        lease.unsubscribe();
        let m = mailbox.lock().unwrap();
        assert!(m.subscriptions.is_empty());
        assert_eq!(m.pending_cdp_cleanup.get(&1), Some(&9));
        assert!(matches!(rx.try_recv().unwrap().command, Command::Shutdown));
    }
    #[test]
    fn incoming_burst_has_1024_slots_but_remains_byte_bounded_and_releases_bytes() {
        let (send, _requests) = mpsc::sync_channel(64);
        let mailbox = Arc::new(Mutex::new(Mailbox::default()));
        let bytes = Arc::new(AtomicUsize::new(0));
        let (tx, receive) = mpsc::sync_channel(1024);
        {
            let mut m = mailbox.lock().unwrap();
            m.epoch = 1;
            m.wanted.insert(
                (1, crate::Surface::Page),
                crate::service::Desired::default(),
            );
            m.subscriptions.insert(
                1,
                Subscription {
                    epoch: 1,
                    tx,
                    bytes: bytes.clone(),
                },
            );
        }
        let lease = Arc::new(Lease {
            id: 1,
            epoch: 1,
            browser_epoch: 1,
            send,
            mailbox: Arc::downgrade(&mailbox),
            live: AtomicBool::new(true),
            output: Arc::new(AtomicUsize::new(0)),
        });
        let receiver = DevToolsReceiver {
            receive,
            bytes: bytes.clone(),
            lease,
        };
        for _ in 0..512 {
            assert!(deliver(
                &mut mailbox.lock().unwrap(),
                1,
                1,
                r#"{"method":"Network.requestWillBeSent","params":{}}"#.into()
            ));
        }
        assert!(bytes.load(Ordering::Acquire) > 0);
        for _ in 0..512 {
            assert!(matches!(
                receiver.recv().unwrap(),
                DevToolsEvent::Message { .. }
            ));
        }
        assert_eq!(bytes.load(Ordering::Acquire), 0);
        let large = format!(
            r#"{{"method":"Network.event","params":{{"body":"{}"}}}}"#,
            "a".repeat(3 * 1024 * 1024)
        );
        assert!(deliver(&mut mailbox.lock().unwrap(), 1, 1, large.clone()));
        assert!(deliver(&mut mailbox.lock().unwrap(), 1, 1, large.clone()));
        assert!(!deliver(&mut mailbox.lock().unwrap(), 1, 1, large));
        assert!(mailbox.lock().unwrap().subscriptions.is_empty());
        assert_eq!(
            mailbox.lock().unwrap().pending_cdp_cleanup.get(&1),
            Some(&1)
        );
        for _ in 0..2 {
            assert!(matches!(
                receiver.recv().unwrap(),
                DevToolsEvent::Message { .. }
            ));
        }
        assert!(matches!(
            receiver.recv().unwrap(),
            DevToolsEvent::Closed { .. }
        ));
        assert_eq!(bytes.load(Ordering::Acquire), 0);
    }
}
