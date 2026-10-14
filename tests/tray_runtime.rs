#![cfg(windows)]

use std::{
    os::windows::process::CommandExt,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, WPARAM},
        System::Threading::CREATE_NO_WINDOW,
        UI::WindowsAndMessaging::*,
    },
    core::BOOL,
};

struct TestApp(Child);

impl Drop for TestApp {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

fn wait_for(label: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if condition() {
            return;
        }
        assert!(Instant::now() < deadline, "Timed out: {label}");
        thread::sleep(Duration::from_millis(20));
    }
}

fn main_window(pid: u32) -> Option<HWND> {
    struct Search {
        pid: u32,
        found: Option<HWND>,
    }
    unsafe extern "system" fn callback(hwnd: HWND, data: LPARAM) -> BOOL {
        unsafe {
            let search = &mut *(data.0 as *mut Search);
            let mut pid = 0;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            let mut title = [0u16; 64];
            let length = GetWindowTextW(hwnd, &mut title).max(0) as usize;
            if pid == search.pid && String::from_utf16_lossy(&title[..length]) == "KeepWindow" {
                search.found = Some(hwnd);
                return BOOL(0);
            }
            BOOL(1)
        }
    }
    let mut search = Search { pid, found: None };
    unsafe {
        let _ = EnumWindows(Some(callback), LPARAM(&mut search as *mut _ as isize));
    }
    search.found
}

#[test]
fn background_app_handles_tray_minimize_close_restore_and_exit_without_rendering() {
    let profile = std::env::temp_dir().join(format!("keep-window-runtime-{}", std::process::id()));
    std::fs::create_dir_all(&profile).unwrap();
    let mut app = TestApp(
        Command::new(env!("CARGO_BIN_EXE_keep-window"))
            .arg("--background")
            .env("LOCALAPPDATA", &profile)
            .creation_flags(CREATE_NO_WINDOW.0)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let mut hwnd = None;
    wait_for("background window creation", || {
        hwnd = main_window(app.0.id());
        hwnd.is_some()
    });
    let hwnd = hwnd.unwrap();
    unsafe {
        assert!(
            !IsWindowVisible(hwnd).as_bool(),
            "--background showed the main window"
        );
        let mut duplicate = Command::new(env!("CARGO_BIN_EXE_keep-window"))
            .env("LOCALAPPDATA", &profile)
            .creation_flags(CREATE_NO_WINDOW.0)
            .spawn()
            .unwrap();
        wait_for("duplicate process exit", || {
            duplicate.try_wait().unwrap().is_some()
        });
        wait_for("duplicate wakes existing window", || {
            IsWindowVisible(hwnd).as_bool()
        });
        PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)).unwrap();
        wait_for("hide after duplicate", || !IsWindowVisible(hwnd).as_bool());
        let mut duplicate_background = Command::new(env!("CARGO_BIN_EXE_keep-window"))
            .arg("--background")
            .env("LOCALAPPDATA", &profile)
            .creation_flags(CREATE_NO_WINDOW.0)
            .spawn()
            .unwrap();
        wait_for("background duplicate exit", || {
            duplicate_background.try_wait().unwrap().is_some()
        });
        assert!(!IsWindowVisible(hwnd).as_bool());
        // The native menu uses these WM_COMMAND IDs; exercise the same path as a real tray click.
        PostMessageW(Some(hwnd), WM_COMMAND, WPARAM(0xA101), LPARAM(0)).unwrap();
        wait_for("tray show from hidden startup", || {
            IsWindowVisible(hwnd).as_bool()
        });
        PostMessageW(
            Some(hwnd),
            WM_SYSCOMMAND,
            WPARAM(SC_MAXIMIZE as usize),
            LPARAM(0),
        )
        .unwrap();
        wait_for("maximize", || IsZoomed(hwnd).as_bool());
        PostMessageW(
            Some(hwnd),
            WM_SYSCOMMAND,
            WPARAM(SC_MINIMIZE as usize),
            LPARAM(0),
        )
        .unwrap();
        wait_for("minimize to tray", || !IsWindowVisible(hwnd).as_bool());
        assert!(IsWindow(Some(hwnd)).as_bool());
        PostMessageW(Some(hwnd), WM_COMMAND, WPARAM(0xA101), LPARAM(0)).unwrap();
        wait_for("tray restore after minimize", || {
            IsWindowVisible(hwnd).as_bool()
        });
        assert!(IsZoomed(hwnd).as_bool(), "Tray show lost maximized bounds");
        PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)).unwrap();
        wait_for("close to tray", || !IsWindowVisible(hwnd).as_bool());
        assert!(IsWindow(Some(hwnd)).as_bool());
        PostMessageW(Some(hwnd), WM_COMMAND, WPARAM(0xA102), LPARAM(0)).unwrap();
        PostMessageW(Some(hwnd), WM_COMMAND, WPARAM(0xA103), LPARAM(0)).unwrap();
    }
    let mut exit = None;
    wait_for("graceful tray exit from hidden window", || {
        exit = app.0.try_wait().unwrap();
        exit.is_some()
    });
    assert!(exit.unwrap().success());
    std::fs::remove_dir(profile).unwrap();
}
