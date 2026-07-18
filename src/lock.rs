use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use thiserror::Error;

#[cfg(unix)]
use fs4::FileExt;

#[derive(Debug, Error)]
pub enum KBLockError {
    #[error("lock held by another process: {held_by:?}")]
    Locked { held_by: Option<String> },
    #[error("lock IO error: {0}")]
    Io(#[from] std::io::Error),
}

pub struct KBLock {
    _file: File,
}

impl KBLock {
    /// Try to acquire an exclusive advisory lock on `{data_dir}/.writer.lock`.
    pub fn acquire(data_dir: &Path) -> Result<Self, KBLockError> {
        let lock_path = data_dir.join(".writer.lock");

        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .read(true)
            .open(&lock_path)?;

        #[cfg(unix)]
        {
            use fs4::FileExt;
            if file.try_lock_exclusive().is_err() {
                let held_by = read_lock_info(&lock_path);
                return Err(KBLockError::Locked { held_by });
            }
        }

        #[cfg(not(unix))]
        {
            // On Windows use fs4 too
            if let Err(_) = file.try_lock_exclusive() {
                let held_by = read_lock_info(&lock_path);
                return Err(KBLockError::Locked { held_by });
            }
        }

        // Write diagnostic info (PID + hostname)
        write_lock_info(&file)?;

        Ok(KBLock { _file: file })
    }
}

fn write_lock_info(mut file: &File) -> std::io::Result<()> {
    let pid = std::process::id();
    let hostname = hostname::get_hostname();
    let info = format!("pid={} host={}", pid, hostname);
    file.set_len(0)?;
    file.write_all(info.as_bytes())
}

fn read_lock_info(path: &Path) -> Option<String> {
    let mut f = File::open(path).ok()?;
    let mut s = String::new();
    f.read_to_string(&mut s).ok()?;
    if s.is_empty() { None } else { Some(s) }
}

mod hostname {
    pub fn get_hostname() -> String {
        #[cfg(unix)]
        {
            use std::ffi::CStr;
            let mut buf = [0i8; 256];
            unsafe {
                libc::gethostname(buf.as_mut_ptr(), buf.len());
                CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned()
            }
        }
        #[cfg(not(unix))]
        {
            std::env::var("COMPUTERNAME").unwrap_or_else(|_| "unknown".into())
        }
    }
}

impl Drop for KBLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use fs4::FileExt;
            let _ = self._file.unlock();
        }
    }
}
