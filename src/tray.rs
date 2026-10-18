use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    mem::size_of,
    rc::Rc,
};

use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM},
        UI::{
            Shell::{
                DefSubclassProc, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP,
                NIIF_WARNING, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NIN_SELECT,
                NOTIFYICON_VERSION_4, NOTIFYICONDATAW, RemoveWindowSubclass, SetWindowSubclass,
                Shell_NotifyIconW,
            },
            WindowsAndMessaging::*,
        },
    },
    core::w,
};

use crate::icon::WindowIcons;

const TRAY_MESSAGE: u32 = WM_APP + 41;
const HIDE_MESSAGE: u32 = WM_APP + 42;
const TRAY_ID: u32 = 1;
const SUBCLASS_ID: usize = 0x4B57;
const SHOW_COMMAND: usize = 0xA101;
const RESTORE_COMMAND: usize = 0xA102;
const EXIT_COMMAND: usize = 0xA103;
const NIN_KEYSELECT: u32 = NIN_SELECT + 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Show,
    RestoreAll,
    Exit,
    Toggle(HWND),
    Release(HWND),
}

struct State {
    data: Cell<NOTIFYICONDATAW>,
    icons: RefCell<WindowIcons>,
    taskbar_created: u32,
    show_message: u32,
    actions: RefCell<VecDeque<Action>>,
}

impl State {
    fn add_icon(&self) -> bool {
        unsafe {
            let data = self.data.get();
            if !Shell_NotifyIconW(NIM_ADD, &data).as_bool() {
                return false;
            }
            let mut version = data;
            version.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            if !Shell_NotifyIconW(NIM_SETVERSION, &version).as_bool() {
                let _ = Shell_NotifyIconW(NIM_DELETE, &data);
                return false;
            }
        }
        true
    }

    fn update_icons(&self, dpi: u32) {
        let mut icons = self.icons.borrow_mut();
        if icons.update_dpi(dpi).is_ok() {
            let mut data = self.data.get();
            data.hIcon = icons.small();
            self.data.set(data);
            unsafe {
                let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
            }
        }
    }

    fn queue(&self, action: Action) {
        self.actions.borrow_mut().push_back(action);
    }

    fn command(&self, command: usize) -> bool {
        match command {
            SHOW_COMMAND => self.queue(Action::Show),
            RESTORE_COMMAND => self.queue(Action::RestoreAll),
            EXIT_COMMAND => self.queue(Action::Exit),
            _ => return false,
        }
        true
    }

    fn show_menu(&self) {
        unsafe {
            let Ok(menu) = CreatePopupMenu() else {
                return;
            };
            let result = (|| -> windows::core::Result<()> {
                let show = crate::window::wide(&crate::locale::text("show_main"));
                let restore = crate::window::wide(&crate::locale::text("restore_all"));
                let exit = crate::window::wide(&crate::locale::text("exit_restore"));
                AppendMenuW(
                    menu,
                    MF_STRING,
                    SHOW_COMMAND,
                    windows::core::PCWSTR(show.as_ptr()),
                )?;
                AppendMenuW(
                    menu,
                    MF_STRING,
                    RESTORE_COMMAND,
                    windows::core::PCWSTR(restore.as_ptr()),
                )?;
                AppendMenuW(menu, MF_SEPARATOR, 0, None)?;
                AppendMenuW(
                    menu,
                    MF_STRING,
                    EXIT_COMMAND,
                    windows::core::PCWSTR(exit.as_ptr()),
                )?;
                let mut cursor = POINT::default();
                GetCursorPos(&mut cursor)?;
                let hwnd = self.data.get().hWnd;
                let _ = SetForegroundWindow(hwnd);
                let _ = TrackPopupMenu(menu, TPM_RIGHTBUTTON, cursor.x, cursor.y, None, hwnd, None);
                // Complete the popup's foreground ownership so clicking elsewhere dismisses it.
                let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
                Ok(())
            })();
            let _ = result;
            let _ = DestroyMenu(menu);
        }
    }
}

pub struct Tray {
    state: Rc<State>,
}

impl Tray {
    pub fn new(hwnd: HWND) -> Result<Self, String> {
        let icons = WindowIcons::new(hwnd).map_err(|error| {
            crate::locale::format(
                "cannot_load_application_icon_error",
                &[("error", &format!("{}", error))],
            )
        })?;
        let mut data = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: TRAY_ID,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP,
            uCallbackMessage: TRAY_MESSAGE,
            hIcon: icons.small(),
            ..Default::default()
        };
        let tooltip = crate::window::wide(&format!(
            "KeepWindow - {}",
            crate::locale::text("window_protection")
        ));
        data.szTip[..tooltip.len()].copy_from_slice(&tooltip);
        let state = Rc::new(State {
            data: Cell::new(data),
            icons: RefCell::new(icons),
            taskbar_created: unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) },
            show_message: crate::instance::show_message(),
            actions: RefCell::new(VecDeque::new()),
        });
        unsafe {
            let message = state.show_message;
            if message == 0 {
                return Err(crate::locale::text(
                    "cannot_register_instance_activation_message",
                ));
            }
            ChangeWindowMessageFilterEx(hwnd, message, MSGFLT_ALLOW, None).map_err(|e| {
                crate::locale::format(
                    "cannot_set_instance_activation_permissions_e",
                    &[("e", &format!("{}", e))],
                )
            })?;
            if !SetWindowSubclass(
                hwnd,
                Some(tray_proc),
                SUBCLASS_ID,
                Rc::as_ptr(&state) as usize,
            )
            .as_bool()
            {
                return Err(crate::locale::text(
                    "cannot_install_main_window_tray_handler",
                ));
            }
        }
        let tray = Self { state };
        if !tray.state.add_icon() {
            return Err(crate::locale::text("cannot_add_tray_icon_the_main_window"));
        }
        unsafe {
            SetPropW(
                hwnd,
                windows::core::PCWSTR(crate::instance::property().as_ptr()),
                Some(windows::Win32::Foundation::HANDLE(std::ptr::dangling_mut())),
            )
            .map_err(|e| {
                crate::locale::format("cannot_mark_main_window_e", &[("e", &format!("{}", e))])
            })?;
        }
        Ok(tray)
    }

    pub fn refresh_language(&self) {
        let mut data = self.state.data.get();
        data.szTip.fill(0);
        let text = crate::window::wide(&format!(
            "KeepWindow - {}",
            crate::locale::text("window_protection")
        ));
        let length = text.len().min(data.szTip.len() - 1);
        data.szTip[..length].copy_from_slice(&text[..length]);
        self.state.data.set(data);
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
        }
    }
    pub fn hide(&self) {
        unsafe {
            let _ = ShowWindow(self.state.data.get().hWnd, SW_HIDE);
        }
    }

    pub fn show(&self) {
        unsafe {
            let hwnd = self.state.data.get().hWnd;
            let command = if IsIconic(hwnd).as_bool() {
                SW_RESTORE
            } else {
                SW_SHOW
            };
            let _ = ShowWindow(hwnd, command);
            let _ = SetForegroundWindow(hwnd);
        }
    }

    pub fn take_actions(&self) -> Vec<Action> {
        self.state.actions.borrow_mut().drain(..).collect()
    }

    pub fn hwnd(&self) -> HWND {
        self.state.data.get().hWnd
    }

    pub fn visible(&self) -> bool {
        unsafe { IsWindowVisible(self.hwnd()).as_bool() }
    }

    pub fn notify(&self, text: &str) {
        let mut data = self.state.data.get();
        data.uFlags = NIF_INFO;
        data.dwInfoFlags = NIIF_WARNING;
        let title = crate::window::wide(&format!(
            "KeepWindow {}",
            crate::locale::text("protection_notice")
        ));
        data.szInfoTitle[..title.len()].copy_from_slice(&title);
        let text: Vec<u16> = text.encode_utf16().take(data.szInfo.len() - 1).collect();
        data.szInfo[..text.len()].copy_from_slice(&text);
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        unsafe {
            let data = self.state.data.get();
            let _ = RemovePropW(
                data.hWnd,
                windows::core::PCWSTR(crate::instance::property().as_ptr()),
            );
            let _ = RemoveWindowSubclass(data.hWnd, Some(tray_proc), SUBCLASS_ID);
            let _ = Shell_NotifyIconW(NIM_DELETE, &data);
        }
    }
}

unsafe extern "system" fn tray_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    data: usize,
) -> LRESULT {
    unsafe {
        // A popup runs a nested message loop; retain state even if shutdown removes the Tray.
        let pointer = data as *const State;
        Rc::increment_strong_count(pointer);
        let state = Rc::from_raw(pointer);
        if message == state.show_message {
            let command = if IsIconic(hwnd).as_bool() {
                SW_RESTORE
            } else {
                SW_SHOW
            };
            let _ = ShowWindow(hwnd, command);
            let _ = SetForegroundWindow(hwnd);
            return LRESULT(1);
        }
        if state.taskbar_created != 0 && message == state.taskbar_created {
            if !state.add_icon() {
                // Keep an access path to the app if Explorer cannot recreate the icon.
                state.queue(Action::Show);
            }
            return LRESULT(0);
        }
        match message {
            WM_HOTKEY => {
                let target = GetAncestor(GetForegroundWindow(), GA_ROOT);
                match wparam.0 & 0xF {
                    0 => state.queue(Action::Toggle(target)),
                    1 => state.queue(Action::Release(target)),
                    2 => state.queue(Action::RestoreAll),
                    _ => {}
                }
                LRESULT(0)
            }
            WM_DPICHANGED => {
                state.update_icons((wparam.0 & 0xFFFF) as u32);
                DefSubclassProc(hwnd, message, wparam, lparam)
            }
            WM_CLOSE | HIDE_MESSAGE => {
                let _ = ShowWindow(hwnd, SW_HIDE);
                LRESULT(0)
            }
            WM_SYSCOMMAND if wparam.0 as u32 & 0xFFF0 == SC_MINIMIZE => {
                let _ = ShowWindow(hwnd, SW_HIDE);
                LRESULT(0)
            }
            WM_SIZE if wparam.0 as u32 == SIZE_MINIMIZED => {
                // Avoid re-entering GPUI's resize handler while it processes WM_SIZE.
                let _ = PostMessageW(Some(hwnd), HIDE_MESSAGE, WPARAM(0), LPARAM(0));
                DefSubclassProc(hwnd, message, wparam, lparam)
            }
            TRAY_MESSAGE => {
                match lparam.0 as u32 & 0xFFFF {
                    WM_LBUTTONDBLCLK | NIN_SELECT | NIN_KEYSELECT => state.queue(Action::Show),
                    WM_CONTEXTMENU => state.show_menu(),
                    _ => {}
                }
                LRESULT(0)
            }
            WM_COMMAND if lparam.0 == 0 && state.command(wparam.0 & 0xFFFF) => LRESULT(0),
            WM_NCDESTROY => {
                let _ = RemoveWindowSubclass(hwnd, Some(tray_proc), SUBCLASS_ID);
                DefSubclassProc(hwnd, message, wparam, lparam)
            }
            _ => DefSubclassProc(hwnd, message, wparam, lparam),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        protection::{Options, Protection},
        window::WindowInfo,
    };

    #[test]
    fn close_and_minimize_keep_window_and_protection_alive_and_drop_unhooks() {
        unsafe {
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("KeepWindow tray test"),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                400,
                300,
                None,
                None,
                None,
                None,
            )
            .unwrap();
            let mut protection = Protection::start(
                WindowInfo::inspect(hwnd).unwrap(),
                Options {
                    mouse_shield: false,
                    ..Options::default()
                },
            )
            .unwrap();
            let original_icons = [ICON_SMALL, ICON_BIG]
                .map(|kind| SendMessageW(hwnd, WM_GETICON, Some(WPARAM(kind as usize)), None));
            let tray = Tray::new(hwnd).unwrap();
            for kind in [ICON_SMALL, ICON_BIG] {
                assert_ne!(
                    SendMessageW(hwnd, WM_GETICON, Some(WPARAM(kind as usize)), None).0,
                    0
                );
            }
            tray.state.update_icons(144);
            assert_eq!(
                SendMessageW(hwnd, WM_GETICON, Some(WPARAM(ICON_SMALL as usize)), None).0,
                tray.state.data.get().hIcon.0 as isize
            );
            SendMessageW(hwnd, WM_CLOSE, None, None);
            assert!(IsWindow(Some(hwnd)).as_bool());
            assert!(!IsWindowVisible(hwnd).as_bool());
            SendMessageW(
                hwnd,
                WM_SYSCOMMAND,
                Some(WPARAM(SC_MINIMIZE as usize)),
                None,
            );
            assert!(IsWindow(Some(hwnd)).as_bool());
            protection.tick().unwrap();
            assert_ne!(
                GetMenuState(GetSystemMenu(hwnd, false), SC_CLOSE, MF_BYCOMMAND) & MF_GRAYED.0,
                0
            );
            SendMessageW(
                hwnd,
                TRAY_MESSAGE,
                None,
                Some(LPARAM(NIN_KEYSELECT as isize)),
            );
            assert_eq!(tray.take_actions(), vec![Action::Show]);
            assert!(tray.take_actions().is_empty());
            SendMessageW(hwnd, WM_COMMAND, Some(WPARAM(RESTORE_COMMAND)), None);
            SendMessageW(hwnd, WM_COMMAND, Some(WPARAM(EXIT_COMMAND)), None);
            assert_eq!(tray.take_actions(), vec![Action::RestoreAll, Action::Exit]);
            protection.restore().unwrap();
            drop(tray);
            for (kind, original) in [ICON_SMALL, ICON_BIG].into_iter().zip(original_icons) {
                assert_eq!(
                    SendMessageW(hwnd, WM_GETICON, Some(WPARAM(kind as usize)), None),
                    original
                );
            }
            SendMessageW(hwnd, WM_CLOSE, None, None);
            assert!(!IsWindow(Some(hwnd)).as_bool());
        }
    }
}
