//! SupaLive 核心：拉取抖音直播 FLV，不經 ffmpeg 直接封裝成分段 MP4 或原始 FLV。

pub mod curl;
pub mod diskbuf;
pub mod flv;
pub mod fmp4;
pub mod i18n;
pub mod live;
pub mod manager;
pub mod merge;
pub mod recorder;
pub mod session;
pub mod sink;
mod sps;

use std::path::PathBuf;
use std::sync::Arc;

pub use recorder::{record, RecordOptions};
pub use sink::{remux_file, Format, SinkOptions};

/// 給 CLI / 之後的桌面介面顯示進度用。
#[derive(Debug, Clone)]
pub enum Event {
    Connecting {
        attempt: u32,
    },
    Connected,
    /// 每秒一次：本次錄製（含重連）累計收到的位元組，以及這一秒的幀率（幀太少算不出來時是 None）
    Stats {
        bytes: u64,
        fps: Option<f32>,
    },
    Stream {
        codec: flv::VideoCodec,
        width: u32,
        height: u32,
    },
    FileOpened(PathBuf),
    FileClosed(PathBuf),
    FileRemoved(PathBuf),
    Merged(PathBuf),
    Progress {
        bytes: u64,
    },
    Disconnected {
        reason: String,
        failures: u32,
        max: u32,
    },
    Warning(String),
    Finished,
    /// 錄製與合併都結束了
    Done,
}

pub type EventFn = Arc<dyn Fn(Event) + Send + Sync>;
