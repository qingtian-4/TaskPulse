//! One GUI writer per configuration root. CLI event writers have separate locks.
use std::fs::File;
use std::path::{Path, PathBuf};

pub struct Instance {
    _lock: File,
    activation: PathBuf,
    previous_activation: Vec<u8>,
}

impl Instance {
    pub fn acquire(root: &Path) -> std::io::Result<Option<Self>> {
        std::fs::create_dir_all(root)?;
        // The OS lock also identifies aliases of the same directory/file.
        let lock = File::options()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(root.join("gui.lock"))?;
        let activation = root.join("gui.activate");
        match lock.try_lock() {
            Ok(()) => Ok(Some(Self {
                _lock: lock,
                previous_activation: std::fs::read(&activation).unwrap_or_default(),
                activation,
            })),
            Err(std::fs::TryLockError::WouldBlock) => {
                std::fs::write(
                    activation,
                    format!("{}:{:?}", std::process::id(), std::time::SystemTime::now()),
                )?;
                Ok(None)
            }
            Err(std::fs::TryLockError::Error(error)) => Err(error),
        }
    }

    pub fn listen(&self, app: tauri::AppHandle) {
        let path = self.activation.clone();
        let mut previous = self.previous_activation.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_millis(250));
            if let Ok(next) = std::fs::read(&path) {
                if next != previous {
                    previous = next;
                    crate::tray::show_popup(&app);
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn one_writer_per_root_and_lock_releases_on_exit() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let first = Instance::acquire(a.path()).unwrap().unwrap();
        assert!(Instance::acquire(a.path()).unwrap().is_none());
        assert!(a.path().join("gui.activate").is_file());
        assert!(Instance::acquire(b.path()).unwrap().is_some());
        drop(first);
        assert!(Instance::acquire(a.path()).unwrap().is_some());
    }
}
