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

    pub fn display_title(&self) -> String {
        if self.title.is_empty() {
            format!("[{}]", self.class)
        } else {
            self.title.clone()
        }
    }

    pub fn same_identity(&self, other: &Self) -> bool {
        self.hwnd == other.hwnd
            && self.pid == other.pid
            && self.thread_id == other.thread_id
            && self.class == other.class
    }

    pub fn matches_query(&self, query: &str) -> bool {
        let searchable = format!(
            "{} {} {} 0x{:x}",
            self.title.to_lowercase(),
            self.class.to_lowercase(),
            self.pid,
            self.hwnd.0 as usize
        );
        query
            .split_whitespace()
            .all(|term| searchable.contains(term))
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

pub fn retain_selection(
    windows: &[WindowInfo],
    selected: Option<&WindowInfo>,
    query: &str,
) -> Option<WindowInfo> {
    let selected = selected?;
    windows
        .iter()
        .find(|item| item.same_identity(selected) && item.matches_query(query))
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(handle: usize, title: &str) -> WindowInfo {
        WindowInfo {
            hwnd: HWND(handle as *mut _),
            pid: 4020,
            thread_id: 100,
            title: title.into(),
            class: "TestWindow".into(),
        }
    }

    #[test]
    fn refresh_keeps_window_identity_after_reorder_and_title_change() {
        let selected = sample(0xabc, "B - Editor");
        let refreshed = [sample(0xdef, "A - Browser"), sample(0xabc, "C - Editor")];
        let retained = retain_selection(&refreshed, Some(&selected), "").unwrap();
        assert_eq!(retained.hwnd, selected.hwnd);
        assert_eq!(retained.title, "C - Editor");
    }

    #[test]
    fn closed_or_recycled_window_does_not_select_another_target() {
        let selected = sample(0xabc, "Editor");
        assert!(retain_selection(&[sample(0xdef, "Browser")], Some(&selected), "").is_none());
        let mut recycled = selected.clone();
        recycled.pid += 1;
        assert!(retain_selection(&[recycled], Some(&selected), "").is_none());
    }

    #[test]
    fn filtering_matches_multiple_fields_and_clears_hidden_selection() {
        let selected = sample(0xabc, "MuMu Emulator");
        let windows = [selected.clone()];
        assert!(retain_selection(&windows, Some(&selected), "mumu 4020 0xabc").is_some());
        assert!(retain_selection(&windows, Some(&selected), "testwindow emulator").is_some());
        assert!(retain_selection(&windows, Some(&selected), "browser").is_none());
        assert!(retain_selection(&windows, None, "").is_none());
    }
}
