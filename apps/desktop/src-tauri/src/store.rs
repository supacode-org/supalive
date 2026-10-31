//! 設定與直播間清單，存成 JSON 放在系統的 App 設定資料夾。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use supalive_core::session::{default_root, DEFAULT_SEGMENT_MINUTES};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub seg_min: u64,
    pub auto_merge: bool,
    pub keep_clips: bool,
    pub warn_multi: bool,
    pub eco: bool,
    pub root: PathBuf,
    /// 每路錄製的寫檔緩衝（MB），4–64
    pub buffer_mb: usize,
    /// 所有錄製加起來的寫檔緩衝（MB），64–256
    pub buffer_total_mb: usize,
    /// 介面語言：system（跟隨系統）、zh-Hant、en
    pub lang: String,
}

impl Settings {
    pub fn buffer_bytes(&self) -> usize {
        self.buffer_mb.clamp(4, 64) << 20
    }

    pub fn buffer_total_bytes(&self) -> usize {
        self.buffer_total_mb.clamp(64, 256) << 20
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            seg_min: DEFAULT_SEGMENT_MINUTES,
            auto_merge: true,
            keep_clips: true,
            warn_multi: true,
            eco: false,
            root: default_root(),
            buffer_mb: 16,
            buffer_total_mb: 64,
            lang: "system".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RecStatus {
    Rec,
    Merging,
    Done,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Recording {
    pub dir: PathBuf,
    /// 毫秒時間戳
    pub started: i64,
    pub ended: Option<i64>,
    pub bytes: u64,
    pub clips: Vec<PathBuf>,
    pub merged: Vec<PathBuf>,
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub status: RecStatus,
    /// 最後一次斷線或錯誤的原因
    pub note: String,
}

impl Default for Recording {
    fn default() -> Self {
        Self {
            dir: PathBuf::new(),
            started: 0,
            ended: None,
            bytes: 0,
            clips: vec![],
            merged: vec![],
            codec: String::new(),
            width: 0,
            height: 0,
            status: RecStatus::Done,
            note: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Room {
    pub id: u64,
    pub name: String,
    pub anchor: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    /// 抖音直播間網址裡的數字 ID，用來辨識同一個直播間
    pub web_rid: Option<String>,
    pub quality: String,
    pub added_at: i64,
    /// 新的在後
    pub recordings: Vec<Recording>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Data {
    pub settings: Settings,
    pub rooms: Vec<Room>,
    pub next_id: u64,
}

impl Data {
    pub fn load(path: &Path) -> Self {
        let mut d: Data = std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        // 上次關閉時還在錄的，一律視為已結束
        for r in &mut d.rooms {
            for rec in &mut r.recordings {
                if rec.status != RecStatus::Done {
                    rec.status = RecStatus::Done;
                    // 沒正常結束的，以最後寫入的分段時間當結束時間
                    let last =
                        rec.clips.iter().filter_map(|c| std::fs::metadata(c).and_then(|m| m.modified()).ok()).max();
                    let last =
                        last.and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64);
                    rec.ended.get_or_insert(last.unwrap_or(rec.started).max(rec.started));
                }
            }
        }
        d.next_id = d.next_id.max(d.rooms.iter().map(|r| r.id).max().unwrap_or(0) + 1);
        d
    }

    pub fn save(&self, path: &Path) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        // 先寫暫存檔再改名，避免寫到一半當機留下壞檔
        let tmp = path.with_extension("json.tmp");
        if let Ok(bytes) = serde_json::to_vec_pretty(self) {
            if std::fs::write(&tmp, bytes).is_ok() {
                let _ = std::fs::rename(&tmp, path);
            }
        }
    }

    pub fn room_mut(&mut self, id: u64) -> Option<&mut Room> {
        self.rooms.iter_mut().find(|r| r.id == id)
    }
}
