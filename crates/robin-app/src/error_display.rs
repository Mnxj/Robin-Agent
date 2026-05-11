#[cfg(not(target_os = "windows"))]
mod inner {
    pub fn show_error(msg: &str) {
        tracing::error!("{msg}");
    }
}

#[cfg(target_os = "windows")]
mod inner {
    pub fn show_error(msg: &str) {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        let title: Vec<u16> = OsStr::new("Robin\0").encode_wide().collect();
        let text: Vec<u16> = OsStr::new(msg).encode_wide().chain(std::iter::once(0)).collect();
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(
                std::ptr::null_mut(),
                text.as_ptr(),
                title.as_ptr(),
                windows_sys::Win32::UI::WindowsAndMessaging::MB_ICONERROR,
            );
        }
    }
}

pub use inner::show_error;