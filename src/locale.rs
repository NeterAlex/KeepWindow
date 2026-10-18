use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        OnceLock,
        atomic::{AtomicU8, Ordering},
    },
};

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
    fn from_code(value: &str) -> Option<Self> {
        match value {
            "zh-CN" => Some(Self::ZhCn),
            "en-US" => Some(Self::EnUs),
            _ => None,
        }
    }
}
type Catalog = BTreeMap<String, String>;
static CURRENT: AtomicU8 = AtomicU8::new(0);
static CATALOGS: OnceLock<[Catalog; 2]> = OnceLock::new();
pub fn placeholders(value: &str) -> BTreeSet<&str> {
    value
        .split('{')
        .skip(1)
        .filter_map(|part| part.split_once('}').map(|(name, _)| name))
        .collect()
}
fn validated(source: &str, reference: &Catalog) -> Option<Catalog> {
    let candidate: Catalog = serde_json::from_str(source).ok()?;
    if candidate.keys().ne(reference.keys())
        || reference
            .iter()
            .any(|(key, value)| placeholders(value) != placeholders(&candidate[key]))
    {
        return None;
    }
    Some(candidate)
}
fn catalogs() -> &'static [Catalog; 2] {
    CATALOGS.get_or_init(|| {
        let zh: Catalog = serde_json::from_str(include_str!("../locales/zh-CN.json"))
            .expect("compiled Chinese catalog");
        let en = validated(include_str!("../locales/en-US.json"), &zh)
            .expect("compiled English catalog");
        let root = std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(|p| p.join("locales")));
        [Language::ZhCn, Language::EnUs].map(|language| {
            let compiled = if language == Language::ZhCn { &zh } else { &en };
            match root
                .as_ref()
                .map(|p| p.join(format!("{}.json", language.code())))
            {
                Some(path) if path.exists() => std::fs::read_to_string(path)
                    .ok()
                    .and_then(|s| validated(&s, &zh))
                    .unwrap_or_else(|| zh.clone()),
                _ => compiled.clone(),
            }
        })
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
    CURRENT.store(u8::from(language == Language::EnUs), Ordering::Relaxed);
}
pub fn from_config(value: Option<&str>) -> Language {
    value
        .and_then(Language::from_code)
        .unwrap_or_else(system_language)
}
pub fn system_language() -> Language {
    unsafe extern "system" {
        fn GetUserDefaultUILanguage() -> u16;
    }
    if unsafe { GetUserDefaultUILanguage() } & 0x3ff == 9 {
        Language::EnUs
    } else {
        Language::ZhCn
    }
}
pub fn text(key: &str) -> String {
    let index = usize::from(current() == Language::EnUs);
    catalogs()[index]
        .get(key)
        .or_else(|| catalogs()[0].get(key))
        .map_or_else(|| key.to_string(), Clone::clone)
}
pub fn format(key: &str, values: &[(&str, &str)]) -> String {
    let template = text(key);
    let mut result = String::new();
    let mut rest = template.as_str();
    while let Some(start) = rest.find('{') {
        result.push_str(&rest[..start]);
        let Some(end) = rest[start..].find('}').map(|end| end + start) else {
            result.push_str(&rest[start..]);
            return result;
        };
        let name = &rest[start + 1..end];
        if let Some((_, value)) = values.iter().find(|(key, _)| *key == name) {
            result.push_str(value);
        } else {
            result.push_str(&rest[start..=end]);
        }
        rest = &rest[end + 1..];
    }
    result.push_str(rest);
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalogs_are_complete() {
        let zh: Catalog = serde_json::from_str(include_str!("../locales/zh-CN.json")).unwrap();
        assert!(validated(include_str!("../locales/en-US.json"), &zh).is_some());
        assert!(validated("{}", &zh).is_none());
        assert!(validated("corrupt", &zh).is_none());
        let mut bad = zh.clone();
        bad.insert("language_switched".into(), "{wrong}".into());
        assert!(validated(&serde_json::to_string(&bad).unwrap(), &zh).is_none());
    }
    #[test]
    fn language_selection() {
        assert_eq!(from_config(Some("en-US")), Language::EnUs);
        assert_eq!(from_config(Some("zh-CN")), Language::ZhCn);
        assert_eq!(from_config(Some("unknown")), system_language());
    }
    #[test]
    fn interpolation_preserves_user_supplied_braces() {
        let label = "Editor {value2}";
        let rendered = format(
            "application_class",
            &[("value1", label), ("value2", "WindowClass")],
        );
        assert!(rendered.contains(label));
        assert!(rendered.contains("WindowClass"));
    }
}
