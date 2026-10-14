#![cfg(windows)]

use std::{
    os::windows::process::CommandExt,
    process::{Child, Command},
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        System::{LibraryLoader::GetModuleHandleW, Threading::CREATE_NO_WINDOW},
        UI::WindowsAndMessaging::*,
    },
    core::{PCWSTR, w},
};

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        if self.0.try_wait().unwrap().is_none() {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}
fn wait(mut check: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !check() {
        assert!(Instant::now() < deadline, "runtime condition timed out");
        std::thread::sleep(Duration::from_millis(20));
    }
}
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}
fn find(title: &str) -> Option<HWND> {
    unsafe { FindWindowW(None, PCWSTR(wide(title).as_ptr())).ok() }
}
fn launch(profile: &std::path::Path) -> Process {
    Process(
        Command::new(env!("CARGO_BIN_EXE_keep-window"))
            .arg("--background")
            .env("LOCALAPPDATA", profile)
            .creation_flags(CREATE_NO_WINDOW.0)
            .spawn()
            .unwrap(),
    )
}
unsafe extern "system" fn target_proc(
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
fn automatic_protection_survives_restart_and_respects_manual_restore() {
    const CHILD_TITLE: &str = "KEEP_WINDOW_AUTO_TEST_TITLE";
    if let Ok(title) = std::env::var(CHILD_TITLE) {
        unsafe {
            let instance = GetModuleHandleW(None).unwrap();
            let class = WNDCLASSW {
                lpfnWndProc: Some(target_proc),
                hInstance: instance.into(),
                lpszClassName: w!("KeepWindow.Auto.Test"),
                ..Default::default()
            };
            assert_ne!(RegisterClassW(&class), 0);
            let title = wide(&title);
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                class.lpszClassName,
                PCWSTR(title.as_ptr()),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                20,
                20,
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
    let title = format!("KeepWindow automatic target {}", std::process::id());
    let child = Process(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "automatic_protection_survives_restart_and_respects_manual_restore",
            ])
            .env(CHILD_TITLE, &title)
            .creation_flags(CREATE_NO_WINDOW.0)
            .spawn()
            .unwrap(),
    );
    wait(|| find(&title).is_some());
    let target = find(&title).unwrap();
    let profile = std::env::temp_dir().join(format!("keep-window-auto-{}", std::process::id()));
    let directory = profile.join("KeepWindow");
    std::fs::create_dir_all(&directory).unwrap();
    let config = serde_json::json!({ "version": 1, "rules": [{ "path": std::env::current_exe().unwrap().to_string_lossy(), "class": "KeepWindow.Auto.Test", "title_contains": "automatic target", "automatic": true, "options": { "system_menu": true, "mouse_shield": false } }] });
    std::fs::write(
        directory.join("config.json"),
        serde_json::to_vec(&config).unwrap(),
    )
    .unwrap();
    let mut app = launch(&profile);
    let disabled = || unsafe {
        GetMenuState(GetSystemMenu(target, false), SC_CLOSE, MF_BYCOMMAND)
            & (MF_DISABLED.0 | MF_GRAYED.0)
            != 0
    };
    wait(disabled);
    // The profile-specific property identifies the background main window by PID.
    fn main_window(pid: u32) -> Option<HWND> {
        struct Search {
            pid: u32,
            hwnd: Option<HWND>,
        }
        unsafe extern "system" fn callback(hwnd: HWND, data: LPARAM) -> windows::core::BOOL {
            unsafe {
                let state = &mut *(data.0 as *mut Search);
                let mut pid = 0;
                GetWindowThreadProcessId(hwnd, Some(&mut pid));
                let mut title = [0u16; 64];
                let length = GetWindowTextW(hwnd, &mut title);
                if pid == state.pid
                    && String::from_utf16_lossy(&title[..length as usize]) == "KeepWindow"
                {
                    state.hwnd = Some(hwnd);
                    return windows::core::BOOL(0);
                }
                windows::core::BOOL(1)
            }
        }
        let mut state = Search { pid, hwnd: None };
        unsafe {
            let _ = EnumWindows(Some(callback), LPARAM(&mut state as *mut Search as isize));
        }
        state.hwnd
    }
    wait(|| main_window(app.0.id()).is_some());
    let main = main_window(app.0.id()).unwrap();
    unsafe {
        PostMessageW(Some(main), WM_COMMAND, WPARAM(0xA102), LPARAM(0)).unwrap();
    }
    wait(|| !disabled());
    let deadline = Instant::now() + Duration::from_millis(1500);
    while Instant::now() < deadline {
        assert!(!disabled(), "manual restore was immediately undone");
        std::thread::sleep(Duration::from_millis(20));
    }
    unsafe {
        PostMessageW(Some(main), WM_COMMAND, WPARAM(0xA103), LPARAM(0)).unwrap();
    }
    wait(|| app.0.try_wait().unwrap().is_some());
    let mut restarted = launch(&profile);
    wait(disabled);
    wait(|| main_window(restarted.0.id()).is_some());
    unsafe {
        PostMessageW(
            main_window(restarted.0.id()),
            WM_COMMAND,
            WPARAM(0xA103),
            LPARAM(0),
        )
        .unwrap();
    }
    wait(|| restarted.0.try_wait().unwrap().is_some());
    assert!(!disabled());
    drop(child);
    std::fs::remove_file(directory.join("config.json")).unwrap();
    std::fs::remove_dir(directory).unwrap();
    std::fs::remove_dir(profile).unwrap();
}
