//! Bounded, cancellable compiler verification using the already configured runtime.
use crate::{Language, ProviderConfig};
use serde_json::Value;
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Debug)]
pub struct TypeScriptRuntime {
    program: PathBuf,
    module: PathBuf,
}
impl TypeScriptRuntime {
    /// Lexical discovery only; actual file access remains on the worker.
    pub fn from_config(config: &ProviderConfig) -> Option<Self> {
        if config.language != Language::TypeScript {
            return None;
        }
        let cli = config.args.iter().map(Path::new).find(|p| {
            p.file_name()
                .is_some_and(|n| n == "cli.mjs" || n == "cli.js")
                && p.parent()
                    .and_then(Path::parent)
                    .and_then(Path::file_name)
                    .is_some_and(|n| n == "typescript-language-server")
        })?;
        let modules = cli.parent()?.parent()?.parent()?;
        Some(Self {
            program: config.program.clone(),
            module: modules.join("typescript/lib/typescript.js"),
        })
    }
    pub fn verify(
        &self,
        root: &Path,
        mut input: Value,
        cancel: &AtomicBool,
    ) -> Result<Value, String> {
        self.verify_with_deadline(root, &mut input, cancel, Duration::from_secs(10))
    }
    fn verify_with_deadline(
        &self,
        root: &Path,
        input: &mut Value,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Value, String> {
        if cancel.load(Ordering::Acquire) {
            return Err("Verificação TypeScript cancelada".into());
        }
        let module = if self.module.is_absolute() {
            self.module.clone()
        } else {
            root.join(&self.module)
        };
        let module = module
            .canonicalize()
            .map_err(|e| format!("Compilador TypeScript indisponível: {e}"))?;
        input["typeScriptModule"] = serde_json::to_value(module).map_err(|e| e.to_string())?;
        let payload = serde_json::to_vec(input).map_err(|e| e.to_string())?;
        if payload.len() > 20 * 1024 * 1024 {
            return Err("Entrada TypeScript excede 20 MiB".into());
        }
        let mut command = Command::new(&self.program);
        command
            .args([
                "--max-old-space-size=256",
                "-e",
                include_str!("../semantic/typescript-rename.cjs"),
            ])
            .current_dir(root)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for key in [
            "HOME",
            "USERPROFILE",
            "PATH",
            "SYSTEMROOT",
            "TMPDIR",
            "TEMP",
            "XDG_CACHE_HOME",
        ] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        command.env("LANG", "C.UTF-8");
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command
            .spawn()
            .map_err(|e| format!("Não foi possível iniciar verificador TypeScript: {e}"))?;
        let mut stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let (sender, receiver) = mpsc::channel();
        let writer = thread::spawn(move || stdin.write_all(&payload));
        let out_sender = sender.clone();
        let out = thread::spawn(move || read_bounded(stdout, 1024 * 1024, out_sender));
        let err = thread::spawn(move || read_bounded(stderr, 64 * 1024, sender));
        let start = Instant::now();
        let mut failure = None;
        let status = loop {
            if cancel.load(Ordering::Acquire) {
                failure = Some("Verificação TypeScript cancelada".to_owned());
            }
            if start.elapsed() >= timeout {
                failure = Some("Verificação TypeScript excedeu 10 segundos".to_owned());
            }
            if receiver.try_recv().is_ok() {
                failure = Some("Saída TypeScript excedeu limite".to_owned());
            }
            if failure.is_some() {
                terminate_group(&mut child);
                break child.wait();
            }
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) => thread::sleep(Duration::from_millis(10)),
                Err(e) => {
                    terminate_group(&mut child);
                    let _ = child.wait();
                    break Err(e);
                }
            }
        };
        // A configured wrapper may exit before its descendants close the pipes.
        terminate_group(&mut child);
        let written = writer.join().map_err(|_| "Falha no envio TypeScript")?;
        let output = out.join().map_err(|_| "Falha na saída TypeScript")??;
        let error = err
            .join()
            .map_err(|_| "Falha no diagnóstico TypeScript")??;
        if let Some(error) = failure {
            return Err(error);
        }
        written.map_err(|e| format!("Falha no envio TypeScript: {e}"))?;
        if !status.map_err(|e| e.to_string())?.success() {
            return Err(format!(
                "Verificador TypeScript falhou: {}",
                String::from_utf8_lossy(&error)
                    .chars()
                    .take(1000)
                    .collect::<String>()
            ));
        }
        serde_json::from_slice(&output).map_err(|e| format!("Resposta TypeScript inválida: {e}"))
    }
}
fn terminate_group(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        unsafe extern "C" {
            fn kill(pid: i32, signal: i32) -> i32;
        }
        // SAFETY: this child was spawned in a new group with its own PID as
        // group ID; the signal targets only that managed process group.
        unsafe {
            kill(-(child.id() as i32), 9);
        }
    }
    let _ = child.kill();
}
fn read_bounded(
    mut reader: impl Read,
    limit: usize,
    overflow: mpsc::Sender<()>,
) -> Result<Vec<u8>, String> {
    let mut result = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let count = reader.read(&mut chunk).map_err(|e| e.to_string())?;
        if count == 0 {
            return Ok(result);
        }
        if result.len() + count > limit {
            let _ = overflow.send(());
            return Err("Saída TypeScript excedeu limite".into());
        }
        result.extend_from_slice(&chunk[..count]);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn runtime_is_derived_only_from_configured_typescript_provider() {
        let mut config = ProviderConfig {
            language: Language::TypeScript,
            program: "/opt/node".into(),
            args: vec![
                "/opt/runtime/node_modules/typescript-language-server/lib/cli.mjs".into(),
                "--stdio".into(),
            ],
        };
        assert_eq!(
            TypeScriptRuntime::from_config(&config).unwrap().module,
            Path::new("/opt/runtime/node_modules/typescript/lib/typescript.js")
        );
        config.language = Language::Php;
        assert!(TypeScriptRuntime::from_config(&config).is_none());
        config.language = Language::TypeScript;
        config.args[0] = "/project/cli.mjs".into();
        assert!(TypeScriptRuntime::from_config(&config).is_none());
    }
    #[cfg(unix)]
    #[test]
    fn deadline_kills_and_reaps_the_worker_process() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("runtime");
        // Keep stdout/stderr inherited by a descendant to catch a parent-only kill.
        std::fs::write(&program, "#!/bin/sh\nsleep 30 &\nwait\n").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        let module = dir.path().join("typescript.js");
        std::fs::write(&module, "").unwrap();
        let runtime = TypeScriptRuntime { program, module };
        let started = Instant::now();
        let error = runtime
            .verify_with_deadline(
                dir.path(),
                &mut serde_json::json!({}),
                &AtomicBool::new(false),
                Duration::from_millis(50),
            )
            .unwrap_err();
        assert!(error.contains("excedeu"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(2));
    }
    #[test]
    fn pre_cancelled_verification_does_not_launch_missing_runtime() {
        let runtime = TypeScriptRuntime {
            program: "/missing/runtime".into(),
            module: "/missing/typescript.js".into(),
        };
        assert!(
            runtime
                .verify(
                    Path::new("/"),
                    serde_json::json!({}),
                    &AtomicBool::new(true)
                )
                .unwrap_err()
                .contains("cancelada")
        );
    }
    #[test]
    fn output_limit_never_truncates_into_a_successful_report() {
        let (sender, receiver) = mpsc::channel();
        assert!(read_bounded(&b"12345"[..], 4, sender).is_err());
        assert!(receiver.try_recv().is_ok());
    }
}
