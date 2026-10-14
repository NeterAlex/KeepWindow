use std::{
    cell::RefCell,
    sync::{Arc, Mutex, OnceLock, Weak, mpsc},
    thread::{self, JoinHandle},
};

use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM},
        System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
        UI::{
            Input::KeyboardAndMouse::{GetAsyncKeyState, VK_ESCAPE},
            WindowsAndMessaging::*,
        },
    },
    core::PCWSTR,
};

use crate::{
    protection::{Options, shield_rect},
    recovery::Identity,
    window::{WindowInfo, is_cloaked},
};

#[derive(Clone)]
struct Target {
    hwnd: usize,
    overlay: usize,
    calibration: usize,
    marker: Vec<u16>,
    options: Options,
}

#[derive(Default)]
struct Registry {
    targets: Vec<Target>,
    picker: Option<PickerState>,
}

enum PickerEvent {
    Hover(usize),
    Selected(Identity),
    Cancel,
    Error(String),
}

struct PickerState {
    events: mpsc::Sender<PickerEvent>,
    hover: usize,
    pressed: Option<Identity>,
    finished: bool,
    error: Option<String>,
}

impl PickerState {
    fn cancel(&mut self) {
        if !self.finished {
            self.finished = true;
            self.pressed = None;
            let _ = self.events.send(PickerEvent::Cancel);
        }
    }
    fn input(&mut self, message: u32, target: Option<WindowInfo>) {
        if self.finished {
            return;
        }
        let hover = target.as_ref().map_or(0, |target| target.hwnd.0 as usize);
        if self.hover != hover {
            self.hover = hover;
            let _ = self.events.send(PickerEvent::Hover(hover));
        }
        if message == WM_LBUTTONDOWN {
            self.pressed = None;
            self.error = None;
            if let Some(target) = &target {
                match Identity::capture(target) {
                    Ok(identity) => self.pressed = Some(identity),
                    Err(error) => self.error = Some(error),
                }
            }
        } else if message == WM_LBUTTONUP
            && let Some(error) = self.error.take()
        {
            self.finished = true;
            let _ = self.events.send(PickerEvent::Error(error));
        } else if message == WM_LBUTTONUP
            && let Some(pressed) = self.pressed.take()
            && target
                .as_ref()
                .and_then(|target| Identity::capture(target).ok())
                .is_some_and(|current| pressed.matches(&current))
        {
            self.finished = true;
            let _ = self.events.send(PickerEvent::Selected(pressed));
        }
    }
}

pub struct MousePicker {
    service: Arc<Service>,
    events: mpsc::Receiver<PickerEvent>,
    pub hover: Option<HWND>,
}

impl MousePicker {
    pub fn new() -> Result<Self, String> {
        let service = Service::get()?;
        let (sender, events) = mpsc::channel();
        {
            let mut registry = service.registry.lock().unwrap_or_else(|e| e.into_inner());
            if registry.picker.is_some() {
                return Err("窗口选择已在进行中。".into());
            }
            registry.picker = Some(PickerState {
                events: sender,
                hover: 0,
                pressed: None,
                finished: false,
                error: None,
            });
        }
        Ok(Self {
            service,
            events,
            hover: None,
        })
    }

    pub fn poll(&mut self) -> Result<Option<Option<WindowInfo>>, String> {
        for event in self.events.try_iter() {
            match event {
                PickerEvent::Hover(hwnd) => {
                    self.hover = (hwnd != 0).then_some(HWND(hwnd as *mut _))
                }
                PickerEvent::Cancel => return Ok(Some(None)),
                PickerEvent::Error(error) => return Err(error),
                PickerEvent::Selected(identity) => {
                    let target = picker_target(HWND(identity.hwnd as usize as *mut _))
                        .ok_or("目标窗口已关闭或不可选择。")?;
                    if !Identity::capture(&target)?.matches(&identity) {
                        return Err("目标窗口身份已变化，请重新选择。".into());
                    }
                    return Ok(Some(Some(target)));
                }
            }
        }
        Ok(None)
    }
}

impl Drop for MousePicker {
    fn drop(&mut self) {
        self.service
            .registry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .picker = None;
    }
}

pub fn picker_target(hwnd: HWND) -> Option<WindowInfo> {
    unsafe {
        let root = GetAncestor(hwnd, GA_ROOT);
        if root == GetDesktopWindow()
            || root == GetShellWindow()
            || !IsWindowVisible(root).as_bool()
            || is_cloaked(root)
        {
            return None;
        }
        let mut pid = 0;
        GetWindowThreadProcessId(root, Some(&mut pid));
        // Avoid synchronous title reads from our UI thread while it may be dropping the picker.
        if pid == std::process::id() {
            return None;
        }
        let target = WindowInfo::inspect(root)?;
        if matches!(
            target.class.as_str(),
            "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
        ) {
            return None;
        }
        Some(target)
    }
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
    pub fn set_calibration(&self, hwnd: Option<HWND>) {
        let mut registry = self
            .service
            .registry
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(target) = registry
            .targets
            .iter_mut()
            .find(|target| target.marker == self.marker)
        {
            target.calibration = hwnd.map_or(0, |hwnd| hwnd.0 as usize);
        }
    }
    pub fn update(&self, options: Options) {
        let mut registry = self
            .service
            .registry
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(target) = registry
            .targets
            .iter_mut()
            .find(|target| target.marker == self.marker)
        {
            target.options = options;
        }
    }
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
                calibration: 0,
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
                    let timer = SetTimer(None, 0, 16, None);
                    if timer == 0 {
                        let _ = ready.send(Err("无法创建窗口选择定时器。".into()));
                        let _ = UnhookWindowsHookEx(hook);
                        HOOK_STATE.with(|slot| *slot.borrow_mut() = None);
                        return;
                    }
                    let mut crosshair = false;
                    if ready.send(Ok(GetCurrentThreadId())).is_ok() {
                        while GetMessageW(&mut message, None, 0, 0).0 > 0 {
                            if message.message == WM_TIMER {
                                HOOK_STATE.with(|slot| {
                                    let slot = slot.borrow();
                                    if let Some((registry, _)) = slot.as_ref() {
                                        let mut registry =
                                            registry.lock().unwrap_or_else(|e| e.into_inner());
                                        if let Some(picker) = &mut registry.picker {
                                            if GetAsyncKeyState(VK_ESCAPE.0 as i32) < 0 {
                                                picker.cancel();
                                            }
                                            if !picker.finished
                                                && let Ok(cursor) = LoadCursorW(None, IDC_CROSS)
                                            {
                                                let _ = SetCursor(Some(cursor));
                                                crosshair = true;
                                            }
                                        } else if crosshair
                                            && let Ok(cursor) = LoadCursorW(None, IDC_ARROW)
                                        {
                                            let _ = SetCursor(Some(cursor));
                                            crosshair = false;
                                        }
                                    }
                                });
                            }
                            let _ = TranslateMessage(&message);
                            DispatchMessageW(&message);
                        }
                    }
                    if crosshair && let Ok(cursor) = LoadCursorW(None, IDC_ARROW) {
                        let _ = SetCursor(Some(cursor));
                    }
                    let _ = KillTimer(None, timer);
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
        if (root.0 as usize != target.hwnd
            && root.0 as usize != target.overlay
            && root.0 as usize != target.calibration)
            || !IsWindowVisible(hwnd).as_bool()
            || IsIconic(hwnd).as_bool()
            || is_cloaked(hwnd)
            || GetPropW(hwnd, PCWSTR(target.marker.as_ptr())).0 as usize != 1
        {
            return false;
        }
    }
    let rect = if target.calibration != 0 {
        let mut rect = windows::Win32::Foundation::RECT::default();
        unsafe { GetWindowRect(HWND(target.calibration as *mut _), &mut rect) }
            .map(|_| rect)
            .map_err(|e| e.to_string())
    } else {
        shield_rect(hwnd, target.options)
    };
    rect.is_ok_and(|rect| {
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
                let root = GetAncestor(WindowFromPoint(event.pt), GA_ROOT);
                let mut registry = registry.lock().unwrap_or_else(|error| error.into_inner());
                if let Some(picker) = &mut registry.picker {
                    picker.input(wparam.0 as u32, picker_target(root));
                    return filter.filter(wparam.0 as u32, (event.mouseData >> 16) as u16, || true);
                }
                filter.filter(wparam.0 as u32, (event.mouseData >> 16) as u16, || {
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

    #[test]
    fn picker_cancels_once_and_excludes_own_and_desktop_windows() {
        unsafe {
            assert!(picker_target(GetDesktopWindow()).is_none());
            assert!(picker_target(GetShellWindow()).is_none());
            let target = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                windows::core::w!("STATIC"),
                windows::core::w!("Own picker target"),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                0,
                0,
                300,
                200,
                None,
                None,
                Some(GetModuleHandleW(None).unwrap().into()),
                None,
            )
            .unwrap();
            assert!(picker_target(target).is_none());
            DestroyWindow(target).unwrap();
        }
        let (events, receiver) = mpsc::channel();
        let mut picker = PickerState {
            events,
            hover: 0,
            pressed: None,
            finished: false,
            error: None,
        };
        picker.cancel();
        picker.cancel();
        picker.input(WM_LBUTTONDOWN, None);
        assert!(matches!(receiver.try_recv(), Ok(PickerEvent::Cancel)));
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn picker_returns_one_click_and_rejects_changed_or_closed_target() {
        unsafe {
            let target = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                windows::core::w!("STATIC"),
                windows::core::w!("Picker event target"),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                300,
                200,
                None,
                None,
                Some(GetModuleHandleW(None).unwrap().into()),
                None,
            )
            .unwrap();
            let info = WindowInfo::inspect(target).unwrap();
            let (events, receiver) = mpsc::channel();
            let mut picker = PickerState {
                events,
                hover: 0,
                pressed: None,
                finished: false,
                error: None,
            };
            picker.input(WM_LBUTTONDOWN, Some(info.clone()));
            picker.pressed.as_mut().unwrap().creation += 1;
            picker.input(WM_LBUTTONUP, Some(info.clone()));
            assert!(!picker.finished);
            picker.input(WM_LBUTTONDOWN, Some(info.clone()));
            picker.input(WM_LBUTTONUP, Some(info.clone()));
            picker.input(WM_LBUTTONDOWN, Some(info.clone()));
            picker.input(WM_LBUTTONUP, Some(info.clone()));
            assert_eq!(
                receiver
                    .try_iter()
                    .filter(|event| matches!(event, PickerEvent::Selected(_)))
                    .count(),
                1
            );
            picker.finished = false;
            picker.input(WM_LBUTTONDOWN, Some(info.clone()));
            DestroyWindow(target).unwrap();
            picker.input(WM_LBUTTONUP, Some(info));
            assert!(!picker.finished);
            assert!(
                receiver
                    .try_iter()
                    .all(|event| !matches!(event, PickerEvent::Selected(_)))
            );
        }
    }
}
