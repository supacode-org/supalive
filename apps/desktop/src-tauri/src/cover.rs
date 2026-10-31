//! 錄製封面：介面停止播放時留下的最後一幅畫面，存成錄製資料夾裡的 `cover.jpg`。
//! 寫入時機和頻率由介面控制，這裡只負責檢查內容和安全地寫檔。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::Serialize;
use supalive_core::tr;

pub const FILE: &str = "cover.jpg";
/// 介面送來的是寬 192 px 的小圖，正常只有幾 KB；超過這個大小一定是送錯了
const MAX_BYTES: usize = 1 << 20;

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Cover {
    pub path: PathBuf,
    /// 毫秒時間戳，介面用來避開舊的快取
    pub mtime: i64,
}

/// 先寫暫存檔再改名，寫到一半中斷也不會留下壞掉的封面。
pub fn write(dir: &Path, data: &[u8]) -> io::Result<()> {
    if !data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, tr!("封面不是 JPEG", "The cover is not a JPEG")));
    }
    if data.len() > MAX_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, tr!("封面太大", "The cover is too large")));
    }
    if !dir.is_dir() {
        return Err(io::Error::new(io::ErrorKind::NotFound, tr!("找不到錄製資料夾", "Recording folder not found")));
    }
    let tmp = dir.join(format!("{FILE}.tmp"));
    fs::write(&tmp, data)?;
    fs::rename(&tmp, dir.join(FILE))
}

/// 讀取某次錄製的封面，沒有就回傳 `None`。
pub fn find(dir: &Path) -> Option<Cover> {
    let path = dir.join(FILE);
    let meta = fs::metadata(&path).ok().filter(|m| m.is_file())?;
    let mtime = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_millis() as i64;
    Some(Cover { path, mtime })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("supalive-cover-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn writes_and_finds() {
        let d = tmp_dir("ok");
        assert_eq!(find(&d), None);
        write(&d, &[0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3]).unwrap();
        let c = find(&d).unwrap();
        assert_eq!(c.path, d.join(FILE));
        assert!(c.mtime > 0);
        assert!(!d.join("cover.jpg.tmp").exists());
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn rejects_bad_input() {
        let d = tmp_dir("bad");
        assert!(write(&d, b"\x89PNG....").is_err());
        let mut big = vec![0xFF, 0xD8, 0xFF];
        big.resize(MAX_BYTES + 1, 0);
        assert!(write(&d, &big).is_err());
        assert!(write(&d.join("gone"), &[0xFF, 0xD8, 0xFF]).is_err());
        assert_eq!(find(&d), None);
        fs::remove_dir_all(&d).unwrap();
    }
}
