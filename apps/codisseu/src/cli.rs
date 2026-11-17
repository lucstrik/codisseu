//! Terminal entry point. Arguments stay typed; no shell command is assembled.
use super::Options;
use std::{
    ffi::OsString,
    fs::OpenOptions,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub(super) fn add_path(options: &mut Options, value: OsString, cwd: &Path) -> Result<(), String> {
    let path = PathBuf::from(value);
    let path = if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    };
    let metadata = path
        .metadata()
        .map_err(|error| format!("Não foi possível abrir “{}”: {error}", path.display()))?;
    let path = path.canonicalize().map_err(|error| error.to_string())?;
    if metadata.is_dir() || codisseu_core::workspace_file::is_workspace_file(&path) {
        if options.workspace.is_some() {
            return Err(
                "Informe uma pasta por janela ou use um arquivo .code-workspace para várias pastas"
                    .into(),
            );
        }
        options.workspace = Some(path);
    } else if metadata.is_file() {
        options.files.push(path);
    } else {
        return Err(format!(
            "“{}” não é uma pasta ou arquivo regular",
            path.display()
        ));
    }
    Ok(())
}

pub(super) fn gui_arguments(options: &Options) -> Vec<OsString> {
    let mut args = Vec::new();
    for (flag, path) in [
        ("--session", options.session.as_ref()),
        ("--profile", options.profile.as_ref()),
        ("--workspace", options.workspace.as_ref()),
        ("--capture", options.capture.as_ref()),
        ("--database", options.database.as_ref()),
        ("--providers", options.providers.as_ref()),
    ] {
        if let Some(path) = path {
            args.push(flag.into());
            args.push(path.as_os_str().to_owned());
        }
    }
    for path in &options.files {
        args.push("--open".into());
        args.push(path.as_os_str().to_owned());
    }
    for url in &options.browsers {
        args.push("--browser".into());
        args.push(url.into());
    }
    for (flag, enabled) in [
        ("--no-restore", options.no_restore),
        ("--close-after-capture", options.close_after_capture),
        ("--start-providers", options.start_providers),
        ("--no-auto-providers", options.no_auto_providers),
        ("--execute-sql", options.execute_sql),
        ("--wait-diagnostics", options.wait_diagnostics),
        (
            "--wait-nonempty-diagnostics",
            options.wait_nonempty_diagnostics,
        ),
        ("--problems", options.problems),
    ] {
        if enabled {
            args.push(flag.into());
        }
    }
    args
}

/// Window restoration and project data have separate identities. An explicit
/// test session remains its own profile unless a shared anchor was requested.
pub(super) fn prepare_profile(options: &mut Options, default_session: &Path) -> Result<(), String> {
    let profile = options
        .profile
        .clone()
        .or_else(|| options.session.clone())
        .unwrap_or_else(|| default_session.to_path_buf());
    if options.new_window {
        options.no_restore = true;
        options.session = Some(super::new_window::session_path(
            options.session.as_deref().unwrap_or(default_session),
        )?);
    } else if options.session.is_none() {
        options.session = Some(default_session.to_path_buf());
    }
    options.profile = Some(profile);
    Ok(())
}

pub(super) fn launch(executable: &Path, options: &Options) -> Result<(), String> {
    let session = options
        .session
        .as_ref()
        .ok_or("Perfil da janela indisponível")?;
    let directory = session
        .parent()
        .ok_or("Pasta do perfil indisponível")?
        .join("cli-logs");
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("Não foi possível criar o log de inicialização: {error}"))?;
    let name = super::new_window::session_path(session)?;
    let log_path = directory
        .join(name.file_name().unwrap())
        .with_extension("log");
    let mut open = OpenOptions::new();
    open.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        open.mode(0o600);
    }
    let log = open.open(&log_path).map_err(|error| error.to_string())?;
    let mut command = Command::new(executable);
    command
        .args(gui_arguments(options))
        .current_dir(std::env::current_dir().map_err(|error| error.to_string())?)
        .stdin(Stdio::null())
        .stdout(log.try_clone().map_err(|error| error.to_string())?)
        .stderr(log);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("Não foi possível abrir o Codisseu: {error}"))?;
    // Catch immediate native/loader errors; a successful spawn is not a claim
    // that the window has loaded its project. Readiness is observed by UI tests.
    let deadline = Instant::now() + Duration::from_millis(200);
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            return if status.success() {
                Ok(())
            } else {
                Err(format!(
                    "O Codisseu não conseguiu iniciar ({status}). Detalhes em {}",
                    log_path.display()
                ))
            };
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}
