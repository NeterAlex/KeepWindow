use std::time::{Duration, Instant};

use windows::{
    Win32::{
        Foundation::{
            CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HLOCAL, HWND, LPARAM,
            LocalFree, WPARAM,
        },
        Security::{
            Authorization::{
                ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
            },
            PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES,
        },
        System::Threading::CreateMutexExW,
        UI::WindowsAndMessaging::*,
    },
    core::{BOOL, PCWSTR, w},
};

use crate::window::wide;

// The profile suffix keeps separate users (and isolated test profiles) independent.
fn namespace() -> String {
    let profile = std::env::var("LOCALAPPDATA")
        .unwrap_or_default()
        .to_lowercase();
    let hash = profile.bytes().fold(0xcbf29ce484222325u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    });
    format!("KeepWindow.{hash:016x}")
}

pub fn show_message() -> u32 {
    unsafe { RegisterWindowMessageW(PCWSTR(wide(&format!("{}.Show", namespace())).as_ptr())) }
}

pub fn property() -> Vec<u16> {
    wide(&format!("{}.Main", namespace()))
}

pub struct Instance(HANDLE);

impl Instance {
    pub fn acquire(background: bool) -> Result<Option<Self>, String> {
        let (instance, existing) = Self::open(&format!("Local\\{}", namespace()))?;
        if !existing {
            return Ok(Some(instance));
        }
        if background {
            return Ok(None);
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            let mut found = HWND::default();
            unsafe extern "system" fn callback(hwnd: HWND, data: LPARAM) -> BOOL {
                unsafe {
                    if GetPropW(hwnd, PCWSTR(property().as_ptr())).0 as usize == 1 {
                        *(data.0 as *mut HWND) = hwnd;
                        return BOOL(0);
                    }
                    BOOL(1)
                }
            }
            unsafe {
                let _ = EnumWindows(Some(callback), LPARAM(&mut found as *mut HWND as isize));
                if !found.0.is_null() {
                    let mut result = 0;
                    if SendMessageTimeoutW(
                        found,
                        show_message(),
                        WPARAM(0),
                        LPARAM(0),
                        SMTO_ABORTIFHUNG | SMTO_BLOCK,
                        500,
                        Some(&mut result),
                    )
                    .0 != 0
                    {
                        return Ok(None);
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        Err(crate::locale::text(
            "the_existing_keepwindow_instance_is_not_responding",
        ))
    }

    fn open(name: &str) -> Result<(Self, bool), String> {
        unsafe {
            let mut descriptor = PSECURITY_DESCRIPTOR::default();
            // Medium integrity and SYNCHRONIZE access allow the same object at either elevation.
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                w!("D:(A;;0x00100000;;;IU)(A;;GA;;;SY)(A;;GA;;;BA)S:(ML;;NW;;;ME)"),
                SDDL_REVISION_1,
                &mut descriptor,
                None,
            )
            .map_err(|e| {
                crate::locale::format(
                    "cannot_create_single_instance_permissions_e",
                    &[("e", &format!("{}", e))],
                )
            })?;
            let attributes = SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor.0,
                bInheritHandle: BOOL(0),
            };
            let name = wide(name);
            let result = CreateMutexExW(Some(&attributes), PCWSTR(name.as_ptr()), 0, 0x00100000);
            let existing = GetLastError() == ERROR_ALREADY_EXISTS;
            let _ = LocalFree(Some(HLOCAL(descriptor.0)));
            result.map(|handle| (Self(handle), existing)).map_err(|e| {
                crate::locale::format(
                    "cannot_acquire_single_instance_lock_e",
                    &[("e", &format!("{}", e))],
                )
            })
        }
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lock_lives_until_last_handle_closes() {
        let name = format!("Local\\KeepWindow.Test.{}", std::process::id());
        let (first, exists) = Instance::open(&name).unwrap();
        assert!(!exists);
        let (second, exists) = Instance::open(&name).unwrap();
        assert!(exists);
        drop(first);
        drop(second);
        let (_, exists) = Instance::open(&name).unwrap();
        assert!(!exists);
    }
}
