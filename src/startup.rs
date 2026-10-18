use std::{os::windows::ffi::OsStrExt, path::Path};

use windows::{
    Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS},
        System::Registry::*,
    },
    core::{PCWSTR, w},
};

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const VALUE: PCWSTR = w!("KeepWindow");

struct Key(HKEY);
impl Drop for Key {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

fn open(subkey: PCWSTR, write: bool) -> Result<Option<Key>, String> {
    let mut key = HKEY::default();
    let status = unsafe {
        if write {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                subkey,
                None,
                None,
                REG_OPTION_NON_VOLATILE,
                KEY_QUERY_VALUE | KEY_SET_VALUE,
                None,
                &mut key,
                None,
            )
        } else {
            RegOpenKeyExW(HKEY_CURRENT_USER, subkey, None, KEY_QUERY_VALUE, &mut key)
        }
    };
    if status == ERROR_FILE_NOT_FOUND && !write {
        return Ok(None);
    }
    status.ok().map_err(|e| {
        crate::locale::format(
            "cannot_access_current_user_startup_entry_e",
            &[("e", &format!("{}", e))],
        )
    })?;
    Ok(Some(Key(key)))
}

fn read(subkey: PCWSTR) -> Result<Option<Vec<u16>>, String> {
    let Some(key) = open(subkey, false)? else {
        return Ok(None);
    };
    let mut kind = REG_VALUE_TYPE::default();
    let mut length = 0;
    let status =
        unsafe { RegQueryValueExW(key.0, VALUE, None, Some(&mut kind), None, Some(&mut length)) };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    status.ok().map_err(|e| {
        crate::locale::format("cannot_read_startup_entry_e", &[("e", &format!("{}", e))])
    })?;
    if kind != REG_SZ || length > 131072 || length % 2 != 0 {
        return Err(crate::locale::text(
            "keepwindow_startup_entry_has_an_invalid_format",
        ));
    }
    let mut buffer = vec![0u16; length as usize / 2];
    unsafe {
        RegQueryValueExW(
            key.0,
            VALUE,
            None,
            Some(&mut kind),
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut length),
        )
    }
    .ok()
    .map_err(|e| {
        crate::locale::format("cannot_read_startup_entry_e", &[("e", &format!("{}", e))])
    })?;
    buffer.truncate(length as usize / 2);
    while buffer.last() == Some(&0) {
        buffer.pop();
    }
    Ok(Some(buffer))
}

fn write(subkey: PCWSTR, command: Option<&[u16]>) -> Result<(), String> {
    if command.is_none() && read(subkey)?.is_none() {
        return Ok(());
    }
    let key = open(subkey, true)?.ok_or(crate::locale::text("startup_entry_unavailable"))?;
    let status = unsafe {
        if let Some(command) = command {
            let terminated: Vec<_> = command.iter().copied().chain(Some(0)).collect();
            let bytes: Vec<_> = terminated
                .iter()
                .flat_map(|value| value.to_le_bytes())
                .collect();
            RegSetValueExW(key.0, VALUE, None, REG_SZ, Some(&bytes))
        } else {
            RegDeleteValueW(key.0, VALUE)
        }
    };
    if command.is_none() && status == ERROR_FILE_NOT_FOUND {
        return Ok(());
    }
    if status != ERROR_SUCCESS {
        return Err(crate::locale::format(
            "cannot_update_current_user_startup_entry",
            &[("value1", &format!("{}", status.0))],
        ));
    }
    Ok(())
}

fn command(path: &Path) -> Result<Vec<u16>, String> {
    if !path.is_absolute() || !path.is_file() {
        return Err(crate::locale::text(
            "executable_path_unavailable_cannot_enable_startup",
        ));
    }
    let path: Vec<_> = path.as_os_str().encode_wide().collect();
    if path
        .iter()
        .any(|value| *value == 0 || *value == u16::from(b'"'))
    {
        return Err(crate::locale::text(
            "executable_path_contains_invalid_characters",
        ));
    }
    let mut result = vec![u16::from(b'"')];
    result.extend(path);
    result.extend("\" --background".encode_utf16());
    Ok(result)
}

pub fn enabled() -> Result<bool, String> {
    let executable = std::env::current_exe().map_err(|e| {
        crate::locale::format("cannot_get_executable_path_e", &[("e", &format!("{}", e))])
    })?;
    Ok(read(RUN_KEY)?
        .is_some_and(|value| command(&executable).is_ok_and(|expected| value == expected)))
}

// Return the previous value so a failed config save can restore the exact registry state.
pub fn set_enabled(enabled: bool) -> Result<Option<Vec<u16>>, String> {
    let executable = std::env::current_exe().map_err(|e| {
        crate::locale::format("cannot_get_executable_path_e", &[("e", &format!("{}", e))])
    })?;
    let command = command(&executable)?;
    let previous = read(RUN_KEY)?;
    write(RUN_KEY, enabled.then_some(command.as_slice()))?;
    Ok(previous)
}

pub fn restore(previous: Option<&[u16]>) -> Result<(), String> {
    write(RUN_KEY, previous)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window::wide;
    #[test]
    fn quotes_executable_with_spaces_and_background_argument() {
        let directory =
            std::env::temp_dir().join(format!("keep window startup {}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("keep window.exe");
        std::fs::write(&path, []).unwrap();
        assert_eq!(
            String::from_utf16(&command(&path).unwrap()).unwrap(),
            format!("\"{}\" --background", path.display())
        );
        assert!(command(Path::new("missing.exe")).is_err());
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
    #[test]
    fn registry_roundtrip_only_touches_named_value() {
        let path = wide(&format!(
            "Software\\KeepWindow.Tests\\Startup.{}",
            std::process::id()
        ));
        let subkey = PCWSTR(path.as_ptr());
        let command = wide("\"C:\\Test App\\keep-window.exe\" --background");
        write(subkey, Some(&command[..command.len() - 1])).unwrap();
        assert_eq!(read(subkey).unwrap().unwrap(), command[..command.len() - 1]);
        write(subkey, None).unwrap();
        assert!(read(subkey).unwrap().is_none());
        write(subkey, None).unwrap();
        unsafe {
            RegDeleteKeyW(HKEY_CURRENT_USER, subkey).ok().unwrap();
        }
    }
}
