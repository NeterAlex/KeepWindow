use std::mem::size_of;

use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, RECT},
        Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute},
        System::Threading::GetCurrentProcessId,
        UI::WindowsAndMessaging::{
            EnumWindows, GetClassNameW, GetShellWindow, GetWindowTextW, GetWindowThreadProcessId,
            IsWindow, IsWindowVisible,
        },
    },
    core::{BOOL, Result},
};

pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

#[derive(Clone, Debug)]
pub struct WindowInfo {
    pub hwnd: HWND,
    pub pid: u32,
    pub thread_id: u32,
    pub title: String,
    pub class: String,
}

impl WindowInfo {
    pub fn inspect(hwnd: HWND) -> Option<Self> {
        unsafe {
            if !IsWindow(Some(hwnd)).as_bool() {
                return None;
            }
            let mut pid = 0;
            let thread_id = GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if thread_id == 0 {
                return None;
            }
            let mut title = [0u16; 1024];
            let title_len = GetWindowTextW(hwnd, &mut title).max(0) as usize;
            let mut class = [0u16; 256];
            let class_len = GetClassNameW(hwnd, &mut class).max(0) as usize;
            Some(Self {
                hwnd,
                pid,
                thread_id,
                title: String::from_utf16_lossy(&title[..title_len]),
                class: String::from_utf16_lossy(&class[..class_len]),
            })
        }
    }

    pub fn label(&self) -> String {
        let title = if self.title.is_empty() {
            format!("[{}]", self.class)
        } else {
            self.title.clone()
        };
        format!(
            "{title}    |    PID {}    |    0x{:X}",
            self.pid, self.hwnd.0 as usize
        )
    }

    pub fn is_same_window(&self) -> bool {
        Self::inspect(self.hwnd).is_some_and(|current| {
            current.pid == self.pid
                && current.thread_id == self.thread_id
                && current.class == self.class
        })
    }
}

pub fn is_cloaked(hwnd: HWND) -> bool {
    let mut cloaked = 0u32;
    unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut _ as *mut _,
            size_of::<u32>() as u32,
        )
        .is_ok()
            && cloaked != 0
    }
}

pub fn enumerate() -> Result<Vec<WindowInfo>> {
    unsafe extern "system" fn callback(hwnd: HWND, data: LPARAM) -> BOOL {
        unsafe {
            if hwnd == GetShellWindow() || !IsWindowVisible(hwnd).as_bool() || is_cloaked(hwnd) {
                return BOOL(1);
            }
            if let Some(info) = WindowInfo::inspect(hwnd)
                && info.pid != GetCurrentProcessId()
            {
                let windows = &mut *(data.0 as *mut Vec<WindowInfo>);
                windows.push(info);
            }
            BOOL(1)
        }
    }

    let mut windows: Vec<WindowInfo> = Vec::new();
    unsafe {
        EnumWindows(Some(callback), LPARAM(&mut windows as *mut _ as isize))?;
    }
    windows.sort_by_key(|window| window.title.to_lowercase());
    Ok(windows)
}

pub fn valid_rect(rect: RECT) -> bool {
    rect.right > rect.left && rect.bottom > rect.top
}
