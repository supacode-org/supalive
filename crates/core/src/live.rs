//! 邊錄邊播：把正在寫入的分段 MP4 同步分發給播放器（例如 WebView 的 MSE）。
//! 保留最近幾分鐘的片段，讓剛打開的播放器可以往回拖一小段。

use std::collections::VecDeque;
use std::sync::Mutex;

use bytes::Bytes;
use tokio::sync::broadcast;

/// 回看視窗的上限，超過就丟掉最舊的片段。
const BACKLOG_SECS: f64 = 180.0;
const BACKLOG_BYTES: usize = 96 << 20;

#[derive(Debug, Clone)]
pub enum LivePacket {
    /// 新檔案開始（ftyp + moov）。`codecs` 可直接給 MediaSource.isTypeSupported 用。
    Init { data: Bytes, codecs: String },
    /// 一組 moof + mdat
    Fragment { data: Bytes, secs: f64 },
}

#[derive(Default)]
struct State {
    init: Option<(Bytes, String)>,
    backlog: VecDeque<(Bytes, f64)>,
    backlog_bytes: usize,
}

pub struct LiveTap {
    tx: broadcast::Sender<LivePacket>,
    state: Mutex<State>,
}

impl std::fmt::Debug for LiveTap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LiveTap")
    }
}

impl Default for LiveTap {
    fn default() -> Self {
        Self { tx: broadcast::channel(256).0, state: Mutex::default() }
    }
}

impl LiveTap {
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn init(&self, data: Bytes, codecs: String) {
        let mut s = self.state.lock().unwrap();
        s.init = Some((data.clone(), codecs.clone()));
        s.backlog.clear();
        s.backlog_bytes = 0;
        let _ = self.tx.send(LivePacket::Init { data, codecs });
    }

    pub(crate) fn fragment(&self, data: Bytes, secs: f64) {
        let mut s = self.state.lock().unwrap();
        s.backlog_bytes += data.len();
        s.backlog.push_back((data.clone(), secs));
        let total: f64 = s.backlog.iter().map(|(_, d)| d).sum();
        let mut dropped = 0.0;
        while s.backlog.len() > 1 && (total - dropped > BACKLOG_SECS || s.backlog_bytes > BACKLOG_BYTES) {
            let (old, d) = s.backlog.pop_front().unwrap();
            s.backlog_bytes -= old.len();
            dropped += d;
        }
        let _ = self.tx.send(LivePacket::Fragment { data, secs });
    }

    /// 回傳目前的 init、回看片段，以及之後的即時片段。三者之間不會漏也不會重複。
    pub fn subscribe(&self) -> (Vec<LivePacket>, broadcast::Receiver<LivePacket>) {
        let s = self.state.lock().unwrap();
        let rx = self.tx.subscribe();
        let mut head = vec![];
        if let Some((data, codecs)) = &s.init {
            head.push(LivePacket::Init { data: data.clone(), codecs: codecs.clone() });
            head.extend(s.backlog.iter().map(|(d, secs)| LivePacket::Fragment { data: d.clone(), secs: *secs }));
        }
        (head, rx)
    }
}
