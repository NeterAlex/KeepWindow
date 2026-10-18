use windows::Win32::{
    Foundation::HWND,
    UI::Input::KeyboardAndMouse::{
        HOT_KEY_MODIFIERS, MOD_NOREPEAT, RegisterHotKey, UnregisterHotKey,
    },
};

pub fn parse(text: &str) -> Result<Option<(u32, u32)>, String> {
    if text.trim().is_empty() {
        return Ok(None);
    }
    let mut modifiers = 0;
    let mut key = None;
    for part in text.trim().to_ascii_uppercase().split('+') {
        match part.trim() {
            "CTRL" | "CONTROL" => modifiers |= 2,
            "ALT" => modifiers |= 1,
            "SHIFT" => modifiers |= 4,
            "WIN" => modifiers |= 8,
            token => {
                let value = if token.len() == 1 && token.as_bytes()[0].is_ascii_alphanumeric() {
                    token.as_bytes()[0] as u32
                } else if let Some(number) = token
                    .strip_prefix('F')
                    .and_then(|n| n.parse::<u32>().ok())
                    .filter(|n| (1..=24).contains(n))
                {
                    0x70 + number - 1
                } else {
                    return Err(crate::locale::text("hotkey_format_ctrl_alt_k_or_ctrl"));
                };
                if key.replace(value).is_some() {
                    return Err(crate::locale::text(
                        "each_hotkey_must_contain_exactly_one_main",
                    ));
                }
            }
        }
    }
    if modifiers == 0 {
        return Err(crate::locale::text(
            "hotkeys_must_include_ctrl_alt_shift_or",
        ));
    }
    Ok(Some((
        modifiers,
        key.ok_or(crate::locale::text("hotkey_is_missing_its_main_key"))?,
    )))
}

pub struct Hotkeys {
    hwnd: HWND,
    active: Vec<i32>,
    generation: bool,
}

impl Hotkeys {
    pub fn new(hwnd: HWND) -> Self {
        Self {
            hwnd,
            active: Vec::new(),
            generation: false,
        }
    }
    pub fn replace(&mut self, values: &[String; 3]) -> Result<(), String> {
        let parsed: Vec<_> = values
            .iter()
            .map(|value| parse(value))
            .collect::<Result<_, _>>()?;
        for (index, value) in parsed.iter().enumerate() {
            if value.is_some() && parsed[..index].contains(value) {
                return Err(crate::locale::text("actions_cannot_share_the_same_hotkey"));
            }
        }
        let base = if self.generation { 0x4B10 } else { 0x4B20 };
        let mut registered = Vec::new();
        for (index, value) in parsed.into_iter().enumerate() {
            if let Some((modifiers, key)) = value {
                let id = base + index as i32;
                if let Err(error) = unsafe {
                    RegisterHotKey(
                        Some(self.hwnd),
                        id,
                        HOT_KEY_MODIFIERS(modifiers) | MOD_NOREPEAT,
                        key,
                    )
                } {
                    for id in registered {
                        unsafe {
                            let _ = UnregisterHotKey(Some(self.hwnd), id);
                        }
                    }
                    return Err(crate::locale::format(
                        "could_not_register_hotkey_it_may_be",
                        &[
                            ("value1", &values[index].to_string()),
                            ("error", &format!("{}", error)),
                        ],
                    ));
                }
                registered.push(id);
            }
        }
        for id in self.active.drain(..) {
            unsafe {
                let _ = UnregisterHotKey(Some(self.hwnd), id);
            }
        }
        self.active = registered;
        self.generation = !self.generation;
        Ok(())
    }
}

impl Drop for Hotkeys {
    fn drop(&mut self) {
        for id in &self.active {
            unsafe {
                let _ = UnregisterHotKey(Some(self.hwnd), *id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_bindings_and_rejects_ambiguous_keys() {
        assert_eq!(parse(" Ctrl+Alt+K ").unwrap(), Some((3, 75)));
        assert_eq!(parse("Shift+F24").unwrap(), Some((4, 0x87)));
        assert_eq!(parse("").unwrap(), None);
        for invalid in ["K", "Ctrl", "Ctrl+A+B", "Ctrl+F25", "Ctrl++"] {
            assert!(parse(invalid).is_err());
        }
    }
}
