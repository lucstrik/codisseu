use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

pub(super) fn session_path(current: &Path) -> Result<PathBuf, String> {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    // Siblings also keep windows opened from a secondary window at the same
    // level. Never reuse the parent's session or any of its sidecar files.
    Ok(current.with_file_name(format!(
        "window-{}-{timestamp}-{}.json",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )))
}

fn command(executable: &Path, session: &Path, profile: &Path) -> Command {
    let mut command = Command::new(executable);
    command
        .arg("--no-restore")
        .arg("--session")
        .arg(session)
        .arg("--profile")
        .arg(profile)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

pub(super) fn executable() -> Result<PathBuf, String> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    // A portable Linux package invokes its private dynamic loader explicitly.
    // /proc/self/exe then names the loader: relaunch through the package entry.
    if cfg!(target_os = "linux")
        && executable
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with("ld-"))
    {
        std::env::var_os("CODISSEU_RESOURCE_DIR")
            .map(PathBuf::from)
            .and_then(|root| {
                root.parent()?
                    .parent()
                    .map(|root| root.join("bin/codisseu"))
            })
            .filter(|path| path.is_file())
            .ok_or_else(|| "Launcher do pacote Codisseu indisponível".into())
    } else {
        Ok(executable)
    }
}

pub(super) fn launch(current_session: &Path, profile: &Path) -> Result<(), String> {
    let executable = executable()?;
    let session = session_path(current_session)?;
    let mut child = command(&executable, &session, profile)
        .spawn()
        .map_err(|error| error.to_string())?;
    // Reap on Unix without tying the child's lifetime to the original window.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_window_has_a_distinct_sibling_profile_and_no_inherited_startup_actions() {
        let original = Path::new("/tmp/profile with spaces/session.json");
        let first = session_path(original).unwrap();
        let second = session_path(original).unwrap();
        let nested = session_path(&first).unwrap();
        assert_ne!(first, original);
        assert_ne!(first, second);
        assert_ne!(nested, first);
        assert_eq!(nested.parent(), original.parent());
        let executable = Path::new("/tmp/Codisseu with spaces/codisseu");
        let cmd = command(executable, &first, original);
        assert_eq!(cmd.get_program(), executable);
        assert_eq!(
            cmd.get_args().collect::<Vec<_>>(),
            vec![
                "--no-restore".as_ref(),
                "--session".as_ref(),
                first.as_os_str(),
                "--profile".as_ref(),
                original.as_os_str(),
            ]
        );
        assert_ne!(
            original.with_extension("jira.json"),
            first.with_extension("jira.json")
        );
    }
}
