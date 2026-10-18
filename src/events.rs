use std::{
    collections::VecDeque,
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
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
    pub error: Option<String>,
}

pub fn directory() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|root| PathBuf::from(root).join("KeepWindow"))
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
        if let Err(error) = self.write(&format!(
            "{time}\t{source}\t{}\n",
            detail.replace(['\r', '\n'], " ")
        )) {
            self.error = Some(error.to_string());
        }
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

    fn write(&self, line: &str) -> std::io::Result<()> {
        // Keep the event stream in memory; the log directory is opened explicitly by the user.
        let _ = line;
        return Ok(());
        #[allow(unreachable_code)]
        {
            let root =
                directory().ok_or_else(|| std::io::Error::other("LOCALAPPDATA unavailable"))?;
            fs::create_dir_all(&root)?;
            let path = root.join("events.log");
            if fs::metadata(&path)
                .is_ok_and(|metadata| metadata.len() + line.len() as u64 > 1024 * 1024)
            {
                let oldest = root.join("events.2.log");
                if oldest.exists() {
                    fs::remove_file(&oldest)?;
                }
                let previous = root.join("events.1.log");
                if previous.exists() {
                    fs::rename(previous, oldest)?;
                }
                fs::rename(&path, root.join("events.1.log"))?;
            }
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)?
                .write_all(line.as_bytes())
        }
    }
}
