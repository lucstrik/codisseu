use std::{
    io::Read,
    path::Path,
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
const OUTPUT_LIMIT: usize = 2 * 1024 * 1024;
const LOG_LIMIT: usize = 256 * 1024;
const ERROR_LIMIT: usize = 64 * 1024;
const TIMEOUT: Duration = Duration::from_secs(30);
pub(super) struct Output {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub truncated: bool,
}
#[cfg(unix)]
fn nonblocking(pipe: &impl std::os::fd::AsRawFd) -> Result<(), String> {
    let fd = pipe.as_raw_fd();
    // Pipe is owned for the duration of this call. fcntl does not retain it.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(format!(
            "Não foi possível configurar saída Docker: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}
fn terminate(child: &mut Child) {
    #[cfg(unix)]
    // process_group(0) makes the child's PID the process group ID; it is our child.
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}
fn drain(pipe: &mut impl Read, output: &mut Vec<u8>, limit: usize) -> Result<bool, String> {
    let mut buffer = [0; 8192];
    // Do not starve cancellation when a producer writes continuously.
    for _ in 0..32 {
        match pipe.read(&mut buffer) {
            Ok(0) => return Ok(false),
            Ok(n) => {
                let remaining = limit.saturating_sub(output.len());
                output.extend_from_slice(&buffer[..n.min(remaining)]);
                if n > remaining {
                    return Ok(true);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(format!("Falha na saída Docker: {e}")),
        }
    }
    Ok(false)
}
pub(super) fn run(
    program: &Path,
    args: &[String],
    cancel: &AtomicBool,
    logs: bool,
) -> Result<Output, String> {
    run_with_timeout(program, args, cancel, logs, TIMEOUT)
}
fn run_with_timeout(
    program: &Path,
    args: &[String],
    cancel: &AtomicBool,
    logs: bool,
    timeout: Duration,
) -> Result<Output, String> {
    if cancel.load(Ordering::SeqCst) {
        return Err("Docker cancelado antes de iniciar".into());
    }
    #[cfg(not(unix))]
    {
        let _ = (program, args, logs, timeout);
        Err("Runner Docker com cancelamento está disponível em Unix; este sistema requer implementação de processos/pipes equivalente".into())
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let mut command = Command::new(program);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .env("LC_ALL", "C")
            .env("NO_COLOR", "1");
        let mut child = command.spawn().map_err(|e| {
            format!(
                "Não foi possível executar Docker ({}): {e}",
                program.display()
            )
        })?;
        let result = (|| {
            let mut stdout = child.stdout.take().ok_or("stdout Docker ausente")?;
            let mut stderr = child.stderr.take().ok_or("stderr Docker ausente")?;
            nonblocking(&stdout)?;
            nonblocking(&stderr)?;
            let started = Instant::now();
            let mut out = Vec::new();
            let mut err = Vec::new();
            loop {
                if cancel.load(Ordering::SeqCst) {
                    return Err(
                        "Operação Docker interrompida; atualize para conferir o estado real".into(),
                    );
                }
                if started.elapsed() > timeout {
                    return Err("Docker excedeu o prazo de 30 segundos; atualize para conferir o estado real".into());
                }
                let out_limit = if logs {
                    LOG_LIMIT.saturating_sub(err.len())
                } else {
                    OUTPUT_LIMIT
                };
                let exceeded = drain(&mut stdout, &mut out, out_limit)?
                    | drain(
                        &mut stderr,
                        &mut err,
                        if logs {
                            LOG_LIMIT.saturating_sub(out.len())
                        } else {
                            ERROR_LIMIT
                        },
                    )?;
                if exceeded {
                    if logs {
                        return Ok(Output {
                            stdout: out,
                            stderr: err,
                            truncated: true,
                        });
                    }
                    return Err("Saída Docker excede 2 MiB ou diagnóstico excede 64 KiB; operação interrompida".into());
                }
                if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                    // The CLI has exited. Kill pipe-owning descendants, then consume
                    // remaining buffered output without assuming a kernel pipe capacity.
                    terminate(&mut child);
                    let mut exceeded = false;
                    loop {
                        let before = out.len() + err.len();
                        exceeded |= drain(
                            &mut stdout,
                            &mut out,
                            if logs {
                                LOG_LIMIT.saturating_sub(err.len())
                            } else {
                                OUTPUT_LIMIT
                            },
                        )? | drain(
                            &mut stderr,
                            &mut err,
                            if logs {
                                LOG_LIMIT.saturating_sub(out.len())
                            } else {
                                ERROR_LIMIT
                            },
                        )?;
                        if exceeded || before == out.len() + err.len() {
                            break;
                        }
                        if cancel.load(Ordering::SeqCst) {
                            return Err("Operação Docker interrompida".into());
                        }
                    }
                    if exceeded && !logs {
                        return Err("Saída Docker excedeu o orçamento".into());
                    }
                    if !status.success() {
                        let error = super::inert(&String::from_utf8_lossy(&err));
                        return Err(format!(
                            "Docker terminou com {}: {}",
                            status
                                .code()
                                .map(|c| c.to_string())
                                .unwrap_or_else(|| "sinal".into()),
                            error.chars().take(2048).collect::<String>()
                        ));
                    }
                    return Ok(Output {
                        stdout: out,
                        stderr: err,
                        truncated: exceeded,
                    });
                }
                std::thread::sleep(Duration::from_millis(8));
            }
        })();
        terminate(&mut child);
        result
    }
}
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn timeout_and_cancel_reap_process_group() {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("pid");
        let command = format!("sleep 30 & echo $! > '{}'; wait", marker.display());
        let args = vec!["-c".into(), command];
        let began = Instant::now();
        let error = run_with_timeout(
            Path::new("/bin/sh"),
            &args,
            &AtomicBool::new(false),
            false,
            Duration::from_millis(120),
        )
        .err()
        .unwrap();
        assert!(error.contains("prazo"));
        assert!(began.elapsed() < Duration::from_secs(2));
        let pid: i32 = std::fs::read_to_string(marker)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        // A killed orphan may briefly be a zombie until PID1 reaps it; it cannot run.
        if let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            assert!(
                stat.split_whitespace()
                    .nth(2)
                    .is_some_and(|state| state == "Z")
            );
        }
        let cancelled = AtomicBool::new(true);
        assert!(run(Path::new("/bin/sh"), &args, &cancelled, false).is_err());
    }
    #[test]
    fn output_is_bounded_and_logs_truncate() {
        let args = vec!["-c".into(), "yes x".into()];
        let out = run(Path::new("/bin/sh"), &args, &AtomicBool::new(false), true).unwrap();
        assert!(out.truncated);
        assert!(out.stdout.len() + out.stderr.len() <= LOG_LIMIT);
        let error = run(Path::new("/bin/sh"), &args, &AtomicBool::new(false), false)
            .err()
            .unwrap();
        assert!(error.contains("excede"));
    }
    #[test]
    fn delayed_stderr_and_stdout_share_log_budget() {
        let args = vec![
            "-c".into(),
            "head -c 131072 /dev/zero >&2; sleep 0.03; head -c 262144 /dev/zero".into(),
        ];
        let out = run(Path::new("/bin/sh"), &args, &AtomicBool::new(false), true).unwrap();
        assert!(out.truncated);
        assert!(out.stdout.len() + out.stderr.len() <= LOG_LIMIT);
    }
    #[test]
    fn complete_stdout_and_stderr_survive_fast_exit() {
        let args = vec![
            "-c".into(),
            "import os;os.write(1,b'x'*524288);os.write(2,b'e'*65536)".into(),
        ];
        let out = run(
            Path::new("/usr/bin/python3"),
            &args,
            &AtomicBool::new(false),
            false,
        )
        .unwrap();
        assert_eq!(out.stdout.len(), 524288);
        assert_eq!(out.stderr.len(), 65536);
        assert!(!out.truncated);
    }
    #[test]
    fn argv_is_literal_and_stderr_failure_is_inert() {
        let args = vec![
            "-c".into(),
            r#"printf '%s' "$1"; printf '\033[31mdenied\033[0m' >&2; exit 7"#.into(),
            "fixture".into(),
            "$(touch /never);x y".into(),
        ];
        let literal_args = vec![
            "-c".into(),
            r#"printf '%s' "$1""#.into(),
            "fixture".into(),
            "$(touch /never);x y".into(),
        ];
        let out = run(
            Path::new("/bin/sh"),
            &literal_args,
            &AtomicBool::new(false),
            false,
        )
        .unwrap();
        assert_eq!(out.stdout, b"$(touch /never);x y");
        let error = run(Path::new("/bin/sh"), &args, &AtomicBool::new(false), false)
            .err()
            .unwrap();
        assert!(error.contains("7: denied"), "{error:?}");
        assert!(!error.contains('\u{1b}'));
    }
}
