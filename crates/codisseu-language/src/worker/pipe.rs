//! Pollable Unix child pipes. A provider Stop must not depend on every
//! descendant dropping inherited descriptors before its I/O threads can join.
#[cfg(unix)]
use std::sync::atomic::Ordering;
use std::{
    io,
    sync::{Arc, atomic::AtomicBool},
};

#[cfg(unix)]
pub(super) struct Pipe<T> {
    inner: T,
    alive: Arc<AtomicBool>,
}

#[cfg(unix)]
impl<T: std::os::fd::AsRawFd> Pipe<T> {
    pub(super) fn new(inner: T, alive: Arc<AtomicBool>) -> io::Result<Self> {
        let fd = inner.as_raw_fd();
        // These descriptors belong to the provider's dedicated transport threads.
        // Nonblocking I/O preserves successful partial reads/writes; readiness
        // polling is needed only when the kernel pipe is empty/full.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { inner, alive })
    }
    fn check_alive(&self) -> io::Result<()> {
        if self.alive.load(Ordering::Acquire) {
            Ok(())
        } else {
            // read_exact/write_all retry Interrupted; cancellation is terminal.
            Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "Provider pipe closed during shutdown",
            ))
        }
    }
    fn wait(&self, events: libc::c_short) -> io::Result<()> {
        loop {
            self.check_alive()?;
            let mut fd = libc::pollfd {
                fd: self.inner.as_raw_fd(),
                events,
                revents: 0,
            };
            let result = unsafe { libc::poll(&mut fd, 1, 50) };
            if result > 0 {
                return Ok(());
            }
            if result < 0 {
                let error = io::Error::last_os_error();
                if error.kind() != io::ErrorKind::Interrupted {
                    return Err(error);
                }
            }
        }
    }
}

#[cfg(unix)]
impl<T: std::os::fd::AsRawFd + io::Read> io::Read for Pipe<T> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        loop {
            self.check_alive()?;
            match self.inner.read(bytes) {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    self.wait(libc::POLLIN)?
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => (),
                result => return result,
            }
        }
    }
}
#[cfg(unix)]
impl<T: std::os::fd::AsRawFd + io::Write> io::Write for Pipe<T> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        loop {
            self.check_alive()?;
            match self.inner.write(bytes) {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    self.wait(libc::POLLOUT)?
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => (),
                result => return result,
            }
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(not(unix))]
pub(super) struct Pipe<T>(T);
#[cfg(not(unix))]
impl<T> Pipe<T> {
    pub(super) fn new(inner: T, _alive: Arc<AtomicBool>) -> io::Result<Self> {
        Ok(Self(inner))
    }
}
#[cfg(not(unix))]
impl<T: io::Read> io::Read for Pipe<T> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.0.read(bytes)
    }
}
#[cfg(not(unix))]
impl<T: io::Write> io::Write for Pipe<T> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{
        fs::File,
        io::{Read, Write},
        os::fd::FromRawFd,
        sync::mpsc,
        thread,
        time::Duration,
    };

    fn pipes() -> (File, File) {
        let mut fds = [-1; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        // pipe returned two owned descriptors; File closes each exactly once.
        unsafe { (File::from_raw_fd(fds[0]), File::from_raw_fd(fds[1])) }
    }

    #[test]
    fn shutdown_interrupts_reader_without_waiting_for_inherited_writer_eof() {
        let (reader, inherited_writer) = pipes();
        let alive = Arc::new(AtomicBool::new(true));
        let mut reader = Pipe::new(reader, alive.clone()).unwrap();
        let (done, result) = mpsc::channel();
        let task = thread::spawn(move || {
            done.send(reader.read_exact(&mut [0])).unwrap();
        });
        thread::sleep(Duration::from_millis(30));
        alive.store(false, Ordering::Release);
        let stopped = result.recv_timeout(Duration::from_secs(2));
        drop(inherited_writer);
        task.join().unwrap();
        assert_eq!(
            stopped.unwrap().unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
    }

    #[test]
    fn shutdown_interrupts_writer_when_inherited_reader_never_drains_pipe() {
        let (inherited_reader, writer) = pipes();
        let alive = Arc::new(AtomicBool::new(true));
        let mut writer = Pipe::new(writer, alive.clone()).unwrap();
        let (done, result) = mpsc::channel();
        let task = thread::spawn(move || {
            done.send(writer.write_all(&vec![b'x'; 4 * 1024 * 1024]))
                .unwrap();
        });
        thread::sleep(Duration::from_millis(30));
        assert!(matches!(result.try_recv(), Err(mpsc::TryRecvError::Empty)));
        alive.store(false, Ordering::Release);
        let stopped = result.recv_timeout(Duration::from_secs(2));
        drop(inherited_reader);
        task.join().unwrap();
        assert_eq!(
            stopped.unwrap().unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
    }

    #[test]
    fn frames_retain_partial_writes_across_backpressure() {
        let (reader, writer) = pipes();
        let alive = Arc::new(AtomicBool::new(true));
        let mut reader = Pipe::new(reader, alive.clone()).unwrap();
        let mut writer = Pipe::new(writer, alive).unwrap();
        let value = serde_json::json!({"text": "ação🌧".repeat(50000)});
        let expected = value.clone();
        let task =
            thread::spawn(move || crate::protocol::write_frame(&mut writer, &value).unwrap());
        thread::sleep(Duration::from_millis(30));
        assert_eq!(crate::protocol::read_frame(&mut reader).unwrap(), expected);
        task.join().unwrap();
    }
}
