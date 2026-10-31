//! 刪除錄製的檔案：預設移到廢紙簍（Windows 是資源回收筒），也可以永久刪除。
//! 只刪錄製紀錄裡記下的資料夾，刪之前先檢查，避免誤刪輸出位置或其他資料夾。

use std::fs;
use std::path::Path;

use supalive_core::tr;

/// 看起來是不是一次錄製的資料夾：裡面要有 clips/、merged/ 或 cover.jpg 其中之一。
fn looks_like_recording(dir: &Path) -> bool {
    ["clips", "merged", "cover.jpg"].iter().any(|n| dir.join(n).exists())
}

fn to_trash(path: &Path) -> Result<(), trash::Error> {
    #[allow(unused_mut)]
    let mut ctx = trash::TrashContext::default();
    // 預設的 Finder 方式會跳出「要控制 Finder」的權限要求，上架版也不能用；改用系統的 NSFileManager
    #[cfg(target_os = "macos")]
    {
        use trash::macos::{DeleteMethod, TrashContextExtMacos};
        ctx.set_delete_method(DeleteMethod::NsFileManager);
    }
    ctx.delete(path)
}

/// 只剩 .DS_Store 之類的系統檔時也算空的。
fn is_empty_dir(dir: &Path) -> bool {
    fs::read_dir(dir).is_ok_and(|mut it| {
        it.all(|e| e.is_ok_and(|e| matches!(e.file_name().to_str(), Some(".DS_Store" | "Thumbs.db" | "desktop.ini"))))
    })
}

/// 刪除一次錄製的資料夾。資料夾已經不在時當作刪好了。
/// 刪完後直播間的資料夾如果空了也一起清掉（輸出位置本身不動）。
pub fn remove_recording_dir(dir: &Path, root: &Path, permanent: bool) -> Result<(), String> {
    if !dir.exists() {
        return Ok(());
    }
    if !dir.is_absolute() || dir == root || root.starts_with(dir) || !dir.is_dir() || !looks_like_recording(dir) {
        return Err(tr!(
            "{} 看起來不是錄製資料夾，沒有刪除",
            "{} does not look like a recording folder; nothing was deleted",
            dir.display()
        ));
    }
    let result = if permanent {
        fs::remove_dir_all(dir).map_err(|e| e.to_string())
    } else {
        to_trash(dir).map_err(|e| e.to_string())
    };
    result.map_err(|e| tr!("無法刪除 {}：{e}", "Cannot delete {}: {e}", dir.display()))?;
    if let Some(parent) = dir.parent() {
        if parent != root && !root.starts_with(parent) && is_empty_dir(parent) {
            let _ = fs::remove_dir_all(parent);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("supalive-files-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        p
    }

    #[test]
    fn deletes_recording_and_empty_room_folder() {
        let root = temp("root");
        let a = root.join("主播").join("2026-10-26-01-00");
        let b = root.join("主播").join("2026-10-26-02-00");
        for d in [&a, &b] {
            fs::create_dir_all(d.join("clips")).unwrap();
            fs::write(d.join("clips").join("1.mp4"), b"x").unwrap();
        }
        remove_recording_dir(&a, &root, true).unwrap();
        assert!(!a.exists() && b.exists(), "只刪指定的那一次");
        fs::write(root.join("主播").join(".DS_Store"), b"").unwrap();
        remove_recording_dir(&b, &root, true).unwrap();
        assert!(!root.join("主播").exists(), "直播間資料夾空了就清掉");
        assert!(root.exists(), "輸出位置不動");
        // 已經不在的資料夾當作刪好了
        remove_recording_dir(&a, &root, true).unwrap();
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn refuses_folders_that_are_not_recordings() {
        let root = temp("refuse");
        let other = root.join("別的東西");
        fs::create_dir_all(&other).unwrap();
        fs::write(other.join("notes.txt"), b"x").unwrap();
        assert!(remove_recording_dir(&other, &root, true).is_err());
        assert!(remove_recording_dir(&root, &root, true).is_err());
        assert!(other.join("notes.txt").exists());
        let _ = fs::remove_dir_all(&root);
    }
}
