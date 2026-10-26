use std::io::Read;
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

// No reader threads; the owned process group and nonblocking pipe are always
// reclaimed within the operation deadline plus a fixed reap budget.
pub(crate) fn output(
    program: &str,
    args: &[&str],
    timeout: Duration,
    limit: usize,
) -> std::io::Result<Output> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()?;
    let result = (|| {
        let mut pipe = child
            .stdout
            .take()
            .ok_or_else(|| std::io::Error::other("missing pipe"))?;
        let fd = pipe.as_raw_fd();
        // SAFETY: live owned pipe; preserve existing descriptor status flags.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let deadline = Instant::now() + timeout;
        let mut body = Vec::new();
        let mut exited = false;
        loop {
            if Instant::now() >= deadline {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "probe deadline",
                ));
            }
            let mut bytes = [0; 8192];
            match pipe.read(&mut bytes) {
                Ok(0) if exited => return Ok(body),
                Ok(n) => {
                    if body.len() + n > limit {
                        return Err(std::io::Error::other("probe output limit"));
                    }
                    body.extend_from_slice(&bytes[..n]);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => (),
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
            if !exited {
                let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
                // WNOWAIT keeps the child PID reserved until group cleanup.
                if unsafe {
                    libc::waitid(
                        libc::P_PID,
                        child.id(),
                        &mut info,
                        libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                    )
                } < 0
                {
                    return Err(std::io::Error::last_os_error());
                }
                #[cfg(target_os = "linux")]
                {
                    exited = unsafe { info.si_pid() } != 0;
                }
                #[cfg(target_os = "macos")]
                {
                    exited = info.si_pid != 0;
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    })();
    // SAFETY: this PID was placed in a new process group at spawn. Do not reap
    // the root until group termination has been requested (no PID reuse).
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let deadline = Instant::now() + Duration::from_millis(500);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            return Err(std::io::Error::other("probe reap deadline"));
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    result.map(|stdout| Output {
        status,
        stdout,
        stderr: vec![],
    })
}
