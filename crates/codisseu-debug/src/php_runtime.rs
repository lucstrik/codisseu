//! Bounded PHP Launch preflight. This runs fixed interpreter metadata code,
//! never the selected program, project autoload, or a generated PHP source file.
use crate::DebugConfiguration;
use std::{
    io::Read,
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const OUTPUT_LIMIT: usize = 8192;
const PROBE: &str = r#"$loaded=extension_loaded('xdebug');$modes=$loaded&&function_exists('xdebug_info')?xdebug_info('mode'):[];echo json_encode(['loaded'=>$loaded,'debug'=>is_array($modes)&&in_array('debug',$modes,true)]);"#;

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        #[cfg(unix)]
        unsafe {
            unsafe extern "C" {
                fn kill(pid: i32, signal: i32) -> i32;
            }
            // The child starts its own group. No external PHP program belongs to it.
            kill(-(self.0.id() as i32), 9);
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn reader(
    pipe: impl Read + Send + 'static,
    overflow: Arc<AtomicBool>,
) -> Result<JoinHandle<Vec<u8>>, String> {
    thread::Builder::new()
        .name("php-debug-preflight-output".into())
        .spawn(move || {
            let mut bytes = Vec::new();
            let _ = pipe.take((OUTPUT_LIMIT + 1) as u64).read_to_end(&mut bytes);
            if bytes.len() > OUTPUT_LIMIT {
                overflow.store(true, Ordering::SeqCst);
            }
            bytes
        })
        .map_err(|_| "Cannot capture PHP debugger preflight output".into())
}

pub(crate) fn validate_launch(
    config: &DebugConfiguration,
    shutdown: &AtomicBool,
) -> Result<(), String> {
    if shutdown.load(Ordering::SeqCst) {
        return Err("PHP debug launch cancelled".into());
    }
    let mut command = Command::new(&config.executable);
    command.env_clear();
    // Match the adapter's inherited environment, then the explicit program env.
    for key in [
        "PATH",
        "HOME",
        "USER",
        "LOGNAME",
        "SHELL",
        "TMPDIR",
        "TEMP",
        "TMP",
        "XDG_RUNTIME_DIR",
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "XAUTHORITY",
        "DBUS_SESSION_BUS_ADDRESS",
        "LANG",
        "LC_ALL",
        "LC_CTYPE",
        "SSL_CERT_FILE",
        "SSL_CERT_DIR",
        "SystemRoot",
        "APPDATA",
        "LOCALAPPDATA",
        "USERPROFILE",
    ] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command
        .envs(&config.env)
        .current_dir(&config.cwd)
        .args([
            "-d",
            "auto_prepend_file=",
            "-d",
            "auto_append_file=",
            "-d",
            "opcache.enable_cli=0",
            "-d",
            "opcache.preload=",
            "-d",
            "xdebug.mode=debug",
            "-d",
            "xdebug.start_with_request=no",
            "-d",
            "xdebug.log=",
            "-r",
            PROBE,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let child = command.spawn().map_err(|_| {
        "Cannot start configured PHP interpreter; select an existing PHP runtime with Xdebug"
            .to_string()
    })?;
    let mut process = Process(child);
    let overflow = Arc::new(AtomicBool::new(false));
    let stdout = reader(
        process
            .0
            .stdout
            .take()
            .ok_or("PHP preflight stdout missing")?,
        overflow.clone(),
    )?;
    let stderr = reader(
        process
            .0
            .stderr
            .take()
            .ok_or("PHP preflight stderr missing")?,
        overflow.clone(),
    )?;
    let deadline = Instant::now() + Duration::from_secs(2);
    let status = loop {
        if shutdown.load(Ordering::SeqCst) {
            break Err("PHP debug launch cancelled".to_string());
        }
        if overflow.load(Ordering::SeqCst) {
            break Err("PHP debugger preflight output exceeded 8 KiB".to_string());
        }
        if Instant::now() >= deadline {
            break Err("PHP debugger preflight timed out after 2 seconds".to_string());
        }
        match process.0.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => thread::sleep(Duration::from_millis(5)),
            Err(_) => break Err("Cannot inspect configured PHP interpreter".to_string()),
        }
    };
    // Kill owned wrapper descendants too, so inherited pipes cannot keep readers alive.
    drop(process);
    let output = stdout
        .join()
        .map_err(|_| "PHP preflight output reader stopped")?;
    let errors = stderr
        .join()
        .map_err(|_| "PHP preflight diagnostic reader stopped")?;
    let status = status?;
    if output.len() > OUTPUT_LIMIT || errors.len() > OUTPUT_LIMIT {
        return Err("PHP debugger preflight output exceeded 8 KiB".into());
    }
    if !status.success() {
        return Err(
            "Configured PHP interpreter failed debugger preflight; verify PHP/Xdebug installation"
                .into(),
        );
    }
    let metadata: serde_json::Value = serde_json::from_slice(&output)
        .map_err(|_| "Cannot verify Xdebug in the configured PHP interpreter".to_string())?;
    if metadata["loaded"] != true {
        return Err("Configured PHP interpreter has no Xdebug extension; select a PHP runtime with Xdebug 3 installed".into());
    }
    if metadata["debug"] != true {
        return Err("Xdebug debug mode is disabled or unsupported; enable Xdebug 3 debug mode and remove XDEBUG_MODE=off from this configuration".into());
    }
    Ok(())
}
