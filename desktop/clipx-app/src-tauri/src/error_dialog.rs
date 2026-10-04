use std::fmt::Display;
use std::os::windows::ffi::OsStrExt;
use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

#[cfg(windows)]
pub fn show_core_error(error: impl Display) {
    let message = format!(
        "Clipx core could not be started: {error}"
    );

    let message: Vec<u16> = std::ffi::OsStr::new(&message)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let title: Vec<u16> = std::ffi::OsStr::new("ClipX")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            message.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}
