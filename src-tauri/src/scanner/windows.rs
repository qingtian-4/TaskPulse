use std::collections::HashMap;
use std::io::Read;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ExitStatus, Output, Stdio};
use std::time::{Duration, Instant};

use super::strategies::{CliStrategy, SCRIPT_RUNTIMES};
use super::{known_terminal, PlatformScan, ProcInfo, ProcessEntry, ProcessSnapshot, WindowCache};
use crate::watcher::TerminalInfo;
use serde::Deserialize;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const PROCESS_SNAPSHOT_SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
@(Get-CimInstance -ClassName Win32_Process |
  Select-Object Name,ProcessId,ParentProcessId,CommandLine,UserModeTime,KernelModeTime,CreationDate) |
  ConvertTo-Json -Compress -Depth 2
"#;

const MAX_PROCESS_SNAPSHOT_BYTES: usize = 8 * 1024 * 1024;
const MAX_AUXILIARY_OUTPUT_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ProcessRecord {
    name: Option<String>,
    process_id: Option<u32>,
    parent_process_id: Option<u32>,
    command_line: Option<String>,
    user_mode_time: Option<u64>,
    kernel_mode_time: Option<u64>,
    creation_date: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
enum ProbeError {
    Spawn,
    Read,
    OutputLimit,
    Timeout,
    Kill,
    Wait,
}

fn process_snapshot() -> Option<HashMap<u32, ProcessRecord>> {
    let output = hidden_output(
        "powershell",
        &[
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            PROCESS_SNAPSHOT_SCRIPT,
        ],
        Duration::from_secs(2),
        MAX_PROCESS_SNAPSHOT_BYTES,
    )
    .ok()?;
    if !output.status.success() || output.stdout.len() > MAX_PROCESS_SNAPSHOT_BYTES {
        return None;
    }
    let rows: Vec<ProcessRecord> = serde_json::from_slice(&output.stdout).ok()?;
    let snapshot = rows
        .into_iter()
        .filter_map(|row| row.process_id.map(|pid| (pid, row)))
        .collect::<HashMap<_, _>>();
    if snapshot.is_empty() {
        return None;
    }
    Some(snapshot)
}

fn hidden_command(program: &str) -> std::process::Command {
    let mut command = std::process::Command::new(program);
    // Scanner probes run in the background. A console child would flash over
    // the tray popup on every polling cycle.
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

fn read_bounded<R: Read>(mut reader: R, max_output_bytes: usize) -> Result<Vec<u8>, ProbeError> {
    let mut output = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => return Ok(output),
            Ok(read) => {
                if output.len().saturating_add(read) > max_output_bytes {
                    return Err(ProbeError::OutputLimit);
                }
                output.extend_from_slice(&chunk[..read]);
            }
            Err(_) => return Err(ProbeError::Read),
        }
    }
}

fn terminate_process_tree(child: &mut Child, wait_budget: Duration) -> Result<(), ProbeError> {
    let pid = child.id().to_string();
    let direct_kill = child.kill().is_ok();
    let mut tree_kill = hidden_command("taskkill")
        .args(["/PID", &pid, "/T", "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok();
    let tree_deadline = Instant::now() + wait_budget.min(Duration::from_millis(500));
    if let Some(tree_kill) = tree_kill.as_mut() {
        loop {
            match tree_kill.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if Instant::now() < tree_deadline => {
                    std::thread::sleep(Duration::from_millis(5))
                }
                Ok(None) => {
                    let _ = tree_kill.kill();
                    let _ = tree_kill.wait();
                    break;
                }
                Err(_) => break,
            }
        }
    }
    let deadline = Instant::now() + wait_budget;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return Ok(()),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            Ok(None) => return Err(ProbeError::Wait),
            Err(_) if direct_kill => return Err(ProbeError::Wait),
            Err(_) => return Err(ProbeError::Kill),
        }
    }
}

fn finish_reader(reader: std::thread::JoinHandle<()>, reap: Result<(), ProbeError>) {
    if reap.is_ok() {
        let _ = reader.join();
    } else {
        // A failed kill/wait must still return within the probe budget. The
        // taskkill tree path normally closes stdout; detach only when the OS
        // refused reaping so the caller cannot block indefinitely on Read.
        std::mem::forget(reader);
    }
}

fn hidden_output(
    program: &str,
    args: &[&str],
    timeout: Duration,
    max_output_bytes: usize,
) -> Result<Output, ProbeError> {
    let mut child = hidden_command(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| ProbeError::Spawn)?;
    let stdout = child.stdout.take().ok_or(ProbeError::Read)?;
    let (output_tx, output_rx) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        let _ = output_tx.send(read_bounded(stdout, max_output_bytes));
    });
    let deadline = Instant::now() + timeout;
    let mut status: Option<ExitStatus> = None;
    let mut stdout_result = None;
    loop {
        if status.is_none() {
            match child.try_wait() {
                Ok(Some(exit)) => status = Some(exit),
                Ok(None) => {}
                Err(_) => {
                    let reap = terminate_process_tree(&mut child, Duration::from_millis(500));
                    finish_reader(reader, reap);
                    return Err(ProbeError::Wait);
                }
            }
        }
        if stdout_result.is_none() {
            match output_rx.try_recv() {
                Ok(result) => stdout_result = Some(result),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    let reap = terminate_process_tree(&mut child, Duration::from_millis(500));
                    finish_reader(reader, reap);
                    return Err(ProbeError::Read);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        if let Some(Err(error)) = stdout_result {
            let reap = terminate_process_tree(&mut child, Duration::from_millis(500));
            finish_reader(reader, reap);
            return Err(error);
        }
        if let Some(Ok(stdout)) = stdout_result.take() {
            if let Some(status) = status.take() {
                let _ = reader.join();
                return if status.success() {
                    Ok(Output {
                        status,
                        stdout,
                        stderr: Vec::new(),
                    })
                } else {
                    Err(ProbeError::Wait)
                };
            }
            stdout_result = Some(Ok(stdout));
        }
        if Instant::now() >= deadline {
            let reap = terminate_process_tree(&mut child, Duration::from_millis(500));
            finish_reader(
                reader,
                reap.as_ref().map(|_| ()).map_err(|_| ProbeError::Wait),
            );
            return Err(reap.err().unwrap_or(ProbeError::Timeout));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Find CLI processes matching any registered strategy.
pub fn find_cli_processes(strategies: &[Box<dyn CliStrategy>]) -> PlatformScan<'_> {
    let Some(snapshot) = process_snapshot() else {
        return PlatformScan {
            matched: Vec::new(),
            snapshot: ProcessSnapshot {
                collected_at: std::time::Instant::now(),
                ppid_map: HashMap::new(),
                processes: HashMap::new(),
                succeeded: false,
            },
        };
    };
    let processes = snapshot
        .into_iter()
        .map(|(pid, row)| {
            let ppid = row.parent_process_id.unwrap_or(0);
            (
                pid,
                ProcessEntry {
                    pid,
                    ppid,
                    name: row.name.unwrap_or_default(),
                    command_line: row.command_line.unwrap_or_default(),
                    process_instance: row.creation_date,
                    user_mode_time: row.user_mode_time.unwrap_or(0),
                    kernel_mode_time: row.kernel_mode_time.unwrap_or(0),
                },
            )
        })
        .collect::<HashMap<_, _>>();
    let ppid_map = processes
        .iter()
        .map(|(pid, row)| (*pid, row.ppid))
        .collect::<HashMap<_, _>>();
    let snapshot = ProcessSnapshot {
        collected_at: std::time::Instant::now(),
        ppid_map,
        processes,
        succeeded: true,
    };
    let mut out = Vec::new();

    for row in snapshot.processes.values() {
        let pid = row.pid;
        let cmd_line = row.command_line.as_str();
        let proc_name = row
            .name
            .as_str()
            .trim()
            .strip_suffix(".exe")
            .unwrap_or(row.name.trim());
        if proc_name.is_empty() {
            continue;
        }

        // Phase 1: Direct process name match (native binaries)
        let strategy = strategies.iter().find(|s| {
            s.process_names()
                .iter()
                .any(|name| name.eq_ignore_ascii_case(proc_name))
        });

        // Phase 2: If process is a script runtime, check CommandLine for script names
        let strategy = match strategy {
            Some(s) => s.as_ref(),
            None => {
                if !SCRIPT_RUNTIMES.contains(&proc_name) {
                    continue;
                }
                match cmd_line.split_whitespace().skip(1).find_map(|arg| {
                    if arg.starts_with('-') {
                        return None;
                    }
                    let arg_name = Path::new(arg)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("");
                    strategies
                        .iter()
                        .find(|s| s.script_names().contains(&arg_name))
                }) {
                    Some(s) => s.as_ref(),
                    None => continue,
                }
            }
        };

        if strategy
            .excluded_substrings()
            .iter()
            .any(|exc| cmd_line.contains(exc))
        {
            continue;
        }

        let ppid = row.ppid;

        // CWD is hard to get on Windows without elevated privileges
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("C:\\"));

        // Get CPU times from the same CIM snapshot (100-nanosecond units).
        let utime = row.user_mode_time / 100_000;
        let stime = row.kernel_mode_time / 100_000;

        out.push((
            ProcInfo {
                pid,
                ppid,
                process_instance: row.process_instance.clone(),
                cwd,
                tty_label: String::new(),
                utime,
                stime,
                instant_cpu: None,
                window_title: None,
                last_active: None,
            },
            strategy,
        ));
    }

    PlatformScan {
        matched: out,
        snapshot,
    }
}

/// Console-subsystem processes (shells) that may appear in the parent chain
/// before the actual GUI terminal emulator. We keep walking when we hit one
/// of these so that we find the outer GUI host (e.g. Windows Terminal) instead
/// of stopping at PowerShell and getting a zero MainWindowHandle.
fn is_console_shell(label: &str) -> bool {
    matches!(label, "PowerShell" | "CMD")
}

pub fn terminal_info(
    cache: &mut WindowCache,
    p: &ProcInfo,
    snapshot: &ProcessSnapshot,
) -> Option<TerminalInfo> {
    let mut cur = p.ppid;
    let mut term_label = String::new();
    let mut term_pid: u32 = 0;

    // Walk parent process chain to find the terminal emulator.
    // Continue past console-only shells (PowerShell, cmd) to find the outer
    // GUI host (e.g. Windows Terminal) which has a valid MainWindowHandle.
    for _ in 0..6 {
        if cur <= 1 {
            break;
        }

        let record = snapshot.processes.get(&cur)?;
        let exe_name = record.name.trim();
        if let Some(label) = known_terminal(exe_name) {
            if is_console_shell(label) {
                // Keep this as a fallback but continue walking for a GUI host
                if term_pid == 0 {
                    term_label = label.to_string();
                    term_pid = cur;
                }
            } else {
                // GUI terminal emulator — prefer this over any previous shell match
                term_label = label.to_string();
                term_pid = cur;
                break;
            }
        }

        cur = record.ppid;
    }

    if term_pid == 0 {
        return None;
    }

    let window_title = cache
        .entry(term_pid)
        .or_insert_with(|| powershell_window_title(term_pid))
        .clone();

    Some(TerminalInfo {
        kind: "windows_native".to_string(),
        focus_id: term_pid.to_string(),
        outer_id: String::new(),
        label: term_label,
        window_title,
    })
}

pub fn count_children(pid: u32, excluded: &[&str], snapshot: &ProcessSnapshot) -> u32 {
    snapshot
        .processes
        .values()
        .filter(|record| record.ppid == pid)
        .filter(|record| {
            !excluded
                .iter()
                .any(|excluded| record.command_line.contains(excluded))
        })
        .count() as u32
}

fn powershell_window_title(pid: u32) -> Option<String> {
    let cmd = format!("(Get-Process -Id {}).MainWindowTitle", pid);
    let output = hidden_output(
        "powershell",
        &["-NoProfile", "-Command", &cmd],
        Duration::from_secs(1),
        MAX_AUXILIARY_OUTPUT_BYTES,
    )
    .ok()?;
    let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

/// Read a single environment variable from a running process on Windows.
///
/// Uses PowerShell `Add-Type` to PInvoke the Windows API:
/// `OpenProcess` → `NtQueryInformationProcess` (PEB address) →
/// `ReadProcessMemory` (ProcessParameters → Environment block) → parse.
///
/// Works for same-user processes. Requires PROCESS_QUERY_INFORMATION |
/// PROCESS_VM_READ access (both granted by default for same-user processes).
/// The result is cached in Scanner.session_id_cache, so PowerShell is only
/// spawned once per PID across scan cycles.
pub fn read_proc_env(pid: u32, key: &str) -> Option<String> {
    // Guard against injection — env var names are always safe ASCII identifiers.
    if !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return None;
    }
    let script = PS_READ_ENV
        .replace("__PID__", &pid.to_string())
        .replace("__KEY__", key);
    let output = hidden_output(
        "powershell",
        &["-NoProfile", "-NonInteractive", "-Command", &script],
        Duration::from_secs(2),
        MAX_AUXILIARY_OUTPUT_BYTES,
    )
    .ok()?;
    let val = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if val.is_empty() {
        None
    } else {
        Some(val)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn cim_snapshot_parses_single_and_multi_process_json_without_csv_splitting() {
        let json = r#"[{"Name":"codex.exe","ProcessId":42,"ParentProcessId":7,"CommandLine":"codex --project C:\\work,中文","UserModeTime":1200000,"KernelModeTime":3400000}]"#
            .as_bytes();
        let rows: Vec<ProcessRecord> = serde_json::from_slice(json).unwrap();
        assert_eq!(rows[0].name.as_deref(), Some("codex.exe"));
        assert_eq!(rows[0].process_id, Some(42));
        assert_eq!(rows[0].parent_process_id, Some(7));
        assert_eq!(
            rows[0].command_line.as_deref(),
            Some("codex --project C:\\work,中文")
        );
        assert_eq!(rows[0].user_mode_time, Some(1_200_000));
        assert_eq!(rows[0].kernel_mode_time, Some(3_400_000));
        let multi = br#"[{"Name":"one.exe","ProcessId":1},{"Name":"two.exe","ProcessId":2}]"#;
        let rows: Vec<ProcessRecord> = serde_json::from_slice(multi).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].process_id, Some(2));
    }

    #[test]
    fn empty_json_is_distinct_from_malformed_cim_output() {
        let empty = serde_json::from_slice::<Vec<ProcessRecord>>(b"[]").unwrap();
        assert!(empty.is_empty());
        assert!(serde_json::from_slice::<Vec<ProcessRecord>>(b"not-json").is_err());
    }

    #[test]
    fn bounded_probe_reader_rejects_output_and_io_failures() {
        assert_eq!(
            read_bounded(Cursor::new(vec![b'x'; 9]), 8),
            Err(ProbeError::OutputLimit)
        );
        struct FailingReader;
        impl Read for FailingReader {
            fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("injected reader failure"))
            }
        }
        assert_eq!(read_bounded(FailingReader, 8), Err(ProbeError::Read));
    }

    #[test]
    fn powershell_timeout_has_a_bounded_reap() {
        let started = Instant::now();
        let result = hidden_output(
            "powershell",
            &[
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Start-Sleep -Seconds 10",
            ],
            Duration::from_millis(200),
            MAX_PROCESS_SNAPSHOT_BYTES,
        );
        assert!(matches!(
            result,
            Err(ProbeError::Timeout | ProbeError::Wait)
        ));
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn oversized_snapshot_is_rejected_before_process_exit() {
        let result = hidden_output(
            "powershell",
            &[
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "$b=New-Object byte[] 9000000; [Console]::OpenStandardOutput().Write($b,0,$b.Length)",
            ],
            Duration::from_secs(5),
            MAX_PROCESS_SNAPSHOT_BYTES,
        );
        assert_eq!(result, Err(ProbeError::OutputLimit));
    }

    #[test]
    fn descendant_stdout_handle_is_reaped_with_the_parent_tree() {
        let started = Instant::now();
        let result = hidden_output(
            "powershell",
            &[
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Start-Process powershell -ArgumentList '-NoProfile','-Command','Start-Sleep -Seconds 10' -NoNewWindow; Start-Sleep -Seconds 10",
            ],
            Duration::from_millis(200),
            MAX_PROCESS_SNAPSHOT_BYTES,
        );
        assert!(matches!(
            result,
            Err(ProbeError::Timeout | ProbeError::Wait)
        ));
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn repeated_probe_failures_recover_without_poisoning_the_next_sample() {
        for _ in 0..3 {
            let result = hidden_output(
                "powershell",
                &[
                    "-NoLogo",
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    "Start-Sleep -Seconds 10",
                ],
                Duration::from_millis(100),
                MAX_PROCESS_SNAPSHOT_BYTES,
            );
            assert!(matches!(
                result,
                Err(ProbeError::Timeout | ProbeError::Wait)
            ));
        }
        let recovered = hidden_output(
            "powershell",
            &[
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Write-Output recovered",
            ],
            Duration::from_secs(2),
            MAX_PROCESS_SNAPSHOT_BYTES,
        )
        .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&recovered.stdout).trim(),
            "recovered"
        );
    }

    #[test]
    fn repeated_timeout_probes_have_a_bounded_resource_budget() {
        let started = Instant::now();
        for _ in 0..50 {
            let result = hidden_output(
                "powershell",
                &[
                    "-NoLogo",
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    "Start-Sleep -Seconds 10",
                ],
                Duration::from_millis(25),
                MAX_PROCESS_SNAPSHOT_BYTES,
            );
            assert!(matches!(
                result,
                Err(ProbeError::Timeout | ProbeError::Wait)
            ));
        }
        assert!(started.elapsed() < Duration::from_secs(30));
        let recovered = hidden_output(
            "powershell",
            &[
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Write-Output recovered",
            ],
            Duration::from_secs(2),
            MAX_PROCESS_SNAPSHOT_BYTES,
        )
        .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&recovered.stdout).trim(),
            "recovered"
        );
    }

    #[test]
    fn process_batches_keep_terminal_trees_and_failure_state_isolated() {
        let entry = |pid: u32, ppid: u32, name: &str| ProcessEntry {
            pid,
            ppid,
            name: name.into(),
            command_line: String::new(),
            process_instance: None,
            user_mode_time: 0,
            kernel_mode_time: 0,
        };
        let batch_a = ProcessSnapshot {
            ppid_map: [(10, 1), (20, 10)].into(),
            processes: [(10, entry(10, 1, "WindowsTerminal.exe"))].into(),
            succeeded: true,
        };
        let batch_b = ProcessSnapshot {
            ppid_map: [(10, 1), (20, 10)].into(),
            processes: [(10, entry(10, 1, "other.exe"))].into(),
            succeeded: false,
        };
        let process = ProcInfo {
            pid: 20,
            ppid: 10,
            process_instance: None,
            cwd: PathBuf::from(r"C:\project"),
            tty_label: String::new(),
            utime: 0,
            stime: 0,
            instant_cpu: None,
            window_title: None,
            last_active: None,
        };
        assert!(batch_a.succeeded);
        assert!(!batch_b.succeeded);
        let mut cache = HashMap::new();
        assert_eq!(
            terminal_info(&mut cache, &process, &batch_a).unwrap().label,
            "Windows Terminal"
        );
        assert!(terminal_info(&mut cache, &process, &batch_b).is_none());
    }
}

/// PowerShell script that reads a process environment variable via Windows APIs.
/// __PID__ and __KEY__ are substituted at runtime.
const PS_READ_ENV: &str = r#"
$p = __PID__; $k = '__KEY__'
if (-not ([System.Management.Automation.PSTypeName]'AgentTrayEnv').Type) {
    Add-Type -TypeDefinition @'
using System; using System.Runtime.InteropServices; using System.Text;
public class AgentTrayEnv {
    const int PROCESS_QUERY_INFORMATION = 0x0400, PROCESS_VM_READ = 0x0010;
    [DllImport("kernel32")] static extern IntPtr OpenProcess(int a, bool b, int c);
    [DllImport("kernel32")] static extern bool ReadProcessMemory(IntPtr h, IntPtr addr, byte[] buf, int sz, out int n);
    [DllImport("kernel32")] static extern bool CloseHandle(IntPtr h);
    [DllImport("ntdll")] static extern int NtQueryInformationProcess(IntPtr h, int cls, ref PBI pbi, int sz, out int ret);
    [StructLayout(LayoutKind.Sequential)] struct PBI {
        IntPtr r1; public IntPtr PebBase; IntPtr r2, r3; UIntPtr r4; IntPtr r5;
    }
    static IntPtr ReadPtr(IntPtr h, IntPtr addr) {
        var b = new byte[IntPtr.Size]; int n;
        ReadProcessMemory(h, addr, b, b.Length, out n);
        return IntPtr.Size == 8
            ? new IntPtr(BitConverter.ToInt64(b, 0))
            : new IntPtr(BitConverter.ToInt32(b, 0));
    }
    public static string Get(int pid, string key) {
        var h = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, false, pid);
        if (h == IntPtr.Zero) return null;
        try {
            var pbi = new PBI(); int ret;
            NtQueryInformationProcess(h, 0, ref pbi, Marshal.SizeOf(typeof(PBI)), out ret);
            bool x64 = IntPtr.Size == 8;
            var pp = ReadPtr(h, IntPtr.Add(pbi.PebBase, x64 ? 0x20 : 0x10));
            var ep = ReadPtr(h, IntPtr.Add(pp, x64 ? 0x80 : 0x48));
            var buf = new byte[65536]; int read;
            ReadProcessMemory(h, ep, buf, buf.Length, out read);
            var env = Encoding.Unicode.GetString(buf, 0, Math.Max(0, read - 2));
            foreach (var pair in env.Split('\0'))
                if (pair.StartsWith(key + "=", StringComparison.OrdinalIgnoreCase))
                    return pair.Substring(key.Length + 1);
            return null;
        } finally { CloseHandle(h); }
    }
}
'@ -ErrorAction SilentlyContinue
}
$r = [AgentTrayEnv]::Get($p, $k)
if ($r) { Write-Output $r }
"#;
