use std::{
    cell::RefCell,
    fs,
    path::PathBuf,
    rc::Rc,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use windows::Win32::Foundation::HWND;
use windows::{
    Win32::UI::WindowsAndMessaging::{GetPropW, RemovePropW},
    core::PCWSTR,
};

use crate::{
    protection::restore_close_state,
    storage::atomic_write,
    window::{self, WindowInfo},
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Identity {
    pub hwnd: u64,
    pub pid: u32,
    pub thread_id: u32,
    pub class: String,
    pub creation: u64,
    pub path: String,
}

impl Identity {
    pub fn capture(target: &WindowInfo) -> Result<Self, String> {
        let creation = window::process_creation(target.pid).ok_or(
            "无法读取目标进程创建时间。请检查权限，管理员窗口需要以管理员身份启动 KeepWindow。",
        )?;
        let path = window::process_path(target.pid).ok_or(
            "无法读取目标程序路径。请检查权限，管理员窗口需要以管理员身份启动 KeepWindow。",
        )?;
        if !target.is_same_window() || window::process_creation(target.pid) != Some(creation) {
            return Err("目标窗口已关闭或重建，请重新选择。".into());
        }
        Ok(Self {
            hwnd: target.hwnd.0 as usize as u64,
            pid: target.pid,
            thread_id: target.thread_id,
            class: target.class.clone(),
            creation,
            path,
        })
    }

    pub fn matches(&self, other: &Self) -> bool {
        self.hwnd == other.hwnd
            && self.pid == other.pid
            && self.thread_id == other.thread_id
            && self.class == other.class
            && self.creation == other.creation
            && self.path.eq_ignore_ascii_case(&other.path)
    }

    fn current(&self) -> Option<WindowInfo> {
        let target = WindowInfo::inspect(HWND(self.hwnd as usize as *mut _))?;
        Self::capture(&target)
            .ok()
            .filter(|current| self.matches(current))
            .map(|_| target)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub identity: Identity,
    pub original_disabled: u32,
    pub marker: Vec<u16>,
}

#[derive(Default, Serialize, Deserialize)]
struct Log {
    version: u32,
    records: Vec<Record>,
}

struct Journal {
    path: Option<PathBuf>,
    records: Vec<Record>,
    writable: bool,
    #[cfg(test)]
    temporary: bool,
}

#[derive(Clone)]
pub struct Recovery(Rc<RefCell<Journal>>);

#[derive(Default)]
struct Report {
    restored: usize,
    skipped: usize,
    failed: usize,
    errors: Vec<String>,
}

impl Recovery {
    pub fn begin() -> (Self, Option<String>) {
        let path = std::env::var_os("LOCALAPPDATA")
            .map(|root| PathBuf::from(root).join("KeepWindow/recovery.json"));
        Self::begin_path(path)
    }

    fn begin_path(path: Option<PathBuf>) -> (Self, Option<String>) {
        let mut journal = Journal {
            path,
            records: Vec::new(),
            writable: true,
            #[cfg(test)]
            temporary: false,
        };
        let mut report = Report::default();
        let mut interrupted = false;
        let result = (|| -> Result<(), String> {
            let path = journal
                .path
                .as_ref()
                .ok_or("无法获取 LOCALAPPDATA，系统关闭保护不可用。")?;
            interrupted = path.with_file_name("running").exists();
            match fs::read(path) {
                Ok(bytes) => match serde_json::from_slice::<Log>(&bytes) {
                    Ok(log)
                        if log.version == 1
                            && log.records.iter().all(|r| {
                                r.original_disabled <= 3
                                    && r.identity.hwnd != 0
                                    && !r.identity.class.is_empty()
                                    && !r.identity.path.is_empty()
                                    && r.marker.last() == Some(&0)
                                    && r.marker.len() > 1
                            }) =>
                    {
                        journal.records = log.records
                    }
                    _ => {
                        let backup = journal.backup_path("corrupt")?;
                        fs::rename(path, &backup)
                            .map_err(|e| format!("损坏恢复日志备份失败：{e}"))?;
                        report
                            .errors
                            .push(format!("恢复日志损坏，已备份到 {}。", backup.display()));
                    }
                },
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(format!("恢复日志读取失败：{e}")),
            }
            // Recover leftover records even if a previous marker deletion was interrupted.
            for record in journal.records.clone() {
                let duplicate = journal
                    .records
                    .iter()
                    .filter(|r| r.identity.hwnd == record.identity.hwnd)
                    .count()
                    > 1;
                if !journal.records.contains(&record) {
                    continue;
                }
                let hwnd = HWND(record.identity.hwnd as usize as *mut _);
                // Window properties outlive a crashed owner, but disappear when the target is destroyed.
                let marked =
                    unsafe { GetPropW(hwnd, PCWSTR(record.marker.as_ptr())).0 as usize == 1 };
                if duplicate || record.identity.current().is_none() || !marked {
                    let skipped: Vec<_> = journal
                        .records
                        .iter()
                        .filter(|r| r.identity.hwnd == record.identity.hwnd)
                        .cloned()
                        .collect();
                    let backup = journal.backup_path("residual")?;
                    atomic_write(
                        &backup,
                        &serde_json::to_vec_pretty(&skipped).map_err(|e| e.to_string())?,
                    )?;
                    let remaining = journal
                        .records
                        .iter()
                        .filter(|r| r.identity.hwnd != record.identity.hwnd)
                        .cloned()
                        .collect();
                    journal.commit(remaining)?;
                    report.skipped += skipped.len();
                } else {
                    match restore_close_state(
                        HWND(record.identity.hwnd as usize as *mut _),
                        record.original_disabled,
                    ) {
                        Ok(()) => {
                            journal.remove(&record.identity)?;
                            unsafe {
                                let _ = RemovePropW(hwnd, PCWSTR(record.marker.as_ptr()));
                            }
                            report.restored += 1;
                        }
                        Err(error) => {
                            report.failed += 1;
                            report.errors.push(error);
                        }
                    }
                }
            }
            atomic_write(
                &journal.path.as_ref().unwrap().with_file_name("running"),
                b"running",
            )?;
            Ok(())
        })();
        if let Err(error) = result {
            journal.writable = false;
            report.errors.push(error);
        }
        let message = if interrupted
            || report.restored + report.skipped + report.failed > 0
            || !report.errors.is_empty()
        {
            Some(format!(
                "启动恢复：成功 {}，跳过 {}（身份不匹配或无法验证，已保留备份），失败 {}。{}",
                report.restored,
                report.skipped,
                report.failed,
                report.errors.join("；")
            ))
        } else {
            None
        };
        (Self(Rc::new(RefCell::new(journal))), message)
    }

    pub fn record(
        &self,
        target: &WindowInfo,
        original_disabled: u32,
        marker: &[u16],
    ) -> Result<Identity, String> {
        let identity = Identity::capture(target)?;
        let mut journal = self.0.borrow_mut();
        if journal
            .records
            .iter()
            .any(|record| record.identity.hwnd == identity.hwnd)
        {
            return Err("该窗口存在待恢复记录，请先重启 KeepWindow 完成恢复。".into());
        }
        let mut records = journal.records.clone();
        records.push(Record {
            identity: identity.clone(),
            original_disabled,
            marker: marker.to_vec(),
        });
        journal.commit(records)?;
        Ok(identity)
    }

    pub fn remove(&self, identity: &Identity) -> Result<(), String> {
        self.0.borrow_mut().remove(identity)
    }

    pub fn finish(&self) -> Result<(), String> {
        let journal = self.0.borrow();
        if !journal.writable || !journal.records.is_empty() {
            return Err("恢复日志仍有待恢复项，保留运行标记供下次启动重试。".into());
        }
        let path = journal.path.as_ref().ok_or("恢复目录不可用。")?;
        for file in [path.clone(), path.with_file_name("running")] {
            match fs::remove_file(file) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(format!("清理恢复标记失败：{e}")),
            }
        }
        Ok(())
    }

    #[cfg(test)]
    pub fn testing() -> Self {
        let path = std::env::temp_dir()
            .join(format!(
                "keep-window-journal-{}-{}",
                std::process::id(),
                nonce()
            ))
            .join("recovery.json");
        let (recovery, error) = Self::begin_path(Some(path));
        assert!(error.is_none());
        recovery.0.borrow_mut().temporary = true;
        recovery
    }
}

fn nonce() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

impl Journal {
    fn backup_path(&self, kind: &str) -> Result<PathBuf, String> {
        Ok(self
            .path
            .as_ref()
            .ok_or("恢复目录不可用。")?
            .with_file_name(format!("recovery-{kind}-{}.json", nonce())))
    }
    fn commit(&mut self, records: Vec<Record>) -> Result<(), String> {
        if !self.writable {
            return Err("恢复日志不可写，无法安全修改系统关闭项。".into());
        }
        let bytes = serde_json::to_vec_pretty(&Log {
            version: 1,
            records: records.clone(),
        })
        .map_err(|e| e.to_string())?;
        atomic_write(self.path.as_ref().ok_or("恢复目录不可用。")?, &bytes)
            .map_err(|e| format!("恢复日志保存失败：{e}"))?;
        self.records = records;
        Ok(())
    }
    fn remove(&mut self, identity: &Identity) -> Result<(), String> {
        if !self.records.iter().any(|r| r.identity == *identity) {
            return Ok(());
        }
        self.commit(
            self.records
                .iter()
                .filter(|r| r.identity != *identity)
                .cloned()
                .collect(),
        )
    }
}

#[cfg(test)]
impl Drop for Journal {
    fn drop(&mut self) {
        if self.temporary
            && let Some(path) = &self.path
        {
            let _ = fs::remove_file(path);
            let _ = fs::remove_file(path.with_file_name("running"));
            let _ = fs::remove_dir(path.parent().unwrap());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::{
        Win32::{
            Foundation::HANDLE, System::LibraryLoader::GetModuleHandleW, UI::WindowsAndMessaging::*,
        },
        core::w,
    };

    struct Target(HWND);
    impl Target {
        fn new() -> Self {
            Self(unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    w!("STATIC"),
                    w!("Recovery target"),
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
                .unwrap()
            })
        }
        fn info(&self) -> WindowInfo {
            WindowInfo::inspect(self.0).unwrap()
        }
        fn state(&self) -> u32 {
            crate::protection::original_close_state(self.0).unwrap()
        }
    }
    impl Drop for Target {
        fn drop(&mut self) {
            unsafe {
                let _ = DestroyWindow(self.0);
            }
        }
    }
    fn seed(target: &Target, recovery: &Recovery, original: u32) -> Identity {
        let marker = window::wide(&format!("KeepWindow.Recovery.Test.{}", nonce()));
        unsafe {
            SetPropW(
                target.0,
                PCWSTR(marker.as_ptr()),
                Some(HANDLE(std::ptr::dangling_mut())),
            )
            .unwrap();
        }
        recovery.record(&target.info(), original, &marker).unwrap()
    }
    fn read_log(recovery: &Recovery) -> Log {
        serde_json::from_slice(&fs::read(recovery.0.borrow().path.as_ref().unwrap()).unwrap())
            .unwrap()
    }
    fn clear_directory(recovery: Recovery) {
        let path = recovery.0.borrow().path.clone().unwrap();
        recovery.finish().unwrap();
        drop(recovery);
        for file in fs::read_dir(path.parent().unwrap()).unwrap() {
            fs::remove_file(file.unwrap().path()).unwrap();
        }
        fs::remove_dir(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn identity_requires_every_field_but_allows_path_case() {
        let target = Target::new();
        let identity = Identity::capture(&target.info()).unwrap();
        let mut cases = vec![identity.clone(); 6];
        cases[0].hwnd += 1;
        cases[1].pid += 1;
        cases[2].thread_id += 1;
        cases[3].class.push('x');
        cases[4].creation += 1;
        cases[5].path.push('x');
        for different in cases {
            assert!(!identity.matches(&different));
        }
        let mut upper = identity.clone();
        upper.path = upper.path.to_uppercase();
        assert!(identity.matches(&upper));
    }

    #[test]
    fn atomic_log_removes_only_restored_record_and_rejects_duplicates() {
        let recovery = Recovery::testing();
        let first = Target::new();
        let second = Target::new();
        let first_id = seed(&first, &recovery, 0);
        let second_id = seed(&second, &recovery, 1);
        assert_eq!(read_log(&recovery).records.len(), 2);
        assert!(
            recovery
                .record(&first.info(), 0, &window::wide("duplicate"))
                .is_err()
        );
        recovery.remove(&first_id).unwrap();
        let remaining = read_log(&recovery).records;
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].identity, second_id);
        assert!(recovery.finish().is_err());
        recovery.remove(&second_id).unwrap();
        assert!(read_log(&recovery).records.is_empty());
    }

    #[test]
    fn startup_restores_matching_windows_and_archives_mismatch_and_duplicates() {
        let recovery = Recovery::testing();
        recovery.0.borrow_mut().temporary = false;
        let first = Target::new();
        let mismatch = Target::new();
        let duplicate = Target::new();
        seed(&first, &recovery, 0);
        seed(&mismatch, &recovery, 0);
        seed(&duplicate, &recovery, 0);
        for target in [&first, &mismatch, &duplicate] {
            restore_close_state(target.0, 1).unwrap();
        }
        let mut records = read_log(&recovery).records;
        records[1].identity.creation += 1;
        records.push(records[2].clone());
        recovery.0.borrow_mut().commit(records).unwrap();
        let path = recovery.0.borrow().path.clone();
        drop(recovery);
        let (recovery, message) = Recovery::begin_path(path);
        let message = message.unwrap();
        assert!(message.contains("成功 1"), "{message}");
        assert!(message.contains("跳过 3"), "{message}");
        assert_eq!(first.state(), 0);
        assert_ne!(mismatch.state(), 0);
        assert_ne!(duplicate.state(), 0);
        assert!(read_log(&recovery).records.is_empty());
        clear_directory(recovery);
    }

    #[test]
    fn failed_restore_retains_record_without_losing_other_windows() {
        let recovery = Recovery::testing();
        recovery.0.borrow_mut().temporary = false;
        let failed = Target::new();
        let successful = Target::new();
        let failed_id = seed(&failed, &recovery, 0);
        seed(&successful, &recovery, 1);
        unsafe {
            DeleteMenu(GetSystemMenu(failed.0, false), SC_CLOSE, MF_BYCOMMAND).unwrap();
        }
        let path = recovery.0.borrow().path.clone();
        drop(recovery);
        let (recovery, message) = Recovery::begin_path(path);
        let message = message.unwrap();
        assert!(
            message.contains("成功 1") && message.contains("失败 1"),
            "{message}"
        );
        assert_eq!(read_log(&recovery).records[0].identity, failed_id);
        assert_ne!(successful.state(), 0);
        assert!(recovery.finish().is_err());
        recovery.remove(&failed_id).unwrap();
        clear_directory(recovery);
    }

    #[test]
    fn corrupt_log_is_backed_up_and_unwritable_log_blocks_protection() {
        let recovery = Recovery::testing();
        recovery.0.borrow_mut().temporary = false;
        let path = recovery.0.borrow().path.clone().unwrap();
        fs::write(&path, b"broken log").unwrap();
        drop(recovery);
        let (recovery, message) = Recovery::begin_path(Some(path.clone()));
        assert!(message.unwrap().contains("恢复日志损坏"));
        let backups: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|file| file.file_name().to_string_lossy().contains("corrupt"))
            .collect();
        assert_eq!(backups.len(), 1);
        assert_eq!(fs::read(backups[0].path()).unwrap(), b"broken log");
        fs::create_dir(&path).unwrap();
        let target = Target::new();
        assert!(
            crate::protection::Protection::start_logged(
                target.info(),
                crate::protection::Options {
                    mouse_shield: false,
                    ..Default::default()
                },
                recovery.clone()
            )
            .is_err()
        );
        assert_eq!(target.state(), 0);
        fs::remove_dir(&path).unwrap();
        clear_directory(recovery);
    }

    #[test]
    fn missing_window_marker_prevents_recovery_of_reused_identity() {
        let recovery = Recovery::testing();
        recovery.0.borrow_mut().temporary = false;
        let target = Target::new();
        seed(&target, &recovery, 0);
        restore_close_state(target.0, 1).unwrap();
        let record = read_log(&recovery).records.remove(0);
        unsafe {
            RemovePropW(target.0, PCWSTR(record.marker.as_ptr())).unwrap();
        }
        let path = recovery.0.borrow().path.clone();
        drop(recovery);
        let (recovery, message) = Recovery::begin_path(path);
        assert!(message.unwrap().contains("跳过 1"));
        assert_ne!(target.state(), 0);
        clear_directory(recovery);
    }
}
