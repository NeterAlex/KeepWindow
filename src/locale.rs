use std::sync::atomic::{AtomicU8, Ordering};

use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    ZhCn,
    EnUs,
}

impl Language {
    pub fn code(self) -> &'static str {
        match self {
            Self::ZhCn => "zh-CN",
            Self::EnUs => "en-US",
        }
    }
    fn as_u8(self) -> u8 {
        match self {
            Self::ZhCn => 0,
            Self::EnUs => 1,
        }
    }
    fn from_code(value: &str) -> Option<Self> {
        match value {
            "zh-CN" => Some(Self::ZhCn),
            "en-US" => Some(Self::EnUs),
            _ => None,
        }
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

fn catalog(language: Language) -> Value {
    let source = match language {
        Language::ZhCn => include_str!("../locales/zh-CN.json"),
        Language::EnUs => include_str!("../locales/en-US.json"),
    };
    serde_json::from_str(source).unwrap_or_else(|_| {
        serde_json::from_str(include_str!("../locales/zh-CN.json")).expect("compiled locale")
    })
}

pub fn current() -> Language {
    if CURRENT.load(Ordering::Relaxed) == 1 {
        Language::EnUs
    } else {
        Language::ZhCn
    }
}
pub fn set(language: Language) {
    CURRENT.store(language.as_u8(), Ordering::Relaxed);
}
pub fn from_config(value: Option<&str>) -> Language {
    value
        .and_then(Language::from_code)
        .unwrap_or_else(system_language)
}
pub fn system_language() -> Language {
    let value = std::env::var("LANG").unwrap_or_default().to_lowercase();
    if value.starts_with("en") {
        Language::EnUs
    } else {
        Language::ZhCn
    }
}
pub fn text(key: &str) -> String {
    let key_language = current();
    let fallback = catalog(Language::ZhCn);
    catalog(key_language)
        .get(key)
        .and_then(Value::as_str)
        .or_else(|| fallback.get(key).and_then(Value::as_str))
        .unwrap_or(key)
        .to_string()
}
pub fn format(key: &str, values: &[(&str, &str)]) -> String {
    let mut result = text(key);
    for (name, value) in values {
        result = result.replace(&format!("{{{name}}}"), value);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalogs_have_same_keys() {
        let zh = catalog(Language::ZhCn);
        let en = catalog(Language::EnUs);
        assert_eq!(
            zh.as_object().unwrap().keys().collect::<Vec<_>>(),
            en.as_object().unwrap().keys().collect::<Vec<_>>()
        );
    }
}
