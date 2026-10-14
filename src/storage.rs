use std::{fs, io::Write, path::Path};

use windows::{
    Win32::Storage::FileSystem::{MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW},
    core::PCWSTR,
};

use crate::window::wide;

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("存储目录不可用。")?;
    fs::create_dir_all(parent).map_err(|e| format!("无法创建存储目录：{e}"))?;
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let result = (|| {
        let mut file = fs::File::create(&temporary).map_err(|e| e.to_string())?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        drop(file);
        let source = wide(&temporary.to_string_lossy());
        let destination = wide(&path.to_string_lossy());
        unsafe {
            MoveFileExW(
                PCWSTR(source.as_ptr()),
                PCWSTR(destination.as_ptr()),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        }
        .map_err(|e| format!("原子保存失败：{e}"))
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}
