//! Bounded static declarations shared by every language; never runs project tools.
use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    io::Read,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, SyncSender},
    time::{Duration, Instant},
};
const MAX_BYTES: u64 = 64 * 1024;
const REFRESH: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub enum ProjectRuntime {
    Php,
    Node,
    Python,
    Rust,
}
impl ProjectRuntime {
    pub fn label(self) -> &'static str {
        match self {
            Self::Php => "PHP",
            Self::Node => "Node.js",
            Self::Python => "Python",
            Self::Rust => "Rust",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct ProjectVersion {
    pub runtime: ProjectRuntime,
    pub value: String,
    pub source: PathBuf,
    pub field: &'static str,
    pub requirement: bool,
}
impl ProjectVersion {
    pub fn tooltip(&self) -> String {
        let meaning = if self.requirement {
            "Requisito de compatibilidade"
        } else {
            "Versão ou canal declarado"
        };
        format!(
            "{meaning} {} do projeto: {}\n{} · {}\nNão comprova a versão da ferramenta em execução.",
            self.runtime.label(),
            self.value,
            self.source.display(),
            self.field
        )
    }
}
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct ProjectVersions {
    pub versions: BTreeMap<ProjectRuntime, ProjectVersion>,
    pub hints: Vec<ProjectRuntime>,
}
type Request = (u64, PathBuf);
type Response = (u64, ProjectVersions);

pub(super) struct Service {
    root: Option<PathBuf>,
    epoch: u64,
    worker: Option<(SyncSender<Request>, Receiver<Response>)>,
    pending: bool,
    next: Instant,
    version: ProjectVersions,
}
impl Default for Service {
    fn default() -> Self {
        Self {
            root: None,
            epoch: 0,
            worker: None,
            pending: false,
            next: Instant::now(),
            version: ProjectVersions::default(),
        }
    }
}
impl Service {
    fn tick(&mut self, root: Option<PathBuf>) {
        if root != self.root {
            self.root = root;
            self.epoch = self.epoch.wrapping_add(1);
            self.version = ProjectVersions::default();
            self.next = Instant::now();
        }
        let mut disconnected = false;
        if let Some((_, receiver)) = &self.worker {
            loop {
                match receiver.try_recv() {
                    Ok((epoch, version)) => {
                        self.pending = false;
                        if epoch == self.epoch {
                            self.version = version;
                        }
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
        }
        if disconnected {
            self.worker = None;
            self.pending = false;
            self.version = ProjectVersions::default();
            self.next = Instant::now() + REFRESH;
        }
        let Some(root) = &self.root else { return };
        if self.pending || Instant::now() < self.next {
            return;
        }
        if self.worker.is_none() {
            let (sender, jobs) = mpsc::sync_channel::<Request>(1);
            let (replies, receiver) = mpsc::sync_channel(1);
            if std::thread::Builder::new()
                .name("codisseu-project-versions".into())
                .spawn(move || {
                    while let Ok((epoch, root)) = jobs.recv() {
                        if replies.send((epoch, detect(&root))).is_err() {
                            break;
                        }
                    }
                })
                .is_err()
            {
                self.next = Instant::now() + REFRESH;
                return;
            }
            self.worker = Some((sender, receiver));
        }
        if self
            .worker
            .as_ref()
            .unwrap()
            .0
            .try_send((self.epoch, root.clone()))
            .is_ok()
        {
            self.pending = true;
            self.next = Instant::now() + REFRESH;
        }
    }
}

impl super::AppService {
    pub fn tick_project_versions(&mut self) {
        let root = self
            .active_document()
            .filter(|d| !d.is_untitled() && !d.is_read_only())
            .map(|d| d.project_root().to_path_buf())
            .or_else(|| self.workspace_default_root().map(PathBuf::from));
        self.project_versions.tick(root);
    }
    pub fn project_version(&self, runtime: ProjectRuntime) -> Option<&ProjectVersion> {
        self.project_versions.version.versions.get(&runtime)
    }
    pub fn project_versions(&self) -> &ProjectVersions {
        &self.project_versions.version
    }
    pub fn project_versions_pending(&self) -> bool {
        self.project_versions.pending
    }
}
fn read(path: &Path) -> Option<String> {
    // NONBLOCK avoids hanging on a FIFO substituted between stat and open.
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_BYTES {
        return None;
    }
    let file = options.open(path).ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > MAX_BYTES {
        return None;
    }
    let mut text = String::new();
    file.take(MAX_BYTES + 1).read_to_string(&mut text).ok()?;
    (text.len() as u64 <= MAX_BYTES).then_some(text)
}
fn exact(value: &str) -> bool {
    let parts: Vec<_> = value.split('.').collect();
    (2..=3).contains(&parts.len())
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.len() <= 3 && p.bytes().all(|c| c.is_ascii_digit()))
}
fn php(root: &Path) -> Option<ProjectVersion> {
    let source = root.join("composer.json");
    let composer: Option<serde_json::Value> =
        read(&source).and_then(|text| serde_json::from_str(&text).ok());
    if let Some(value) = composer
        .as_ref()
        .and_then(|v| v.pointer("/config/platform/php"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| exact(v))
    {
        return Some(ProjectVersion {
            value: value.into(),
            source,
            field: "config.platform.php",
            runtime: ProjectRuntime::Php,
            requirement: false,
        });
    }
    let pin = root.join(".php-version");
    if let Some(value) = read(&pin).as_deref().map(str::trim).filter(|v| exact(v)) {
        return Some(ProjectVersion {
            value: value.into(),
            source: pin,
            field: ".php-version",
            runtime: ProjectRuntime::Php,
            requirement: false,
        });
    }
    let value = composer.as_ref()?.pointer("/require/php")?.as_str()?.trim();
    if value.is_empty()
        || !value.bytes().any(|c| c.is_ascii_digit() || c == b'*')
        || value.len() > 96
        || !value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b" .^~<>=!|*+-@_,".contains(&c))
    {
        return None;
    }
    Some(ProjectVersion {
        value: value.into(),
        source,
        field: "require.php",
        runtime: ProjectRuntime::Php,
        requirement: true,
    })
}

fn declaration(
    root: &Path,
    runtime: ProjectRuntime,
    file: &str,
    field: &'static str,
    value: &str,
    requirement: bool,
) -> ProjectVersion {
    ProjectVersion {
        runtime,
        value: value.trim().into(),
        source: root.join(file),
        field,
        requirement,
    }
}
fn numeric(value: &str) -> bool {
    let value = value.strip_prefix('v').unwrap_or(value);
    let parts: Vec<_> = value.split('.').collect();
    (1..=3).contains(&parts.len())
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.len() <= 3 && p.bytes().all(|c| c.is_ascii_digit()))
}
fn constraint(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && value.bytes().any(|c| c.is_ascii_digit() || c == b'*')
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b" .^~<>=!|*+-@_,".contains(&c))
}
fn node_pin(value: &str) -> bool {
    numeric(value)
        || matches!(value, "node" | "stable" | "lts/*")
        || value.strip_prefix("lts/").is_some_and(|s| {
            !s.is_empty()
                && s.len() <= 32
                && s.bytes().all(|c| c.is_ascii_alphabetic() || c == b'-')
        })
}
fn rust_channel(value: &str) -> bool {
    value.len() <= 96
        && (numeric(value)
            || matches!(value, "stable" | "beta" | "nightly")
            || ["nightly-", "beta-", "stable-"].iter().any(|prefix| {
                value.strip_prefix(prefix).is_some_and(|v| {
                    v.len() == 10 && v.bytes().all(|c| c.is_ascii_digit() || c == b'-')
                })
            }))
}
fn node(root: &Path) -> Option<ProjectVersion> {
    for file in [".nvmrc", ".node-version"] {
        if let Some(value) = read(&root.join(file))
            .as_deref()
            .map(str::trim)
            .filter(|s| node_pin(s))
        {
            return Some(declaration(
                root,
                ProjectRuntime::Node,
                file,
                if file == ".nvmrc" {
                    ".nvmrc"
                } else {
                    ".node-version"
                },
                value,
                false,
            ));
        }
    }
    let package: serde_json::Value =
        serde_json::from_str(&read(&root.join("package.json"))?).ok()?;
    for (pointer, field, requirement) in [
        ("/volta/node", "volta.node", false),
        ("/engines/node", "engines.node", true),
    ] {
        if let Some(value) = package
            .pointer(pointer)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| {
                if requirement {
                    constraint(v)
                } else {
                    node_pin(v)
                }
            })
        {
            return Some(declaration(
                root,
                ProjectRuntime::Node,
                "package.json",
                field,
                value,
                requirement,
            ));
        }
    }
    None
}
fn python(root: &Path) -> Option<ProjectVersion> {
    if let Some(value) = read(&root.join(".python-version"))
        .as_deref()
        .map(str::trim)
        .filter(|v| exact(v))
    {
        return Some(declaration(
            root,
            ProjectRuntime::Python,
            ".python-version",
            ".python-version",
            value,
            false,
        ));
    }
    let project: toml::Value = toml::from_str(&read(&root.join("pyproject.toml"))?).ok()?;
    let value = project
        .get("project")?
        .get("requires-python")?
        .as_str()?
        .trim();
    constraint(value).then(|| {
        declaration(
            root,
            ProjectRuntime::Python,
            "pyproject.toml",
            "project.requires-python",
            value,
            true,
        )
    })
}
fn rust(root: &Path) -> Option<ProjectVersion> {
    for file in ["rust-toolchain.toml", "rust-toolchain"] {
        let Some(text) = read(&root.join(file)) else {
            continue;
        };
        let parsed: Option<toml::Value> = toml::from_str(&text).ok();
        let value = parsed
            .as_ref()
            .and_then(|t| t.get("toolchain"))
            .and_then(|t| t.get("channel"))
            .and_then(|c| c.as_str())
            .or_else(|| (file == "rust-toolchain").then_some(text.trim()));
        if let Some(value) = value.map(str::trim).filter(|v| rust_channel(v)) {
            return Some(declaration(
                root,
                ProjectRuntime::Rust,
                file,
                "toolchain.channel",
                value,
                false,
            ));
        }
    }
    let cargo: toml::Value = toml::from_str(&read(&root.join("Cargo.toml"))?).ok()?;
    for (package, field) in [
        (cargo.get("package"), "package.rust-version"),
        (
            cargo.get("workspace").and_then(|w| w.get("package")),
            "workspace.package.rust-version",
        ),
    ] {
        if let Some(value) = package
            .and_then(|p| p.get("rust-version"))
            .and_then(|v| v.as_str())
            .filter(|v| exact(v))
        {
            return Some(declaration(
                root,
                ProjectRuntime::Rust,
                "Cargo.toml",
                field,
                &format!(">={value}"),
                true,
            ));
        }
    }
    None
}
fn detect(root: &Path) -> ProjectVersions {
    let mut result = ProjectVersions::default();
    for (runtime, version, markers) in [
        (
            ProjectRuntime::Php,
            php(root),
            &["composer.json", ".php-version"][..],
        ),
        (
            ProjectRuntime::Node,
            node(root),
            &["package.json", ".nvmrc", ".node-version"][..],
        ),
        (
            ProjectRuntime::Python,
            python(root),
            &[
                "pyproject.toml",
                ".python-version",
                "requirements.txt",
                "setup.py",
            ][..],
        ),
        (
            ProjectRuntime::Rust,
            rust(root),
            &["Cargo.toml", "rust-toolchain.toml", "rust-toolchain"][..],
        ),
    ] {
        if let Some(version) = version {
            result.versions.insert(runtime, version);
        }
        if markers
            .iter()
            .any(|file| std::fs::symlink_metadata(root.join(file)).is_ok_and(|m| m.is_file()))
        {
            result.hints.push(runtime);
        }
    }
    result
}

#[cfg(test)]
#[path = "project_versions_tests.rs"]
mod tests;
