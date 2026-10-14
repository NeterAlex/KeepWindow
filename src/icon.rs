use std::{mem::size_of, sync::Arc};

use gpui_kit::RenderImage;
use image::{Frame, RgbaImage};
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        Graphics::Gdi::{
            BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection,
            DIB_RGB_COLORS, DeleteDC, DeleteObject, GdiFlush, HBITMAP, HDC, HGDIOBJ, SelectObject,
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi},
            Shell::ExtractIconExW,
            WindowsAndMessaging::{
                CopyIcon, DI_NORMAL, DestroyIcon, DrawIconEx, GCLP_HICON, GCLP_HICONSM,
                GetClassLongPtrW, HICON, ICON_BIG, ICON_SMALL, ICON_SMALL2, IMAGE_ICON, IsWindow,
                LR_DEFAULTCOLOR, LoadImageW, SM_CXICON, SM_CXSMICON, SMTO_ABORTIFHUNG, SMTO_BLOCK,
                SendMessageTimeoutW, SendMessageW, WM_GETICON, WM_SETICON,
            },
        },
    },
    core::PCWSTR,
};

use crate::window::{WindowInfo, process_path, wide};

const ROW_ICON_SIZE: i32 = 32;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct WindowIconKey {
    hwnd: usize,
    pid: u32,
    thread_id: u32,
    class: String,
}

impl From<&WindowInfo> for WindowIconKey {
    fn from(target: &WindowInfo) -> Self {
        Self {
            hwnd: target.hwnd.0 as usize,
            pid: target.pid,
            thread_id: target.thread_id,
            class: target.class.clone(),
        }
    }
}

impl WindowIconKey {
    fn is_current(&self) -> bool {
        WindowInfo::inspect(HWND(self.hwnd as *mut _))
            .is_some_and(|target| Self::from(&target) == *self)
    }
}

pub fn load_window_icon(key: &WindowIconKey) -> Option<Arc<RenderImage>> {
    if !key.is_current() {
        return None;
    }
    let hwnd = HWND(key.hwnd as *mut _);
    let icon = target_icon(hwnd).or_else(|| executable_icon(key.pid))?;
    let pixels = rasterize(icon.0).ok()?;
    if !key.is_current() {
        return None;
    }
    let buffer = RgbaImage::from_raw(ROW_ICON_SIZE as u32, ROW_ICON_SIZE as u32, pixels)?;
    Some(Arc::new(RenderImage::new([Frame::new(buffer)])))
}

fn target_icon(hwnd: HWND) -> Option<Icon> {
    unsafe {
        for kind in [ICON_SMALL2, ICON_SMALL, ICON_BIG] {
            let mut result = 0;
            if SendMessageTimeoutW(
                hwnd,
                WM_GETICON,
                WPARAM(kind as usize),
                LPARAM(0),
                SMTO_ABORTIFHUNG | SMTO_BLOCK,
                75,
                Some(&mut result),
            )
            .0 == 0
            {
                break;
            }
            if result != 0
                && let Ok(icon) = CopyIcon(HICON(result as *mut _))
            {
                return Some(Icon(icon));
            }
        }
        for index in [GCLP_HICONSM, GCLP_HICON] {
            let result = GetClassLongPtrW(hwnd, index);
            if result != 0
                && let Ok(icon) = CopyIcon(HICON(result as *mut _))
            {
                return Some(Icon(icon));
            }
        }
    }
    None
}

fn executable_icon(pid: u32) -> Option<Icon> {
    let path = wide(&process_path(pid)?);
    let mut small = HICON::default();
    unsafe {
        ExtractIconExW(PCWSTR(path.as_ptr()), 0, None, Some(&mut small), 1);
    }
    (!small.0.is_null()).then_some(Icon(small))
}

struct IconSurface {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
}

impl Drop for IconSurface {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.previous);
            let _ = DeleteObject(HGDIOBJ(self.bitmap.0));
            let _ = DeleteDC(self.dc);
        }
    }
}

fn rasterize(icon: HICON) -> windows::core::Result<Vec<u8>> {
    unsafe {
        let dc = CreateCompatibleDC(None);
        if dc.0.is_null() {
            return Err(windows::core::Error::from_thread());
        }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: ROW_ICON_SIZE,
                biHeight: -ROW_ICON_SIZE,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let bitmap = match CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(bitmap) => bitmap,
            Err(error) => {
                let _ = DeleteDC(dc);
                return Err(error);
            }
        };
        let previous = SelectObject(dc, HGDIOBJ(bitmap.0));
        let _surface = IconSurface {
            dc,
            bitmap,
            previous,
        };
        let length = (ROW_ICON_SIZE * ROW_ICON_SIZE * 4) as usize;
        let draw = |background| -> windows::core::Result<Vec<u8>> {
            std::ptr::write_bytes(bits, background, length);
            DrawIconEx(
                dc,
                0,
                0,
                icon,
                ROW_ICON_SIZE,
                ROW_ICON_SIZE,
                0,
                None,
                DI_NORMAL,
            )?;
            GdiFlush().ok()?;
            Ok(std::slice::from_raw_parts(bits.cast::<u8>(), length).to_vec())
        };
        let black = draw(0)?;
        let white = draw(255)?;
        Ok(straight_bgra(&black, &white))
    }
}

fn straight_bgra(black: &[u8], white: &[u8]) -> Vec<u8> {
    // Drawing over both backgrounds recovers alpha for modern and legacy masked icons.
    black
        .as_chunks::<4>()
        .0
        .iter()
        .zip(white.as_chunks::<4>().0.iter())
        .flat_map(|(black, white)| {
            let alpha = 255
                - (0..3)
                    .map(|index| white[index].saturating_sub(black[index]))
                    .max()
                    .unwrap_or(255);
            let channel = |index: usize| {
                if alpha == 0 {
                    0
                } else {
                    ((u32::from(black[index]) * 255 + u32::from(alpha) / 2) / u32::from(alpha))
                        .min(255) as u8
                }
            };
            [channel(0), channel(1), channel(2), alpha]
        })
        .collect()
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use windows::{
        Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, IDI_INFORMATION, LoadIconW, WINDOW_EX_STYLE,
            WS_OVERLAPPEDWINDOW,
        },
        core::w,
    };

    #[test]
    fn alpha_conversion_preserves_transparent_and_translucent_pixels() {
        let black = [0, 0, 0, 0, 10, 20, 30, 0, 32, 64, 96, 128];
        let white = [255, 255, 255, 255, 10, 20, 30, 0, 159, 191, 223, 255];
        assert_eq!(
            straight_bgra(&black, &white),
            [0, 0, 0, 0, 10, 20, 30, 255, 64, 128, 191, 128]
        );
    }

    #[test]
    fn native_window_icon_is_copied_and_rasterized_without_destroying_source() {
        unsafe {
            let target = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("Window icon test"),
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
            let borrowed = LoadIconW(None, IDI_INFORMATION).unwrap();
            SendMessageW(
                target,
                WM_SETICON,
                Some(WPARAM(ICON_SMALL as usize)),
                Some(LPARAM(borrowed.0 as isize)),
            );
            let info = WindowInfo::inspect(target).unwrap();
            let key = WindowIconKey::from(&info);
            let mut renamed = info.clone();
            renamed.title = "Renamed".into();
            assert_eq!(key, WindowIconKey::from(&renamed));
            let image = load_window_icon(&key).unwrap();
            let pixels = image.as_bytes(0).unwrap();
            assert_eq!(pixels.len(), (ROW_ICON_SIZE * ROW_ICON_SIZE * 4) as usize);
            assert!(pixels.as_chunks::<4>().0.iter().any(|pixel| pixel[3] == 0));
            assert!(pixels.as_chunks::<4>().0.iter().any(|pixel| pixel[3] != 0));
            drop(image);
            let copy = Icon(CopyIcon(borrowed).unwrap());
            assert!(rasterize(copy.0).is_ok());
            DestroyWindow(target).unwrap();
            assert!(load_window_icon(&key).is_none());
        }
    }
}
