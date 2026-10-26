//! Windows-only, owned probe trees. No taskkill race, blocking reader thread,
//! or process outside the job between creation and assignment (Windows 10+).
use std::mem::{size_of, size_of_val};
use std::os::windows::process::ExitStatusExt;
use std::process::Output;
use std::ptr::{null, null_mut};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::Storage::FileSystem::*;
use windows_sys::Win32::System::JobObjects::*;
use windows_sys::Win32::System::Pipes::*;
use windows_sys::Win32::System::Threading::*;

const REAP_BUDGET: Duration = Duration::from_millis(500);

#[derive(Debug, PartialEq, Eq)]
pub(super) enum ProbeError {
    Spawn,
    Read,
    OutputLimit,
    Timeout,
    Kill,
    Wait,
}

struct Handle(HANDLE);
impl Handle {
    fn new(raw: HANDLE) -> Result<Self, ProbeError> {
        if raw.is_null() || raw == INVALID_HANDLE_VALUE {
            Err(ProbeError::Spawn)
        } else {
            Ok(Self(raw))
        }
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: this RAII value uniquely owns a valid kernel handle.
        unsafe { CloseHandle(self.0) };
    }
}

struct Attributes {
    // usize storage supplies pointer alignment required by the attribute list.
    storage: Vec<usize>,
}
impl Attributes {
    fn new() -> Result<Self, ProbeError> {
        let mut bytes = 0;
        // SAFETY: the first call queries the allocation size, as documented.
        unsafe { InitializeProcThreadAttributeList(null_mut(), 2, 0, &mut bytes) };
        if bytes == 0 {
            return Err(ProbeError::Spawn);
        }
        let mut storage = vec![0usize; bytes.div_ceil(size_of::<usize>())];
        // SAFETY: allocated, aligned storage has the requested byte capacity.
        if unsafe {
            InitializeProcThreadAttributeList(storage.as_mut_ptr().cast(), 2, 0, &mut bytes)
        } == 0
        {
            // No initialized attribute list to delete on failure.
            return Err(ProbeError::Spawn);
        }
        Ok(Self { storage })
    }
    fn ptr(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.storage.as_mut_ptr().cast()
    }
}
impl Drop for Attributes {
    fn drop(&mut self) {
        // SAFETY: new() returned only after initializing this allocation.
        unsafe { DeleteProcThreadAttributeList(self.ptr()) };
    }
}

struct Probe {
    job: Handle,
    process: Handle,
    stdout: Handle,
}

fn quote(argument: &str) -> String {
    // Windows CRT argument quoting; no cmd.exe or PowerShell interpolation.
    let mut quoted = String::from("\"");
    let mut slashes = 0;
    for ch in argument.chars() {
        if ch == '\\' {
            slashes += 1;
            continue;
        }
        quoted.extend(std::iter::repeat_n(
            '\\',
            slashes * if ch == '"' { 2 } else { 1 },
        ));
        if ch == '"' {
            quoted.push('\\');
        }
        quoted.push(ch);
        slashes = 0;
    }
    quoted.extend(std::iter::repeat_n('\\', slashes * 2));
    quoted.push('"');
    quoted
}

impl Probe {
    fn spawn(program: &str, args: &[&str]) -> Result<Self, ProbeError> {
        if program.contains('\0') || args.iter().any(|arg| arg.contains('\0')) {
            return Err(ProbeError::Spawn);
        }
        // SAFETY: all output pointers refer to initialized/aligned locals;
        // handles are wrapped immediately, and every error drops owned resources.
        unsafe {
            let job = Handle::new(CreateJobObjectW(null(), null()))?;
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of_val(&limits) as u32,
            ) == 0
            {
                return Err(ProbeError::Spawn);
            }
            let security = SECURITY_ATTRIBUTES {
                nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: null_mut(),
                bInheritHandle: 1,
            };
            let mut read = null_mut();
            let mut write = null_mut();
            if CreatePipe(&mut read, &mut write, &security, 0) == 0 {
                return Err(ProbeError::Spawn);
            }
            let stdout = Handle::new(read)?;
            let writer = Handle::new(write)?;
            if SetHandleInformation(stdout.0, HANDLE_FLAG_INHERIT, 0) == 0 {
                return Err(ProbeError::Spawn);
            }
            let nul: Vec<u16> = "NUL\0".encode_utf16().collect();
            let input = Handle::new(CreateFileW(
                nul.as_ptr(),
                GENERIC_READ,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                &security,
                OPEN_EXISTING,
                0,
                null_mut(),
            ))?;
            let error = Handle::new(CreateFileW(
                nul.as_ptr(),
                GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                &security,
                OPEN_EXISTING,
                0,
                null_mut(),
            ))?;
            let mut attributes = Attributes::new()?;
            let jobs = [job.0];
            let handles = [input.0, writer.0, error.0];
            if UpdateProcThreadAttribute(
                attributes.ptr(),
                0,
                PROC_THREAD_ATTRIBUTE_JOB_LIST as usize,
                jobs.as_ptr().cast(),
                size_of_val(&jobs),
                null_mut(),
                null(),
            ) == 0
                || UpdateProcThreadAttribute(
                    attributes.ptr(),
                    0,
                    PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                    handles.as_ptr().cast(),
                    size_of_val(&handles),
                    null_mut(),
                    null(),
                ) == 0
            {
                return Err(ProbeError::Spawn);
            }
            let mut startup = STARTUPINFOEXW::default();
            startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
            startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
            startup.StartupInfo.hStdInput = input.0;
            startup.StartupInfo.hStdOutput = writer.0;
            startup.StartupInfo.hStdError = error.0;
            startup.lpAttributeList = attributes.ptr();
            let mut command = std::iter::once(program)
                .chain(args.iter().copied())
                .map(quote)
                .collect::<Vec<_>>()
                .join(" ")
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect::<Vec<_>>();
            let mut process = PROCESS_INFORMATION::default();
            // JOB_LIST assigns before the initial thread runs. HANDLE_LIST
            // prevents concurrent probes from inheriting one another's pipes.
            if CreateProcessW(
                null(),
                command.as_mut_ptr(),
                null(),
                null(),
                1,
                CREATE_NO_WINDOW | EXTENDED_STARTUPINFO_PRESENT,
                null(),
                null(),
                &startup.StartupInfo,
                &mut process,
            ) == 0
            {
                return Err(ProbeError::Spawn);
            }
            let process_handle = Handle::new(process.hProcess)?;
            let _thread = Handle::new(process.hThread)?;
            Ok(Self {
                job,
                process: process_handle,
                stdout,
            })
        }
    }

    fn active_processes(&self) -> Result<u32, ProbeError> {
        let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        // SAFETY: correctly sized writable output structure for this class.
        if unsafe {
            QueryInformationJobObject(
                self.job.0,
                JobObjectBasicAccountingInformation,
                (&mut accounting as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                size_of_val(&accounting) as u32,
                null_mut(),
            )
        } == 0
        {
            return Err(ProbeError::Wait);
        }
        Ok(accounting.ActiveProcesses)
    }

    fn reap(&self) -> Result<(), ProbeError> {
        // SAFETY: only this probe's job is terminated; no PID/name based kills.
        let killed = unsafe { TerminateJobObject(self.job.0, 1) } != 0;
        wait_for_empty_job(
            killed,
            || {
                let active = self.active_processes()?;
                // Termination is asynchronous. Do not equate a removed job member
                // with the direct process handle already becoming signalled.
                match unsafe { WaitForSingleObject(self.process.0, 0) } {
                    WAIT_OBJECT_0 => Ok(active),
                    WAIT_TIMEOUT => Ok(active.max(1)),
                    _ => Err(ProbeError::Wait),
                }
            },
            REAP_BUDGET,
        )
    }

    fn collect(&self, timeout: Duration, limit: usize) -> Result<Output, ProbeError> {
        let deadline = Instant::now() + timeout;
        let mut output = Vec::new();
        let mut exit = None;
        loop {
            if Instant::now() >= deadline {
                return Err(ProbeError::Timeout);
            }
            if exit.is_none() {
                // Check exit before reading, then drain the pipe again after
                // observing exit so a final write cannot race an empty peek.
                match unsafe { WaitForSingleObject(self.process.0, 0) } {
                    WAIT_OBJECT_0 => {
                        let mut code = 0;
                        if unsafe { GetExitCodeProcess(self.process.0, &mut code) } == 0
                            || code != 0
                        {
                            return Err(ProbeError::Wait);
                        }
                        exit = Some(code);
                    }
                    WAIT_TIMEOUT => {}
                    _ => return Err(ProbeError::Wait),
                }
            }
            let mut available = 0;
            // This read handle has exactly one reader, is never inherited, and
            // has no outstanding IO. Peek + a read of <= available bytes never
            // waits for an inherited writer to close. No reader threads exist.
            // SAFETY: valid exclusively owned pipe and output pointers.
            let peeked = unsafe {
                PeekNamedPipe(
                    self.stdout.0,
                    null_mut(),
                    0,
                    null_mut(),
                    &mut available,
                    null_mut(),
                )
            };
            if peeked == 0 && unsafe { GetLastError() } != ERROR_BROKEN_PIPE {
                return Err(ProbeError::Read);
            }
            if available > 0 {
                let mut bytes = [0u8; 8192];
                let mut read = 0;
                let count = available.min(bytes.len() as u32);
                // SAFETY: exactly one reader; count is <= available and capacity.
                if unsafe {
                    ReadFile(
                        self.stdout.0,
                        bytes.as_mut_ptr(),
                        count,
                        &mut read,
                        null_mut(),
                    )
                } == 0
                {
                    return Err(ProbeError::Read);
                }
                if output.len().saturating_add(read as usize) > limit {
                    return Err(ProbeError::OutputLimit);
                }
                output.extend_from_slice(&bytes[..read as usize]);
                continue;
            }
            if let Some(code) = exit {
                return Ok(Output {
                    status: std::process::ExitStatus::from_raw(code),
                    stdout: output,
                    stderr: vec![],
                });
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

fn wait_for_empty_job(
    killed: bool,
    mut active: impl FnMut() -> Result<u32, ProbeError>,
    budget: Duration,
) -> Result<(), ProbeError> {
    let deadline = Instant::now() + budget;
    loop {
        if active()? == 0 {
            return Ok(());
        }
        if !killed {
            return Err(ProbeError::Kill);
        }
        if Instant::now() >= deadline {
            return Err(ProbeError::Wait);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

pub(super) fn output(
    program: &str,
    args: &[&str],
    timeout: Duration,
    limit: usize,
) -> Result<Output, ProbeError> {
    let started = Instant::now();
    let probe = Probe::spawn(program, args)?;
    let result = probe.collect(timeout.saturating_sub(started.elapsed()), limit);
    // Reap descendants even when the direct parent exited successfully. A
    // cleanup error overrides partial output; it can never become trusted data.
    // Close of the non-inheritable job handle is the final kill-on-close guard.
    probe.reap()?;
    result
}

#[cfg(test)]
pub(super) struct Fixture(Probe);
#[cfg(test)]
impl Fixture {
    pub(super) fn start(program: &str) -> Self {
        Self(
            Probe::spawn(
                program,
                &[
                    "--ignored",
                    "--exact",
                    "scanner::windows::probe::tests::owned_leaf_fixture",
                ],
            )
            .unwrap(),
        )
    }
}
#[cfg(test)]
impl Drop for Fixture {
    fn drop(&mut self) {
        assert!(
            self.0.reap().is_ok(),
            "fixture process tree did not reach zero"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use windows_sys::Win32::System::Diagnostics::ToolHelp::*;

    const LEAF: &str = "scanner::windows::probe::tests::owned_leaf_fixture";
    const TREE: &str = "scanner::windows::probe::tests::owned_tree_fixture";
    const STRESS: &str = "scanner::windows::probe::tests::owned_resource_worker";

    #[test]
    #[ignore = "isolated child fixture, never an Agent/model invocation"]
    fn owned_leaf_fixture() {
        std::thread::sleep(Duration::from_secs(120));
    }

    #[test]
    #[ignore = "isolated child fixture with an inherited stdout descendant"]
    fn owned_tree_fixture() {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", LEAF, "--nocapture"])
            .stdout(std::process::Stdio::inherit())
            .spawn()
            .unwrap();
        println!("owned-fixture-child={}", child.id());
        std::io::stdout().flush().unwrap();
        // Reap on normal exit as well; the owning probe normally terminates
        // this entire job before either fixture reaches its normal exit.
        child.wait().unwrap();
    }

    fn job_process_handles(probe: &Probe) -> Vec<Handle> {
        let deadline = Instant::now() + Duration::from_secs(5);
        while probe.active_processes().unwrap() < 2 {
            assert!(
                Instant::now() < deadline,
                "fixture descendant did not start"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        // Pointer-aligned storage for a small fixed test-only process list.
        let mut ids = [0usize; 32];
        let list = ids.as_mut_ptr().cast::<JOBOBJECT_BASIC_PROCESS_ID_LIST>();
        unsafe {
            assert_ne!(
                QueryInformationJobObject(
                    probe.job.0,
                    JobObjectBasicProcessIdList,
                    list.cast(),
                    size_of_val(&ids) as u32,
                    null_mut()
                ),
                0
            );
            let count = (*list).NumberOfProcessIdsInList as usize;
            assert!(count >= 2);
            let pids = std::slice::from_raw_parts(
                std::ptr::addr_of!((*list).ProcessIdList).cast::<usize>(),
                count,
            );
            pids.iter()
                .map(|pid| {
                    Handle::new(OpenProcess(
                        PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
                        0,
                        *pid as u32,
                    ))
                    .unwrap()
                })
                .collect()
        }
    }

    fn resources() -> (u32, u32) {
        unsafe {
            let mut handles = 0;
            assert_ne!(GetProcessHandleCount(GetCurrentProcess(), &mut handles), 0);
            let snapshot = Handle::new(CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0)).unwrap();
            let mut entry = THREADENTRY32 {
                dwSize: size_of::<THREADENTRY32>() as u32,
                ..Default::default()
            };
            let mut threads = 0;
            let mut found = Thread32First(snapshot.0, &mut entry);
            while found != 0 {
                if entry.th32OwnerProcessID == GetCurrentProcessId() {
                    threads += 1;
                }
                found = Thread32Next(snapshot.0, &mut entry);
            }
            (handles, threads)
        }
    }

    #[test]
    #[ignore = "called by the bounded parent test in an isolated test process"]
    fn owned_resource_worker() {
        let executable = std::env::current_exe().unwrap();
        let program = executable.to_str().unwrap();
        // Warm up thread/runtime bookkeeping before comparing exact resources.
        let warm = Probe::spawn(program, &["--ignored", "--exact", LEAF]).unwrap();
        warm.reap().unwrap();
        drop(warm);
        let baseline = resources();
        let mut durations = vec![];
        for iteration in 0..50 {
            let start = Instant::now();
            let mut probe =
                Probe::spawn(program, &["--ignored", "--exact", TREE, "--nocapture"]).unwrap();
            let processes = job_process_handles(&probe);
            if iteration % 2 == 0 {
                // Real OS read failure: an event is a valid handle but no pipe.
                probe.stdout = unsafe { Handle::new(CreateEventW(null(), 0, 0, null())).unwrap() };
                assert_eq!(
                    probe.collect(Duration::from_secs(1), 65536),
                    Err(ProbeError::Read)
                );
            } else {
                assert_eq!(
                    probe.collect(Duration::from_millis(10), 65536),
                    Err(ProbeError::Timeout)
                );
            }
            probe.reap().unwrap();
            assert_eq!(probe.active_processes().unwrap(), 0);
            for process in processes {
                assert_eq!(
                    unsafe { WaitForSingleObject(process.0, 500) },
                    WAIT_OBJECT_0,
                    "owned descendant survived bounded exit verification"
                );
            }
            drop(probe);
            durations.push(start.elapsed());
            // No reads run on auxiliary threads; there is nothing to detach or
            // join. Every iteration verifies the whole tree reached zero.
            assert_eq!(
                resources(),
                baseline,
                "resources grew at iteration {iteration}"
            );
        }
        durations.sort();
        println!("50 owned trees: alive=0, reader_threads=0, handles/threads={baseline:?}->{:?}, p50={:?}, p95={:?}, max={:?}",
            resources(), durations[25], durations[47], durations[49]);
        let recovered = output(
            "powershell",
            &["-NoProfile", "-Command", "Write-Output recovered"],
            Duration::from_secs(5),
            65536,
        )
        .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&recovered.stdout).trim(),
            "recovered"
        );
    }

    #[test]
    fn fifty_abnormal_trees_leave_no_owned_processes_threads_or_handles() {
        let executable = std::env::current_exe().unwrap();
        let result = output(
            executable.to_str().unwrap(),
            &[
                "--ignored",
                "--exact",
                STRESS,
                "--nocapture",
                "--test-threads=1",
            ],
            Duration::from_secs(90),
            65536,
        )
        .unwrap();
        println!("{}", String::from_utf8_lossy(&result.stdout));
    }

    #[test]
    fn cleanup_faults_are_bounded_and_never_report_success() {
        assert_eq!(
            wait_for_empty_job(false, || Ok(1), REAP_BUDGET),
            Err(ProbeError::Kill)
        );
        assert_eq!(
            wait_for_empty_job(true, || Err(ProbeError::Wait), REAP_BUDGET),
            Err(ProbeError::Wait)
        );
        let start = Instant::now();
        assert_eq!(
            wait_for_empty_job(true, || Ok(1), Duration::from_millis(10)),
            Err(ProbeError::Wait)
        );
        assert!(start.elapsed() < Duration::from_secs(1));
    }
}
