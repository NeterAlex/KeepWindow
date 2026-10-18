use std::{fs, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::{protection::Options, storage::atomic_write, window::WindowInfo};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Rule {
    #[serde(default)]
    pub name: String,
    pub path: String,
    pub class: String,
    pub title_contains: String,
    pub automatic: bool,
    pub options: Options,
}

impl Rule {
    pub fn matches(&self, path: &str, target: &WindowInfo) -> bool {
        self.path.to_lowercase() == path.to_lowercase()
            && self.class == target.class
            && target
                .title
                .to_lowercase()
                .contains(&self.title_contains.to_lowercase())
    }

    pub fn same_key(&self, other: &Self) -> bool {
        self.path.to_lowercase() == other.path.to_lowercase()
            && self.class == other.class
            && self.title_contains.to_lowercase() == other.title_contains.to_lowercase()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    pub version: u32,
    pub rules: Vec<Rule>,
    #[serde(default)]
    pub startup_enabled: bool,
    #[serde(default)]
    pub hotkeys: [String; 3],
    #[serde(default)]
    pub list: ListPreferences,
    #[serde(default)]
    pub language: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ListPreferences {
    pub protected_only: bool,
    pub grouped: bool,
    pub show_auxiliary: bool,
    pub advanced_columns: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: 2,
            rules: Vec::new(),
            startup_enabled: false,
            hotkeys: Default::default(),
            list: ListPreferences::default(),
            language: None,
        }
    }
}

impl Config {
    pub fn matching_rule(
        &self,
        path: &str,
        target: &WindowInfo,
        automatic_only: bool,
    ) -> Option<(usize, &Rule)> {
        self.rules
            .iter()
            .enumerate()
            .filter(|(_, rule)| (!automatic_only || rule.automatic) && rule.matches(path, target))
            .min_by_key(|(index, rule)| (rule.title_contains.is_empty(), *index))
    }

    pub fn upsert(&mut self, rule: Rule, editing: Option<usize>) {
        let existing = self.rules.iter().position(|item| item.same_key(&rule));
        match (editing, existing) {
            (Some(index), Some(other)) if index != other => {
                self.rules[other] = rule;
                self.rules.remove(index);
            }
            (Some(index), _) => self.rules[index] = rule,
            (_, Some(index)) => self.rules[index] = rule,
            _ => self.rules.push(rule),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.version != 2 {
            return Err(crate::locale::format(
                "unsupported_configuration_version_original_file_unchanged",
                &[("value1", &format!("{}", self.version))],
            ));
        }
        if let Some(language) = &self.language
            && !matches!(language.as_str(), "zh-CN" | "en-US")
        {
            return Err(crate::locale::text("unsupported_language_setting"));
        }
        for (index, rule) in self.rules.iter().enumerate() {
            if rule.path.is_empty() || rule.class.is_empty() {
                return Err(crate::locale::text(
                    "a_configuration_rule_is_missing_its_application",
                ));
            }
            rule.options.validate()?;
            if self.rules[..index].iter().any(|other| other.same_key(rule)) {
                return Err(crate::locale::text(
                    "configuration_contains_duplicate_rules",
                ));
            }
        }
        Ok(())
    }
}

pub struct Store {
    path: Option<PathBuf>,
    writable: bool,
}

impl Store {
    pub fn load() -> (Self, Config, Option<String>) {
        let path = std::env::var_os("LOCALAPPDATA")
            .map(|root| PathBuf::from(root).join("KeepWindow/config.json"));
        Self::load_path(path)
    }

    fn load_path(path: Option<PathBuf>) -> (Self, Config, Option<String>) {
        let result = match &path {
            None => Err(crate::locale::text(
                "localappdata_unavailable_cannot_save_configuration",
            )),
            Some(path) => match fs::read(path) {
                Ok(bytes) => serde_json::from_slice::<Config>(&bytes)
                    .map_err(|e| {
                        crate::locale::format(
                            "cannot_read_configuration_e_original_file_unchanged",
                            &[("e", &format!("{}", e))],
                        )
                    })
                    .and_then(|mut config| {
                        if config.version == 1 {
                            if config.version == 1 {
                                config.startup_enabled = false;
                            }
                            config.version = 2;
                            config.validate()?;
                            atomic_write(
                                path,
                                &serde_json::to_vec_pretty(&config).map_err(|e| e.to_string())?,
                            )?;
                        }
                        config.validate()?;
                        Ok(config)
                    }),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
                Err(e) => Err(crate::locale::format(
                    "cannot_read_configuration_e",
                    &[("e", &format!("{}", e))],
                )),
            },
        };
        match result {
            Ok(config) => (
                Self {
                    path,
                    writable: true,
                },
                config,
                None,
            ),
            Err(error) => (
                Self {
                    path,
                    writable: false,
                },
                Config::default(),
                Some(error),
            ),
        }
    }

    pub fn save(&self, config: &Config) -> Result<(), String> {
        if !self.writable {
            return Err(crate::locale::text(
                "configuration_is_read_only_repair_or_move",
            ));
        }
        config.validate()?;
        let path = self
            .path
            .as_ref()
            .ok_or(crate::locale::text("configuration_directory_unavailable"))?;
        let bytes = serde_json::to_vec_pretty(config).map_err(|e| e.to_string())?;
        atomic_write(path, &bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Foundation::HWND;

    #[test]
    fn language_is_optional_and_does_not_change_config_version() {
        let mut config: Config = serde_json::from_str("{\"version\":2,\"rules\":[]}").unwrap();
        assert!(config.language.is_none());
        for language in ["zh-CN", "en-US"] {
            config.language = Some(language.into());
            assert!(config.validate().is_ok());
            let restored: Config =
                serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
            assert_eq!(restored.language, config.language);
            assert_eq!(restored.version, 2);
        }
        config.language = Some("invalid".into());
        assert!(config.validate().is_err());
    }

    fn target() -> WindowInfo {
        WindowInfo {
            hwnd: HWND::default(),
            pid: 1,
            thread_id: 2,
            title: "Report - Editor".into(),
            class: "Editor".into(),
        }
    }
    fn rule(title: &str) -> Rule {
        Rule {
            name: String::new(),
            path: "C:\\Editor.exe".into(),
            class: "Editor".into(),
            title_contains: title.into(),
            automatic: false,
            options: Options::default(),
        }
    }

    #[test]
    fn matching_prioritizes_title_and_respects_opt_in() {
        let mut config = Config::default();
        config.upsert(rule(""), None);
        config.upsert(rule("REPORT"), None);
        assert_eq!(
            config
                .matching_rule("c:\\editor.EXE", &target(), false)
                .unwrap()
                .0,
            1
        );
        assert!(
            config
                .matching_rule("c:\\editor.exe", &target(), true)
                .is_none()
        );
        config.rules[0].automatic = true;
        assert_eq!(
            config
                .matching_rule("c:\\editor.exe", &target(), true)
                .unwrap()
                .0,
            0
        );
        let mut renamed = target();
        renamed.title = "Other document".into();
        assert_eq!(
            config
                .matching_rule("c:\\editor.exe", &renamed, false)
                .unwrap()
                .0,
            0
        );
        assert!(
            config
                .matching_rule("c:\\different.exe", &target(), false)
                .is_none()
        );
    }

    #[test]
    fn save_reload_and_invalid_files_remain_intact() {
        let directory =
            std::env::temp_dir().join(format!("keep-window-config-test-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("config.json");
        let (store, mut config, _) = Store::load_path(Some(path.clone()));
        config.upsert(rule(""), None);
        config.upsert(rule(""), None);
        assert_eq!(config.rules.len(), 1);
        store.save(&config).unwrap();
        assert_eq!(Store::load_path(Some(path.clone())).1.rules, config.rules);
        for invalid in ["invalid", "{\"version\":99,\"rules\":[]}"] {
            fs::write(&path, invalid).unwrap();
            let (store, config, error) = Store::load_path(Some(path.clone()));
            assert!(error.is_some());
            assert!(store.save(&config).is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), invalid);
        }
        fs::remove_file(&path).unwrap();
        fs::remove_dir(&directory).unwrap();
    }

    #[test]
    fn version_one_migration_preserves_rules_and_disables_startup() {
        let directory =
            std::env::temp_dir().join(format!("keep-window-migration-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("config.json");
        let rules = vec![rule("Report")];
        fs::write(
            &path,
            serde_json::to_vec(&serde_json::json!({"version": 1, "rules": rules})).unwrap(),
        )
        .unwrap();
        let (_, migrated, error) = Store::load_path(Some(path.clone()));
        assert!(error.is_none());
        assert_eq!(migrated.version, 2);
        assert_eq!(migrated.rules, rules);
        assert!(!migrated.startup_enabled);
        let saved: Config = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved.version, 2);
        assert_eq!(saved.rules, rules);
        assert!(
            !serde_json::from_str::<Config>("{\"version\":2,\"rules\":[]}")
                .unwrap()
                .startup_enabled
        );
        fs::remove_file(path).unwrap();
        fs::remove_dir(directory).unwrap();
    }
}
