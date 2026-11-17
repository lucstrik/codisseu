use super::MAX_OUTPUT;
use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
pub(crate) struct Output {
    pub text: String,
    pub code: Option<i32>,
    pub success: bool,
    pub truncated: bool,
    pub cancelled: bool,
    pub timeout: bool,
}
/// Console text is inert; ANSI/OSC never reaches the terminal or clipboard API.
pub(crate) fn inert(bytes: &[u8]) -> String {
    let mut state = 0;
    String::from_utf8_lossy(bytes)
        .chars()
        .filter(|&c| match state {
            0 => {
                if c == '\u{1b}' {
                    state = 1;
                    false
                } else {
                    !c.is_control() || matches!(c, '\n' | '\t' | '\r')
                }
            }
            1 => {
                state = match c {
                    '[' => 2,
                    ']' => 3,
                    _ => 0,
                };
                false
            }
            2 => {
                if ('@'..='~').contains(&c) {
                    state = 0;
                }
                false
            }
            3 => {
                if c == '\u{7}' {
                    state = 0;
                } else if c == '\u{1b}' {
                    state = 4;
                }
                false
            }
            _ => {
                state = if c == '\\' { 0 } else { 3 };
                false
            }
        })
        .collect()
}
#[cfg(unix)]
fn nonblocking(pipe: &impl std::os::fd::AsRawFd) -> Result<(), String> {
    let fd = pipe.as_raw_fd();
    // Own pipe: flags change only this worker's descriptor; no retained pointer.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(())
}
#[cfg(unix)]
fn terminate(child: &mut std::process::Child) {
    // Only the group created by process_group(0) for this child is targeted.
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}
#[cfg(unix)]
fn drain(
    pipe: &mut impl std::io::Read,
    bytes: &mut Vec<u8>,
    truncated: &mut bool,
) -> Result<(), String> {
    let mut buffer = [0; 8192];
    for _ in 0..32 {
        match pipe.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => {
                let available = MAX_OUTPUT.saturating_sub(bytes.len());
                bytes.extend_from_slice(&buffer[..n.min(available)]);
                *truncated |= n > available;
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(())
}
pub(crate) fn run_with_output(
    program: &Path,
    args: &[String],
    root: &Path,
    timeout: Duration,
    cancel: &AtomicBool,
    output: &impl Fn(String),
) -> Result<Output, String> {
    if cancel.load(Ordering::SeqCst) {
        return Ok(Output {
            text: String::new(),
            code: None,
            success: false,
            truncated: false,
            cancelled: true,
            timeout: false,
        });
    }
    #[cfg(not(unix))]
    {
        let _ = (program, args, root, timeout, output);
        Err("Runner PHP com cancelamento de grupo disponível em Unix; plataforma ainda não validada".into())
    }
    #[cfg(unix)]
    {
        use std::{
            os::unix::process::CommandExt,
            process::{Command, Stdio},
        };
        let mut child = Command::new(program)
            .args(args)
            .current_dir(root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .env("NO_COLOR", "1")
            .env("TERM", "dumb")
            .env("XDEBUG_MODE", "off")
            .spawn()
            .map_err(|e| format!("Não foi possível iniciar o runner: {e}"))?;
        let result = (|| {
            let mut out = child.stdout.take().ok_or("Runner sem stdout")?;
            let mut err = child.stderr.take().ok_or("Runner sem stderr")?;
            nonblocking(&out)?;
            nonblocking(&err)?;
            let start = Instant::now();
            let mut emitted = Instant::now();
            let mut bytes = Vec::new();
            let mut truncated = false;
            let mut last_length = 0;
            loop {
                drain(&mut out, &mut bytes, &mut truncated)?;
                drain(&mut err, &mut bytes, &mut truncated)?;
                if emitted.elapsed() > Duration::from_millis(100) && bytes.len() != last_length {
                    output(inert(&bytes));
                    last_length = bytes.len();
                    emitted = Instant::now();
                }
                let cancelled = cancel.load(Ordering::SeqCst);
                let expired = start.elapsed() >= timeout;
                if cancelled || expired {
                    terminate(&mut child);
                    drain(&mut out, &mut bytes, &mut truncated)?;
                    drain(&mut err, &mut bytes, &mut truncated)?;
                    return Ok(Output {
                        text: inert(&bytes),
                        code: None,
                        success: false,
                        truncated,
                        cancelled,
                        timeout: expired && !cancelled,
                    });
                }
                if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                    terminate(&mut child);
                    // All writers are now gone. Drain remaining bounded kernel buffers.
                    for _ in 0..16 {
                        let before = bytes.len();
                        drain(&mut out, &mut bytes, &mut truncated)?;
                        drain(&mut err, &mut bytes, &mut truncated)?;
                        if before == bytes.len() {
                            break;
                        }
                    }
                    return Ok(Output {
                        text: inert(&bytes),
                        code: status.code(),
                        success: status.success(),
                        truncated,
                        cancelled: false,
                        timeout: false,
                    });
                }
                std::thread::sleep(Duration::from_millis(8));
            }
        })();
        terminate(&mut child);
        result
    }
}

pub(super) fn run(
    program: &Path,
    args: &[String],
    root: &Path,
    timeout: Duration,
    cancel: &AtomicBool,
    sender: &std::sync::mpsc::SyncSender<super::Event>,
) -> Result<Output, String> {
    run_with_output(program, args, root, timeout, cancel, &|text| {
        let _ = sender.try_send(super::Event::Output(text));
    })
}
