//! Personal worklogs in the selected period only; bounded and cancellable.
use crate::{
    client::JiraClient,
    model::string,
    period::{self, Period},
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

pub const BATCH_REQUESTS: usize = 120;
const CONCURRENT_WORKLOGS: usize = 4;
const STEP_TIMEOUT: Duration = Duration::from_secs(10);
const BATCH_TIME: Duration = Duration::from_secs(15);
/// Explicit read-only integration entry point. Identity always comes from /myself.
/// A bounded read can return Paused; it must never be presented as a complete total.
pub async fn read_personal(
    client: &mut JiraClient,
    period: Period,
    budget: usize,
) -> Result<Snapshot, String> {
    let identity = client.connect().await?;
    let mut scan = Scan::new(period, identity.account)?;
    let budget = budget.clamp(1, BATCH_REQUESTS);
    while scan.requests < budget {
        match scan.step(client, budget - scan.requests).await {
            Ok(true) => return Ok(scan.snapshot(State::Complete, None)),
            Ok(false) => {}
            Err(e) => return Ok(scan.snapshot(State::Error, Some(e))),
        }
    }
    Ok(scan.snapshot(
        State::Paused,
        Some("Leitura limitada; resultado parcial.".into()),
    ))
}
const MAX_ENTRIES: usize = 20_000;
const MAX_ISSUES: usize = 50_000;
#[derive(Clone, Debug)]
pub struct Entry {
    pub id: String,
    pub key: String,
    pub title: String,
    pub date: String,
    pub time: String,
    pub seconds: i64,
    pub note: String,
    pub instant: i64,
}
#[derive(Clone, Debug, Default)]
pub struct Day {
    pub date: String,
    pub seconds: i64,
    pub cards: usize,
    pub records: usize,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Loading,
    Complete,
    Paused,
    Error,
}
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub period: Period,
    pub entries: Vec<Entry>,
    pub days: BTreeMap<String, Day>,
    pub seconds: i64,
    pub cards: usize,
    pub inspected: usize,
    pub requests: usize,
    pub state: State,
    pub message: Option<String>,
}
impl Snapshot {
    pub fn from_entries(
        period: Period,
        mut entries: Vec<Entry>,
        state: State,
        inspected: usize,
        requests: usize,
        message: Option<String>,
    ) -> Self {
        entries.sort_by(|a, b| {
            a.instant
                .cmp(&b.instant)
                .then_with(|| a.key.cmp(&b.key))
                .then_with(|| a.id.cmp(&b.id))
        });
        let mut days: BTreeMap<String, Day> = BTreeMap::new();
        let mut cards = HashSet::new();
        let mut day_cards: BTreeMap<String, HashSet<String>> = BTreeMap::new();
        let mut seconds = 0;
        for e in &entries {
            seconds += e.seconds;
            cards.insert(e.key.clone());
            let d = days.entry(e.date.clone()).or_insert_with(|| Day {
                date: e.date.clone(),
                ..Default::default()
            });
            d.seconds += e.seconds;
            d.records += 1;
            day_cards
                .entry(e.date.clone())
                .or_default()
                .insert(e.key.clone());
        }
        for (k, c) in day_cards {
            if let Some(d) = days.get_mut(&k) {
                d.cards = c.len();
            }
        }
        Self {
            period,
            entries,
            days,
            seconds,
            cards: cards.len(),
            inspected,
            requests,
            state,
            message,
        }
    }
}
#[derive(Clone, Debug)]
struct Candidate {
    key: String,
    title: String,
}
struct Current {
    issue: Candidate,
    offset: usize,
}
struct Scan {
    period: Period,
    from: i64,
    to: i64,
    account: String,
    entries: Vec<Entry>,
    ids: HashSet<String>,
    seen: HashSet<String>,
    queue: VecDeque<Candidate>,
    current: Vec<Current>,
    cursor: Option<String>,
    page_tokens: HashSet<String>,
    search_done: bool,
    inspected: usize,
    requests: usize,
}
impl Scan {
    fn new(period: Period, account: String) -> Result<Self, String> {
        let (from, to) = period.instants()?;
        Ok(Self {
            period,
            from,
            to,
            account,
            entries: vec![],
            ids: HashSet::new(),
            seen: HashSet::new(),
            queue: VecDeque::new(),
            current: vec![],
            cursor: None,
            page_tokens: HashSet::new(),
            search_done: false,
            inspected: 0,
            requests: 0,
        })
    }
    fn snapshot(&self, state: State, message: Option<String>) -> Snapshot {
        Snapshot::from_entries(
            self.period.clone(),
            self.entries.clone(),
            state,
            self.inspected,
            self.requests,
            message,
        )
    }
    fn accept(&mut self, v: &Value, issue: &Candidate) -> Result<(), String> {
        // Never retain another author's time, text or identity in a snapshot.
        if v["author"]["accountId"].as_str() != Some(self.account.as_str()) {
            return Ok(());
        }
        let (date, time, instant) = period::local_day(&string(v, "started"))?;
        if instant < self.from || instant >= self.to {
            return Ok(());
        }
        let id = string(v, "id");
        if id.is_empty() {
            return Err(
                "Apontamento pessoal sem identidade; resultado não pode ser confirmado".into(),
            );
        }
        let identity = format!("{}:{id}", issue.key);
        if self.ids.contains(&identity) {
            return Ok(());
        }
        let seconds = v["timeSpentSeconds"]
            .as_i64()
            .filter(|n| *n > 0 && *n <= 31 * 24 * 3600)
            .ok_or("Apontamento pessoal com duração inválida")?;
        if self.entries.len() >= MAX_ENTRIES {
            return Err(
                "Limite de20mil apontamentos atingido. Os valores exibidos são parciais.".into(),
            );
        }
        let note = crate::content::markdown(&v["comment"])
            .chars()
            .take(240)
            .collect();
        self.ids.insert(identity);
        self.entries.push(Entry {
            id,
            key: issue.key.clone(),
            title: issue.title.clone(),
            date,
            time,
            seconds,
            note,
            instant,
        });
        Ok(())
    }
    async fn step(&mut self, c: &JiraClient, remaining: usize) -> Result<bool, String> {
        let parallel = CONCURRENT_WORKLOGS.min(remaining);
        while self.current.len() < parallel {
            let Some(issue) = self.queue.pop_front() else {
                break;
            };
            self.current.push(Current { issue, offset: 0 });
        }
        if !self.current.is_empty() {
            // Borrowed futures are dropped on cancellation/timeout; cursors advance
            // only after a full validated response. One page per card in flight.
            async fn load(
                c: &JiraClient,
                current: Option<&Current>,
                from: i64,
                to: i64,
            ) -> Option<Result<(Vec<Value>, usize), String>> {
                let current = current?;
                Some(
                    c.period_worklogs(&current.issue.key, current.offset, from, to)
                        .await,
                )
            }
            let active = self.current.len().min(parallel);
            let items = &self.current[..active];
            let (a, b, d, e) = tokio::join!(
                load(c, items.first(), self.from, self.to),
                load(c, items.get(1), self.from, self.to),
                load(c, items.get(2), self.from, self.to),
                load(c, items.get(3), self.from, self.to),
            );
            let mut error = None;
            let mut pending = Vec::new();
            let current = std::mem::take(&mut self.current);
            for (mut current, result) in current.into_iter().zip([a, b, d, e]) {
                let Some(result) = result else {
                    pending.push(current);
                    continue;
                };
                self.requests += 1;
                let result = result.and_then(|(values, total)| {
                    if values.is_empty() && current.offset < total {
                        return Err(
                            "O Jira interrompeu a paginação dos apontamentos; resultado parcial"
                                .into(),
                        );
                    }
                    for value in &values {
                        self.accept(value, &current.issue)?;
                    }
                    Ok((values.len(), total))
                });
                match result {
                    Ok((count, total)) if current.offset + count < total => {
                        current.offset += count;
                        pending.push(current);
                    }
                    Ok(_) => self.inspected += 1,
                    Err(e) => {
                        error.get_or_insert(e);
                        pending.push(current);
                    }
                }
            }
            self.current = pending;
            if let Some(error) = error {
                return Err(error);
            }
            return Ok(self.search_done && self.current.is_empty() && self.queue.is_empty());
        }
        if self.search_done {
            return Ok(true);
        }
        let (v, next) = c
            .timesheet_candidates(&self.period, self.cursor.clone())
            .await?;
        self.requests += 1;
        for issue in v {
            let key = string(&issue, "key");
            if !crate::issue_key(&key) {
                return Err("O Jira retornou uma chave de card inválida no relatório".into());
            }
            if self.seen.contains(&key) {
                continue;
            }
            if self.seen.len() >= MAX_ISSUES {
                return Err("Limite de50mil cards conferidos atingido; resultado parcial".into());
            }
            self.seen.insert(key.clone());
            self.queue.push_back(Candidate {
                key,
                title: string(&issue["fields"], "summary")
                    .chars()
                    .take(512)
                    .collect(),
            });
        }
        if let Some(next) = &next
            && !self.page_tokens.insert(next.clone())
        {
            return Err("Cursor repetido pelo Jira; resultado parcial".into());
        }
        self.search_done = next.is_none();
        self.cursor = next;
        Ok(self.search_done && self.queue.is_empty())
    }
}
struct Session {
    client: Arc<JiraClient>,
    account: String,
}
struct Job {
    id: u64,
    session: Arc<Session>,
    scan: Box<Scan>,
}
struct Update {
    id: u64,
    view: Arc<Snapshot>,
    resume: Option<Box<Scan>>,
    done: bool,
}
#[derive(Default)]
struct Mailbox {
    job: Option<Job>,
    update: Option<Update>,
    stop: bool,
}
pub struct Service {
    pub view: Option<Arc<Snapshot>>,
    pub busy: bool,
    pub stale: bool,
    pub error: Option<String>,
    session: Option<Arc<Session>>,
    mail: Arc<(Mutex<Mailbox>, Condvar)>,
    generation: Arc<AtomicU64>,
    started: bool,
    resume: Option<Box<Scan>>,
}
impl Default for Service {
    fn default() -> Self {
        Self {
            view: None,
            busy: false,
            stale: false,
            error: None,
            session: None,
            mail: Arc::new((Mutex::new(Mailbox::default()), Condvar::new())),
            generation: Arc::new(AtomicU64::new(0)),
            started: false,
            resume: None,
        }
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut m) = self.mail.0.lock() {
            m.stop = true;
            m.job = None;
            self.mail.1.notify_one();
        }
    }
}
impl Service {
    pub(crate) fn connection(&mut self, c: Option<(JiraClient, String)>) {
        self.cancel();
        self.session = c.map(|(client, account)| {
            Arc::new(Session {
                client: Arc::new(client),
                account,
            })
        });
        self.view = None;
        self.resume = None;
        self.error = None;
        self.stale = self.session.is_some();
    }
    pub fn cancel(&mut self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
        self.busy = false;
        self.resume = None;
        if let Some(v) = &self.view
            && v.state == State::Loading
        {
            let mut v = v.as_ref().clone();
            v.state = State::Paused;
            v.message =
                Some("Consulta interrompida. Atualize para conferir o período completo.".into());
            self.view = Some(Arc::new(v));
        }
        if let Ok(mut m) = self.mail.0.lock() {
            m.job = None;
            m.update = None;
        }
    }
    pub fn start(&mut self, period: Period) -> Result<(), String> {
        let session = self
            .session
            .as_ref()
            .ok_or("Conecte a conta Jira para consultar suas horas")?
            .clone();
        let scan = Box::new(Scan::new(period, session.account.clone())?);
        self.cancel();
        self.view = Some(Arc::new(scan.snapshot(State::Loading, None)));
        self.stale = false;
        self.error = None;
        self.enqueue(session, scan)
    }
    pub fn continue_scan(&mut self) -> Result<(), String> {
        let session = self
            .session
            .as_ref()
            .ok_or("Conecte sua conta Jira")?
            .clone();
        let scan = self
            .resume
            .take()
            .ok_or("Atualize para reiniciar a consulta")?;
        self.error = None;
        self.enqueue(session, scan)
    }
    fn enqueue(&mut self, session: Arc<Session>, scan: Box<Scan>) -> Result<(), String> {
        if !self.started {
            let mail = self.mail.clone();
            let generation = self.generation.clone();
            std::thread::Builder::new()
                .name("codisseu-jira-timesheet".into())
                .spawn(move || worker(mail, generation))
                .map_err(|_| "Não foi possível iniciar a consulta de horas")?;
            self.started = true;
        }
        let id = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        let mut m = self.mail.0.lock().map_err(|_| "Consulta indisponível")?;
        m.job = Some(Job { id, session, scan });
        m.update = None;
        self.busy = true;
        self.mail.1.notify_one();
        Ok(())
    }
    pub fn resumable(&self) -> bool {
        !self.busy && self.resume.is_some()
    }
    pub fn poll(&mut self) -> bool {
        let update = self.mail.0.lock().ok().and_then(|mut m| m.update.take());
        if let Some(u) = update
            && u.id == self.generation.load(Ordering::Relaxed)
        {
            self.busy = !u.done;
            self.error = if u.view.state == State::Error {
                u.view.message.clone()
            } else {
                None
            };
            self.view = Some(u.view);
            self.resume = u.resume;
            return true;
        }
        false
    }
}
fn publish(mail: &Mutex<Mailbox>, id: u64, view: Snapshot, resume: Option<Box<Scan>>, done: bool) {
    if let Ok(mut m) = mail.lock() {
        m.update = Some(Update {
            id,
            view: Arc::new(view),
            resume,
            done,
        });
    }
}
fn worker(mail: Arc<(Mutex<Mailbox>, Condvar)>, generation: Arc<AtomicU64>) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build();
    loop {
        let job = {
            let Ok(mut m) = mail.0.lock() else { return };
            while m.job.is_none() && !m.stop {
                m = match mail.1.wait(m) {
                    Ok(m) => m,
                    Err(_) => return,
                };
            }
            if m.stop {
                return;
            }
            m.job.take().unwrap()
        };
        let Job {
            id,
            session,
            mut scan,
        } = job;
        let Ok(rt) = &rt else {
            let snapshot = scan.snapshot(
                State::Error,
                Some(
                    "Não foi possível iniciar a consulta de horas. Atualize para tentar novamente."
                        .into(),
                ),
            );
            publish(&mail.0, id, snapshot, None, true);
            continue;
        };
        let initial_requests = scan.requests;
        let batch_started = Instant::now();
        let mut last = Instant::now();
        loop {
            if id != generation.load(Ordering::Relaxed) {
                break;
            }
            let remaining = BATCH_REQUESTS - (scan.requests - initial_requests);
            let result=rt.block_on(async {
                tokio::select! {
                    result=tokio::time::timeout(STEP_TIMEOUT, scan.step(&session.client, remaining))=>Some(result.unwrap_or_else(|_| Err("O Jira demorou mais de10s nesta etapa. Os valores são parciais; tente continuar a consulta.".into()))),
                    _=async {while id==generation.load(Ordering::Relaxed){tokio::time::sleep(Duration::from_millis(25)).await}}=>None,
                }
            });
            if id != generation.load(Ordering::Relaxed) {
                break;
            }
            match result {
                None => break,
                Some(Err(e)) => {
                    let snapshot = scan.snapshot(State::Error, Some(e));
                    publish(&mail.0, id, snapshot, Some(scan), true);
                    break;
                }
                Some(Ok(true)) => {
                    publish(
                        &mail.0,
                        id,
                        scan.snapshot(State::Complete, None),
                        None,
                        true,
                    );
                    break;
                }
                Some(Ok(false)) => {
                    if scan.requests - initial_requests >= BATCH_REQUESTS
                        || batch_started.elapsed() >= BATCH_TIME
                    {
                        let snapshot = scan.snapshot(
                            State::Paused,
                            Some(
                                "Parcial: continue a conferência para obter o total do período."
                                    .into(),
                            ),
                        );
                        publish(&mail.0, id, snapshot, Some(scan), true);
                        break;
                    }
                    if last.elapsed() >= Duration::from_millis(250) {
                        publish(
                            &mail.0,
                            id,
                            scan.snapshot(State::Loading, None),
                            None,
                            false,
                        );
                        last = Instant::now();
                    }
                }
            }
        }
    }
}
#[cfg(test)]
#[path = "timesheet_tests.rs"]
mod tests;
