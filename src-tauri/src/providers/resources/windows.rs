//! Bounded, on-demand counters for the TaskPulse process and its live children.
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use windows_sys::Win32::{
    Foundation::{
        GetLastError, ERROR_NO_MORE_FILES, FILETIME, HANDLE, INVALID_HANDLE_VALUE, WAIT_TIMEOUT,
    },
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
            TH32CS_SNAPPROCESS,
        },
        Threading::{
            GetCurrentProcess, GetProcessTimes, OpenProcess, WaitForSingleObject,
            PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
        },
    },
};

const MAX_SNAPSHOT_ROWS: usize = 32_768;
const MAX_OWNED_PROCESSES: usize = 256;

#[repr(C)]
#[derive(Default)]
struct MemoryCounters {
    cb: u32,
    faults: u32,
    peak_working: usize,
    working: usize,
    peak_paged: usize,
    paged: usize,
    peak_nonpaged: usize,
    nonpaged: usize,
    pagefile: usize,
    peak_pagefile: usize,
    private: usize,
}

#[link(name = "psapi")]
extern "system" {
    fn GetProcessMemoryInfo(process: HANDLE, counters: *mut MemoryCounters, size: u32) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetSystemTimePreciseAsFileTime(now: *mut FILETIME);
}

fn ticks(value: FILETIME) -> u64 {
    (u64::from(value.dwHighDateTime) << 32) | u64::from(value.dwLowDateTime)
}

fn memory(handle: HANDLE) -> Option<MemoryCounters> {
    let cb = std::mem::size_of::<MemoryCounters>() as u32;
    let mut counters = MemoryCounters {
        cb,
        ..Default::default()
    };
    // SAFETY: handle is live and counters is an initialized EX-sized buffer.
    (unsafe { GetProcessMemoryInfo(handle, &mut counters, cb) } != 0).then_some(counters)
}

fn times(handle: HANDLE) -> Option<(u64, u64)> {
    let [mut creation, mut exit, mut kernel, mut user] = [FILETIME::default(); 4];
    // SAFETY: a live process handle and four writable FILETIME outputs.
    if unsafe { GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) } == 0 {
        return None;
    }
    Some((
        ticks(creation),
        ticks(kernel).saturating_add(ticks(user)) / 10_000,
    ))
}

#[derive(Clone, Copy)]
struct Child {
    pid: u32,
    webview: bool,
}

fn children() -> Option<(HashMap<u32, Vec<Child>>, u64)> {
    let mut started = FILETIME::default();
    // SAFETY: initialized output. The timestamp limits PID reuse races.
    unsafe { GetSystemTimePreciseAsFileTime(&mut started) };
    // SAFETY: process snapshot owns a newly returned kernel handle.
    let raw = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if raw == INVALID_HANDLE_VALUE {
        return None;
    }
    let snapshot = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut row = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    // SAFETY: snapshot is valid and row is initialized with the documented size.
    if unsafe { Process32FirstW(snapshot.as_raw_handle(), &mut row) } == 0 {
        return None;
    }
    let mut children = HashMap::<u32, Vec<Child>>::new();
    for _ in 0..MAX_SNAPSHOT_ROWS {
        let len = row
            .szExeFile
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(row.szExeFile.len());
        let webview = String::from_utf16_lossy(&row.szExeFile[..len])
            .eq_ignore_ascii_case("msedgewebview2.exe");
        children
            .entry(row.th32ParentProcessID)
            .or_default()
            .push(Child {
                pid: row.th32ProcessID,
                webview,
            });
        // SAFETY: same owned snapshot and initialized output as above.
        if unsafe { Process32NextW(snapshot.as_raw_handle(), &mut row) } == 0 {
            return (unsafe { GetLastError() } == ERROR_NO_MORE_FILES)
                .then_some((children, ticks(started)));
        }
    }
    None
}

#[derive(Default, Serialize)]
struct TreeSample {
    complete: bool,
    includes_webview_children: bool,
    processes: usize,
    webview_processes: usize,
    working_set_bytes: usize,
    private_bytes: usize,
    webview_working_set_bytes: usize,
    cpu_lifetime_ms: u64,
}

impl TreeSample {
    fn add(&mut self, memory: &MemoryCounters, cpu: u64, webview: bool) {
        self.processes += 1;
        self.working_set_bytes = self.working_set_bytes.saturating_add(memory.working);
        self.private_bytes = self.private_bytes.saturating_add(memory.private);
        self.cpu_lifetime_ms = self.cpu_lifetime_ms.saturating_add(cpu);
        if webview {
            self.includes_webview_children = true;
            self.webview_processes += 1;
            self.webview_working_set_bytes = self
                .webview_working_set_bytes
                .saturating_add(memory.working);
        }
    }
}

fn tree_sample(root: &MemoryCounters, root_times: (u64, u64)) -> Option<TreeSample> {
    let (mut children, started) = children()?;
    let root_pid = std::process::id();
    let mut queue = vec![(root_pid, root_times.0)];
    let mut seen = HashSet::from([root_pid]);
    let mut sample = TreeSample {
        complete: true,
        ..Default::default()
    };
    sample.add(root, root_times.1, false);
    while let Some((parent, parent_birth)) = queue.pop() {
        for child in children.remove(&parent).into_iter().flatten() {
            if seen.len() == MAX_OWNED_PROCESSES {
                sample.complete = false;
                return Some(sample);
            }
            if !seen.insert(child.pid) {
                continue;
            }
            // SAFETY: read-only process rights, non-inheritable handle.
            let raw = unsafe {
                OpenProcess(
                    PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                    0,
                    child.pid,
                )
            };
            if raw.is_null() {
                sample.complete = false;
                continue;
            }
            let process = unsafe { OwnedHandle::from_raw_handle(raw) };
            let handle = process.as_raw_handle();
            let Some((birth, cpu)) = times(handle) else {
                sample.complete = false;
                continue;
            };
            // The parent must have existed when this child was created, and
            // the child must predate the snapshot from which its PID came.
            if birth < parent_birth
                || birth > started
                || unsafe { WaitForSingleObject(handle, 0) } != WAIT_TIMEOUT
            {
                sample.complete = false;
                continue;
            }
            queue.push((child.pid, birth));
            if let Some(memory) = memory(handle) {
                sample.add(&memory, cpu, child.webview);
            } else {
                sample.complete = false;
            }
        }
    }
    Some(sample)
}

pub(super) fn process_memory() -> Value {
    // SAFETY: borrowed pseudo-handle for this process, never closed.
    let handle = unsafe { GetCurrentProcess() };
    let Some(root) = memory(handle) else {
        return json!({"available": false});
    };
    let process_tree = times(handle).and_then(|times| tree_sample(&root, times));
    // Keep the original root-only fields for existing consumers.
    json!({
        "available": true,
        "private_bytes": root.private,
        "working_set_bytes": root.working,
        "peak_working_set_bytes": root.peak_working,
        "includes_webview_children": false,
        "process_tree": process_tree,
    })
}
