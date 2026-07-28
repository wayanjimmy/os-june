use std::{fs, io, path::Path};

#[cfg(target_os = "macos")]
use std::{fs::File, os::fd::AsRawFd};

pub(crate) enum ReplaceExistingFileOutcome {
    Replaced,
    NotReplaced(io::Error),
    RecoveryRequired(io::Error),
}

#[cfg(target_os = "macos")]
pub(crate) fn preserve_replacement_metadata(
    source: &File,
    _source_path: &Path,
    staged: &File,
    _staged_path: &Path,
) -> io::Result<()> {
    // Copy ACLs and extended attributes without copying the data fork or file
    // timestamps. The staged replacement owns those values.
    let result = unsafe {
        libc::fcopyfile(
            source.as_raw_fd(),
            staged.as_raw_fd(),
            std::ptr::null_mut(),
            libc::COPYFILE_ACL | libc::COPYFILE_XATTR,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(windows)]
pub(crate) fn preserve_replacement_metadata(
    _source: &fs::File,
    source_path: &Path,
    _staged: &fs::File,
    staged_path: &Path,
) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Security::{
        GetFileSecurityW, SetFileSecurityW, DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
    };

    let source: Vec<u16> = source_path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let staged: Vec<u16> = staged_path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let mut required = 0;
    unsafe {
        GetFileSecurityW(
            PCWSTR(source.as_ptr()),
            DACL_SECURITY_INFORMATION.0,
            None,
            0,
            &mut required,
        );
    }
    if required == 0 {
        return Err(io::Error::last_os_error());
    }
    let word_size = std::mem::size_of::<usize>();
    let word_count = (required as usize).div_ceil(word_size);
    let mut descriptor_storage = vec![0_usize; word_count];
    let descriptor = PSECURITY_DESCRIPTOR(descriptor_storage.as_mut_ptr().cast());
    let descriptor_capacity = (descriptor_storage.len() * word_size) as u32;
    let read = unsafe {
        GetFileSecurityW(
            PCWSTR(source.as_ptr()),
            DACL_SECURITY_INFORMATION.0,
            Some(descriptor),
            descriptor_capacity,
            &mut required,
        )
    };
    if !read.as_bool() {
        return Err(io::Error::last_os_error());
    }
    let applied = unsafe {
        SetFileSecurityW(
            PCWSTR(staged.as_ptr()),
            DACL_SECURITY_INFORMATION,
            descriptor,
        )
    };
    if applied.as_bool() {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(all(not(target_os = "macos"), not(windows)))]
pub(crate) fn preserve_replacement_metadata(
    _source: &fs::File,
    _source_path: &Path,
    _staged: &fs::File,
    _staged_path: &Path,
) -> io::Result<()> {
    Ok(())
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
