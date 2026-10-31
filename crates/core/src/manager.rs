//! 多個直播間同時錄製的管理器。
//!
//! 每個錄製各自擁有 HTTP 連線、停止訊號、資料夾與檔案，彼此不共用可變狀態；
//! 管理器只負責登記、防止同一直播間被重複錄製、統一停止。數量不設上限，
//! 「同時超過 2 個可能觸發風控」由介面層提示。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{bail, Result};
use tokio::sync::{broadcast, watch};
use tokio::task::JoinHandle;

use crate::diskbuf::{WriteBudget, DEFAULT_TOTAL_BYTES};
use crate::live::{LivePacket, LiveTap};
use crate::session::{self, SessionDirs};
use crate::tr;
use crate::{record, Event, EventFn, Format, RecordOptions, SinkOptions};

pub type RecordingId = u64;

/// 帶錄製編號的事件回呼，給介面區分是哪個直播間。
pub type ManagerEventFn = Arc<dyn Fn(RecordingId, Event) + Send + Sync>;

#[derive(Debug, Clone)]
pub struct StartRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
    /// 直播間名稱，用於資料夾命名
    pub room_name: Option<String>,
    /// 直播間唯一識別（例如 web_rid），用來防止重複錄製；不給就從拉流網址推斷
    pub room_key: Option<String>,
    pub base_dir: PathBuf,
    pub segment_secs: u64,
    pub format: Format,
    pub merge: bool,
    pub keep_clips: bool,
    pub retries: u32,
    /// 這一路的寫檔緩衝上限（位元組），攢滿才寫進磁碟。見 `diskbuf`
    pub buffer: usize,
}

#[derive(Debug, Clone)]
pub struct RecordingInfo {
    pub id: RecordingId,
    pub room_key: String,
    pub room_name: Option<String>,
    pub dir: PathBuf,
}

struct Entry {
    info: RecordingInfo,
    tap: Arc<LiveTap>,
    stop: watch::Sender<bool>,
    handle: Option<JoinHandle<()>>,
}

#[derive(Clone)]
pub struct Manager {
    inner: Arc<Mutex<HashMap<RecordingId, Entry>>>,
    next_id: Arc<AtomicU64>,
    events: ManagerEventFn,
    /// 所有錄製共用的寫檔緩衝總量
    budget: Arc<WriteBudget>,
}

/// 等到程式被要求結束：Ctrl+C（SIGINT），macOS 與 Linux 另外接 SIGTERM（系統關機、`kill`）。
/// 收到後應該停止所有錄製並等它們封裝完，緩衝裡的資料才會寫進磁碟。
pub async fn termination() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut term = match signal(SignalKind::terminate()) {
            Ok(s) => s,
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
                return;
            }
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

/// 抖音拉流網址形如 .../stream-123456_or4.flv，去掉畫質後綴後當作直播間識別。
pub fn room_key_from_url(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let file = path.rsplit('/').next().unwrap_or(path);
    let stem = file.rsplit_once('.').map_or(file, |(s, _)| s);
    stem.split('_').next().unwrap_or(stem).to_ascii_lowercase()
}

impl Manager {
    pub fn new(events: ManagerEventFn) -> Self {
        Self {
            inner: Default::default(),
            next_id: Arc::new(AtomicU64::new(1)),
            events,
            budget: WriteBudget::new(DEFAULT_TOTAL_BYTES),
        }
    }

    /// 所有錄製加起來的寫檔緩衝上限（位元組），立刻生效。
    pub fn set_buffer_total(&self, bytes: usize) {
        self.budget.set_limit(bytes);
    }

    pub fn start(&self, req: StartRequest) -> Result<RecordingId> {
        let room_key = req.room_key.clone().unwrap_or_else(|| room_key_from_url(&req.url));
        let mut map = self.inner.lock().unwrap();
        if map.values().any(|e| e.info.room_key == room_key) {
            bail!(tr!("這個直播間已經在錄製中", "This room is already being recorded"));
        }
        let dirs = session::session_dirs(&req.base_dir, req.room_name.as_deref())?;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (stop_tx, stop_rx) = watch::channel(false);
        let info = RecordingInfo { id, room_key, room_name: req.room_name.clone(), dir: dirs.root.clone() };
        let tap = Arc::new(LiveTap::new());
        map.insert(id, Entry { info, tap: tap.clone(), stop: stop_tx, handle: None });
        let handle = tokio::spawn(self.clone().run(id, req, dirs, stop_rx, tap));
        map.get_mut(&id).unwrap().handle = Some(handle);
        Ok(id)
    }

    async fn run(
        self,
        id: RecordingId,
        req: StartRequest,
        dirs: SessionDirs,
        stop: watch::Receiver<bool>,
        tap: Arc<LiveTap>,
    ) {
        let all = self.events.clone();
        let events: EventFn = Arc::new(move |e| all(id, e));
        let sink = SinkOptions {
            dir: dirs.clips.clone(),
            segment_secs: req.segment_secs,
            keep: 0,
            format: req.format,
            prefix: String::new(),
            tap: Some(tap),
            buffer: req.buffer,
            budget: Some(self.budget.clone()),
        };
        let opts = RecordOptions { url: req.url, headers: req.headers, sink, retries: req.retries };
        let clips = match record(opts, stop, events.clone()).await {
            Ok(c) => c,
            Err(e) => {
                events(Event::Warning(tr!("錄製出錯：{e:#}", "Recording error: {e:#}")));
                list_files(&dirs.clips, req.format)
            }
        };
        if req.merge {
            // 合併是大量檔案 IO，放到阻塞執行緒，不拖慢其他直播間的錄製
            let ev = events.clone();
            let merged =
                tokio::task::spawn_blocking(move || session::finish(&dirs, &clips, req.format, req.keep_clips, &ev))
                    .await;
            match merged {
                Ok(Err(e)) => events(Event::Warning(tr!("合併失敗：{e:#}", "Merge failed: {e:#}"))),
                Err(e) => events(Event::Warning(tr!("合併失敗：{e}", "Merge failed: {e}"))),
                Ok(Ok(_)) => {}
            }
        }
        self.inner.lock().unwrap().remove(&id);
        events(Event::Done);
    }

    /// 邊錄邊播：取得這個錄製的即時片段。錄製已結束時回傳 None。
    pub fn subscribe(&self, id: RecordingId) -> Option<(Vec<LivePacket>, broadcast::Receiver<LivePacket>)> {
        let tap = self.inner.lock().unwrap().get(&id)?.tap.clone();
        Some(tap.subscribe())
    }

    pub fn stop(&self, id: RecordingId) {
        if let Some(e) = self.inner.lock().unwrap().get(&id) {
            let _ = e.stop.send(true);
        }
    }

    pub fn stop_all(&self) {
        for e in self.inner.lock().unwrap().values() {
            let _ = e.stop.send(true);
        }
    }

    pub fn active(&self) -> Vec<RecordingInfo> {
        let mut v: Vec<_> = self.inner.lock().unwrap().values().map(|e| e.info.clone()).collect();
        v.sort_by_key(|i| i.id);
        v
    }

    /// 等所有錄製（含合併）結束。
    pub async fn wait_all(&self) {
        loop {
            let handles: Vec<_> = self.inner.lock().unwrap().values_mut().filter_map(|e| e.handle.take()).collect();
            if handles.is_empty() {
                if self.inner.lock().unwrap().is_empty() {
                    return;
                }
                tokio::task::yield_now().await;
                continue;
            }
            for h in handles {
                let _ = h.await;
            }
        }
    }
}

pub fn list_files(dir: &std::path::Path, format: Format) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == format.ext())).collect())
        .unwrap_or_default();
    v.sort();
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn room_key() {
        assert_eq!(
            room_key_from_url("https://pull-flv-l26.douyincdn.com/stage/stream-1234_or4.flv?expire=1"),
            "stream-1234"
        );
        assert_eq!(room_key_from_url("https://x/stage/stream-1234_hd.flv"), "stream-1234");
        assert_eq!(room_key_from_url("https://x/live/abc.flv"), "abc");
    }
}
