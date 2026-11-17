use crate::{
    worker::{Worker, WorkerEvent},
    *,
};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::mpsc::{self, Receiver, SyncSender},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
};
struct Handle {
    shutdown: Arc<AtomicBool>,
    tx: SyncSender<DebugCommand>,
    join: Option<JoinHandle<DebugSession>>,
    breakpoint_updates: BTreeMap<PathBuf, Vec<Breakpoint>>,
}
pub struct DebugService {
    pub sessions: Vec<DebugSession>,
    pub breakpoints: Vec<Breakpoint>,
    pub error: Option<String>,
    workers: BTreeMap<SessionId, Handle>,
    rx: Receiver<WorkerEvent>,
    tx: SyncSender<WorkerEvent>,
    next: u64,
    breakpoints_enabled: bool,
    breakpoint_revision: u64,
}
impl Default for DebugService {
    fn default() -> Self {
        let (tx, rx) = mpsc::sync_channel(8);
        Self {
            sessions: vec![],
            breakpoints: vec![],
            error: None,
            workers: BTreeMap::new(),
            rx,
            tx,
            next: 1,
            breakpoints_enabled: true,
            breakpoint_revision: 0,
        }
    }
}
impl DebugService {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn has_active_sessions(&self) -> bool {
        self.sessions.iter().any(|x| x.state.is_active())
    }
    pub fn is_busy(&self) -> bool {
        self.sessions
            .iter()
            .any(|x| x.state.is_active() && x.pending > 0)
            || self
                .workers
                .values()
                .any(|h| !h.breakpoint_updates.is_empty())
    }
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        for handle in self.workers.values_mut() {
            changed |= flush_breakpoint_updates(handle);
        }
        while let Ok(event) = self.rx.try_recv() {
            changed = true;
            let WorkerEvent::Session(s) = event;
            if let Some(old) = self.sessions.iter_mut().find(|x| x.id == s.id) {
                *old = *s;
            }
        }
        let finished: Vec<_> = self
            .workers
            .iter()
            .filter_map(|(id, h)| h.join.as_ref().filter(|x| x.is_finished()).map(|_| *id))
            .collect();
        for id in finished {
            if let Some(mut h) = self.workers.remove(&id)
                && let Some(join) = h.join.take()
            {
                match join.join() {
                    Ok(final_session) => {
                        if let Some(s) = self.sessions.iter_mut().find(|x| x.id == id) {
                            *s = final_session;
                            changed = true;
                        }
                    }
                    Err(_) => {
                        if let Some(s) = self.sessions.iter_mut().find(|x| x.id == id) {
                            s.state = DebugState::Failed;
                            s.pending = 0;
                            s.last_error = Some("Debug worker exited unexpectedly".into());
                            changed = true;
                        }
                    }
                }
            }
        }
        changed
    }
    pub fn execute(&mut self, command: DebugCommand) -> Result<Option<SessionId>, String> {
        self.error = None;
        let result = self.execute_inner(command);
        if let Err(error) = &result {
            self.error = Some(error.clone())
        }
        result
    }
    fn execute_inner(&mut self, command: DebugCommand) -> Result<Option<SessionId>, String> {
        match command {
            DebugCommand::Start(config) => {
                if !config.trusted {
                    return Err(
                        "Authorize this debug configuration before executing project code".into(),
                    );
                }
                if (config.integrated_browser
                    || config.browser_tab.is_some()
                    || config.browser_inspect_uri.is_some())
                    && (!config.integrated_browser
                        || config.runtime != DebugRuntime::Chrome
                        || config.request != DebugRequest::Attach
                        || config.browser_tab.is_none()
                        || config.browser_inspect_uri.is_none()
                        || config.port == 0)
                {
                    return Err(
                        "Select a live integrated browser tab and authorize a new debug connection"
                            .into(),
                    );
                }
                if config.runtime == DebugRuntime::Rust && config.request != DebugRequest::Launch {
                    return Err(
                        "Rust debugging currently supports Launch of a compiled binary".into(),
                    );
                }
                if config.runtime == DebugRuntime::Python {
                    if config.request != DebugRequest::Launch {
                        return Err(
                            "Python debugging currently supports explicit Launch only".into()
                        );
                    }
                    if config.program.is_some() == config.module.is_some() {
                        return Err(
                            "Python debug requires exactly one file or module target".into()
                        );
                    }
                    if config
                        .module
                        .as_ref()
                        .is_some_and(|m| m.trim().is_empty() || m.len() > 1024 || m.contains('\0'))
                    {
                        return Err(
                            "Python debug module is empty or exceeds the supported limits".into(),
                        );
                    }
                    if config.program.as_ref().is_some_and(|p| {
                        p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pyi"))
                    }) {
                        return Err(
                            "Python .pyi files are type stubs and cannot be executed".into()
                        );
                    }
                } else if config.module.is_some() {
                    return Err("A module debug target is supported only for Python".into());
                }
                if config.runtime == DebugRuntime::Php
                    && config.request != DebugRequest::Launch
                    && config.listen_host.parse::<std::net::IpAddr>().is_err()
                {
                    return Err("Xdebug listen host must be an explicit IP address".into());
                }
                if self.workers.len() >= 8 {
                    return Err("At most eight concurrent debug sessions are supported".into());
                }
                if config.cwd.as_os_str().is_empty() || !config.cwd.is_dir() {
                    return Err("Debug working directory does not exist".into());
                }
                if config.request == DebugRequest::Launch
                    && config.runtime != DebugRuntime::Chrome
                    && !(config.runtime == DebugRuntime::Python && config.module.is_some())
                    && config.program.as_ref().is_none_or(|x| !x.is_file())
                {
                    return Err("Select an existing program to debug".into());
                }
                let id = self.next;
                self.next += 1;
                let session = DebugSession::new(id, &config);
                self.sessions.push(session.clone());
                if self.sessions.len() > 32
                    && let Some(i) = self.sessions.iter().position(|x| !x.state.is_active())
                {
                    self.sessions.remove(i);
                }
                let (tx, rx) = mpsc::sync_channel(32);
                let events = self.tx.clone();
                let breakpoints = self.effective_breakpoints(None);
                let shutdown = Arc::new(AtomicBool::new(false));
                let worker_shutdown = shutdown.clone();
                let join = std::thread::Builder::new()
                    .name(format!("codisseu-debug-{id}"))
                    .spawn(move || {
                        Worker::run(session, config, breakpoints, rx, events, worker_shutdown)
                    })
                    .map_err(|x| x.to_string())?;
                self.workers.insert(
                    id,
                    Handle {
                        shutdown,
                        tx,
                        join: Some(join),
                        breakpoint_updates: BTreeMap::new(),
                    },
                );
                Ok(Some(id))
            }
            DebugCommand::SetBreakpointsEnabled(enabled) => {
                self.breakpoints_enabled = enabled;
                self.breakpoint_revision += 1;
                for point in &mut self.breakpoints {
                    point.revision = self.breakpoint_revision;
                }
                let paths: std::collections::BTreeSet<_> =
                    self.breakpoints.iter().map(|x| x.path.clone()).collect();
                for path in paths {
                    self.broadcast_breakpoints(&path)?;
                }
                Ok(None)
            }
            DebugCommand::ToggleBreakpoint { path, line } => {
                if line == 0 {
                    return Err("Breakpoint lines are one-based".into());
                }
                let mut lines: Vec<u32> = self
                    .breakpoints
                    .iter()
                    .filter(|x| x.path == path && x.line != line)
                    .map(|x| x.line)
                    .collect();
                if !self
                    .breakpoints
                    .iter()
                    .any(|x| x.path == path && x.line == line)
                {
                    lines.push(line)
                }
                self.set_breakpoints(path, lines)?;
                Ok(None)
            }
            DebugCommand::SetBreakpoints { path, lines } => {
                self.set_breakpoints(path, lines)?;
                Ok(None)
            }
            DebugCommand::SetBreakpoint(point) => {
                let path = point.path.clone();
                let mut points: Vec<_> = self
                    .breakpoints
                    .iter()
                    .filter(|b| b.path == path && b.line != point.line)
                    .cloned()
                    .collect();
                points.push(point);
                self.set_source_breakpoints(path, points)?;
                Ok(None)
            }
            DebugCommand::SetSourceBreakpoints { path, breakpoints } => {
                self.set_source_breakpoints(path, breakpoints)?;
                Ok(None)
            }
            cmd => {
                let id = match &cmd {
                    DebugCommand::Stop(id)
                    | DebugCommand::Continue(id)
                    | DebugCommand::Next(id)
                    | DebugCommand::StepIn(id)
                    | DebugCommand::StepOut(id)
                    | DebugCommand::Pause(id)
                    | DebugCommand::Threads(id) => *id,
                    DebugCommand::SelectThread { session, .. }
                    | DebugCommand::SelectFrame { session, .. }
                    | DebugCommand::Variables { session, .. }
                    | DebugCommand::Evaluate { session, .. }
                    | DebugCommand::SetExceptionBreakpoints { session, .. } => *session,
                    _ => unreachable!(),
                };
                let state = self
                    .sessions
                    .iter()
                    .find(|x| x.id == id)
                    .ok_or("Unknown debug session")?
                    .state;
                if !state.is_active() {
                    return Err("Debug session has ended".into());
                }
                if let DebugCommand::SetExceptionBreakpoints { filters, .. } = &cmd {
                    validate_exception_filters(
                        filters,
                        &self
                            .sessions
                            .iter()
                            .find(|s| s.id == id)
                            .unwrap()
                            .capabilities,
                    )?;
                }
                if matches!(cmd, DebugCommand::Pause(_)) && state != DebugState::Running {
                    return Err("The debug target must be running before pausing".into());
                }
                let needs_stop = matches!(
                    cmd,
                    DebugCommand::Continue(_)
                        | DebugCommand::Next(_)
                        | DebugCommand::StepIn(_)
                        | DebugCommand::StepOut(_)
                        | DebugCommand::SelectThread { .. }
                        | DebugCommand::SelectFrame { .. }
                        | DebugCommand::Variables { .. }
                        | DebugCommand::Evaluate { .. }
                );
                if needs_stop && state != DebugState::Stopped {
                    return Err("Pause the debug session first".into());
                }
                let h = self.workers.get(&id).ok_or("Debug worker unavailable")?;
                if matches!(cmd, DebugCommand::Stop(_)) {
                    h.shutdown.store(true, Ordering::Release);
                    return Ok(None);
                }
                h.tx.try_send(cmd)
                    .map_err(|x| format!("Debug command queue unavailable: {x}"))?;
                Ok(None)
            }
        }
    }
    fn set_breakpoints(&mut self, path: PathBuf, mut lines: Vec<u32>) -> Result<(), String> {
        if lines.contains(&0)
            || lines.len() + self.breakpoints.iter().filter(|x| x.path != path).count() > 512
        {
            return Err("Invalid breakpoint list (one-based, maximum 512 total)".into());
        }
        lines.sort_unstable();
        lines.dedup();
        let points = lines
            .into_iter()
            .map(|line| {
                self.breakpoints
                    .iter()
                    .find(|p| p.path == path && p.line == line)
                    .cloned()
                    .unwrap_or_else(|| Breakpoint::new(path.clone(), line))
            })
            .collect();
        self.set_source_breakpoints(path, points)
    }
    pub fn breakpoints_enabled(&self) -> bool {
        self.breakpoints_enabled
    }
    fn effective_breakpoints(&self, path: Option<&PathBuf>) -> Vec<Breakpoint> {
        self.breakpoints
            .iter()
            .filter(|b| path.is_none_or(|p| &b.path == p))
            .cloned()
            .map(|mut b| {
                b.enabled &= self.breakpoints_enabled;
                b.verified = false;
                b.message = None;
                b
            })
            .collect()
    }
    fn broadcast_breakpoints(&mut self, path: &PathBuf) -> Result<(), String> {
        let points = self.effective_breakpoints(Some(path));
        for h in self.workers.values_mut() {
            h.breakpoint_updates.insert(path.clone(), points.clone());
            flush_breakpoint_updates(h);
        }
        Ok(())
    }
    fn set_source_breakpoints(
        &mut self,
        path: PathBuf,
        mut points: Vec<Breakpoint>,
    ) -> Result<(), String> {
        if points.len() + self.breakpoints.iter().filter(|b| b.path != path).count() > 512 {
            return Err("Maximum 512 breakpoints supported".into());
        }
        for point in &mut points {
            if point.path != path || point.line == 0 || point.line > 1_000_000 {
                return Err("Breakpoint path/line invalid (1..1,000,000)".into());
            }
            point.options.validate()?;
            point.verified = false;
            point.message = None;
        }
        points.sort_by_key(|b| b.line);
        if points.windows(2).any(|b| b[0].line == b[1].line) {
            return Err("Duplicate breakpoint line".into());
        }
        self.breakpoint_revision += 1;
        for point in &mut points {
            point.revision = self.breakpoint_revision;
        }
        self.breakpoints.retain(|b| b.path != path);
        self.breakpoints.extend(points);
        self.broadcast_breakpoints(&path)
    }
}

// Coalesce by source while the bounded worker queue is full. New properties
// never get lost because typing/evaluation temporarily occupies that queue.
fn flush_breakpoint_updates(handle: &mut Handle) -> bool {
    let mut changed = false;
    while let Some((path, points)) = handle.breakpoint_updates.first_key_value() {
        let command = DebugCommand::SetSourceBreakpoints {
            path: path.clone(),
            breakpoints: points.clone(),
        };
        match handle.tx.try_send(command) {
            Ok(()) => {
                handle.breakpoint_updates.pop_first();
                changed = true;
            }
            Err(mpsc::TrySendError::Full(_)) => break,
            // The final worker snapshot reports termination/failure. Desired
            // settings remain in the service/profile for the next session.
            Err(mpsc::TrySendError::Disconnected(_)) => {
                handle.breakpoint_updates.clear();
                break;
            }
        }
    }
    changed
}

pub(crate) fn validate_exception_filters(
    filters: &[String],
    capabilities: &serde_json::Value,
) -> Result<(), String> {
    if filters.len() > 32 || filters.iter().any(|f| f.len() > 256) {
        return Err("Exception breakpoint filters exceed the supported limits".into());
    }
    for filter in filters {
        if !capabilities["exceptionBreakpointFilters"]
            .as_array()
            .is_some_and(|list| list.iter().any(|v| v["filter"].as_str() == Some(filter)))
        {
            return Err(format!("Unsupported exception breakpoint filter: {filter}"));
        }
    }
    Ok(())
}
impl Drop for DebugService {
    fn drop(&mut self) {
        for h in self.workers.values() {
            h.shutdown.store(true, Ordering::Release);
        }
        for (_, mut h) in std::mem::take(&mut self.workers) {
            if let Some(join) = h.join.take() {
                let _ = join.join();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_worker_queue_coalesces_latest_properties_without_dropping_updates() {
        let mut service = DebugService::new();
        let (tx, rx) = mpsc::sync_channel(1);
        tx.try_send(DebugCommand::Threads(1)).unwrap();
        service.workers.insert(
            1,
            Handle {
                shutdown: Arc::new(AtomicBool::new(false)),
                tx,
                join: None,
                breakpoint_updates: BTreeMap::new(),
            },
        );
        let path = PathBuf::from("fixture.php");
        for i in 0..100 {
            let mut point = Breakpoint::new(path.clone(), 3);
            point.options.condition = format!("$i == {i}");
            service.execute(DebugCommand::SetBreakpoint(point)).unwrap();
        }
        assert_eq!(service.workers[&1].breakpoint_updates.len(), 1);
        rx.try_recv().unwrap();
        service.poll();
        let DebugCommand::SetSourceBreakpoints { breakpoints, .. } = rx.try_recv().unwrap() else {
            panic!()
        };
        assert_eq!(breakpoints[0].options.condition, "$i == 99");
        assert!(service.workers[&1].breakpoint_updates.is_empty());
    }
    #[test]
    fn options_disabled_and_master_mute_survive_other_breakpoint_changes() {
        let mut service = DebugService::new();
        let path = PathBuf::from("fixture.php");
        let mut point = Breakpoint::new(path.clone(), 3);
        point.enabled = false;
        point.options.condition = "$i === 3".into();
        point.options.log_message = "i={$i}".into();
        service
            .execute(DebugCommand::SetBreakpoint(point.clone()))
            .unwrap();
        for enabled in [false, true, false] {
            service
                .execute(DebugCommand::SetBreakpointsEnabled(enabled))
                .unwrap();
            service
                .execute(DebugCommand::ToggleBreakpoint {
                    path: path.clone(),
                    line: 4,
                })
                .unwrap();
            let existing = service.breakpoints.iter().find(|b| b.line == 3).unwrap();
            assert!(!existing.enabled);
            assert_eq!(existing.options, point.options);
        }
        assert!(
            service
                .effective_breakpoints(None)
                .iter()
                .all(|b| !b.enabled)
        );
        let original = service.breakpoints.clone();
        let mut invalid = point.clone();
        invalid.options.condition.push('\0');
        assert!(
            service
                .execute(DebugCommand::SetBreakpoint(invalid))
                .is_err()
        );
        assert_eq!(service.breakpoints.len(), original.len());
        assert_eq!(service.breakpoints[0].options, original[0].options);
        point.line = 1_000_001;
        assert!(service.execute(DebugCommand::SetBreakpoint(point)).is_err());
    }
    #[test]
    fn stop_bypasses_a_full_command_queue() {
        let mut service = DebugService::new();
        let (tx, _rx) = mpsc::sync_channel(1);
        tx.try_send(DebugCommand::Threads(1)).unwrap();
        let shutdown = Arc::new(AtomicBool::new(false));
        service
            .sessions
            .push(DebugSession::new(1, &DebugConfiguration::default()));
        service.workers.insert(
            1,
            Handle {
                shutdown: shutdown.clone(),
                tx,
                join: None,
                breakpoint_updates: BTreeMap::new(),
            },
        );
        service.execute(DebugCommand::Stop(1)).unwrap();
        assert!(shutdown.load(Ordering::Acquire));
    }
    #[test]
    fn full_snapshot_queue_cannot_hide_terminal_adapter_failure() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("source.js");
        std::fs::write(&path, "console.log(1);").unwrap();
        let mut service = DebugService::new();
        for _ in 0..8 {
            service
                .tx
                .try_send(WorkerEvent::Session(Box::new(DebugSession::new(
                    999,
                    &DebugConfiguration::default(),
                ))))
                .ok()
                .unwrap();
        }
        let mut config = DebugConfiguration::for_file(path, temp.path());
        config.trusted = true;
        config.adapter = Some(temp.path().join("missing-adapter.js"));
        let id = service
            .execute(DebugCommand::Start(config))
            .unwrap()
            .unwrap();
        let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !service.workers[&id].join.as_ref().unwrap().is_finished() {
            assert!(std::time::Instant::now() < end);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        service.poll();
        let session = service.sessions.iter().find(|s| s.id == id).unwrap();
        assert_eq!(session.state, DebugState::Failed);
        assert!(
            session
                .last_error
                .as_ref()
                .unwrap()
                .contains("adapter not found")
        );
        assert_eq!(session.pending, 0);
    }
}
