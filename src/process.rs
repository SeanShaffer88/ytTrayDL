//! Owned subprocesses with pre-execution tree containment and bounded output.
//! Callbacks receive chunks, not lines; they must return promptly. No module
//! initialization starts work. Dropping an owner terminates and reaps its tree.
use std::ffi::{OsStr, OsString};
use std::io::{self, Read};
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[derive(Clone, Default, Debug)]
pub struct CancellationToken(Arc<AtomicBool>);
impl CancellationToken {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}
#[derive(Clone, Debug)]
pub struct ProcessSpec {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub current_dir: Option<PathBuf>,
}
impl ProcessSpec {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            args: vec![],
            current_dir: None,
        }
    }
    pub fn arg(mut self, arg: impl AsRef<OsStr>) -> Self {
        self.args.push(arg.as_ref().to_owned());
        self
    }
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.args
            .extend(args.into_iter().map(|a| a.as_ref().to_owned()));
        self
    }
}
#[derive(Clone, Copy, Debug)]
pub struct ProcessLimits {
    pub timeout: Duration,
    pub stdout_bytes: usize,
    pub stderr_bytes: usize,
    pub line_bytes: usize,
}
impl Default for ProcessLimits {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            stdout_bytes: 8 * 1024 * 1024,
            stderr_bytes: 256 * 1024,
            line_bytes: 1024 * 1024,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputStream {
    Stdout,
    Stderr,
}
#[derive(Debug)]
pub enum ProcessError {
    Spawn(io::Error),
    Io(io::Error),
    Timeout,
    Cancelled,
    OutputLimit { stream: OutputStream },
    Exit { code: Option<i32>, stderr: String },
}
impl std::fmt::Display for ProcessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spawn(e) => write!(f, "process spawn/containment failed: {e}"),
            Self::Io(e) => write!(f, "process I/O failed: {e}"),
            Self::Timeout => f.write_str("process deadline exceeded"),
            Self::Cancelled => f.write_str("process cancelled"),
            Self::OutputLimit { stream } => write!(f, "process {stream:?} output limit exceeded"),
            Self::Exit { code, stderr } => write!(f, "process exited {code:?}: {stderr}"),
        }
    }
}
impl std::error::Error for ProcessError {}
#[derive(Debug)]
pub struct ProcessOutput {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Native handles stay in the platform module. Unix uses an independent process
/// group established by Command before exec; unsupported platforms fail closed.
pub struct OwnedProcess {
    child: Child,
    #[cfg(test)]
    fail_termination_once: bool,
    #[cfg(unix)]
    group: Option<libc::pid_t>,
    #[cfg(windows)]
    tree: crate::platform::ProcessTree,
}
impl OwnedProcess {
    pub fn spawn(spec: &ProcessSpec) -> Result<Self, ProcessError> {
        let mut command = Command::new(&spec.program);
        command
            .args(&spec.args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(dir) = &spec.current_dir {
            command.current_dir(dir);
        }
        #[cfg(windows)]
        {
            let (child, tree) =
                crate::platform::ProcessTree::spawn(&mut command).map_err(ProcessError::Spawn)?;
            Ok(Self {
                child,
                tree,
                #[cfg(test)]
                fail_termination_once: false,
            })
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
            let child = command.spawn().map_err(ProcessError::Spawn)?;
            let group = Some(child.id() as libc::pid_t);
            Ok(Self {
                child,
                group,
                #[cfg(test)]
                fail_termination_once: false,
            })
        }
        #[cfg(not(any(windows, unix)))]
        {
            Err(ProcessError::Spawn(io::Error::new(
                io::ErrorKind::Unsupported,
                "process-tree containment unavailable",
            )))
        }
    }
    pub fn id(&self) -> u32 {
        self.child.id()
    }
    pub fn try_wait(&mut self) -> Result<Option<ExitStatus>, ProcessError> {
        #[cfg(unix)]
        if self.group.is_some() {
            // Observe exit without reaping: the leader reserves its PID until
            // the initial group signal, so it cannot name a recycled group.
            let mut info = std::mem::MaybeUninit::<libc::siginfo_t>::zeroed();
            let result = unsafe {
                libc::waitid(
                    libc::P_PID,
                    self.child.id() as libc::id_t,
                    info.as_mut_ptr(),
                    libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                )
            };
            if result != 0 {
                return Err(ProcessError::Io(io::Error::last_os_error()));
            }
            if unsafe { info.assume_init().si_pid() } == 0 {
                return Ok(None);
            }
            self.terminate_tree().map_err(ProcessError::Io)?;
        }
        self.child.try_wait().map_err(ProcessError::Io)
    }
    fn terminate_tree(&mut self) -> io::Result<()> {
        #[cfg(test)]
        if std::mem::take(&mut self.fail_termination_once) {
            return Err(io::Error::other("injected termination failure"));
        }
        #[cfg(windows)]
        {
            self.tree.terminate()
        }
        #[cfg(unix)]
        {
            terminate_group(&mut self.group, |group| {
                // The unreaped leader reserves this identity until this signal.
                if unsafe { libc::kill(-group, libc::SIGKILL) } == 0 {
                    Ok(())
                } else {
                    let error = io::Error::last_os_error();
                    if error.raw_os_error() == Some(libc::ESRCH) {
                        Ok(())
                    } else {
                        Err(error)
                    }
                }
            })
        }
        #[cfg(not(any(windows, unix)))]
        {
            self.child.kill()
        }
    }
    pub fn terminate_and_wait(&mut self) -> Result<ExitStatus, ProcessError> {
        self.terminate_tree().map_err(ProcessError::Io)?;
        self.child.wait().map_err(ProcessError::Io)
    }
    pub fn capture(
        self,
        limits: ProcessLimits,
        token: &CancellationToken,
    ) -> Result<ProcessOutput, ProcessError> {
        self.collect(limits, token, false, |_, _| {})
    }
    /// Streaming retains bounded tails without a lifetime output cap. A single
    /// unterminated line exceeding line_bytes still fails and stops the tree.
    pub fn stream<F: FnMut(OutputStream, &[u8])>(
        self,
        limits: ProcessLimits,
        token: &CancellationToken,
        callback: F,
    ) -> Result<ProcessOutput, ProcessError> {
        self.collect(limits, token, true, callback)
    }
    fn collect<F: FnMut(OutputStream, &[u8])>(
        mut self,
        limits: ProcessLimits,
        token: &CancellationToken,
        streaming: bool,
        mut callback: F,
    ) -> Result<ProcessOutput, ProcessError> {
        let (tx, rx) = mpsc::sync_channel(16);
        let stdout = self.child.stdout.take().expect("owned stdout");
        let stderr = self.child.stderr.take().expect("owned stderr");
        let stop_readers = Arc::new(AtomicBool::new(false));
        let readers = [
            spawn_reader(
                stdout,
                OutputStream::Stdout,
                tx.clone(),
                stop_readers.clone(),
            ),
            spawn_reader(stderr, OutputStream::Stderr, tx, stop_readers.clone()),
        ];
        let started = Instant::now();
        let mut buffers = [Vec::new(), Vec::new()];
        let mut lines = [0usize; 2];
        let mut done = 0;
        let mut failure = None;
        let mut status = None;
        // Drain until both readers acknowledge EOF. On root exit kill any
        // surviving descendants first, so inherited pipe handles cannot hang.
        while done < 2 {
            if failure.is_none() {
                if token.is_cancelled() {
                    failure = Some(ProcessError::Cancelled);
                } else if started.elapsed() >= limits.timeout {
                    failure = Some(ProcessError::Timeout);
                }
            }
            if failure.is_some() {
                if status.is_some() {
                    break;
                }
                match self.terminate_and_wait() {
                    Ok(s) => status = Some(s),
                    Err(e) => {
                        failure = Some(e);
                        break;
                    }
                }
            }
            match rx.recv_timeout(Duration::from_millis(10)) {
                Ok(Message::Data(stream, chunk)) => {
                    let index = if stream == OutputStream::Stdout { 0 } else { 1 };
                    let cap = if index == 0 {
                        limits.stdout_bytes
                    } else {
                        limits.stderr_bytes
                    };
                    for b in &chunk {
                        if *b == b'\n' || *b == b'\r' {
                            lines[index] = 0;
                        } else {
                            lines[index] = lines[index].saturating_add(1);
                        }
                        if lines[index] > limits.line_bytes && failure.is_none() {
                            failure = Some(ProcessError::OutputLimit { stream });
                        }
                    }
                    if !streaming
                        && buffers[index].len().saturating_add(chunk.len()) > cap
                        && failure.is_none()
                    {
                        failure = Some(ProcessError::OutputLimit { stream });
                    }
                    if failure.is_none()
                        && std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            callback(stream, &chunk)
                        }))
                        .is_err()
                    {
                        failure = Some(ProcessError::Io(io::Error::other(
                            "output callback panicked",
                        )));
                    }
                    if streaming {
                        if chunk.len() >= cap {
                            buffers[index].clear();
                            buffers[index].extend_from_slice(&chunk[chunk.len() - cap..]);
                        } else {
                            let excess = buffers[index]
                                .len()
                                .saturating_add(chunk.len())
                                .saturating_sub(cap);
                            buffers[index].drain(..excess);
                            buffers[index].extend_from_slice(&chunk);
                        }
                    } else {
                        let take = chunk.len().min(cap.saturating_sub(buffers[index].len()));
                        buffers[index].extend_from_slice(&chunk[..take]);
                    }
                }
                Ok(Message::Done(result)) => {
                    done += 1;
                    if let Err(e) = result {
                        if failure.is_none() {
                            failure = Some(ProcessError::Io(e));
                            break;
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if status.is_none() {
                match self.try_wait() {
                    Ok(Some(s)) => {
                        status = Some(s);
                        if let Err(e) = self.terminate_tree() {
                            failure = Some(ProcessError::Io(e));
                            break;
                        }
                    }
                    Ok(None) => {}
                    Err(e) => {
                        failure = Some(e);
                        break;
                    }
                }
            }
        }
        // Even a child which closes both pipes may keep running.
        while status.is_none() && failure.is_none() {
            if token.is_cancelled() {
                failure = Some(ProcessError::Cancelled);
                break;
            }
            if started.elapsed() >= limits.timeout {
                failure = Some(ProcessError::Timeout);
                break;
            }
            match self.try_wait() {
                Ok(s) => status = s,
                Err(e) => {
                    failure = Some(e);
                    break;
                }
            }
            if status.is_none() {
                thread::sleep(Duration::from_millis(5));
            }
        }
        let cleanup = self.terminate_and_wait();
        // Receiver is dropped before join if cleanup failed so queue producers
        // cannot remain blocked trying to send output.
        stop_readers.store(true, Ordering::Release);
        drop(rx);
        for reader in readers {
            if reader.join().is_err() && failure.is_none() {
                failure = Some(ProcessError::Io(io::Error::other("output reader panicked")));
            }
        }
        let final_status = cleanup?;
        if let Some(e) = failure {
            return Err(e);
        }
        let [stdout, stderr] = buffers;
        if !final_status.success() {
            return Err(ProcessError::Exit {
                code: final_status.code(),
                stderr: String::from_utf8_lossy(&stderr).into_owned(),
            });
        }
        Ok(ProcessOutput {
            status: final_status,
            stdout,
            stderr,
        })
    }
}
impl Drop for OwnedProcess {
    fn drop(&mut self) {
        if self.terminate_tree().is_ok() || self.child.kill().is_ok() {
            let _ = self.child.wait();
        }
    }
}
enum Message {
    Data(OutputStream, Vec<u8>),
    Done(io::Result<()>),
}
#[cfg(windows)]
trait ProcessPipe: Read + std::os::windows::io::AsRawHandle {}
#[cfg(windows)]
impl<T: Read + std::os::windows::io::AsRawHandle> ProcessPipe for T {}
#[cfg(unix)]
trait ProcessPipe: Read + std::os::fd::AsRawFd {}
#[cfg(unix)]
impl<T: Read + std::os::fd::AsRawFd> ProcessPipe for T {}
#[cfg(not(any(windows, unix)))]
trait ProcessPipe: Read {}
#[cfg(not(any(windows, unix)))]
impl<T: Read> ProcessPipe for T {}
fn spawn_reader<R: ProcessPipe + Send + 'static>(
    mut pipe: R,
    stream: OutputStream,
    tx: mpsc::SyncSender<Message>,
    stop: Arc<AtomicBool>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let result = (|| {
            #[cfg(unix)]
            {
                let fd = pipe.as_raw_fd();
                let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
                if flags == -1
                    || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1
                {
                    return Err(io::Error::last_os_error());
                }
            }
            let mut buffer = [0; 8192];
            loop {
                if stop.load(Ordering::Acquire) {
                    return Ok(());
                }
                #[cfg(windows)]
                let read = crate::platform::read_owned_pipe(&mut pipe, &mut buffer, &stop);
                #[cfg(not(windows))]
                let read = pipe.read(&mut buffer);
                match read {
                    Ok(0) => return Ok(()),
                    Ok(n) => {
                        if tx
                            .send(Message::Data(stream, buffer[..n].to_vec()))
                            .is_err()
                        {
                            return Ok(());
                        }
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(e) => return Err(e),
                }
            }
        })();
        let _ = tx.send(Message::Done(result));
    })
}
pub fn run_capture(
    spec: &ProcessSpec,
    limits: ProcessLimits,
    token: &CancellationToken,
) -> Result<ProcessOutput, ProcessError> {
    if token.is_cancelled() {
        return Err(ProcessError::Cancelled);
    }
    OwnedProcess::spawn(spec)?.capture(limits, token)
}

#[cfg(unix)]
fn terminate_group(
    group: &mut Option<libc::pid_t>,
    signal: impl FnOnce(libc::pid_t) -> io::Result<()>,
) -> io::Result<()> {
    if let Some(id) = *group {
        signal(id)?;
        // A successful SIGKILL covers every current member. Never signal this
        // numeric identity again after the leader may have been reaped.
        *group = None;
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod unix_tests {
    use super::*;
    #[test]
    fn retired_group_is_never_signaled_again() {
        let mut group = Some(123);
        let mut calls = 0;
        terminate_group(&mut group, |_| {
            calls += 1;
            Ok(())
        })
        .unwrap();
        for _ in 0..3 {
            terminate_group(&mut group, |_| panic!("retired group signaled")).unwrap();
        }
        assert_eq!(calls, 1);
    }
    #[test]
    fn failed_signal_keeps_reserved_identity() {
        let mut group = Some(123);
        assert!(terminate_group(&mut group, |_| Err(io::Error::other(
            "injected termination failure"
        )))
        .is_err());
        assert_eq!(group, Some(123));
    }
}

#[cfg(test)]
mod cleanup_tests {
    use super::*;
    #[test]
    fn termination_failure_with_inherited_pipe_returns_promptly() {
        let executable = std::env::current_exe().unwrap();
        let fixture = executable
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(format!("process_fixture{}", std::env::consts::EXE_SUFFIX));
        let path = std::env::temp_dir().join(format!(
            "yttraydl-failed-cleanup-{}.txt",
            std::process::id()
        ));
        let mut owner =
            OwnedProcess::spawn(&ProcessSpec::new(fixture).arg("orphan").arg(&path)).unwrap();
        owner.fail_termination_once = true;
        let started = Instant::now();
        let result = owner.capture(ProcessLimits::default(), &CancellationToken::default());
        assert!(
            matches!(result, Err(ProcessError::Io(ref e)) if e.to_string() == "injected termination failure")
        );
        assert!(started.elapsed() < Duration::from_secs(2));
        let length = std::fs::metadata(&path).unwrap().len();
        thread::sleep(Duration::from_millis(75));
        assert_eq!(std::fs::metadata(&path).unwrap().len(), length);
        std::fs::remove_file(path).unwrap();
    }
}
