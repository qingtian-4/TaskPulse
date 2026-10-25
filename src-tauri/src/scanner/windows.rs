use std::collections::HashMap;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::strategies::{CliStrategy, SCRIPT_RUNTIMES};
use super::{known_terminal, ProcInfo, WindowCache};
use crate::watcher::TerminalInfo;
use serde::Deserialize;

type StrategyMatch<'a> = (ProcInfo, &'a dyn CliStrategy);
type ProcessScan<'a> = (Vec<StrategyMatch<'a>>, HashMap<u32, u32>);

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const PROCESS_SNAPSHOT_SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
@(Get-CimInstance -ClassName Win32_Process |
  Select-Object Name,ProcessId,ParentProcessId,CommandLine,UserModeTime,KernelModeTime) |
  ConvertTo-Json -Compress -Depth 2
"#;

const MAX_PROCESS_SNAPSHOT_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ProcessRecord {
    name: Option<String>,
    process_id: Option<u32>,
    parent_process_id: Option<u32>,
    command_line: Option<String>,
    user_mode_time: Option<u64>,
    kernel_mode_time: Option<u64>,
}

static PROCESS_SNAPSHOT: OnceLock<Mutex<HashMap<u32, ProcessRecord>>> = OnceLock::new();
static LAST_SCAN_SUCCEEDED: AtomicBool = AtomicBool::new(true);

pub fn last_scan_succeeded() -> bool {
    LAST_SCAN_SUCCEEDED.load(Ordering::Relaxed)
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
    )?;
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

fn hidden_output(program: &str, args: &[&str], timeout: Duration) -> Option<Output> {
    let mut child = hidden_command(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return child.wait_with_output().ok(),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

/// Find CLI processes matching any registered strategy.
pub fn find_cli_processes(strategies: &[Box<dyn CliStrategy>]) -> ProcessScan<'_> {
    if strategies.is_empty() {
        LAST_SCAN_SUCCEEDED.store(true, Ordering::Relaxed);
        return (Vec::new(), HashMap::new());
    }

    let Some(snapshot) = process_snapshot() else {
        LAST_SCAN_SUCCEEDED.store(false, Ordering::Relaxed);
        return (Vec::new(), HashMap::new());
    };
    LAST_SCAN_SUCCEEDED.store(true, Ordering::Relaxed);
    let cache = PROCESS_SNAPSHOT.get_or_init(|| Mutex::new(HashMap::new()));
    *cache.lock().unwrap_or_else(|error| error.into_inner()) = snapshot.clone();
    let mut out = Vec::new();
    let ppid_map = snapshot
        .iter()
        .filter_map(|(pid, row)| row.parent_process_id.map(|parent| (*pid, parent)))
        .collect::<HashMap<_, _>>();

    for row in snapshot.values() {
        let Some(pid) = row.process_id else { continue };
        let cmd_line = row.command_line.as_deref().unwrap_or("");
        let proc_name = row
            .name
            .as_deref()
            .unwrap_or("")
            .trim()
            .strip_suffix(".exe")
            .unwrap_or(row.name.as_deref().unwrap_or("").trim());
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

        let ppid = row.parent_process_id.unwrap_or(0);

        // CWD is hard to get on Windows without elevated privileges
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("C:\\"));

        // Get CPU times from the same CIM snapshot (100-nanosecond units).
        let utime = row.user_mode_time.unwrap_or(0) / 100_000;
        let stime = row.kernel_mode_time.unwrap_or(0) / 100_000;

        out.push((
            ProcInfo {
                pid,
                ppid,
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

    (out, ppid_map)
}

/// Console-subsystem processes (shells) that may appear in the parent chain
/// before the actual GUI terminal emulator. We keep walking when we hit one
/// of these so that we find the outer GUI host (e.g. Windows Terminal) instead
/// of stopping at PowerShell and getting a zero MainWindowHandle.
fn is_console_shell(label: &str) -> bool {
    matches!(label, "PowerShell" | "CMD")
}

pub fn terminal_info(cache: &mut WindowCache, p: &ProcInfo) -> Option<TerminalInfo> {
    let snapshot = PROCESS_SNAPSHOT
        .get()
        .and_then(|cache| cache.lock().ok().map(|snapshot| snapshot.clone()))?;
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

        let record = snapshot.get(&cur)?;
        let exe_name = record.name.as_deref().unwrap_or("").trim();
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

        cur = record.parent_process_id.unwrap_or(0);
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

pub fn count_children(pid: u32, excluded: &[&str]) -> u32 {
    let Some(cache) = PROCESS_SNAPSHOT.get() else {
        return 0;
    };
    let snapshot = cache.lock().unwrap_or_else(|error| error.into_inner());
    snapshot
        .values()
        .filter(|record| record.parent_process_id == Some(pid))
        .filter(|record| {
            let command_line = record.command_line.as_deref().unwrap_or("");
            !excluded
                .iter()
                .any(|excluded| command_line.contains(excluded))
        })
        .count() as u32
}

fn powershell_window_title(pid: u32) -> Option<String> {
    let cmd = format!("(Get-Process -Id {}).MainWindowTitle", pid);
    let output = hidden_output(
        "powershell",
        &["-NoProfile", "-Command", &cmd],
        Duration::from_secs(1),
    )?;
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
    )?;
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

    #[test]
    fn cim_snapshot_parses_command_lines_and_cpu_times_without_csv_splitting() {
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
    }

    #[test]
    fn empty_or_malformed_cim_output_is_not_a_successful_snapshot() {
        assert!(serde_json::from_slice::<Vec<ProcessRecord>>(b"[]")
            .unwrap()
            .is_empty());
        assert!(serde_json::from_slice::<Vec<ProcessRecord>>(b"not-json").is_err());
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
