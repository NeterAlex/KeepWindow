use std::{
    cell::RefCell,
    sync::{Arc, Mutex, OnceLock, Weak, mpsc},
    thread::{self, JoinHandle},
};

use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM},
        System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
        UI::WindowsAndMessaging::*,
    },
    core::PCWSTR,
};

use crate::{
    protection::{Options, shield_rect},
    window::is_cloaked,
};

#[derive(Clone)]
struct Target {
    hwnd: usize,
    overlay: usize,
    marker: Vec<u16>,
    options: Options,
}

#[derive(Default)]
struct Registry {
    targets: Vec<Target>,
}

struct Service {
    registry: Arc<Mutex<Registry>>,
    thread_id: u32,
    worker: Option<JoinHandle<()>>,
}

static SERVICE: OnceLock<Mutex<Weak<Service>>> = OnceLock::new();

thread_local! {
    static HOOK_STATE: RefCell<Option<(Arc<Mutex<Registry>>, ClickFilter)>> = const { RefCell::new(None) };
}

// The hook sees two down/up pairs for a double click; latch each swallowed press
// until its release even if the cursor leaves the protected area in between.
#[derive(Default)]
struct ClickFilter {
    pressed: [bool; 4],
}

impl ClickFilter {
    fn filter(&mut self, message: u32, x_button: u16, hit: impl FnOnce() -> bool) -> bool {
        let (button, down) = match message {
            WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => (0, true),
            WM_LBUTTONUP => (0, false),
            WM_RBUTTONDOWN | WM_RBUTTONDBLCLK => (1, true),
            WM_RBUTTONUP => (1, false),
            WM_MBUTTONDOWN | WM_MBUTTONDBLCLK => (2, true),
            WM_MBUTTONUP => (2, false),
            WM_XBUTTONDOWN | WM_XBUTTONDBLCLK => (if x_button == 1 { 2 } else { 3 }, true),
            WM_XBUTTONUP => (if x_button == 1 { 2 } else { 3 }, false),
            _ => return false,
        };
        if down {
            self.pressed[button] = hit();
            self.pressed[button]
        } else {
            std::mem::take(&mut self.pressed[button])
        }
    }
}

pub struct MouseRegistration {
    service: Arc<Service>,
    marker: Vec<u16>,
}

impl MouseRegistration {
    pub fn new(
        hwnd: HWND,
        overlay: Option<HWND>,
        marker: &[u16],
        options: Options,
    ) -> Result<Self, String> {
        let service = Service::get()?;
        service
            .registry
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .targets
            .push(Target {
                hwnd: hwnd.0 as usize,
                overlay: overlay.map_or(0, |hwnd| hwnd.0 as usize),
                marker: marker.to_vec(),
                options,
            });
        Ok(Self {
            service,
            marker: marker.to_vec(),
        })
    }
}

impl Drop for MouseRegistration {
    fn drop(&mut self) {
        self.service
            .registry
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .targets
            .retain(|target| target.marker != self.marker);
    }
}

impl Service {
    fn get() -> Result<Arc<Self>, String> {
        let mut shared = SERVICE
            .get_or_init(|| Mutex::new(Weak::new()))
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(service) = shared.upgrade() {
            return Ok(service);
        }
        let registry = Arc::new(Mutex::new(Registry::default()));
        let hook_registry = registry.clone();
        let (ready, started) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || unsafe {
            let mut message = MSG::default();
            let _ = PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE);
            HOOK_STATE
                .with(|slot| *slot.borrow_mut() = Some((hook_registry, ClickFilter::default())));
            let hook = SetWindowsHookExW(
                WH_MOUSE_LL,
                Some(mouse_proc),
                GetModuleHandleW(None).ok().map(Into::into),
                0,
            );
            match hook {
                Ok(hook) => {
                    if ready.send(Ok(GetCurrentThreadId())).is_ok() {
                        while GetMessageW(&mut message, None, 0, 0).0 > 0 {
                            let _ = TranslateMessage(&message);
                            DispatchMessageW(&message);
                        }
                    }
                    let _ = UnhookWindowsHookEx(hook);
                }
                Err(error) => {
                    let _ = ready.send(Err(format!("无法安装鼠标保护：{error}")));
                }
            }
            HOOK_STATE.with(|slot| *slot.borrow_mut() = None);
        });
        match started.recv().map_err(|error| error.to_string())? {
            Ok(thread_id) => {
                let service = Arc::new(Self {
                    registry,
                    thread_id,
                    worker: Some(worker),
                });
                *shared = Arc::downgrade(&service);
                Ok(service)
            }
            Err(error) => {
                let _ = worker.join();
                Err(error)
            }
        }
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        unsafe {
            let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn hit_target(target: &Target, point: POINT, root: HWND) -> bool {
    let hwnd = HWND(target.hwnd as *mut _);
    unsafe {
        if (root.0 as usize != target.hwnd && root.0 as usize != target.overlay)
            || !IsWindowVisible(hwnd).as_bool()
            || IsIconic(hwnd).as_bool()
            || is_cloaked(hwnd)
            || GetPropW(hwnd, PCWSTR(target.marker.as_ptr())).0 as usize != 1
        {
            return false;
        }
    }
    shield_rect(hwnd, target.options).is_ok_and(|rect| {
        point.x >= rect.left && point.x < rect.right && point.y >= rect.top && point.y < rect.bottom
    })
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if code == HC_ACTION as i32 {
            let event = &*(lparam.0 as *const MSLLHOOKSTRUCT);
            let blocked = HOOK_STATE.with(|slot| {
                let mut slot = slot.borrow_mut();
                let Some((registry, filter)) = slot.as_mut() else {
                    return false;
                };
                filter.filter(wparam.0 as u32, (event.mouseData >> 16) as u16, || {
                    let root = GetAncestor(WindowFromPoint(event.pt), GA_ROOT);
                    let registry = registry.lock().unwrap_or_else(|error| error.into_inner());
                    registry
                        .targets
                        .iter()
                        .any(|target| hit_target(target, event.pt, root))
                })
            });
            if blocked {
                return LRESULT(1);
            }
        }
        CallNextHookEx(None, code, wparam, lparam)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn double_click_blocks_both_down_up_pairs_and_restores_normal_clicks() {
        let mut filter = ClickFilter::default();
        for _ in 0..2 {
            assert!(filter.filter(WM_LBUTTONDOWN, 0, || true));
            assert!(filter.filter(WM_LBUTTONUP, 0, || false));
        }
        assert!(!filter.filter(WM_LBUTTONDOWN, 0, || false));
        assert!(!filter.filter(WM_LBUTTONUP, 0, || true));
    }

    #[test]
    fn release_is_swallowed_after_pointer_leaves_and_mouse_move_is_ignored() {
        let mut filter = ClickFilter::default();
        assert!(filter.filter(WM_LBUTTONDOWN, 0, || true));
        assert!(!filter.filter(WM_MOUSEMOVE, 0, || panic!(
            "Move should not inspect windows"
        )));
        assert!(filter.filter(WM_LBUTTONUP, 0, || false));
        assert!(!filter.filter(WM_LBUTTONUP, 0, || true));
    }
}
