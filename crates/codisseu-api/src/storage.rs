use crate::*;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[derive(Clone)]
pub struct ApiStorage {
    root: PathBuf,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProfileVersion(Option<[u8; 32]>);

pub(crate) fn snapshot_fingerprint(workspace: &ApiWorkspace) -> Result<[u8; 32], String> {
    serde_json::to_vec(&persistent_snapshot(workspace))
        .map(|bytes| Sha256::digest(bytes).into())
        .map_err(|_| "Could not serialize API workspace".into())
}
#[derive(serde::Serialize, serde::Deserialize)]
struct SavedWorkspace {
    schema_version: u32,
    project: PathBuf,
    workspace: ApiWorkspace,
}
impl ApiStorage {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
    pub fn path_for(&self, project: &Path) -> Result<PathBuf, String> {
        let canonical =
            fs::canonicalize(project).map_err(|_| "Project directory is unavailable")?;
        let key = format!(
            "{:x}",
            Sha256::digest(canonical.as_os_str().as_encoded_bytes())
        );
        Ok(self.root.join(format!("{key}.json")))
    }
    pub fn load(&self, project: &Path) -> Result<ApiWorkspace, String> {
        self.load_versioned(project).map(|(workspace, _)| workspace)
    }
    pub(crate) fn load_versioned(
        &self,
        project: &Path,
    ) -> Result<(ApiWorkspace, ProfileVersion), String> {
        let path = self.path_for(project)?;
        let Some(bytes) = read_profile(&path)? else {
            return Ok((ApiWorkspace::default(), ProfileVersion(None)));
        };
        let version = ProfileVersion(Some(Sha256::digest(&bytes).into()));
        let saved: SavedWorkspace = serde_json::from_slice(&bytes)
            .map_err(|_| "Invalid API profile (original preserved)")?;
        if saved.schema_version != 1 || saved.workspace.schema_version != 1 {
            return Err("Unsupported API profile version".into());
        }
        if fs::canonicalize(project).ok().as_ref() != Some(&saved.project) {
            return Err("API profile belongs to another project".into());
        }
        Ok((saved.workspace, version))
    }
    pub fn save(&self, project: &Path, workspace: &ApiWorkspace) -> Result<(), String> {
        let path = self.path_for(project)?;
        let expected = profile_version(&path)?;
        self.save_versioned(project, workspace, &expected)
            .map(|_| ())
    }
    pub(crate) fn save_versioned(
        &self,
        project: &Path,
        workspace: &ApiWorkspace,
        expected: &ProfileVersion,
    ) -> Result<ProfileVersion, String> {
        let path = self.path_for(project)?;
        fs::create_dir_all(&self.root).map_err(|_| "Could not create API profile directory")?;
        if fs::symlink_metadata(&self.root)
            .map_err(|_| "Could not inspect API profile directory")?
            .file_type()
            .is_symlink()
        {
            return Err("API profile directory cannot be a symlink".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&self.root, fs::Permissions::from_mode(0o700))
                .map_err(|_| "Could not restrict API directory permissions")?;
        }
        // A permanent sibling lock serializes Codisseu processes through the
        // compare and atomic replace. Never unlink it: a second lock inode
        // would let another window bypass an existing holder.
        let lock_path = path.with_extension("lock");
        let mut options = fs::OpenOptions::new();
        options.create(true).read(true).write(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(nix::libc::O_NOFOLLOW);
        }
        if fs::symlink_metadata(&lock_path).is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            return Err("API profile lock cannot be a symlink".into());
        }
        let lock = options
            .open(lock_path)
            .map_err(|_| "Could not open API profile lock")?;
        if !lock
            .metadata()
            .map_err(|_| "Could not inspect API profile lock")?
            .is_file()
        {
            return Err("API profile lock must be a regular file".into());
        }
        lock.try_lock().map_err(|error| match error {
            fs::TryLockError::WouldBlock => "API profile is being saved in another window; save refused and local recovery retained",
            fs::TryLockError::Error(_) => "Could not lock API profile; local recovery retained",
        })?;
        if &profile_version(&path)? != expected {
            return Err(
                "API profile changed in another window; save refused and local recovery retained"
                    .into(),
            );
        }
        if path.exists()
            && fs::symlink_metadata(&path)
                .map_err(|_| "Could not inspect API profile")?
                .file_type()
                .is_symlink()
        {
            return Err("API profile cannot be a symlink".into());
        }
        let saved = SavedWorkspace {
            schema_version: 1,
            project: fs::canonicalize(project).map_err(|_| "Project directory is unavailable")?,
            workspace: persistent_snapshot(workspace),
        };
        let data =
            serde_json::to_vec_pretty(&saved).map_err(|_| "Could not serialize API profile")?;
        if data.len() > 16 * 1024 * 1024 {
            return Err("API workspace exceeds 16 MiB".into());
        }
        let mut temporary = tempfile::NamedTempFile::new_in(&self.root)
            .map_err(|_| "Could not create atomic API profile")?;
        temporary
            .write_all(&data)
            .map_err(|_| "Could not write API profile (original preserved)")?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|_| "Could not sync API profile (original preserved)")?;
        if &profile_version(&path)? != expected {
            return Err(
                "API profile changed during save; original and local recovery retained".into(),
            );
        }
        temporary
            .persist(&path)
            .map_err(|_| "Could not replace API profile (original preserved)")?;
        #[cfg(unix)]
        {
            fs::File::open(&self.root)
                .and_then(|f| f.sync_all())
                .map_err(|_| "API profile replaced, but directory sync failed")?;
        }
        Ok(ProfileVersion(Some(Sha256::digest(&data).into())))
    }
}

fn profile_version(path: &Path) -> Result<ProfileVersion, String> {
    Ok(ProfileVersion(
        read_profile(path)?.map(|bytes| Sha256::digest(bytes).into()),
    ))
}

fn read_profile(path: &Path) -> Result<Option<Vec<u8>>, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
            ) =>
        {
            return Ok(None);
        }
        Err(_) => return Err("Could not inspect API profile".into()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("API profile must be a regular file, not a symlink".into());
    }
    if metadata.len() > 16 * 1024 * 1024 {
        return Err("API profile exceeds 16 MiB".into());
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(nix::libc::O_NOFOLLOW);
    }
    let file = options
        .open(path)
        .map_err(|_| "Could not read API profile")?;
    if !file
        .metadata()
        .map_err(|_| "Could not inspect API profile")?
        .is_file()
    {
        return Err("API profile must be a regular file".into());
    }
    let mut bytes = Vec::new();
    file.take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Could not read API profile")?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err("API profile exceeds 16 MiB".into());
    }
    Ok(Some(bytes))
}
fn keep_template(value: &mut String) {
    if !(value.starts_with("{{") && value.ends_with("}}")) {
        value.clear();
    }
}
fn auth_snapshot(auth: &mut Auth) {
    match auth {
        Auth::Bearer { token } => keep_template(token),
        Auth::Basic { password, .. } => keep_template(password),
        Auth::ApiKey { value, .. } => keep_template(value),
        _ => {}
    }
}
fn clear_rows(rows: &mut [KeyValue]) {
    for row in rows {
        if row.secret || sensitive_key(&row.key) {
            row.secret = true;
            row.value.clear();
        }
    }
}
fn request_snapshot(request: &mut ApiRequest) {
    clear_rows(&mut request.path_variables);
    auth_snapshot(&mut request.auth);
    clear_rows(&mut request.params);
    for row in &mut request.headers {
        if row.secret
            || matches!(
                row.key.to_ascii_lowercase().as_str(),
                "authorization" | "cookie" | "proxy-authorization" | "x-api-key"
            )
        {
            keep_template(&mut row.value);
            row.secret = true;
        }
    }
    match &mut request.body {
        RequestBody::UrlEncoded(rows) | RequestBody::FormData(rows) => clear_rows(rows),
        _ => {}
    }
}
fn item_snapshots(items: &mut [CollectionItem]) {
    for item in items {
        match item {
            CollectionItem::Request(request) => request_snapshot(request),
            CollectionItem::Folder { items, auth, .. } => {
                auth_snapshot(auth);
                item_snapshots(items);
            }
        }
    }
}
/// Non-secret request definitions persist; declared secret rows and credentials are session-only.
/// Bodies/scripts/spec text are user-authored source and may contain literals. Use secret variables.
pub fn persistent_snapshot(workspace: &ApiWorkspace) -> ApiWorkspace {
    let mut snapshot = workspace.clone();
    clear_rows(&mut snapshot.globals);
    for env in &mut snapshot.environments {
        clear_rows(&mut env.variables);
    }
    for c in &mut snapshot.collections {
        clear_rows(&mut c.variables);
        auth_snapshot(&mut c.auth);
        item_snapshots(&mut c.items);
    }
    snapshot
}

fn sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.contains("token")
        || key.contains("password")
        || key.contains("secret")
        || key.contains("api_key")
        || key == "apikey"
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        process::{Command, Stdio},
        time::{Duration, Instant},
    };

    #[test]
    fn cross_process_compare_and_replace_keeps_exactly_one_new_version() {
        const ROOT: &str = "CODISSEU_API_CAS_TEST_ROOT";
        const SIDE: &str = "CODISSEU_API_CAS_TEST_SIDE";
        if let (Some(root), Some(side)) = (std::env::var_os(ROOT), std::env::var_os(SIDE)) {
            let root = PathBuf::from(root);
            let side = side.to_str().unwrap();
            let project = root.join("project");
            let storage = ApiStorage::new(root.join("profile"));
            let (mut workspace, version) = storage.load_versioned(&project).unwrap();
            workspace.new_collection(side);
            fs::write(root.join(format!("{side}.ready")), "ready").unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            while !root.join("release").exists() {
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(2));
            }
            let result = storage.save_versioned(&project, &workspace, &version);
            fs::write(
                root.join(format!("{side}.result")),
                match result {
                    Ok(_) => "saved".to_owned(),
                    Err(error) => {
                        assert!(error.contains("another window"), "{error}");
                        "conflict".to_owned()
                    }
                },
            )
            .unwrap();
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let project = root.join("project");
        fs::create_dir(&project).unwrap();
        let storage = ApiStorage::new(root.join("profile"));
        storage.save(&project, &ApiWorkspace::default()).unwrap();
        let mut children = ["one", "two"].map(|side| {
            Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "storage::tests::cross_process_compare_and_replace_keeps_exactly_one_new_version", "--nocapture"])
                .env(ROOT, root).env(SIDE, side)
                .stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap()
        });
        let deadline = Instant::now() + Duration::from_secs(10);
        while !root.join("one.ready").exists() || !root.join("two.ready").exists() {
            assert!(
                Instant::now() < deadline,
                "both child processes must load the same version"
            );
            for child in &mut children {
                assert!(
                    child.try_wait().unwrap().is_none(),
                    "child exited before the shared release"
                );
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        fs::write(root.join("release"), "release").unwrap();
        for child in children {
            let output = child.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let mut results = ["one", "two"]
            .map(|side| fs::read_to_string(root.join(format!("{side}.result"))).unwrap());
        results.sort();
        assert_eq!(results, ["conflict", "saved"]);
        let workspace = storage.load(&project).unwrap();
        assert_eq!(workspace.collections.len(), 1);
        assert!(["one", "two"].contains(&workspace.collections[0].name.as_str()));
    }
}
