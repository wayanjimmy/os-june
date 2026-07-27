use std::{fs, io, path::Path};

pub(crate) enum ReplaceExistingFileOutcome {
    Replaced,
    NotReplaced(io::Error),
    RecoveryRequired(io::Error),
}

#[cfg(windows)]
fn windows_io_error(error: windows::core::Error) -> io::Error {
    let hresult = error.code().0 as u32;
    if hresult & 0xffff_0000 == 0x8007_0000 {
        io::Error::from_raw_os_error((hresult & 0xffff) as i32)
    } else {
        io::Error::other(error)
    }
}

#[cfg(windows)]
pub(crate) fn publish_new_file(temp_path: &Path, path: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};
    let temp: Vec<u16> = temp_path.as_os_str().encode_wide().chain(Some(0)).collect();
    let target: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        MoveFileExW(
            PCWSTR(temp.as_ptr()),
            PCWSTR(target.as_ptr()),
            MOVEFILE_WRITE_THROUGH,
        )
    }
    .map_err(windows_io_error)
}

#[cfg(not(windows))]
pub(crate) fn publish_new_file(temp_path: &Path, path: &Path) -> io::Result<()> {
    fs::hard_link(temp_path, path)?;
    let _ = fs::remove_file(temp_path);
    Ok(())
}

#[cfg(windows)]
pub(crate) fn replace_existing_file(
    temp_path: &Path,
    path: &Path,
    backup_path: &Path,
) -> ReplaceExistingFileOutcome {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{ReplaceFileW, REPLACE_FILE_FLAGS};
    let temp: Vec<u16> = temp_path.as_os_str().encode_wide().chain(Some(0)).collect();
    let target: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let backup: Vec<u16> = backup_path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let result = unsafe {
        ReplaceFileW(
            PCWSTR(target.as_ptr()),
            PCWSTR(temp.as_ptr()),
            PCWSTR(backup.as_ptr()),
            REPLACE_FILE_FLAGS(0),
            None,
            None,
        )
    };
    match result {
        Ok(()) => {
            let _ = fs::remove_file(backup_path);
            ReplaceExistingFileOutcome::Replaced
        }
        Err(error) => {
            let error = windows_io_error(error);
            if matches!(error.raw_os_error(), Some(1175 | 1176 | 1177)) {
                ReplaceExistingFileOutcome::RecoveryRequired(error)
            } else {
                ReplaceExistingFileOutcome::NotReplaced(error)
            }
        }
    }
}

#[cfg(not(windows))]
pub(crate) fn replace_existing_file(
    temp_path: &Path,
    path: &Path,
    _backup_path: &Path,
) -> ReplaceExistingFileOutcome {
    match fs::rename(temp_path, path) {
        Ok(()) => ReplaceExistingFileOutcome::Replaced,
        Err(error) => ReplaceExistingFileOutcome::NotReplaced(error),
    }
}

#[cfg(windows)]
pub(crate) fn replace_file(temp_path: &Path, path: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        MoveFileExW, ReplaceFileW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
        REPLACEFILE_WRITE_THROUGH,
    };
    let temp: Vec<u16> = temp_path.as_os_str().encode_wide().chain(Some(0)).collect();
    let target: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        if path.exists() {
            ReplaceFileW(
                PCWSTR(target.as_ptr()),
                PCWSTR(temp.as_ptr()),
                PCWSTR::null(),
                REPLACEFILE_WRITE_THROUGH,
                None,
                None,
            )
        } else {
            MoveFileExW(
                PCWSTR(temp.as_ptr()),
                PCWSTR(target.as_ptr()),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        }
    }
    .map_err(|_| io::Error::last_os_error())
}

#[cfg(not(windows))]
pub(crate) fn replace_file(temp_path: &Path, path: &Path) -> io::Result<()> {
    fs::rename(temp_path, path)
}
