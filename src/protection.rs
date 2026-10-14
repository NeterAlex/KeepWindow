use std::{
    mem::size_of,
    time::{SystemTime, UNIX_EPOCH},
};

use windows::{
    Win32::{
        Foundation::{COLORREF, HANDLE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::{
            Dwm::{
                DWMWA_CAPTION_BUTTON_BOUNDS, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute,
            },
            Gdi::{
                BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, FillRect, HGDIOBJ,
                PAINTSTRUCT,
            },
        },
        System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentProcessId},
        UI::{HiDpi::GetDpiForWindow, WindowsAndMessaging::*},
    },
    core::{PCWSTR, w},
};

use crate::{
    mouse::MouseRegistration,
    recovery::{Identity, Recovery},
    window::{WindowInfo, is_cloaked, valid_rect, wide},
};

const OVERLAY_CLASS: PCWSTR = w!("KeepWindow.CloseShield");

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Options {
    pub system_menu: bool,
    pub mouse_shield: bool,
    pub width: i32,
    pub height: i32,
    pub right_offset: i32,
    pub top_offset: i32,
    pub opacity: i32,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            system_menu: true,
            mouse_shield: true,
            width: 46,
            height: 32,
            right_offset: 0,
            top_offset: 0,
            opacity: 28,
        }
    }
}

impl Options {
    pub fn validate(&self) -> Result<(), String> {
        if !self.system_menu && !self.mouse_shield {
            return Err("请至少启用一种保护方式。".into());
        }
        if !(1..=512).contains(&self.width)
            || !(1..=512).contains(&self.height)
            || !(-512..=512).contains(&self.right_offset)
            || !(-512..=512).contains(&self.top_offset)
            || !(5..=80).contains(&self.opacity)
        {
            return Err("遮罩尺寸、偏移或透明度超出允许范围。".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct ProtectionStatus {
    pub system_menu: bool,
    pub mouse_shield: bool,
    pub recovery_failed: bool,
    pub error: Option<String>,
}

impl ProtectionStatus {
    pub fn label(&self) -> &'static str {
        if self.recovery_failed {
            "恢复失败"
        } else {
            match (self.system_menu, self.mouse_shield) {
                (true, true) => "组合保护",
                (true, false) => "系统保护",
                (false, true) => "鼠标保护",
                _ => "未保护",
            }
        }
    }
    pub fn description(&self) -> String {
        let mut text = format!(
            "系统保护：{}；鼠标保护：{}（需确认覆盖位置）",
            if self.system_menu {
                "生效"
            } else {
                "未生效"
            },
            if self.mouse_shield {
                "生效"
            } else {
                "未生效"
            }
        );
        if let Some(error) = &self.error {
            text.push_str(&format!("；{error}"));
        }
        text
    }
}

struct MenuProtection {
    menu: HMENU,
    original_disabled: MENU_ITEM_FLAGS,
}

pub struct Protection {
    pub target: WindowInfo,
    pub options: Options,
    menu: Option<MenuProtection>,
    recovery: Recovery,
    pending: Option<Identity>,
    overlay: Option<HWND>,
    mouse: Option<MouseRegistration>,
    marker: Vec<u16>,
    pub recovery_failed: bool,
    pub error: Option<String>,
    pub calibrating: bool,
}

impl Protection {
    #[cfg(test)]
    pub fn start(target: WindowInfo, options: Options) -> Result<Self, String> {
        Self::start_logged(target, options, Recovery::testing())
    }

    pub fn start_logged(
        target: WindowInfo,
        options: Options,
        recovery: Recovery,
    ) -> Result<Self, String> {
        options.validate()?;
        if !target.is_same_window() {
            return Err("目标窗口已关闭，请刷新窗口列表。".into());
        }
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let marker = wide(&format!(
            "KeepWindow.{}.{}",
            unsafe { GetCurrentProcessId() },
            nonce
        ));
        let mut protection = Self {
            target,
            options,
            menu: None,
            recovery,
            pending: None,
            overlay: None,
            mouse: None,
            marker,
            recovery_failed: false,
            error: None,
            calibrating: false,
        };
        unsafe {
            // A property disappears with the window, so a recycled HWND is never restored or shielded.
            SetPropW(protection.target.hwnd, PCWSTR(protection.marker.as_ptr()), Some(HANDLE(std::ptr::dangling_mut())))
                .map_err(|error| format!("无法访问目标窗口：{error}\n如果目标以管理员身份运行，请也以管理员身份启动 KeepWindow。"))?;
        }
        if options.system_menu {
            let menu = unsafe { GetSystemMenu(protection.target.hwnd, false) };
            let has_close = !menu.0.is_null()
                && unsafe { GetMenuState(menu, SC_CLOSE, MF_BYCOMMAND) } != u32::MAX;
            if has_close || !options.mouse_shield {
                protection.disable_close()?;
            } else {
                protection.error = Some("目标没有标准关闭项，仅鼠标保护生效。".into());
            }
        }
        if options.mouse_shield {
            protection.overlay = Some(
                create_overlay(options.opacity)
                    .map_err(|error| format!("无法创建鼠标遮罩：{error}"))?,
            );
            protection.mouse = Some(MouseRegistration::new(
                protection.target.hwnd,
                protection.overlay,
                &protection.marker,
                options,
            )?);
        }
        protection.tick()?;
        Ok(protection)
    }

    pub fn alive(&self) -> bool {
        self.target.is_same_window()
            && unsafe { GetPropW(self.target.hwnd, PCWSTR(self.marker.as_ptr())).0 as usize == 1 }
    }

    pub fn status(&self) -> ProtectionStatus {
        let system_menu = self.menu.as_ref().is_some_and(|menu| unsafe {
            self.alive() && GetSystemMenu(self.target.hwnd, false) == menu.menu && {
                let state = GetMenuState(menu.menu, SC_CLOSE, MF_BYCOMMAND);
                state != u32::MAX && state & (MF_DISABLED.0 | MF_GRAYED.0) != 0
            }
        });
        ProtectionStatus {
            system_menu,
            mouse_shield: self.mouse.is_some() && self.alive(),
            recovery_failed: self.recovery_failed,
            error: self.error.clone(),
        }
    }

    pub fn update_options(&mut self, options: Options) -> Result<(), String> {
        options.validate()?;
        if !self.alive() {
            return Err("目标窗口已关闭。".into());
        }
        if self.recovery_failed {
            return Err("请先重试恢复，再开启保护。".into());
        }
        if options.mouse_shield {
            shield_rect(self.target.hwnd, options)?;
        }
        // Prepare fallible mouse resources before changing the system menu.
        let mut new_overlay = None;
        let mut new_mouse = None;
        if options.mouse_shield && self.mouse.is_none() {
            let overlay = create_overlay(options.opacity).map_err(|e| e.to_string())?;
            match MouseRegistration::new(self.target.hwnd, Some(overlay), &self.marker, options) {
                Ok(mouse) => {
                    new_overlay = Some(overlay);
                    new_mouse = Some(mouse);
                }
                Err(error) => {
                    unsafe {
                        let _ = DestroyWindow(overlay);
                    }
                    return Err(error);
                }
            }
        }
        if options.mouse_shield
            && let Some(overlay) = self.overlay
        {
            unsafe {
                SetLayeredWindowAttributes(
                    overlay,
                    COLORREF(0),
                    (options.opacity * 255 / 100) as u8,
                    LWA_ALPHA,
                )
            }
            .map_err(|e| format!("无法更新遮罩透明度：{e}"))?;
        }
        let result = (|| {
            if options.system_menu && self.menu.is_none() {
                let menu = unsafe { GetSystemMenu(self.target.hwnd, false) };
                let has_close = !menu.0.is_null()
                    && unsafe { GetMenuState(menu, SC_CLOSE, MF_BYCOMMAND) } != u32::MAX;
                if has_close || !options.mouse_shield {
                    self.disable_close()?;
                }
            }
            if !options.system_menu
                && let Some(menu) = &self.menu
            {
                set_close_state(self.target.hwnd, menu.menu, menu.original_disabled)?;
                self.menu = None;
                self.clear_record()?;
            }
            Ok::<(), String>(())
        })();
        if let Err(error) = result {
            if self.pending.is_some() && self.menu.is_none() {
                self.recovery_failed = true;
            }
            drop(new_mouse);
            if let Some(overlay) = new_overlay {
                unsafe {
                    let _ = DestroyWindow(overlay);
                }
            }
            if let Some(overlay) = self.overlay {
                unsafe {
                    let _ = SetLayeredWindowAttributes(
                        overlay,
                        COLORREF(0),
                        (self.options.opacity * 255 / 100) as u8,
                        LWA_ALPHA,
                    );
                }
            }
            self.error = Some(error.clone());
            return Err(error);
        }
        if !options.mouse_shield {
            self.mouse = None;
            if let Some(overlay) = self.overlay.take() {
                unsafe {
                    let _ = DestroyWindow(overlay);
                }
            }
        } else {
            if let Some(overlay) = new_overlay {
                self.overlay = Some(overlay);
                self.mouse = new_mouse;
            }
            if let Some(mouse) = &self.mouse {
                mouse.update(options);
            }
        }
        self.options = options;
        self.error = (options.system_menu && self.menu.is_none())
            .then(|| "目标没有标准关闭项，仅鼠标保护生效。".into());
        Ok(())
    }

    pub fn set_calibration(&mut self, hwnd: Option<HWND>) {
        self.calibrating = hwnd.is_some();
        if let Some(mouse) = &self.mouse {
            mouse.set_calibration(hwnd);
        }
        if self.calibrating
            && let Some(overlay) = self.overlay
        {
            unsafe {
                let _ = ShowWindow(overlay, SW_HIDE);
            }
        }
    }

    pub fn tick(&mut self) -> Result<(), String> {
        if !self.alive() {
            return Err("目标窗口已关闭或重建，保护已结束。".into());
        }
        if let Some(menu) = &self.menu {
            unsafe {
                if GetSystemMenu(self.target.hwnd, false) != menu.menu {
                    return Err("目标重建了系统菜单，请重新开启保护。".into());
                }
                let state = GetMenuState(menu.menu, SC_CLOSE, MF_BYCOMMAND);
                if state == u32::MAX {
                    return Err("目标移除了系统关闭命令，请改用鼠标遮罩。".into());
                }
                if state & (MF_DISABLED.0 | MF_GRAYED.0) == 0 {
                    set_close_state(self.target.hwnd, menu.menu, MF_GRAYED)?;
                }
            }
        }
        if let Some(overlay) = self.overlay {
            unsafe {
                let foreground = GetForegroundWindow();
                let active = GetAncestor(foreground, GA_ROOT) == self.target.hwnd;
                if self.calibrating
                    || !active
                    || IsIconic(self.target.hwnd).as_bool()
                    || !IsWindowVisible(self.target.hwnd).as_bool()
                    || is_cloaked(self.target.hwnd)
                {
                    let _ = ShowWindow(overlay, SW_HIDE);
                    return Ok(());
                }
                let rect = shield_rect(self.target.hwnd, self.options)?;
                SetWindowPos(
                    overlay,
                    Some(HWND_TOPMOST),
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                    SWP_NOACTIVATE | SWP_SHOWWINDOW,
                )
                .map_err(|error| format!("无法移动鼠标遮罩：{error}"))?;
            }
        }
        Ok(())
    }

    pub fn restore(&mut self) -> Result<(), String> {
        self.restore_with(set_close_state)
    }

    fn restore_with(
        &mut self,
        set_state: impl FnOnce(HWND, HMENU, MENU_ITEM_FLAGS) -> Result<(), String>,
    ) -> Result<(), String> {
        self.mouse = None;
        if let Some(overlay) = self.overlay.take() {
            unsafe {
                let _ = DestroyWindow(overlay);
            }
        }
        if !self.alive() {
            self.menu = None;
            self.clear_record()?;
            self.recovery_failed = false;
            return Ok(());
        }
        if let Some(menu) = &self.menu {
            unsafe {
                if GetSystemMenu(self.target.hwnd, false) == menu.menu
                    && GetMenuState(menu.menu, SC_CLOSE, MF_BYCOMMAND) != u32::MAX
                    && let Err(error) =
                        set_state(self.target.hwnd, menu.menu, menu.original_disabled)
                {
                    self.recovery_failed = true;
                    self.error = Some(error.clone());
                    return Err(error);
                }
            }
            self.menu = None;
        }
        if let Err(error) = self.clear_record() {
            self.recovery_failed = true;
            self.error = Some(error.clone());
            return Err(error);
        }
        unsafe {
            let _ = RemovePropW(self.target.hwnd, PCWSTR(self.marker.as_ptr()));
        }
        self.recovery_failed = false;
        Ok(())
    }

    fn clear_record(&mut self) -> Result<(), String> {
        if let Some(identity) = &self.pending {
            self.recovery.remove(identity)?;
            self.pending = None;
        }
        Ok(())
    }

    fn disable_close(&mut self) -> Result<(), String> {
        let menu = unsafe { GetSystemMenu(self.target.hwnd, false) };
        let original_disabled = original_close_state(self.target.hwnd)?;
        let identity = self
            .recovery
            .record(&self.target, original_disabled, &self.marker)?;
        self.pending = Some(identity.clone());
        if !self.alive()
            || !Identity::capture(&self.target).is_ok_and(|current| identity.matches(&current))
        {
            self.clear_record()?;
            return Err("目标窗口已关闭或重建，请重新选择。".into());
        }
        self.menu = Some(MenuProtection {
            menu,
            original_disabled: MENU_ITEM_FLAGS(original_disabled),
        });
        if let Err(error) = set_close_state(self.target.hwnd, menu, MF_GRAYED) {
            if set_close_state(self.target.hwnd, menu, MENU_ITEM_FLAGS(original_disabled)).is_ok() {
                self.menu = None;
                self.clear_record()?;
            } else {
                self.recovery_failed = true;
            }
            return Err(error);
        }
        Ok(())
    }
}

impl Drop for Protection {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

pub fn original_close_state(hwnd: HWND) -> Result<u32, String> {
    unsafe {
        let menu = GetSystemMenu(hwnd, false);
        let state = if menu.0.is_null() {
            u32::MAX
        } else {
            GetMenuState(menu, SC_CLOSE, MF_BYCOMMAND)
        };
        if state == u32::MAX {
            return Err("目标没有标准系统关闭命令。请取消“禁用系统关闭”，启用“鼠标遮罩”。".into());
        }
        Ok(state & (MF_DISABLED.0 | MF_GRAYED.0))
    }
}

pub fn restore_close_state(hwnd: HWND, state: u32) -> Result<(), String> {
    original_close_state(hwnd)?;
    set_close_state(
        hwnd,
        unsafe { GetSystemMenu(hwnd, false) },
        MENU_ITEM_FLAGS(state),
    )
}

fn set_close_state(hwnd: HWND, menu: HMENU, state: MENU_ITEM_FLAGS) -> Result<(), String> {
    unsafe {
        // EnableMenuItem returns the PREVIOUS flags: zero is a successful previously enabled item.
        if EnableMenuItem(menu, SC_CLOSE, MF_BYCOMMAND | state).0 == -1 {
            return Err("修改关闭命令失败。请检查目标窗口权限。".into());
        }
        let observed = GetMenuState(menu, SC_CLOSE, MF_BYCOMMAND);
        let mask = MF_DISABLED.0 | MF_GRAYED.0;
        if observed == u32::MAX || (observed & mask != 0) != (state.0 & mask != 0) {
            return Err("目标窗口拒绝了关闭命令修改。".into());
        }
        let _ = DrawMenuBar(hwnd);
        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_NOMOVE
                | SWP_NOSIZE
                | SWP_NOZORDER
                | SWP_NOACTIVATE
                | SWP_FRAMECHANGED
                | SWP_ASYNCWINDOWPOS,
        );
        Ok(())
    }
}

pub fn shield_rect(hwnd: HWND, options: Options) -> Result<RECT, String> {
    unsafe {
        let mut frame = RECT::default();
        if DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut frame as *mut _ as *mut _,
            size_of::<RECT>() as u32,
        )
        .is_err()
            || !valid_rect(frame)
        {
            GetWindowRect(hwnd, &mut frame).map_err(|error| error.to_string())?;
        }
        let mut origin = RECT::default();
        GetWindowRect(hwnd, &mut origin).map_err(|error| error.to_string())?;
        let mut buttons = RECT::default();
        let mut anchor = POINT {
            x: frame.right,
            y: frame.top,
        };
        if DwmGetWindowAttribute(
            hwnd,
            DWMWA_CAPTION_BUTTON_BOUNDS,
            &mut buttons as *mut _ as *mut _,
            size_of::<RECT>() as u32,
        )
        .is_ok()
            && valid_rect(buttons)
        {
            anchor.x = origin.left + buttons.right;
            anchor.y = origin.top + buttons.top;
        }
        let dpi = GetDpiForWindow(hwnd).max(96);
        Ok(rect_from_anchor(anchor, dpi, options))
    }
}

fn rect_from_anchor(anchor: POINT, dpi: u32, options: Options) -> RECT {
    let scale = |value: i32| (i64::from(value) * i64::from(dpi) / 96) as i32;
    let right = anchor.x - scale(options.right_offset);
    let top = anchor.y + scale(options.top_offset);
    RECT {
        left: right - scale(options.width),
        top,
        right,
        bottom: top + scale(options.height),
    }
}

pub fn register_overlay_class() -> windows::core::Result<()> {
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSW {
            style: CS_DBLCLKS,
            lpfnWndProc: Some(overlay_proc),
            hInstance: instance.into(),
            lpszClassName: OVERLAY_CLASS,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            return Err(windows::core::Error::from_thread());
        }
        Ok(())
    }
}

fn create_overlay(opacity: i32) -> windows::core::Result<HWND> {
    unsafe {
        // Deliberately omit WS_EX_TRANSPARENT: the shield must consume mouse clicks.
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            OVERLAY_CLASS,
            w!("KeepWindow shield"),
            WS_POPUP,
            0,
            0,
            46,
            32,
            None,
            None,
            Some(GetModuleHandleW(None)?.into()),
            None,
        )?;
        if let Err(error) =
            SetLayeredWindowAttributes(hwnd, COLORREF(0), (opacity * 255 / 100) as u8, LWA_ALPHA)
        {
            let _ = DestroyWindow(hwnd);
            return Err(error);
        }
        Ok(hwnd)
    }
}

unsafe extern "system" fn overlay_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match message {
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_NCHITTEST => LRESULT(HTCLIENT as isize),
            WM_LBUTTONDOWN | WM_LBUTTONUP | WM_LBUTTONDBLCLK | WM_RBUTTONDOWN | WM_RBUTTONUP
            | WM_RBUTTONDBLCLK | WM_MBUTTONDOWN | WM_MBUTTONUP | WM_MBUTTONDBLCLK
            | WM_NCLBUTTONDOWN | WM_NCLBUTTONUP | WM_NCLBUTTONDBLCLK => LRESULT(0),
            WM_PAINT => {
                let mut paint = PAINTSTRUCT::default();
                let dc = BeginPaint(hwnd, &mut paint);
                let mut rect = RECT::default();
                let _ = GetClientRect(hwnd, &mut rect);
                let brush = CreateSolidBrush(COLORREF(0x00328C18));
                FillRect(dc, &rect, brush);
                let _ = DeleteObject(HGDIOBJ(brush.0));
                let _ = EndPaint(hwnd, &paint);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        os::windows::process::CommandExt,
        process::{Child, Command, Stdio},
        time::{Duration, Instant},
    };

    struct TestProcess(Child);

    #[test]
    fn live_updates_switch_components_and_preserve_original_menu_state() {
        let _ = register_overlay_class();
        unsafe {
            let target = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("Live options test"),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                500,
                300,
                None,
                None,
                Some(GetModuleHandleW(None).unwrap().into()),
                None,
            )
            .unwrap();
            let mut guard = Protection::start(
                WindowInfo::inspect(target).unwrap(),
                Options {
                    mouse_shield: false,
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(guard.status().label(), "系统保护");
            guard
                .update_options(Options {
                    width: 80,
                    opacity: 45,
                    ..Default::default()
                })
                .unwrap();
            assert_eq!(guard.status().label(), "组合保护");
            assert_eq!(guard.options.width, 80);
            let failure = guard.restore_with(|_, _, _| Err("injected menu restore failure".into()));
            assert!(failure.is_err());
            assert_eq!(guard.status().label(), "恢复失败");
            assert!(guard.status().system_menu);
            assert!(!guard.status().mouse_shield);
            guard.restore().unwrap();
            guard = Protection::start(WindowInfo::inspect(target).unwrap(), Options::default())
                .unwrap();
            let previous = guard.options;
            assert!(
                guard
                    .update_options(Options {
                        width: 0,
                        ..previous
                    })
                    .is_err()
            );
            assert_eq!(guard.options, previous);
            guard
                .update_options(Options {
                    system_menu: false,
                    ..previous
                })
                .unwrap();
            assert_eq!(guard.status().label(), "鼠标保护");
            assert_eq!(
                GetMenuState(GetSystemMenu(target, false), SC_CLOSE, MF_BYCOMMAND)
                    & (MF_DISABLED.0 | MF_GRAYED.0),
                0
            );
            guard.restore().unwrap();
            DestroyWindow(target).unwrap();
        }
    }

    #[test]
    fn missing_menu_reports_only_actual_mouse_protection() {
        let _ = register_overlay_class();
        unsafe {
            let target = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("No system menu"),
                WS_POPUP,
                0,
                0,
                500,
                300,
                None,
                None,
                Some(GetModuleHandleW(None).unwrap().into()),
                None,
            )
            .unwrap();
            let mut guard =
                Protection::start(WindowInfo::inspect(target).unwrap(), Options::default())
                    .unwrap();
            assert_eq!(guard.status().label(), "鼠标保护");
            assert!(guard.status().error.is_some());
            guard.restore().unwrap();
            DestroyWindow(target).unwrap();
        }
    }

    impl Drop for TestProcess {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    unsafe extern "system" fn test_target_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        unsafe {
            if message == WM_DESTROY {
                PostQuitMessage(0);
                LRESULT(0)
            } else {
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
        }
    }

    #[test]
    fn native_menu_works_across_processes_and_direct_close_bypasses_it() {
        const CHILD_TITLE: &str = "KEEP_WINDOW_TEST_CHILD_TITLE";
        if let Some(title) = std::env::var_os(CHILD_TITLE) {
            unsafe {
                let instance = GetModuleHandleW(None).unwrap();
                let class = WNDCLASSW {
                    hInstance: instance.into(),
                    lpszClassName: w!("KeepWindow.Foreign.Test"),
                    lpfnWndProc: Some(test_target_proc),
                    ..Default::default()
                };
                assert_ne!(RegisterClassW(&class), 0);
                let title = wide(&title.to_string_lossy());
                CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    w!("KeepWindow.Foreign.Test"),
                    PCWSTR(title.as_ptr()),
                    WS_OVERLAPPEDWINDOW,
                    0,
                    0,
                    500,
                    300,
                    None,
                    None,
                    Some(instance.into()),
                    None,
                )
                .unwrap();
                let mut message = MSG::default();
                while GetMessageW(&mut message, None, 0, 0).0 > 0 {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            return;
        }

        let options = Options {
            mouse_shield: false,
            ..Options::default()
        };
        let title = format!("KeepWindow foreign test {}", std::process::id());
        let mut child = TestProcess(Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "protection::tests::native_menu_works_across_processes_and_direct_close_bypasses_it"])
            .env(CHILD_TITLE, &title)
            .creation_flags(windows::Win32::System::Threading::CREATE_NO_WINDOW.0)
            .stdout(Stdio::null())
            .spawn().unwrap());
        let title = wide(&title);
        let deadline = Instant::now() + Duration::from_secs(5);
        let target = loop {
            if let Ok(target) = unsafe { FindWindowW(None, PCWSTR(title.as_ptr())) } {
                break target;
            }
            assert!(
                Instant::now() < deadline,
                "Foreign test window did not start"
            );
            std::thread::sleep(Duration::from_millis(20));
        };
        let info = WindowInfo::inspect(target).unwrap();
        assert_ne!(info.pid, std::process::id());
        let menu = unsafe { GetSystemMenu(target, false) };
        let mut guard = Protection::start(info.clone(), options).unwrap();
        unsafe {
            assert_ne!(GetMenuState(menu, SC_CLOSE, MF_BYCOMMAND) & MF_GRAYED.0, 0);
        }
        guard.restore().unwrap();
        unsafe {
            assert_eq!(
                GetMenuState(menu, SC_CLOSE, MF_BYCOMMAND) & (MF_DISABLED.0 | MF_GRAYED.0),
                0
            );
        }
        let guard = Protection::start(info, options).unwrap();
        unsafe {
            PostMessageW(Some(target), WM_CLOSE, WPARAM(0), LPARAM(0)).unwrap();
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(exit) = child.0.try_wait().unwrap() {
                assert!(exit.success());
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Foreign test window did not close"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!guard.alive());
    }

    #[test]
    fn shield_geometry_scales_and_supports_negative_monitor_coordinates() {
        let options = Options {
            width: 46,
            height: 32,
            right_offset: 4,
            top_offset: 2,
            ..Options::default()
        };
        let rect = rect_from_anchor(POINT { x: -100, y: -200 }, 144, options);
        assert_eq!(
            (rect.left, rect.top, rect.right, rect.bottom),
            (-175, -197, -106, -149)
        );
    }

    #[test]
    fn native_menu_restores_previous_state_and_survives_destroyed_target() {
        let options = Options {
            mouse_shield: false,
            ..Options::default()
        };
        unsafe {
            let target = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("KeepWindow test target"),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                500,
                300,
                None,
                None,
                Some(GetModuleHandleW(None).unwrap().into()),
                None,
            )
            .unwrap();
            let menu = GetSystemMenu(target, false);
            let info = WindowInfo::inspect(target).unwrap();
            let mut guard = Protection::start(info.clone(), options).unwrap();
            assert_ne!(GetMenuState(menu, SC_CLOSE, MF_BYCOMMAND) & MF_GRAYED.0, 0);
            // Reapply protection when an application re-enables its Close item.
            let _ = EnableMenuItem(menu, SC_CLOSE, MF_BYCOMMAND | MF_ENABLED);
            guard.tick().unwrap();
            assert_ne!(GetMenuState(menu, SC_CLOSE, MF_BYCOMMAND) & MF_GRAYED.0, 0);
            guard.restore().unwrap();
            assert_eq!(
                GetMenuState(menu, SC_CLOSE, MF_BYCOMMAND) & (MF_DISABLED.0 | MF_GRAYED.0),
                0
            );
            let _ = EnableMenuItem(menu, SC_CLOSE, MF_BYCOMMAND | MF_GRAYED);
            let mut guard = Protection::start(info.clone(), options).unwrap();
            guard.restore().unwrap();
            assert_ne!(GetMenuState(menu, SC_CLOSE, MF_BYCOMMAND) & MF_GRAYED.0, 0);
            let guard = Protection::start(info, options).unwrap();
            DestroyWindow(target).unwrap();
            assert!(!guard.alive());
            drop(guard);
        }
    }

    #[test]
    fn shield_consumes_mouse_without_activation_and_is_destroyed_on_restore() {
        let _ = register_overlay_class();
        unsafe {
            let target = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("Shield test target"),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                500,
                300,
                None,
                None,
                Some(GetModuleHandleW(None).unwrap().into()),
                None,
            )
            .unwrap();
            let options = Options {
                mouse_shield: true,
                system_menu: false,
                ..Options::default()
            };
            let mut guard =
                Protection::start(WindowInfo::inspect(target).unwrap(), options).unwrap();
            let overlay = guard.overlay.unwrap();
            let style = GetWindowLongPtrW(overlay, GWL_EXSTYLE) as u32;
            assert_ne!(style & WS_EX_NOACTIVATE.0, 0);
            assert_eq!(style & WS_EX_TRANSPARENT.0, 0);
            assert_eq!(
                SendMessageW(overlay, WM_MOUSEACTIVATE, None, None).0,
                MA_NOACTIVATE as isize
            );
            assert_eq!(SendMessageW(overlay, WM_LBUTTONDOWN, None, None).0, 0);
            assert!(!IsWindowVisible(overlay).as_bool());
            guard.restore().unwrap();
            assert!(!IsWindow(Some(overlay)).as_bool());
            DestroyWindow(target).unwrap();
        }
    }
}
