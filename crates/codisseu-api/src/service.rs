use crate::storage::{ProfileVersion, snapshot_fingerprint};
use crate::*;
use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
    sync::mpsc,
    thread,
};

#[derive(Clone, Debug)]
pub struct HistoryEntry {
    pub request_id: String,
    pub name: String,
    pub method: String,
    pub status: u16,
    pub elapsed_ms: u64,
    pub timestamp_ms: u64,
}
#[derive(Clone, Debug)]
pub struct RunResult {
    pub request_id: String,
    pub name: String,
    pub method: String,
    pub status: Option<u16>,
    pub elapsed_ms: u64,
    pub tests_passed: usize,
    pub tests_failed: usize,
    pub error: Option<String>,
}
#[derive(Clone, Debug)]
pub struct CollectionRun {
    pub collection_id: String,
    pub name: String,
    pub total: usize,
    pub completed: usize,
    pub tests_passed: usize,
    pub tests_failed: usize,
    pub finished: bool,
    pub cancelled: bool,
    pub error: Option<String>,
    pub results: Vec<RunResult>,
}
#[derive(Clone, Debug)]
pub struct FailedSave {
    pub project: PathBuf,
    pub workspace: ApiWorkspace,
    pub error: String,
    baseline: Option<ProfileBaseline>,
}
#[derive(Clone, Debug)]
struct ProfileBaseline {
    version: ProfileVersion,
    fingerprint: [u8; 32],
}
#[derive(Clone, Debug)]
pub struct RunOptions {
    pub iterations: u32,
    pub data: Vec<Variables>,
}
impl Default for RunOptions {
    fn default() -> Self {
        Self {
            iterations: 1,
            data: vec![],
        }
    }
}
impl RunOptions {
    pub fn from_json(iterations: u32, source: &str) -> Result<Self, String> {
        if source.len() > 1024 * 1024 {
            return Err("Runner data exceeds 1 MiB".into());
        }
        if source.trim().is_empty() {
            return Ok(Self {
                iterations,
                data: vec![],
            });
        }
        let rows: Vec<serde_json::Value> = serde_json::from_str(source)
            .map_err(|_| "Runner data must be an array of JSON objects")?;
        if rows.is_empty() || rows.len() > 256 {
            return Err("Runner data requires 1–256 objects".into());
        }
        let mut data = vec![];
        for row in rows {
            let object = row
                .as_object()
                .ok_or("Each runner row must be a JSON object")?;
            if object.len() > 256 {
                return Err("Runner row exceeds 256 variables".into());
            }
            let mut variables = Variables::new();
            for (key, value) in object {
                variables.insert(
                    key.clone(),
                    value
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| value.to_string()),
                );
            }
            data.push(variables);
        }
        Ok(Self { iterations, data })
    }
}
#[derive(Clone)]
struct RunStep {
    id: String,
    iteration: u32,
    data: Variables,
}
enum Work {
    Switch {
        generation: u64,
        project: PathBuf,
        previous: Option<(PathBuf, ApiWorkspace)>,
    },
    Save {
        generation: u64,
        project: PathBuf,
        workspace: ApiWorkspace,
        baseline: Option<ProfileBaseline>,
    },
    StartMock {
        generation: u64,
        epoch: u64,
        definition: MockDefinition,
    },
}
enum Event {
    Switched(u64, Result<ApiWorkspace, String>, Option<FailedSave>, bool),
    Saved(u64, Option<FailedSave>),
    MockStarted(u64, u64, Result<MockServer, String>),
}
struct RunningRequest {
    generation: u64,
    environment: Option<String>,
    collection: Option<String>,
    id: String,
    name: String,
    method: String,
    job: RequestJob,
}

fn save_project(
    storage: &ApiStorage,
    current: &mut Option<(PathBuf, ProfileBaseline)>,
    project: PathBuf,
    workspace: ApiWorkspace,
    expected: Option<ProfileBaseline>,
) -> Option<FailedSave> {
    let baseline = expected.or_else(|| {
        current
            .as_ref()
            .filter(|(owner, _)| owner == &project)
            .map(|(_, baseline)| baseline.clone())
    });
    let result: Result<(), String> = (|| {
        let baseline = baseline
            .as_ref()
            .ok_or("API profile has no loaded version; save refused")?;
        let fingerprint = snapshot_fingerprint(&workspace)?;
        if fingerprint == baseline.fingerprint {
            return Ok(());
        }
        let version = storage.save_versioned(&project, &workspace, &baseline.version)?;
        if let Some((owner, current)) = current.as_mut()
            && owner == &project
            && current.version == baseline.version
        {
            *current = ProfileBaseline {
                version,
                fingerprint,
            };
        }
        Ok(())
    })();
    result.err().map(|error| FailedSave {
        project,
        workspace,
        error,
        baseline,
    })
}

/// In-memory UI-facing model; filesystem, HTTP, JavaScript and listeners run on workers.
pub struct ApiService {
    pub workspace: ApiWorkspace,
    pub project: Option<PathBuf>,
    upload_root: Option<PathBuf>,
    pub error: Option<String>,
    pub loading: bool,
    pub saving: bool,
    pub failed_saves: Vec<FailedSave>,
    pub responses: BTreeMap<String, ExecutionResult>,
    pub response_revision: u64,
    pub history: Vec<HistoryEntry>,
    pub runner: Option<CollectionRun>,
    run_queue: VecDeque<RunStep>,
    run_context: crate::scripts::ScriptContext,
    run_items: Vec<String>,
    run_scripts: bool,
    run_environment: Option<String>,
    pub running_request: Option<String>,
    pub mock_url: Option<String>,
    pub mock_starting: bool,
    generation: u64,
    tx: Option<mpsc::SyncSender<Work>>,
    rx: mpsc::Receiver<Event>,
    io_join: Option<thread::JoinHandle<()>>,
    pending_io: usize,
    pending_saves: usize,
    writable: bool,
    shutdown: bool,
    client: ApiClient,
    running: Option<RunningRequest>,
    retired_requests: Vec<RequestJob>,
    mock: Option<MockServer>,
    mock_epoch: u64,
    retired_mocks: Vec<MockServer>,
}
impl ApiService {
    pub fn new(profile_root: PathBuf) -> Self {
        let (tx, work) = mpsc::sync_channel::<Work>(16);
        let (events, rx) = mpsc::channel();
        let storage = ApiStorage::new(profile_root);
        let io_join = thread::Builder::new()
            .name("codisseu-api-storage".into())
            .spawn(move || {
                let mut baseline = None;
                while let Ok(task) = work.recv() {
                    let event = match task {
                        Work::Switch {
                            generation,
                            project,
                            previous,
                        } => {
                            let had_previous = previous.is_some();
                            let error = previous.and_then(|(project, workspace)| {
                                save_project(&storage, &mut baseline, project, workspace, None)
                            });
                            let loaded = storage.load_versioned(&project).and_then(
                                |(workspace, version)| {
                                    let fingerprint = snapshot_fingerprint(&workspace)?;
                                    baseline = Some((
                                        project,
                                        ProfileBaseline {
                                            version,
                                            fingerprint,
                                        },
                                    ));
                                    Ok(workspace)
                                },
                            );
                            if loaded.is_err() {
                                baseline = None;
                            }
                            Event::Switched(generation, loaded, error, had_previous)
                        }
                        Work::Save {
                            generation,
                            project,
                            workspace,
                            baseline: expected,
                        } => Event::Saved(
                            generation,
                            save_project(&storage, &mut baseline, project, workspace, expected),
                        ),
                        Work::StartMock {
                            generation,
                            epoch,
                            definition,
                        } => Event::MockStarted(generation, epoch, MockServer::start(definition)),
                    };
                    if events.send(event).is_err() {
                        break;
                    }
                }
            })
            .ok();
        let error = if io_join.is_none() {
            Some("API storage worker could not start".into())
        } else {
            None
        };
        Self {
            workspace: ApiWorkspace::default(),
            project: None,
            upload_root: None,
            error,
            loading: false,
            saving: false,
            failed_saves: vec![],
            responses: BTreeMap::new(),
            response_revision: 0,
            history: vec![],
            runner: None,
            run_queue: VecDeque::new(),
            run_context: crate::scripts::ScriptContext::default(),
            run_items: vec![],
            run_scripts: false,
            run_environment: None,
            running_request: None,
            mock_url: None,
            mock_starting: false,
            generation: 0,
            tx: Some(tx),
            rx,
            io_join,
            pending_io: 0,
            pending_saves: 0,
            writable: false,
            shutdown: false,
            client: ApiClient::default(),
            running: None,
            retired_requests: vec![],
            mock: None,
            mock_epoch: 0,
            retired_mocks: vec![],
        }
    }
    fn enqueue(&mut self, work: Work) -> Result<(), String> {
        self.tx
            .as_ref()
            .ok_or("API service is shutting down")?
            .try_send(work)
            .map_err(|_| "API storage queue is busy; try again")?;
        self.pending_io += 1;
        Ok(())
    }
    pub fn set_project(&mut self, project: PathBuf) -> Result<(), String> {
        let upload_root = Some(project.clone());
        self.set_project_with_root(project, upload_root)
    }
    /// Storage identity and the relative upload base are distinct for multi-root
    /// workspace files. The caller supplies an available declared folder.
    pub fn set_project_with_root(
        &mut self,
        project: PathBuf,
        upload_root: Option<PathBuf>,
    ) -> Result<(), String> {
        if self.project.as_ref() == Some(&project) {
            self.upload_root = upload_root;
            return Ok(());
        }
        if self.writable && self.pending_saves + self.failed_saves.len() >= 8 {
            return Err(
                "API recovery queue is full; retry failed saves before switching projects".into(),
            );
        }
        let generation = self.generation.wrapping_add(1);
        let previous = if self.writable {
            self.project
                .clone()
                .map(|p| (p, persistent_snapshot(&self.workspace)))
        } else {
            None
        };
        let had_previous = previous.is_some();
        // One queue entry owns both old save and new load: refusal changes no state.
        self.enqueue(Work::Switch {
            generation,
            project: project.clone(),
            previous,
        })?;
        if had_previous {
            self.pending_saves += 1;
            self.saving = true;
        }
        self.cancel();
        self.stop_mock();
        self.client.clear_cookies();
        self.generation = generation;
        self.project = Some(project);
        self.upload_root = upload_root;
        self.workspace = ApiWorkspace::default();
        self.responses.clear();
        self.response_revision = self.response_revision.wrapping_add(1);
        self.history.clear();
        self.runner = None;
        self.writable = false;
        self.loading = true;
        self.error = None;
        Ok(())
    }
    pub fn clear_project(&mut self) -> Result<(), String> {
        if self.writable {
            self.save()?;
        }
        self.cancel();
        self.stop_mock();
        self.generation = self.generation.wrapping_add(1);
        self.project = None;
        self.upload_root = None;
        self.workspace = ApiWorkspace::default();
        self.responses.clear();
        self.response_revision = self.response_revision.wrapping_add(1);
        self.history.clear();
        self.runner = None;
        self.writable = false;
        self.loading = false;
        self.client.clear_cookies();
        Ok(())
    }
    pub fn can_save(&self) -> bool {
        self.writable && !self.loading && !self.shutdown
    }
    pub fn save(&mut self) -> Result<(), String> {
        if self.project.is_none() {
            return Ok(());
        }
        if self.shutdown {
            return Err("API service is shutting down".into());
        }
        if !self.writable || self.loading {
            return Err("API workspace has not loaded successfully; save is disabled to preserve the profile".into());
        }
        if self.pending_saves + self.failed_saves.len() >= 8 {
            return Err("API save/recovery queue is full; retry failed saves".into());
        }
        let project = self
            .project
            .clone()
            .ok_or("Open a project to save collections")?;
        self.enqueue(Work::Save {
            generation: self.generation,
            project,
            workspace: persistent_snapshot(&self.workspace),
            baseline: None,
        })?;
        self.pending_saves += 1;
        self.saving = true;
        Ok(())
    }
    fn retain_failed_save(&mut self, failed: FailedSave) {
        self.failed_saves
            .retain(|copy| copy.project != failed.project);
        self.failed_saves.push(failed);
    }
    pub fn retry_failed_saves(&mut self) -> Result<(), String> {
        let failures = std::mem::take(&mut self.failed_saves);
        let mut pending = failures.into_iter();
        while let Some(failed) = pending.next() {
            if let Err(error) = self.enqueue(Work::Save {
                generation: self.generation,
                project: failed.project.clone(),
                workspace: failed.workspace.clone(),
                baseline: failed.baseline.clone(),
            }) {
                self.failed_saves.push(failed);
                self.failed_saves.extend(pending);
                return Err(error);
            }
            self.pending_saves += 1;
            self.saving = true;
        }
        self.error = None;
        Ok(())
    }
    pub fn new_collection(&mut self, name: impl Into<String>) -> String {
        self.workspace.new_collection(name)
    }
    pub fn new_request(&mut self, collection_id: &str, name: impl Into<String>) -> Option<String> {
        self.workspace.new_request(collection_id, name)
    }
    pub fn new_environment(&mut self, name: impl Into<String>) -> String {
        self.workspace.new_environment(name)
    }
    pub fn send(&mut self, id: &str, scripts_enabled: bool) -> Result<(), String> {
        if self.runner.as_ref().is_some_and(|r| !r.finished) {
            return Err("A collection run is already active".into());
        }
        self.send_internal(id, scripts_enabled)
    }
    fn send_internal(&mut self, id: &str, scripts_enabled: bool) -> Result<(), String> {
        if self.shutdown || self.loading || !self.writable {
            return Err("Open a project and wait for the API workspace to load".into());
        }
        if self.running.is_some() || self.retired_requests.len() >= 2 {
            return Err("A request is already running or cancellation is still pending".into());
        }
        let (mut request, _) = self
            .workspace
            .request_context(id)
            .ok_or("Request no longer exists")?;
        request.upload_root = self.upload_root.clone();
        let name = request.name.clone();
        let method = request.method.clone();
        let mut environment = Variables::new();
        if let Some(env) = self.workspace.active_environment() {
            merge_variables(&mut environment, &env.variables);
        }
        let mut collection_variables = Variables::new();
        if let Some(collection) = self.workspace.collection_for_request(id) {
            merge_variables(&mut collection_variables, &collection.variables);
        }
        let mut context = if self.runner.as_ref().is_some_and(|r| !r.finished) {
            self.run_context.clone()
        } else {
            crate::scripts::ScriptContext {
                iteration_count: 1,
                ..Default::default()
            }
        };
        merge_variables(&mut context.globals, &self.workspace.globals);
        context.location = self.workspace.request_path(id);
        let job = self.client.start_for_workspace(
            request,
            environment,
            collection_variables,
            self.workspace.active_environment().is_some(),
            scripts_enabled,
            context,
        )?;
        let collection = self
            .workspace
            .collection_for_request(id)
            .map(|c| c.id.clone());
        self.running = Some(RunningRequest {
            generation: self.generation,
            environment: self.workspace.active_environment.clone(),
            collection,
            id: id.into(),
            name,
            method,
            job,
        });
        self.running_request = Some(id.into());
        self.error = None;
        Ok(())
    }
    pub fn run_collection(&mut self, id: &str, scripts_enabled: bool) -> Result<(), String> {
        self.run_with_options(id, scripts_enabled, RunOptions::default())
    }
    pub fn run_with_options(
        &mut self,
        id: &str,
        scripts_enabled: bool,
        options: RunOptions,
    ) -> Result<(), String> {
        if self.running.is_some() || self.runner.as_ref().is_some_and(|r| !r.finished) {
            return Err("A request or collection is already running".into());
        }
        if options.iterations == 0 || options.iterations > 256 || options.data.len() > 256 {
            return Err("Runner supports 1–256 iterations".into());
        }
        let (items, name) = if let Some(c) = self.workspace.collections.iter().find(|c| c.id == id)
        {
            (&c.items, c.name.clone())
        } else if let Some(CollectionItem::Folder { items, name, .. }) = self.workspace.item(id) {
            (items, name.clone())
        } else {
            return Err("Collection or folder no longer exists".into());
        };
        fn flatten(items: &[CollectionItem], ids: &mut VecDeque<String>) -> Result<(), String> {
            for item in items {
                match item {
                    CollectionItem::Request(r) => {
                        if ids.len() >= 256 {
                            return Err("Collection runner is limited to 256 requests".into());
                        }
                        ids.push_back(r.id.clone());
                    }
                    CollectionItem::Folder { items, .. } => flatten(items, ids)?,
                }
            }
            Ok(())
        }
        let mut queue = VecDeque::new();
        flatten(items, &mut queue)?;
        if queue.is_empty() {
            return Err("Collection has no requests".into());
        }
        let count = if options.data.is_empty() {
            options.iterations
        } else {
            options.data.len() as u32
        };
        if queue.len() * count as usize > 256 {
            return Err("Runner is limited to 256 total requests".into());
        }
        self.run_items = queue.iter().cloned().collect();
        let steps = (0..count)
            .flat_map(|iteration| {
                queue.iter().map(move |id| RunStep {
                    id: id.clone(),
                    iteration,
                    data: Variables::new(),
                })
            })
            .map(|mut step| {
                step.data = options
                    .data
                    .get(step.iteration as usize)
                    .cloned()
                    .unwrap_or_default();
                step
            })
            .collect::<VecDeque<_>>();
        self.run_context = crate::scripts::ScriptContext {
            iteration_count: count,
            ..Default::default()
        };
        self.runner = Some(CollectionRun {
            collection_id: id.into(),
            name,
            total: steps.len(),
            completed: 0,
            tests_passed: 0,
            tests_failed: 0,
            finished: false,
            cancelled: false,
            error: None,
            results: vec![],
        });
        self.run_queue = steps;
        self.run_scripts = scripts_enabled;
        self.run_environment = self.workspace.active_environment.clone();
        self.advance_runner();
        Ok(())
    }
    fn route_runner(&mut self, script: &ScriptResult) {
        if self.runner.as_ref().is_none_or(|r| r.finished) || !script.next_request_set {
            return;
        }
        let current = self.run_context.iteration;
        let future = self
            .run_queue
            .iter()
            .filter(|s| s.iteration != current)
            .cloned()
            .collect::<Vec<_>>();
        let mut next = VecDeque::new();
        if let Some(target) = &script.next_request {
            let index = self
                .run_items
                .iter()
                .position(|id| id == target)
                .or_else(|| {
                    self.run_items.iter().position(|id| {
                        self.workspace
                            .find_request(id)
                            .is_some_and(|r| &r.name == target)
                    })
                });
            let Some(index) = index else {
                if let Some(run) = &mut self.runner {
                    run.error = Some(
                        "Next request is outside the selected collection/folder or does not exist"
                            .into(),
                    );
                    run.finished = true;
                }
                self.run_queue.clear();
                return;
            };
            for id in &self.run_items[index..] {
                next.push_back(RunStep {
                    id: id.clone(),
                    iteration: current,
                    data: self.run_context.iteration_data.clone(),
                });
            }
        }
        next.extend(future);
        if let Some(run) = &mut self.runner {
            if run.completed + next.len() > 256 {
                run.error = Some("Workflow exceeded the limit of 256 total requests".into());
                run.finished = true;
                self.run_queue.clear();
                return;
            }
            run.total = run.completed + next.len();
        }
        self.run_queue = next;
    }
    fn advance_runner(&mut self) {
        if self.running.is_some() || self.runner.as_ref().is_none_or(|r| r.finished) {
            return;
        }
        if self.run_environment != self.workspace.active_environment {
            if let Some(run) = &mut self.runner {
                run.error = Some("Environment changed during collection run".into());
                run.finished = true;
            }
            self.run_queue.clear();
            return;
        }
        if let Some(step) = self.run_queue.pop_front() {
            self.run_context.iteration = step.iteration;
            self.run_context.iteration_data = step.data;
            if let Err(error) = self.send_internal(&step.id, self.run_scripts) {
                if let Some(run) = &mut self.runner {
                    run.error = Some(error);
                    run.finished = true;
                }
                self.run_queue.clear();
            }
        } else if let Some(run) = &mut self.runner {
            run.finished = true;
            self.run_context = crate::scripts::ScriptContext::default();
        }
    }
    pub fn cancel(&mut self) {
        self.run_context = crate::scripts::ScriptContext::default();
        self.run_queue.clear();
        if let Some(run) = &mut self.runner
            && !run.finished
        {
            run.cancelled = true;
            run.finished = true;
        }
        if let Some(running) = self.running.take() {
            running.job.cancel();
            self.retired_requests.push(running.job);
        }
        self.running_request = None;
    }
    pub fn start_mock(&mut self, id: &str) -> Result<(), String> {
        if self.mock_starting || self.shutdown {
            return Err("Mock start is already pending or service is shutting down".into());
        }
        let definition = self
            .workspace
            .mocks
            .iter()
            .find(|m| m.id == id)
            .ok_or("Mock no longer exists")?
            .clone();
        self.stop_mock();
        self.enqueue(Work::StartMock {
            generation: self.generation,
            epoch: self.mock_epoch,
            definition,
        })?;
        self.mock_starting = true;
        Ok(())
    }
    pub fn stop_mock(&mut self) {
        self.mock_epoch = self.mock_epoch.wrapping_add(1);
        if let Some(mock) = self.mock.take() {
            mock.stop();
            self.retired_mocks.push(mock);
        }
        self.mock_url = None;
    }
    pub fn is_busy(&self) -> bool {
        self.pending_io > 0
            || self.running.is_some()
            || !self.retired_requests.is_empty()
            || !self.retired_mocks.is_empty()
            || (self.shutdown
                && (self.mock.is_some() || self.io_join.as_ref().is_some_and(|j| !j.is_finished())))
    }
    pub fn shutdown(&mut self) {
        if self.shutdown {
            return;
        }
        self.shutdown = true;
        self.cancel();
        self.stop_mock();
        self.tx.take();
    }
    /// Returns true when observable model or progress changed. Drops obsolete project results.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(event) = self.rx.try_recv() {
            changed = true;
            self.pending_io = self.pending_io.saturating_sub(1);
            match event {
                Event::Switched(generation, result, save_error, had_previous) => {
                    if had_previous {
                        self.pending_saves = self.pending_saves.saturating_sub(1);
                        self.saving = self.pending_saves > 0;
                    }
                    if let Some(failed) = save_error {
                        self.error = Some(format!(
                            "A previous project API profile could not be saved (recovery copy retained): {}",
                            failed.error
                        ));
                        self.retain_failed_save(failed);
                    }
                    if generation == self.generation && !self.shutdown {
                        self.loading = false;
                        match result {
                            Ok(workspace) => {
                                self.workspace = workspace;
                                self.writable = true;
                            }
                            Err(error) => {
                                self.error = Some(error);
                                self.writable = false;
                            }
                        }
                    }
                }
                Event::Saved(generation, result) => {
                    self.pending_saves = self.pending_saves.saturating_sub(1);
                    self.saving = self.pending_saves > 0;
                    if let Some(failed) = result {
                        self.error = Some(
                            if generation == self.generation
                                && self.project.as_ref() == Some(&failed.project)
                            {
                                format!(
                                    "API profile could not be saved (recovery copy retained): {}",
                                    failed.error
                                )
                            } else {
                                format!(
                                    "A previous project API profile could not be saved (recovery copy retained): {}",
                                    failed.error
                                )
                            },
                        );
                        self.retain_failed_save(failed);
                    }
                }
                Event::MockStarted(generation, epoch, result) => {
                    self.mock_starting = false;
                    match result {
                        Ok(mock)
                            if generation == self.generation
                                && epoch == self.mock_epoch
                                && !self.shutdown =>
                        {
                            self.mock_url = Some(mock.url.clone());
                            self.mock = Some(mock);
                        }
                        Ok(mock) => {
                            mock.stop();
                            self.retired_mocks.push(mock);
                        }
                        Err(error) if generation == self.generation => self.error = Some(error),
                        _ => {}
                    }
                }
            }
        }
        if let Some(running) = self.running.as_mut()
            && let Some(result) = running.job.try_result()
        {
            let running = self.running.take().expect("running request");
            self.running_request = None;
            changed = true;
            match result {
                Ok(result) if running.generation == self.generation && !self.shutdown => {
                    if let Some(environment) = running.environment {
                        self.workspace.apply_environment_updates(
                            &environment,
                            &result.script_result.environment_updates,
                        );
                        if let Some(env) = self
                            .workspace
                            .environments
                            .iter_mut()
                            .find(|e| e.id == environment)
                        {
                            env.variables.retain(|r| {
                                !result.script_result.environment_unsets.contains(&r.key)
                            });
                        }
                    }
                    self.workspace
                        .globals
                        .retain(|r| !result.script_result.global_unsets.contains(&r.key));
                    for (key, value) in &result.script_result.global_updates {
                        if let Some(row) = self.workspace.globals.iter_mut().find(|r| r.key == *key)
                        {
                            row.value = value.clone();
                            row.secret = true;
                        } else {
                            self.workspace.globals.push(KeyValue {
                                key: key.clone(),
                                value: value.clone(),
                                secret: true,
                                ..Default::default()
                            });
                        }
                    }
                    if self.runner.as_ref().is_some_and(|r| !r.finished) {
                        self.run_context.locals = result.script_result.locals.clone();
                    }
                    if let Some(collection) = running
                        .collection
                        .and_then(|id| self.workspace.collections.iter_mut().find(|c| c.id == id))
                    {
                        collection
                            .variables
                            .retain(|r| !result.script_result.collection_unsets.contains(&r.key));
                        for (key, value) in &result.script_result.collection_updates {
                            if let Some(row) =
                                collection.variables.iter_mut().find(|r| r.key == *key)
                            {
                                row.value = value.clone();
                                row.secret = true;
                            } else {
                                collection.variables.push(KeyValue {
                                    key: key.clone(),
                                    value: value.clone(),
                                    secret: true,
                                    ..KeyValue::default()
                                });
                            }
                        }
                    }
                    if let Some(run) = &mut self.runner
                        && !run.finished
                    {
                        let passed = result
                            .script_result
                            .tests
                            .iter()
                            .filter(|t| t.passed)
                            .count();
                        let failed = result.script_result.tests.len() - passed;
                        run.completed += 1;
                        run.tests_passed += passed;
                        run.tests_failed += failed;
                        run.results.push(RunResult {
                            request_id: running.id.clone(),
                            name: running.name.clone(),
                            method: running.method.clone(),
                            status: (!result.script_result.skipped)
                                .then_some(result.response.status),
                            elapsed_ms: result.response.elapsed_ms,
                            tests_passed: passed,
                            tests_failed: failed,
                            error: result.script_error.clone(),
                        });
                        if result.script_error.is_some() {
                            run.error = result.script_error.clone();
                            run.finished = true;
                            self.run_queue.clear();
                        }
                    }
                    self.history.insert(
                        0,
                        HistoryEntry {
                            request_id: running.id.clone(),
                            name: running.name,
                            method: running.method,
                            status: result.response.status,
                            elapsed_ms: result.response.elapsed_ms,
                            timestamp_ms: std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_millis() as u64,
                        },
                    );
                    self.history.truncate(100);
                    while self.responses.len() >= 8 && !self.responses.contains_key(&running.id) {
                        if let Some(id) = self.responses.keys().next().cloned() {
                            self.responses.remove(&id);
                        }
                    }
                    self.route_runner(&result.script_result);
                    self.responses.insert(running.id.clone(), result);
                    self.response_revision = self.response_revision.wrapping_add(1);
                }
                Err(error) if running.generation == self.generation && !self.shutdown => {
                    if let Some(run) = &mut self.runner
                        && !run.finished
                    {
                        run.completed += 1;
                        run.results.push(RunResult {
                            request_id: running.id.clone(),
                            name: running.name,
                            method: running.method,
                            status: None,
                            elapsed_ms: 0,
                            tests_passed: 0,
                            tests_failed: 0,
                            error: Some(error.clone()),
                        });
                        run.error = Some(error.clone());
                        run.finished = true;
                        self.run_queue.clear();
                    }
                    self.error = Some(error);
                }
                _ => {}
            }
            if !running.job.is_finished() {
                self.retired_requests.push(running.job);
            }
        }
        self.advance_runner();
        let previous = self.retired_requests.len();
        self.retired_requests.retain(|job| !job.is_finished());
        changed |= self.retired_requests.len() != previous;
        let previous = self.retired_mocks.len();
        self.retired_mocks.retain(|mock| !mock.is_finished());
        changed |= self.retired_mocks.len() != previous;
        if self.shutdown
            && self.pending_io == 0
            && self.io_join.as_ref().is_some_and(|j| j.is_finished())
        {
            if let Some(join) = self.io_join.take() {
                let _ = join.join();
            }
            changed = true;
        }
        changed
    }
}
impl Drop for ApiService {
    fn drop(&mut self) {
        self.shutdown();
    }
}
