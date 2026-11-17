//! Optional managed Node.js 22.13+ runtime. A compatibility subset, not a security
//! boundary for untrusted code. Explicit script enablement is always required.
use crate::{ApiRequest, ApiResponse, KeyValue, Variables};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScriptTest {
    pub name: String,
    pub passed: bool,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScriptResult {
    #[serde(default)]
    pub skipped: bool,
    #[serde(default)]
    pub next_request_set: bool,
    #[serde(default)]
    pub next_request: Option<String>,
    #[serde(default)]
    pub global_updates: Variables,
    #[serde(default)]
    pub global_unsets: Vec<String>,
    #[serde(default)]
    pub locals: Variables,
    #[serde(default)]
    pub request_headers: Option<Vec<KeyValue>>,
    #[serde(default)]
    pub request_params: Option<Vec<KeyValue>>,
    #[serde(default)]
    pub request_method: Option<String>,
    pub environment_updates: Variables,
    pub collection_updates: Variables,
    pub environment_unsets: Vec<String>,
    pub collection_unsets: Vec<String>,
    pub tests: Vec<ScriptTest>,
    pub logs: Vec<String>,
}

pub fn run_script(
    script: &str,
    variables: &Variables,
    response: Option<&ApiResponse>,
) -> Result<ScriptResult, String> {
    run_script_scoped(
        script,
        variables,
        &Variables::new(),
        true,
        response,
        &Arc::new(AtomicBool::new(false)),
    )
}
pub(crate) fn run_script_scoped(
    script: &str,
    environment: &Variables,
    collection: &Variables,
    environment_available: bool,
    response: Option<&ApiResponse>,
    cancel: &Arc<AtomicBool>,
) -> Result<ScriptResult, String> {
    run_script_context(
        script,
        environment,
        collection,
        environment_available,
        response,
        cancel,
        &ScriptContext::default(),
    )
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct ScriptContext {
    pub globals: Variables,
    pub locals: Variables,
    pub iteration_data: Variables,
    pub iteration: u32,
    pub iteration_count: u32,
    #[serde(skip)]
    pub request: Option<ApiRequest>,
    pub location: Vec<String>,
}
pub(crate) fn run_script_context(
    script: &str,
    environment: &Variables,
    collection: &Variables,
    environment_available: bool,
    response: Option<&ApiResponse>,
    cancel: &Arc<AtomicBool>,
    context: &ScriptContext,
) -> Result<ScriptResult, String> {
    if script.trim().is_empty() {
        return Ok(ScriptResult {
            locals: context.locals.clone(),
            ..ScriptResult::default()
        });
    }
    if script.len() > 64 * 1024 {
        return Err("Script exceeds 64 KiB".into());
    }
    let data = serde_json::json!({"environment":environment,"collection":collection,"environment_available":environment_available,"context":context,"request":context.request,"response":response.map(|r|serde_json::json!({"code":r.status,"status":reqwest::StatusCode::from_u16(r.status).ok().and_then(|s|s.canonical_reason()).unwrap_or(""),"body":r.text(),"responseTime":r.elapsed_ms,"headers":r.headers}))});
    let api = include_str!("script-api.js");
    let program = format!(
        r#"(() => {{ "use strict"; const data = {data};
{api}
try {{ (function() {{
{script}
}}).call(undefined); }} catch(error) {{ if(error!==skipSignal)throw error; }}
return JSON.stringify({{environment_updates:updates,collection_updates:collection,environment_unsets:envUnset,collection_unsets:collUnset,global_updates:globalUpdates,global_unsets:globalUnset,locals:local,skipped,next_request_set:nextRequestSet,next_request:nextRequest,request_headers:request?headerRows:null,request_params:request?queryRows:null,request_method:request?.method??null,tests,logs}});
}})()"#
    );
    let input = serde_json::to_vec(&program).map_err(|_| "Could not prepare script")?;
    if input.len() > 16 * 1024 * 1024 {
        return Err("Script context exceeds 16 MiB".into());
    }
    let temporary =
        tempfile::tempdir().map_err(|_| "Could not prepare script runtime directory")?;
    let mut command = Command::new(codisseu_runtime::node()?);
    command
        .args([
            "--permission",
            "--jitless",
            "--max-old-space-size=64",
            "--disable-proto=throw",
            "--eval",
            include_str!("script-runtime.js"),
        ])
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .current_dir(temporary.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: only async-signal-safe setrlimit syscalls run between fork and exec.
        unsafe {
            command.pre_exec(|| {
                use nix::sys::resource::{Resource, setrlimit};
                setrlimit(Resource::RLIMIT_CORE, 0, 0).map_err(std::io::Error::from)?;
                setrlimit(Resource::RLIMIT_CPU, 2, 2).map_err(std::io::Error::from)?;
                #[cfg(target_os = "linux")]
                setrlimit(Resource::RLIMIT_DATA, 256 * 1024 * 1024, 256 * 1024 * 1024)
                    .map_err(std::io::Error::from)?;
                Ok(())
            });
        }
    }
    let mut child = command
        .spawn()
        .map_err(|_| "Scripts require Node.js 22.13+ with --permission support")?;
    let mut stdin = child.stdin.take().ok_or("Script stdin unavailable")?;
    let stdout = child.stdout.take().ok_or("Script stdout unavailable")?;
    let writer = thread::spawn(move || stdin.write_all(&input));
    let reader = thread::spawn(move || {
        let mut data = vec![];
        stdout
            .take(1024 * 1024 + 1)
            .read_to_end(&mut data)
            .map(|_| data)
    });
    let started = Instant::now();
    let mut interrupted = false;
    let status = loop {
        if cancel.load(Ordering::Acquire) || started.elapsed() > Duration::from_millis(2500) {
            interrupted = true;
            let _ = child.kill();
            break child.wait();
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(error);
            }
        }
    };
    let _ = writer.join();
    let output = reader
        .join()
        .map_err(|_| "Script output worker stopped")?
        .map_err(|_| "Could not read script result")?;
    if interrupted {
        return Err(if cancel.load(Ordering::Acquire) {
            "Script cancelled"
        } else {
            "Script exceeded 2500 ms and was terminated"
        }
        .into());
    }
    if matches!(&status,Ok(s) if s.code()==Some(5)) {
        return Err("Select an environment before scripts modify environment variables; no script changes applied".into());
    }
    if matches!(&status,Ok(s) if s.code()==Some(6)) {
        return Err("Variable value is null or undefined; no script changes applied".into());
    }
    if !status.is_ok_and(|s| s.success()) {
        return Err("JavaScript failed or exceeded runtime/resource limits; no script changes applied (Node.js 22.13+ required)".into());
    }
    if output.len() > 1024 * 1024 {
        return Err("Script output exceeds 1 MiB".into());
    }
    serde_json::from_slice(&output).map_err(|_| "Invalid script output".into())
}
