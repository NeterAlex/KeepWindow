use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi},
            WindowsAndMessaging::{
                DestroyIcon, HICON, ICON_BIG, ICON_SMALL, IMAGE_ICON, IsWindow, LR_DEFAULTCOLOR,
                LoadImageW, SM_CXICON, SM_CXSMICON, SendMessageW, WM_GETICON, WM_SETICON,
            },
        },
    },
    core::PCWSTR,
};

struct Icon(HICON);

impl Icon {
    fn load(size: i32) -> windows::core::Result<Self> {
        unsafe {
            let module = GetModuleHandleW(None)?;
            // Resource ID 1 is also the executable's Explorer icon.
            let handle = LoadImageW(
                Some(module.into()),
                PCWSTR(std::ptr::without_provenance(1)),
                IMAGE_ICON,
                size,
                size,
                LR_DEFAULTCOLOR,
            )?;
            Ok(Self(HICON(handle.0)))
        }
    }
}

impl Drop for Icon {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyIcon(self.0);
        }
    }
}

pub struct WindowIcons {
    hwnd: HWND,
    small: Icon,
    large: Icon,
    previous: [LRESULT; 2],
}

impl WindowIcons {
    pub fn new(hwnd: HWND) -> windows::core::Result<Self> {
        let (small, large) = Self::load_for_dpi(unsafe { GetDpiForWindow(hwnd) })?;
        let previous = unsafe {
            [
                SendMessageW(
                    hwnd,
                    WM_SETICON,
                    Some(WPARAM(ICON_SMALL as usize)),
                    Some(LPARAM(small.0.0 as isize)),
                ),
                SendMessageW(
                    hwnd,
                    WM_SETICON,
                    Some(WPARAM(ICON_BIG as usize)),
                    Some(LPARAM(large.0.0 as isize)),
                ),
            ]
        };
        Ok(Self {
            hwnd,
            small,
            large,
            previous,
        })
    }

    fn load_for_dpi(dpi: u32) -> windows::core::Result<(Icon, Icon)> {
        let dpi = if dpi == 0 { 96 } else { dpi };
        unsafe {
            Ok((
                Icon::load(GetSystemMetricsForDpi(SM_CXSMICON, dpi).max(16))?,
                Icon::load(GetSystemMetricsForDpi(SM_CXICON, dpi).max(32))?,
            ))
        }
    }

    pub fn small(&self) -> HICON {
        self.small.0
    }

    pub fn update_dpi(&mut self, dpi: u32) -> windows::core::Result<()> {
        let (small, large) = Self::load_for_dpi(dpi)?;
        unsafe {
            SendMessageW(
                self.hwnd,
                WM_SETICON,
                Some(WPARAM(ICON_SMALL as usize)),
                Some(LPARAM(small.0.0 as isize)),
            );
            SendMessageW(
                self.hwnd,
                WM_SETICON,
                Some(WPARAM(ICON_BIG as usize)),
                Some(LPARAM(large.0.0 as isize)),
            );
        }
        self.small = small;
        self.large = large;
        Ok(())
    }
}

impl Drop for WindowIcons {
    fn drop(&mut self) {
        unsafe {
            if !IsWindow(Some(self.hwnd)).as_bool() {
                return;
            }
            for ((kind, owned), previous) in [(ICON_SMALL, self.small.0), (ICON_BIG, self.large.0)]
                .into_iter()
                .zip(self.previous)
            {
                let kind = Some(WPARAM(kind as usize));
                if SendMessageW(self.hwnd, WM_GETICON, kind, None).0 == owned.0 as isize {
                    SendMessageW(self.hwnd, WM_SETICON, kind, Some(LPARAM(previous.0)));
                }
            }
        }
    }
}
