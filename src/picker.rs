use windows::{
    Win32::{
        Foundation::{COLORREF, HWND, LPARAM, LRESULT, WPARAM},
        Graphics::Gdi::{COLOR_HIGHLIGHT, GetSysColorBrush},
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::*,
    },
    core::w,
};

use crate::{
    mouse::{MousePicker, picker_target},
    window::WindowInfo,
};

pub struct Picker {
    mouse: MousePicker,
    borders: Vec<HWND>,
}

impl Picker {
    pub fn new() -> Result<Self, String> {
        let mouse = MousePicker::new()?;
        let mut picker = Self {
            mouse,
            borders: Vec::new(),
        };
        unsafe {
            let instance = GetModuleHandleW(None).map_err(|e| e.to_string())?;
            let class = WNDCLASSW {
                lpfnWndProc: Some(border_proc),
                hInstance: instance.into(),
                lpszClassName: w!("KeepWindow.Picker"),
                hbrBackground: GetSysColorBrush(COLOR_HIGHLIGHT),
                ..Default::default()
            };
            let _ = RegisterClassW(&class);
            for _ in 0..4 {
                let hwnd = CreateWindowExW(
                    WS_EX_TOPMOST
                        | WS_EX_TOOLWINDOW
                        | WS_EX_NOACTIVATE
                        | WS_EX_LAYERED
                        | WS_EX_TRANSPARENT,
                    class.lpszClassName,
                    w!("KeepWindow picker"),
                    WS_POPUP | WS_DISABLED,
                    0,
                    0,
                    1,
                    1,
                    None,
                    None,
                    Some(instance.into()),
                    None,
                )
                .map_err(|e| e.to_string())?;
                picker.borders.push(hwnd);
                SetLayeredWindowAttributes(hwnd, COLORREF(0), 210, LWA_ALPHA)
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(picker)
    }

    pub fn tick(&mut self) -> Result<Option<Option<WindowInfo>>, String> {
        let outcome = self.mouse.poll()?;
        let mut rect = windows::Win32::Foundation::RECT::default();
        let target = self.mouse.hover.and_then(picker_target);
        unsafe {
            if outcome.is_some()
                || target
                    .as_ref()
                    .is_none_or(|target| GetWindowRect(target.hwnd, &mut rect).is_err())
            {
                for hwnd in &self.borders {
                    let _ = ShowWindow(*hwnd, SW_HIDE);
                }
            } else {
                let thickness = target.as_ref().map_or(3, |target| {
                    (windows::Win32::UI::HiDpi::GetDpiForWindow(target.hwnd).max(96) * 3 / 96)
                        as i32
                });
                let width = rect.right - rect.left;
                let height = rect.bottom - rect.top;
                for (hwnd, (x, y, width, height)) in self.borders.iter().zip([
                    (rect.left, rect.top, width, thickness),
                    (rect.left, rect.bottom - thickness, width, thickness),
                    (rect.left, rect.top, thickness, height),
                    (rect.right - thickness, rect.top, thickness, height),
                ]) {
                    SetWindowPos(
                        *hwnd,
                        Some(HWND_TOPMOST),
                        x,
                        y,
                        width,
                        height,
                        SWP_NOACTIVATE | SWP_SHOWWINDOW,
                    )
                    .map_err(|e| e.to_string())?;
                }
            }
        }
        Ok(outcome)
    }
}

impl Drop for Picker {
    fn drop(&mut self) {
        unsafe {
            for hwnd in &self.borders {
                let _ = DestroyWindow(*hwnd);
            }
        }
    }
}

unsafe extern "system" fn border_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}
