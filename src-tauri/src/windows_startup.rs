//! Windows login registration. Only an explicit enable creates an entry, in HKCU.
//! Existing owned entries retain their scope; quoting repair never enables them.
use std::path::Path;
use windows_registry::{Key, Value, CURRENT_USER, LOCAL_MACHINE};

const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
const ENABLED: [u8; 12] = [2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

fn optional<T>(value: windows_registry::Result<T>) -> Result<Option<T>, String> {
    match value {
        Ok(value) => Ok(Some(value)),
        Err(e) if e.code().0 as u32 == 0x80070002 => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

pub(super) fn command(exe: &Path) -> Result<String, String> {
    let path = exe.to_str().ok_or("Invalid startup executable path")?;
    if !exe.is_absolute() || path.chars().any(|c| c == '"' || c.is_control()) {
        return Err("Invalid startup executable path".into());
    }
    let quoted = format!("\"{path}\"");
    if quoted.encode_utf16().count() > 260 {
        return Err("Startup executable path exceeds the Windows Run limit".into());
    }
    Ok(quoted)
}

struct Registration<'a> {
    user: &'a Key,
    machine: &'a Key,
    name: &'a str,
    quoted: String,
}

impl Registration<'_> {
    fn existing(&self, hive: &Key) -> Result<Option<String>, String> {
        let Some(key) = optional(hive.open(RUN))? else {
            return Ok(None);
        };
        let Some(value) = optional(key.get_string(self.name))? else {
            return Ok(None);
        };
        // Both forms were written by TaskPulse. Other commands are not ours to
        // silently rewrite or delete merely because the value name matches.
        if !value.trim().eq_ignore_ascii_case(&self.quoted)
            && !value
                .trim()
                .eq_ignore_ascii_case(self.quoted.trim_matches('"'))
        {
            return Err("Login startup entry points to a different command".into());
        }
        Ok(Some(value))
    }

    fn approval(&self, hive: &Key) -> Result<Option<Value>, String> {
        match optional(hive.open(APPROVED))? {
            Some(key) => optional(key.get_value(self.name)),
            None => Ok(None),
        }
    }

    fn enabled(&self) -> Result<bool, String> {
        let mut registered = false;
        for hive in [self.user, self.machine] {
            let Some(value) = self.existing(hive)? else {
                continue;
            };
            if value.trim() != self.quoted {
                return Err(
                    "Login startup command needs repair; check registry permissions".into(),
                );
            }
            registered = true;
        }
        if !registered {
            return Ok(false);
        }
        // Task Manager can disable a machine entry for the current user too.
        for hive in [self.user, self.machine] {
            if let Some(value) = self.approval(hive)? {
                if value.ty() != windows_registry::Type::Bytes || value.len() != 12 {
                    return Err("Unknown Windows login startup approval state".into());
                }
                match u32::from_le_bytes(value[..4].try_into().unwrap()) {
                    2 | 6 => {}
                    3 | 7 => return Ok(false),
                    _ => return Err("Unknown Windows login startup approval state".into()),
                }
            }
        }
        Ok(true)
    }

    fn repair_quoting(&self) -> Result<(), String> {
        for hive in [self.user, self.machine] {
            if let Some(value) = self.existing(hive)? {
                if value.trim() != self.quoted {
                    hive.options()
                        .write()
                        .open(RUN)
                        .and_then(|key| key.set_string(self.name, &self.quoted))
                        .map_err(|e| format!("Cannot repair login startup command: {e}"))?;
                }
            }
        }
        Ok(())
    }

    fn set_enabled(&self, enabled: bool) -> Result<(), String> {
        // Read and validate ownership before modifying either scope.
        let user = self.existing(self.user)?;
        let machine = self.existing(self.machine)?;
        if !enabled {
            // Machine access can fail for an ordinary user; fail before deleting
            // the user's entry, and never report an unsuccessful disable as off.
            for (hive, present) in [(self.machine, machine), (self.user, user)] {
                if present.is_some() {
                    hive.options()
                        .write()
                        .open(RUN)
                        .and_then(|key| key.remove_value(self.name))
                        .map_err(|e| e.to_string())?;
                }
            }
            return Ok(());
        }
        // Retain a legacy machine entry's existing scope. New entries are always
        // current-user, including when TaskPulse itself happens to be elevated.
        let hive = if machine.is_some() {
            self.machine
        } else {
            self.user
        };
        let previous = if machine.is_some() { machine } else { user };
        let key = hive.create(RUN).map_err(|e| e.to_string())?;
        let approvals: Vec<_> = [self.machine, self.user]
            .into_iter()
            .map(|h| self.approval(h).map(|v| (h, v)))
            .collect::<Result<_, _>>()?;
        key.set_string(self.name, &self.quoted)
            .map_err(|e| e.to_string())?;
        for (scope, value) in &approvals {
            if value.is_some() {
                let result = scope.options().write().open(APPROVED).and_then(|key| {
                    key.set_bytes(self.name, windows_registry::Type::Bytes, &ENABLED)
                });
                if let Err(error) = result {
                    // Preserve the confirmed state on a partial registration failure.
                    let rollback = match &previous {
                        Some(old) => key.set_string(self.name, old),
                        None => key.remove_value(self.name),
                    };
                    for (scope, old) in &approvals {
                        if let Some(old) = old {
                            let _ = scope
                                .options()
                                .write()
                                .open(APPROVED)
                                .and_then(|key| key.set_value(self.name, old));
                        }
                    }
                    return Err(format!(
                        "Cannot enable login startup: {error}; registration rollback: {rollback:?}"
                    ));
                }
            }
        }
        Ok(())
    }
}

pub fn repair(name: &str, exe: &Path) -> Result<(), String> {
    Registration {
        user: CURRENT_USER,
        machine: LOCAL_MACHINE,
        name,
        quoted: command(exe)?,
    }
    .repair_quoting()
}
pub fn enabled(name: &str, exe: &Path) -> Result<bool, String> {
    Registration {
        user: CURRENT_USER,
        machine: LOCAL_MACHINE,
        name,
        quoted: command(exe)?,
    }
    .enabled()
}
pub fn set_enabled(name: &str, exe: &Path, enabled: bool) -> Result<(), String> {
    Registration {
        user: CURRENT_USER,
        machine: LOCAL_MACHINE,
        name,
        quoted: command(exe)?,
    }
    .set_enabled(enabled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_startup_command_cannot_select_a_space_prefix_executable() {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::{
            Foundation::CloseHandle,
            System::Threading::{
                CreateProcessW, QueryFullProcessImageNameW, TerminateProcess, WaitForSingleObject,
                CREATE_NO_WINDOW, CREATE_SUSPENDED, PROCESS_INFORMATION, STARTUPINFOW,
            },
        };
        struct Suspended(PROCESS_INFORMATION);
        impl Drop for Suspended {
            fn drop(&mut self) {
                unsafe {
                    TerminateProcess(self.0.hProcess, 0);
                    WaitForSingleObject(self.0.hProcess, 5000);
                    CloseHandle(self.0.hThread);
                    CloseHandle(self.0.hProcess);
                }
            }
        }
        fn selected(command: &str) -> std::path::PathBuf {
            let mut wide: Vec<u16> = std::ffi::OsStr::new(command)
                .encode_wide()
                .chain(Some(0))
                .collect();
            let mut startup: STARTUPINFOW = unsafe { std::mem::zeroed() };
            startup.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
            let mut process: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
            assert_ne!(
                unsafe {
                    CreateProcessW(
                        std::ptr::null(),
                        wide.as_mut_ptr(),
                        std::ptr::null(),
                        std::ptr::null(),
                        0,
                        CREATE_SUSPENDED | CREATE_NO_WINDOW,
                        std::ptr::null(),
                        std::ptr::null(),
                        &startup,
                        &mut process,
                    )
                },
                0
            );
            let owned = Suspended(process);
            let mut name = vec![0u16; 32768];
            let mut length = name.len() as u32;
            assert_ne!(
                unsafe {
                    QueryFullProcessImageNameW(owned.0.hProcess, 0, name.as_mut_ptr(), &mut length)
                },
                0
            );
            std::path::PathBuf::from(String::from_utf16(&name[..length as usize]).unwrap())
                .canonicalize()
                .unwrap()
        }
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("测试 App");
        std::fs::create_dir(&folder).unwrap();
        let exe = folder.join("agent-tray.exe");
        let prefix = temp.path().join("测试.exe");
        let system = std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32/cmd.exe");
        std::fs::copy(&system, &exe).unwrap();
        std::fs::copy(&system, &prefix).unwrap();
        // Positive control reproduces the old ambiguous registration. Neither
        // process executes: both are suspended and terminated by owned handles.
        assert_eq!(
            selected(exe.to_str().unwrap()),
            prefix.canonicalize().unwrap()
        );
        assert_eq!(
            selected(&command(&exe).unwrap()),
            exe.canonicalize().unwrap()
        );
    }

    #[test]
    fn quoted_unicode_startup_path_and_invalid_inputs() {
        assert_eq!(
            command(Path::new(r"C:\Users\测试 User\TaskPulse\agent-tray.exe")).unwrap(),
            "\"C:\\Users\\测试 User\\TaskPulse\\agent-tray.exe\""
        );
        for invalid in ["relative.exe", "C:\\bad\"path.exe", "C:\\bad\npath.exe"] {
            assert!(command(Path::new(invalid)).is_err());
        }
    }

    #[test]
    fn isolated_registry_registration_repair_and_system_disable() {
        // A private, randomly named registry tree, never an actual startup key.
        let temp = tempfile::tempdir().unwrap();
        let path = format!(
            r"Software\TaskPulseTests\{}",
            temp.path().file_name().unwrap().to_str().unwrap()
        );
        assert!(optional(CURRENT_USER.open(&path)).unwrap().is_none());
        let transaction = windows_registry::Transaction::new().unwrap();
        let root = CURRENT_USER
            .options()
            .read()
            .write()
            .create()
            .transaction(&transaction)
            .open(&path)
            .unwrap();
        let user = root.create("User").unwrap();
        let machine = root.create("Machine").unwrap();
        let registration = Registration {
            user: &user,
            machine: &machine,
            name: "agent-tray",
            quoted: command(Path::new(r"C:\Test App\agent-tray.exe")).unwrap(),
        };
        let result = std::panic::catch_unwind(|| {
            assert!(!registration.enabled().unwrap());
            registration.repair_quoting().unwrap();
            assert!(optional(user.open(RUN)).unwrap().is_none());
            registration.set_enabled(true).unwrap();
            assert_eq!(
                user.open(RUN).unwrap().get_string("agent-tray").unwrap(),
                registration.quoted
            );
            assert!(optional(machine.open(RUN)).unwrap().is_none());
            let approved = user.create(APPROVED).unwrap();
            let disabled = [3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
            approved
                .set_bytes("agent-tray", windows_registry::Type::Bytes, &disabled)
                .unwrap();
            user.create(RUN)
                .unwrap()
                .set_string("agent-tray", r"C:\Test App\agent-tray.exe ")
                .unwrap();
            assert!(registration.enabled().is_err());
            registration.repair_quoting().unwrap();
            assert!(!registration.enabled().unwrap());
            assert_eq!(&*approved.get_value("agent-tray").unwrap(), &disabled);
            registration.set_enabled(true).unwrap();
            assert!(registration.enabled().unwrap());
            for state in [258u32, 8] {
                let mut unknown = [0u8; 12];
                unknown[..4].copy_from_slice(&state.to_le_bytes());
                approved
                    .set_bytes("agent-tray", windows_registry::Type::Bytes, &unknown)
                    .unwrap();
                assert!(registration.enabled().is_err());
            }
            approved
                .set_bytes("agent-tray", windows_registry::Type::Bytes, &ENABLED)
                .unwrap();
            registration.set_enabled(false).unwrap();
            assert!(!registration.enabled().unwrap());
            machine
                .create(RUN)
                .unwrap()
                .set_string("agent-tray", r"C:\Test App\agent-tray.exe")
                .unwrap();
            registration.repair_quoting().unwrap();
            assert_eq!(
                machine.open(RUN).unwrap().get_string("agent-tray").unwrap(),
                registration.quoted
            );
            registration.set_enabled(false).unwrap();
            user.create(RUN)
                .unwrap()
                .set_string("agent-tray", "unrelated.exe")
                .unwrap();
            assert!(registration.set_enabled(false).is_err());
            assert_eq!(
                user.open(RUN).unwrap().get_string("agent-tray").unwrap(),
                "unrelated.exe"
            );
        });
        drop(user);
        drop(machine);
        drop(root);
        // The entire private registry fixture rolls back, including on failure.
        drop(transaction);
        assert!(optional(CURRENT_USER.open(&path)).unwrap().is_none());
        if let Err(panic) = result {
            std::panic::resume_unwind(panic);
        }
    }
}
