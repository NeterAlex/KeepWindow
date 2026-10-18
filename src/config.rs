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
            return Err(format!(
                "不支持的配置版本：{}。原文件未修改。",
                self.version
            ));
        }
        for (index, rule) in self.rules.iter().enumerate() {
            if rule.path.is_empty() || rule.class.is_empty() {
                return Err("配置规则缺少程序路径或窗口类名。".into());
            }
            rule.options.validate()?;
            if self.rules[..index].iter().any(|other| other.same_key(rule)) {
                return Err("配置中存在重复规则。".into());
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
            None => Err("无法获取 LOCALAPPDATA，配置保存不可用。".into()),
            Some(path) => match fs::read(path) {
                Ok(bytes) => serde_json::from_slice::<Config>(&bytes)
                    .map_err(|e| format!("配置读取失败：{e}。原文件未修改。"))
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
                Err(e) => Err(format!("配置读取失败：{e}")),
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
            return Err("配置不可写，请先修复或移走原配置后重启。".into());
        }
        config.validate()?;
        let path = self.path.as_ref().ok_or("配置目录不可用。")?;
        let bytes = serde_json::to_vec_pretty(config).map_err(|e| e.to_string())?;
        atomic_write(path, &bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Foundation::HWND;

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
