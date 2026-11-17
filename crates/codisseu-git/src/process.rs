use crate::{GitError, GitErrorKind, GitOutput};
use std::{
    ffi::OsString,
    io::{Read, Write},
    path::Path,
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
pub(crate) const MAX_OUTPUT: usize = 8 * 1024 * 1024;
const MAX_STDERR: usize = 256 * 1024;
#[cfg(unix)]
fn nonblocking<T: std::os::fd::AsRawFd>(pipe: &T) -> Result<(), GitError> {
    use nix::fcntl::{FcntlArg, OFlag, fcntl};
    let flags = fcntl(pipe.as_raw_fd(), FcntlArg::F_GETFL)
        .map_err(|e| GitError::new(GitErrorKind::Io, e.to_string()))?;
    fcntl(
        pipe.as_raw_fd(),
        FcntlArg::F_SETFL(OFlag::from_bits_truncate(flags) | OFlag::O_NONBLOCK),
    )
    .map_err(|e| GitError::new(GitErrorKind::Io, e.to_string()))?;
    Ok(())
}
#[cfg(unix)]
fn kill(child: &mut std::process::Child) {
    use nix::{
        sys::signal::{Signal, killpg},
        unistd::Pid,
    };
    let _ = killpg(Pid::from_raw(child.id() as i32), Signal::SIGKILL);
    let _ = child.kill();
    let _ = child.wait();
}
#[cfg(not(unix))]
fn kill(child: &mut std::process::Child) {
    let _ = child.kill();
    let _ = child.wait();
}
fn drain(pipe: &mut impl Read, output: &mut Vec<u8>, limit: usize) -> Result<(), GitError> {
    let mut buffer = [0u8; 32768];
    loop {
        match pipe.read(&mut buffer) {
            Ok(0) => return Ok(()),
            Ok(n) => {
                if output.len() + n > limit {
                    return Err(GitError::new(
                        GitErrorKind::OutputLimit,
                        "Saída Git excede o orçamento; processo interrompido, confira o estado do repositório",
                    ));
                }
                output.extend_from_slice(&buffer[..n]);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        }
    }
}
pub(crate) fn run(
    root: &Path,
    args: &[OsString],
    stdin: Option<&[u8]>,
    cancel: &AtomicBool,
    timeout: Duration,
    environment: &[(OsString, OsString)],
) -> Result<GitOutput, GitError> {
    if cancel.load(Ordering::SeqCst) {
        return Err(GitError::new(
            GitErrorKind::Cancelled,
            "Git cancelado antes de iniciar",
        ));
    }
    if stdin.is_some_and(|input| input.len() > MAX_OUTPUT) {
        return Err(GitError::invalid("Entrada Git excede8 MiB"));
    }
    let mut read_options = Vec::<OsString>::new();
    if args.first().is_some_and(|a| {
        matches!(
            a.to_str(),
            Some(
                "status" | "diff" | "log" | "show" | "blame" | "ls-files" | "ls-tree" | "diff-tree"
            )
        )
    }) {
        // Rendering a commit must never launch a configured GPG helper.
        // Keep this read-only override out of explicitly authorized write hooks.
        read_options.extend(["-c".into(), "log.showSignature=false".into()]);
        let config_args = [
            "config",
            "--name-only",
            "-z",
            "--get-regexp",
            r"^filter\..*\.(clean|smudge|process|required)$",
        ]
        .map(OsString::from);
        let configured = run(root, &config_args, None, cancel, timeout, &[])?;
        if configured.status.success() {
            for raw in configured
                .stdout
                .split(|b| *b == 0)
                .filter(|v| !v.is_empty())
            {
                let key = std::str::from_utf8(raw)
                    .map_err(|_| GitError::unsupported("Configuração de filtro Git não Unicode"))?;
                let Some((driver, _)) = key.rsplit_once('.') else {
                    continue;
                };
                for suffix in ["clean=", "smudge=", "process=", "required=false"] {
                    read_options.push("-c".into());
                    read_options.push(format!("{driver}.{suffix}").into());
                }
            }
        }
    }
    let mut command = Command::new("git");
    command
        .current_dir(root)
        .args([
            "--no-pager",
            "--no-optional-locks",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "color.ui=never",
            "-c",
            "protocol.ext.allow=never",
        ])
        .args(&read_options)
        .args(args)
        .env("LC_ALL", "C")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .env("GIT_SEQUENCE_EDITOR", "true")
        .env("GIT_LITERAL_PATHSPECS", "1")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_ASKPASS", "true")
        .env("SSH_ASKPASS", "true")
        .env("SSH_ASKPASS_REQUIRE", "never")
        .env(
            "GIT_SSH_COMMAND",
            "ssh -oBatchMode=yes -oStrictHostKeyChecking=yes",
        );
    for key in [
        "GIT_DIR",
        "GIT_COMMON_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_NAMESPACE",
        "GIT_CONFIG_PARAMETERS",
        "GIT_CONFIG_COUNT",
    ] {
        command.env_remove(key);
    }
    for (key, value) in environment {
        command.env(key, value);
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn()?;
    let input = child
        .stdin
        .take()
        .ok_or_else(|| GitError::invalid("stdin Git ausente"))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| GitError::invalid("stdout Git ausente"))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| GitError::invalid("stderr Git ausente"))?;
    #[cfg(unix)]
    {
        if let Err(error) = nonblocking(&input)
            .and_then(|_| nonblocking(&stdout))
            .and_then(|_| nonblocking(&stderr))
        {
            kill(&mut child);
            return Err(error);
        }
    }
    #[cfg(not(unix))]
    {
        kill(&mut child);
        return Err(GitError::unsupported(
            "Runner Git com cancelamento limitado está disponível em Unix; backend Windows requer Job Objects/pipes assíncronos",
        ));
    }
    let started = Instant::now();
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut offset = 0;
    let input_bytes = stdin.unwrap_or_default();
    let mut input = Some(input);
    loop {
        let error = if cancel.load(Ordering::SeqCst) {
            Some(GitError::new(
                GitErrorKind::Cancelled,
                "Git interrompido; confira o resultado antes de repetir alterações",
            ))
        } else if started.elapsed() > timeout {
            Some(GitError::new(
                GitErrorKind::Timeout,
                "Prazo Git excedido; confira o resultado antes de repetir alterações",
            ))
        } else {
            None
        };
        if let Some(error) = error {
            kill(&mut child);
            return Err(error);
        }
        if let Err(error) = drain(&mut stdout, &mut out, MAX_OUTPUT)
            .and_then(|_| drain(&mut stderr, &mut err, MAX_STDERR))
        {
            kill(&mut child);
            return Err(error);
        }
        if let Some(pipe) = &mut input {
            if offset == input_bytes.len() {
                input = None;
            } else {
                match pipe.write(&input_bytes[offset..]) {
                    Ok(n) => offset += n,
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                        ) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => input = None,
                    Err(error) => {
                        kill(&mut child);
                        return Err(error.into());
                    }
                }
            }
        }
        if let Some(status) = child.try_wait()? {
            drain(&mut stdout, &mut out, MAX_OUTPUT)?;
            drain(&mut stderr, &mut err, MAX_STDERR)?;
            return Ok(GitOutput {
                stdout: out,
                stderr: String::from_utf8_lossy(&err).into_owned(),
                status,
            });
        }
        std::thread::sleep(Duration::from_millis(8));
    }
}
