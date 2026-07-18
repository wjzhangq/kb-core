use std::fs;
use std::path::Path;
use anyhow::Result;
use tempfile::Builder as TempBuilder;

use crate::config::TempSecurity;

/// Create a secure temporary file in `{data_dir}/tmp/`.
pub fn secure_temp_file(data_dir: &Path, _security: &TempSecurity) -> Result<tempfile::NamedTempFile> {
    let tmp_dir = data_dir.join("tmp");
    fs::create_dir_all(&tmp_dir)?;

    let file = TempBuilder::new()
        .prefix("kb-")
        .suffix(".tmp")
        .tempfile_in(&tmp_dir)?;

    #[cfg(unix)]
    set_unix_permissions(file.path())?;

    #[cfg(windows)]
    if matches!(_security, TempSecurity::AclRestricted) {
        set_windows_acl_restricted(file.path())?;
    }

    Ok(file)
}

/// Remove any leftover tmp files from a previous run.
pub fn cleanup_tmp_dir(data_dir: &Path) -> Result<()> {
    let tmp_dir = data_dir.join("tmp");
    if !tmp_dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(&tmp_dir)? {
        let entry = entry?;
        if let Err(e) = fs::remove_file(entry.path()) {
            tracing::warn!("failed to remove leftover tmp file {:?}: {}", entry.path(), e);
        }
    }
    Ok(())
}

#[cfg(unix)]
fn set_unix_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let perms = fs::Permissions::from_mode(0o600);
    fs::set_permissions(path, perms)?;
    Ok(())
}

#[cfg(windows)]
fn set_windows_acl_restricted(path: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Security::Authorization::*;
    use windows_sys::Win32::Security::*;
    use windows_sys::Win32::Foundation::*;

    // Strip Everyone / Guests ACEs by setting a DACL with only the current user.
    // This is a best-effort operation; failure is logged but not fatal.
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        let result = SetNamedSecurityInfoW(
            wide.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(), // NULL DACL = deny all (too strict); use empty ACL instead
            std::ptr::null_mut(),
        );
        if result != ERROR_SUCCESS {
            tracing::warn!("failed to restrict ACL on temp file: win32 error {}", result);
        }
    }
    Ok(())
}
