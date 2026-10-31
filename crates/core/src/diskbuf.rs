//! 寫檔緩衝：錄製的資料先留在記憶體，攢到上限才一次寫進磁碟，減少頻繁的小寫入。
//!
//! 每路錄製有自己的上限；所有錄製另外共用一個總上限（`WriteBudget`）。
//! 加起來超過總上限時，正在寫的那一路先把自己的緩衝寫出去，不追求精準。
//! 檔案關閉時（`flush`）和物件被釋放時（包括出錯、panic）都會把剩下的寫出去。

use std::fs::File;
use std::io::{self, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

pub const DEFAULT_ROOM_BYTES: usize = 16 << 20;
pub const DEFAULT_TOTAL_BYTES: usize = 64 << 20;

/// 所有錄製共用的緩衝總量。
#[derive(Debug)]
pub struct WriteBudget {
    used: AtomicUsize,
    limit: AtomicUsize,
}

impl WriteBudget {
    pub fn new(limit: usize) -> Arc<Self> {
        Arc::new(Self { used: AtomicUsize::new(0), limit: AtomicUsize::new(limit) })
    }

    /// 改總上限，立刻生效。
    pub fn set_limit(&self, limit: usize) {
        self.limit.store(limit, Ordering::Relaxed);
    }

    /// 目前所有錄製緩衝中、還沒寫進磁碟的位元組。
    pub fn used(&self) -> usize {
        self.used.load(Ordering::Relaxed)
    }

    fn would_exceed(&self, extra: usize) -> bool {
        self.used() + extra > self.limit.load(Ordering::Relaxed)
    }
}

pub struct BufferedFile {
    file: File,
    buf: Vec<u8>,
    limit: usize,
    budget: Option<Arc<WriteBudget>>,
}

impl BufferedFile {
    /// `limit` 是這一路的緩衝上限，0 表示不緩衝、每次都直接寫。
    pub fn new(file: File, limit: usize, budget: Option<Arc<WriteBudget>>) -> Self {
        Self { file, buf: Vec::new(), limit, budget }
    }

    fn over_budget(&self, extra: usize) -> bool {
        self.budget.as_ref().is_some_and(|b| b.would_exceed(extra))
    }

    /// 把緩衝整批寫進檔案。寫失敗時資料也丟掉，不再佔著總量。
    fn write_out(&mut self) -> io::Result<()> {
        if self.buf.is_empty() {
            return Ok(());
        }
        let result = self.file.write_all(&self.buf);
        if let Some(b) = &self.budget {
            b.used.fetch_sub(self.buf.len(), Ordering::Relaxed);
        }
        self.buf.clear();
        result
    }
}

impl Write for BufferedFile {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if self.buf.len() + data.len() > self.limit || self.over_budget(data.len()) {
            self.write_out()?;
        }
        // 單筆就超過上限，或總量被別的直播間佔滿了：直接寫，不進緩衝
        if data.len() > self.limit || self.over_budget(data.len()) {
            self.file.write_all(data)?;
            return Ok(data.len());
        }
        self.buf.extend_from_slice(data);
        if let Some(b) = &self.budget {
            b.used.fetch_add(data.len(), Ordering::Relaxed);
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.write_out()?;
        self.file.flush()
    }
}

impl Drop for BufferedFile {
    fn drop(&mut self) {
        // 出錯或 panic 時沒有走到正常的關檔流程，這裡是最後一次寫出的機會
        let _ = self.write_out();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("supalive-diskbuf-{name}-{}", std::process::id()))
    }

    fn size(p: &std::path::Path) -> u64 {
        std::fs::metadata(p).unwrap().len()
    }

    #[test]
    fn writes_in_batches() {
        let p = temp("batch");
        let mut w = BufferedFile::new(File::create(&p).unwrap(), 100, None);
        w.write_all(&[1; 60]).unwrap();
        assert_eq!(size(&p), 0, "未滿上限時不寫");
        w.write_all(&[2; 60]).unwrap();
        assert_eq!(size(&p), 60, "放不下時先把前一批寫出去");
        w.write_all(&[3; 150]).unwrap();
        assert_eq!(size(&p), 270, "單筆超過上限直接寫");
        w.write_all(&[4; 10]).unwrap();
        w.flush().unwrap();
        assert_eq!(size(&p), 280);
        drop(w);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn shares_total_budget_and_writes_on_drop() {
        let (pa, pb) = (temp("a"), temp("b"));
        let budget = WriteBudget::new(100);
        let mut a = BufferedFile::new(File::create(&pa).unwrap(), 80, Some(budget.clone()));
        let mut b = BufferedFile::new(File::create(&pb).unwrap(), 80, Some(budget.clone()));
        a.write_all(&[1; 70]).unwrap();
        assert_eq!(budget.used(), 70);
        // 加起來會超過總上限：b 自己沒有緩衝可寫，這一筆直接寫進檔案
        b.write_all(&[2; 40]).unwrap();
        assert_eq!((size(&pa), size(&pb), budget.used()), (0, 40, 70));
        b.write_all(&[2; 20]).unwrap();
        assert_eq!(budget.used(), 90);
        // a 再寫會超過總上限：先把自己的 70 寫出去
        a.write_all(&[1; 20]).unwrap();
        assert_eq!((size(&pa), budget.used()), (70, 40));
        drop(a);
        drop(b);
        assert_eq!((size(&pa), size(&pb), budget.used()), (90, 60, 0), "釋放時寫出剩下的");
        let _ = std::fs::remove_file(&pa);
        let _ = std::fs::remove_file(&pb);
    }
}
