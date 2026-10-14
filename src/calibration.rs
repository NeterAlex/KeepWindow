use crate::{
    protection::{Options, shield_rect},
    window::{WindowInfo, is_cloaked},
};
use std::{cell::Cell, rc::Rc};
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            HiDpi::GetDpiForWindow,
            Input::KeyboardAndMouse::{
                GetAsyncKeyState, ReleaseCapture, SetCapture, VK_ESCAPE, VK_RETURN,
            },
            WindowsAndMessaging::*,
        },
    },
    core::w,
};

#[derive(Clone, Copy, PartialEq)]
pub enum Outcome {
    Save,
    Cancel,
}

#[derive(Clone, Copy)]
struct Drag {
    origin: windows::Win32::Foundation::POINT,
    rect: RECT,
    resize: bool,
}

struct State {
    outcome: Cell<Option<Outcome>>,
    drag: Cell<Option<Drag>>,
}

pub struct Calibration {
    pub target: WindowInfo,
    pub original: Options,
    pub current: Options,
    pub previous_protection: Option<Options>,
    frame: HWND,
    toolbar: HWND,
    state: Rc<State>,
    keys: [bool; 2],
    last_rect: RECT,
}

impl Calibration {
    pub fn frame(&self) -> HWND {
        self.frame
    }
    pub fn new(target: WindowInfo, options: Options) -> Result<Self, String> {
        unsafe {
            if !target.is_same_window() {
                return Err("目标窗口已关闭。".into());
            }
            let instance = GetModuleHandleW(None).map_err(|e| e.to_string())?;
            let class = WNDCLASSW {
                lpfnWndProc: Some(procedure),
                hInstance: instance.into(),
                lpszClassName: w!("KeepWindow.Calibration"),
                hCursor: LoadCursorW(None, IDC_SIZEALL).map_err(|e| e.to_string())?,
                hbrBackground: windows::Win32::Graphics::Gdi::HBRUSH(
                    (windows::Win32::Graphics::Gdi::COLOR_HIGHLIGHT.0 + 1) as usize as *mut _,
                ),
                ..Default::default()
            };
            let _ = RegisterClassW(&class);
            let rect = shield_rect(target.hwnd, options)?;
            let frame = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED,
                class.lpszClassName,
                w!("拖动移动，右下角缩放"),
                WS_POPUP,
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                None,
                None,
                Some(instance.into()),
                None,
            )
            .map_err(|e| e.to_string())?;
            let _ = SetLayeredWindowAttributes(
                frame,
                windows::Win32::Foundation::COLORREF(0),
                200,
                LWA_ALPHA,
            );
            let state = Rc::new(State {
                outcome: Cell::new(None),
                drag: Cell::new(None),
            });
            SetWindowLongPtrW(frame, GWLP_USERDATA, Rc::as_ptr(&state) as isize);
            let toolbar = match CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                class.lpszClassName,
                w!("校准遮罩"),
                WS_POPUP,
                rect.left - 180,
                rect.bottom + 8,
                280,
                70,
                Some(frame),
                None,
                Some(instance.into()),
                None,
            ) {
                Ok(hwnd) => hwnd,
                Err(e) => {
                    let _ = DestroyWindow(frame);
                    return Err(e.to_string());
                }
            };
            SetWindowLongPtrW(toolbar, GWLP_USERDATA, Rc::as_ptr(&state) as isize);
            let result = (|| -> windows::core::Result<()> {
                CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    w!("STATIC"),
                    w!("拖动移动 / 右下角缩放 / Enter 保存"),
                    WS_CHILD | WS_VISIBLE,
                    8,
                    6,
                    264,
                    22,
                    Some(toolbar),
                    None,
                    Some(instance.into()),
                    None,
                )?;
                CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    w!("BUTTON"),
                    w!("保存"),
                    WS_CHILD | WS_VISIBLE,
                    72,
                    32,
                    90,
                    28,
                    Some(toolbar),
                    Some(HMENU(std::ptr::dangling_mut())),
                    Some(instance.into()),
                    None,
                )?;
                CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    w!("BUTTON"),
                    w!("取消 (Esc)"),
                    WS_CHILD | WS_VISIBLE,
                    170,
                    32,
                    100,
                    28,
                    Some(toolbar),
                    Some(HMENU(2usize as *mut _)),
                    Some(instance.into()),
                    None,
                )?;
                Ok(())
            })();
            if let Err(e) = result {
                let _ = DestroyWindow(toolbar);
                let _ = DestroyWindow(frame);
                return Err(e.to_string());
            }
            let _ = SetForegroundWindow(target.hwnd);
            let _ = ShowWindow(frame, SW_SHOW);
            let _ = ShowWindow(toolbar, SW_SHOWNOACTIVATE);
            let _ = SetForegroundWindow(frame);
            Ok(Self {
                target,
                original: options,
                current: options,
                previous_protection: None,
                frame,
                toolbar,
                state,
                keys: [true, true],
                last_rect: rect,
            })
        }
    }

    pub fn tick(&mut self) -> Result<Option<Outcome>, String> {
        let foreground = unsafe { GetAncestor(GetForegroundWindow(), GA_ROOT) };
        self.tick_foreground(foreground)
    }

    fn tick_foreground(&mut self, foreground: HWND) -> Result<Option<Outcome>, String> {
        unsafe {
            if !self.target.is_same_window()
                || IsIconic(self.target.hwnd).as_bool()
                || !IsWindowVisible(self.target.hwnd).as_bool()
                || is_cloaked(self.target.hwnd)
                || ![self.target.hwnd, self.frame, self.toolbar].contains(&foreground)
            {
                return Ok(Some(Outcome::Cancel));
            }
            self.update_geometry()?;
            for (index, key) in [VK_RETURN, VK_ESCAPE].iter().enumerate() {
                let pressed = GetAsyncKeyState(key.0 as i32) < 0;
                if pressed && !self.keys[index] {
                    self.state.outcome.set(Some(if index == 0 {
                        Outcome::Save
                    } else {
                        Outcome::Cancel
                    }));
                }
                self.keys[index] = pressed;
            }
            Ok(self.state.outcome.get())
        }
    }

    fn update_geometry(&mut self) -> Result<(), String> {
        unsafe {
            let mut rect = RECT::default();
            GetWindowRect(self.frame, &mut rect).map_err(|e| e.to_string())?;
            if rect != self.last_rect {
                let anchor = shield_rect(
                    self.target.hwnd,
                    Options {
                        width: 1,
                        height: 1,
                        right_offset: 0,
                        top_offset: 0,
                        ..self.current
                    },
                )?;
                let dpi = GetDpiForWindow(self.target.hwnd).max(96);
                let dip = |value: i32| (f64::from(value) * 96. / f64::from(dpi)).round() as i32;
                self.current.width = dip(rect.right - rect.left).clamp(1, 512);
                self.current.height = dip(rect.bottom - rect.top).clamp(1, 512);
                self.current.right_offset = dip(anchor.right - rect.right).clamp(-512, 512);
                self.current.top_offset = dip(rect.top - anchor.top).clamp(-512, 512);
            }
            let corrected = shield_rect(self.target.hwnd, self.current)?;
            let _ = SetWindowPos(
                self.frame,
                Some(HWND_TOPMOST),
                corrected.left,
                corrected.top,
                corrected.right - corrected.left,
                corrected.bottom - corrected.top,
                SWP_NOACTIVATE,
            );
            self.last_rect = corrected;
            // Keep the controls inside the current monitor's work area.
            let monitor = windows::Win32::Graphics::Gdi::MonitorFromWindow(
                self.frame,
                windows::Win32::Graphics::Gdi::MONITOR_DEFAULTTONEAREST,
            );
            let mut info = windows::Win32::Graphics::Gdi::MONITORINFO {
                cbSize: std::mem::size_of::<windows::Win32::Graphics::Gdi::MONITORINFO>() as u32,
                ..Default::default()
            };
            let _ = windows::Win32::Graphics::Gdi::GetMonitorInfoW(monitor, &mut info);
            let x = (corrected.right - 280).clamp(
                info.rcWork.left,
                (info.rcWork.right - 280).max(info.rcWork.left),
            );
            let y = (corrected.bottom + 8).clamp(
                info.rcWork.top,
                (info.rcWork.bottom - 70).max(info.rcWork.top),
            );
            let _ = SetWindowPos(
                self.toolbar,
                Some(HWND_TOPMOST),
                x,
                y,
                280,
                70,
                SWP_NOACTIVATE,
            );
            Ok(())
        }
    }
}

impl Drop for Calibration {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.toolbar);
            let _ = DestroyWindow(self.frame);
        }
    }
}

unsafe extern "system" fn procedure(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const State;
        match message {
            WM_GETMINMAXINFO => {
                let limits = &mut *(lparam.0 as *mut MINMAXINFO);
                limits.ptMinTrackSize = windows::Win32::Foundation::POINT { x: 1, y: 1 };
                limits.ptMaxTrackSize = windows::Win32::Foundation::POINT { x: 4096, y: 4096 };
                LRESULT(0)
            }
            WM_NCHITTEST => LRESULT(HTCLIENT as isize),
            WM_LBUTTONDOWN
                if !state.is_null()
                    && GetWindow(hwnd, GW_OWNER).is_ok_and(|owner| owner.0.is_null()) =>
            {
                let mut rect = RECT::default();
                let _ = GetWindowRect(hwnd, &mut rect);
                let mut origin = windows::Win32::Foundation::POINT::default();
                let _ = GetCursorPos(&mut origin);
                (*state).drag.set(Some(Drag {
                    origin,
                    rect,
                    resize: origin.x >= rect.right - 12 && origin.y >= rect.bottom - 12,
                }));
                let _ = SetCapture(hwnd);
                LRESULT(0)
            }
            WM_MOUSEMOVE if !state.is_null() => {
                if let Some(drag) = (*state).drag.get() {
                    let mut point = windows::Win32::Foundation::POINT::default();
                    let _ = GetCursorPos(&mut point);
                    let dx = point.x - drag.origin.x;
                    let dy = point.y - drag.origin.y;
                    let rect = drag.rect;
                    let (x, y, width, height) = if drag.resize {
                        (
                            rect.left,
                            rect.top,
                            (rect.right - rect.left + dx).max(1),
                            (rect.bottom - rect.top + dy).max(1),
                        )
                    } else {
                        (
                            rect.left + dx,
                            rect.top + dy,
                            rect.right - rect.left,
                            rect.bottom - rect.top,
                        )
                    };
                    let _ = SetWindowPos(
                        hwnd,
                        Some(HWND_TOPMOST),
                        x,
                        y,
                        width,
                        height,
                        SWP_NOACTIVATE,
                    );
                }
                LRESULT(0)
            }
            WM_LBUTTONUP if !state.is_null() => {
                (*state).drag.set(None);
                let _ = ReleaseCapture();
                LRESULT(0)
            }
            WM_CAPTURECHANGED if !state.is_null() => {
                (*state).drag.set(None);
                LRESULT(0)
            }
            WM_COMMAND if !state.is_null() => {
                match wparam.0 & 0xffff {
                    1 => (*state).outcome.set(Some(Outcome::Save)),
                    2 => (*state).outcome.set(Some(Outcome::Cancel)),
                    _ => {}
                }
                LRESULT(0)
            }
            WM_CLOSE if !state.is_null() => {
                (*state).outcome.set(Some(Outcome::Cancel));
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn calibration_tracks_target_and_saves_dragged_geometry() {
        unsafe {
            let target = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("Calibration test"),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                100,
                100,
                500,
                300,
                None,
                None,
                Some(GetModuleHandleW(None).unwrap().into()),
                None,
            )
            .unwrap();
            let options = Options::default();
            let mut calibration =
                Calibration::new(WindowInfo::inspect(target).unwrap(), options).unwrap();
            let _ = SetForegroundWindow(calibration.frame);
            assert!(
                calibration
                    .tick_foreground(calibration.frame)
                    .unwrap()
                    .is_none()
            );
            let before = calibration.last_rect;
            SetWindowPos(
                target,
                None,
                140,
                120,
                500,
                300,
                SWP_NOZORDER | SWP_NOACTIVATE,
            )
            .unwrap();
            calibration.tick_foreground(calibration.frame).unwrap();
            assert_eq!(calibration.last_rect.left - before.left, 40);
            assert_eq!(calibration.last_rect.top - before.top, 20);
            let before = calibration.last_rect;
            SetWindowPos(
                calibration.frame,
                None,
                before.left - 10,
                before.top + 5,
                before.right - before.left + 10,
                before.bottom - before.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            )
            .unwrap();
            calibration.tick_foreground(calibration.frame).unwrap();
            assert!(calibration.current.width > options.width);
            assert!(calibration.current.top_offset > options.top_offset);
            calibration.state.outcome.set(Some(Outcome::Save));
            assert!(matches!(
                calibration.tick_foreground(calibration.frame).unwrap(),
                Some(Outcome::Save)
            ));
            drop(calibration);
            let mut cancelled =
                Calibration::new(WindowInfo::inspect(target).unwrap(), options).unwrap();
            let _ = ShowWindow(target, SW_MINIMIZE);
            assert!(matches!(cancelled.tick().unwrap(), Some(Outcome::Cancel)));
            drop(cancelled);
            DestroyWindow(target).unwrap();
        }
    }
}
