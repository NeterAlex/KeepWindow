use std::{
    collections::VecDeque,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

pub struct Event {
    pub time: u64,
    pub source: String,
    pub detail: String,
    pub count: usize,
}

#[derive(Default)]
pub struct Events {
    pub entries: VecDeque<Event>,
}

pub struct Diagnostic {
    pub protection_statuses: Vec<String>,
    pub attempts_waiting: Vec<bool>,
    pub releases: usize,
    pub windows: Vec<(String, String)>,
}

pub fn export(path: &Path, diagnostic: &str) -> Result<(), String> {
    crate::storage::atomic_write(path, diagnostic.as_bytes())
}

impl Events {
    pub fn push(&mut self, source: &str, detail: impl Into<String>) {
        let detail = detail.into();
        if let Some(last) = self.entries.back_mut()
            && last.source == source
            && last.detail == detail
        {
            last.count += 1;
            return;
        }
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.entries.push_back(Event {
            time,
            source: source.into(),
            detail,
            count: 1,
        });
        if self.entries.len() > 200 {
            self.entries.pop_front();
        }
    }

    pub fn diagnostic(&self, snapshot: &Diagnostic, include_private: bool) -> String {
        let mut text = crate::locale::format(
            "keepwindow_windows_protected_awaiting_retry_temporarily_released",
            &[
                ("value1", env!("CARGO_PKG_VERSION")),
                (
                    "value2",
                    &std::env::var("OS").unwrap_or_else(|_| "Windows".into()),
                ),
                ("value3", &snapshot.protection_statuses.len().to_string()),
                ("value4", &snapshot.attempts_waiting.len().to_string()),
                ("value5", &snapshot.releases.to_string()),
            ],
        );
        for status in &snapshot.protection_statuses {
            text.push_str(&crate::locale::format("status", &[("value1", status)]));
        }
        for &waiting in &snapshot.attempts_waiting {
            text.push_str(&crate::locale::text(if waiting {
                "protection_failed_awaiting_retry"
            } else {
                "protection_failed_manual_action_required"
            }));
        }
        if include_private {
            for event in &self.entries {
                text.push_str(&crate::locale::format(
                    "times",
                    &[
                        ("value1", &event.time.to_string()),
                        ("value2", &event.source),
                        ("value3", &event.detail),
                        ("value4", &event.count.to_string()),
                    ],
                ));
            }
            for (title, path) in &snapshot.windows {
                text.push_str(&format!("{title} | {path}\n"));
            }
        } else {
            text.push_str(&crate::locale::text(
                "privacy_mode_window_titles_paths_and_free",
            ));
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_and_utf8_export_follow_privacy_setting() {
        let mut events = Events::default();
        events.push("private-source", "private-error");
        let snapshot = Diagnostic {
            protection_statuses: vec!["protected".into()],
            attempts_waiting: vec![true, false],
            releases: 1,
            windows: vec![(
                "private-title \u{4e2d}\u{6587} \u{1f600}".into(),
                "C:\\private-path.exe".into(),
            )],
        };
        let private = events.diagnostic(&snapshot, false);
        let full = events.diagnostic(&snapshot, true);
        for value in [
            "private-source",
            "private-error",
            "private-title",
            "private-path",
        ] {
            assert!(!private.contains(value));
            assert!(full.contains(value));
        }
        assert!(private.contains("protected"));
        let path =
            std::env::temp_dir().join(format!("keep-window-diagnostic-{}.txt", std::process::id()));
        for text in [&full, &private] {
            export(&path, text).unwrap();
            assert_eq!(std::fs::read(&path).unwrap(), text.as_bytes());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), *text);
        }
        std::fs::write(&path, b"blocker").unwrap();
        assert!(export(&path.join("missing.txt"), &private).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"blocker");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn event_history_is_bounded_and_coalesces_repetitions() {
        let mut events = Events::default();
        events.push("test", "same");
        events.push("test", "same");
        assert_eq!(events.entries.len(), 1);
        assert_eq!(events.entries[0].count, 2);
        for index in 0..201 {
            events.push("test", index.to_string());
        }
        assert_eq!(events.entries.len(), 200);
        assert_eq!(events.entries.front().unwrap().detail, "1");
    }
}
