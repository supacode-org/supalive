//! 拉流、斷線重連，並把資料交給封裝層。

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use tokio::sync::watch;

use crate::flv::{parse_video, Demuxer, Video, TAG_VIDEO};
use crate::sink::{new_sink, Sink, SinkOptions};
use crate::tr;
use crate::{Event, EventFn};

pub const DEFAULT_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 \
     (KHTML, like Gecko) Chrome/156.0.0.0 Safari/537.36";

#[derive(Debug, Clone)]
pub struct RecordOptions {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub sink: SinkOptions,
    /// 連續失敗幾次視為直播結束
    pub retries: u32,
}

/// 錄到直播結束、重試用盡，或 `stop` 變成 true 為止。回傳產生的切片。
pub async fn record(opts: RecordOptions, mut stop: watch::Receiver<bool>, events: EventFn) -> Result<Vec<PathBuf>> {
    let client = build_client(&opts.headers)?;
    let mut sink = new_sink(opts.sink.clone(), events.clone());
    let mut failures = 0u32;
    let mut attempt = 0u32;
    let mut total = 0u64;
    while !*stop.borrow() {
        attempt += 1;
        events(Event::Connecting { attempt });
        let started = Instant::now();
        let result = record_once(&client, &opts, sink.as_mut(), &mut stop, &events, &mut total).await;
        // 每次斷線都封好目前的檔案，重連後開新檔
        sink.close()?;
        if *stop.borrow() {
            break;
        }
        if started.elapsed() > Duration::from_secs(60) {
            failures = 0; // 曾穩定錄製過，重新計算重試次數
        }
        failures += 1;
        let reason = match result {
            Ok(()) => tr!("串流結束", "Stream ended"),
            Err(e) => format!("{e:#}"),
        };
        events(Event::Disconnected { reason, failures, max: opts.retries });
        if failures >= opts.retries {
            break;
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(5)) => {}
            _ = stop.changed() => {}
        }
    }
    events(Event::Finished);
    Ok(sink.created().to_vec())
}

fn build_client(headers: &[(String, String)]) -> Result<reqwest::Client> {
    use reqwest::header::{HeaderMap, HeaderName, HeaderValue, USER_AGENT};
    let mut map = HeaderMap::new();
    map.insert(USER_AGENT, HeaderValue::from_static(DEFAULT_UA));
    for (k, v) in headers {
        let name = HeaderName::from_bytes(k.as_bytes())
            .with_context(|| tr!("無效的請求頭名稱 {k}", "Invalid header name {k}"))?;
        let value = HeaderValue::from_str(v).with_context(|| tr!("無效的請求頭值 {k}", "Invalid header value {k}"))?;
        map.insert(name, value);
    }
    Ok(reqwest::Client::builder()
        .default_headers(map)
        .connect_timeout(Duration::from_secs(15))
        // 超過這麼久沒收到資料就當作斷線
        .read_timeout(Duration::from_secs(20))
        .build()?)
}

async fn record_once(
    client: &reqwest::Client,
    opts: &RecordOptions,
    sink: &mut (dyn Sink + Send),
    stop: &mut watch::Receiver<bool>,
    events: &EventFn,
    total: &mut u64,
) -> Result<()> {
    let mut resp = client.get(&opts.url).send().await.with_context(|| tr!("連線失敗", "Connection failed"))?;
    let status = resp.status();
    if !status.is_success() {
        bail!("HTTP {status}");
    }
    events(Event::Connected);
    let mut demux = Demuxer::new();
    let mut received = 0u64;
    let mut last_report = Instant::now();
    let mut last_stats = Instant::now();
    let mut fps = FrameRate::default();
    loop {
        let chunk = tokio::select! {
            c = resp.chunk() => c.with_context(|| tr!("串流中斷", "Stream interrupted"))?,
            _ = stop.changed() => return Ok(()),
        };
        let Some(chunk) = chunk else {
            return Ok(());
        };
        received += chunk.len() as u64;
        *total += chunk.len() as u64;
        demux.push(&chunk);
        while let Some(tag) = demux.next_tag()? {
            if tag.kind == TAG_VIDEO && matches!(parse_video(&tag.data), Video::Frame { .. }) {
                fps.push(tag.timestamp);
            }
            sink.on_tag(&tag)?;
        }
        if last_stats.elapsed() >= Duration::from_secs(1) {
            events(Event::Stats { bytes: *total, fps: fps.take() });
            last_stats = Instant::now();
        }
        if last_report.elapsed() > Duration::from_secs(60) {
            events(Event::Progress { bytes: received });
            last_report = Instant::now();
        }
    }
}

/// 從影片時間戳估算幀率：區間內的幀數除以時間戳的跨度。
/// 用時間戳而不是收到的速度，剛連上時 CDN 一次送來好幾秒的緩衝也不會算錯。
#[derive(Default)]
struct FrameRate {
    count: u32,
    first: Option<u32>,
    last: u32,
}

impl FrameRate {
    fn push(&mut self, ts: u32) {
        self.first.get_or_insert(ts);
        self.last = ts;
        self.count += 1;
    }

    /// 回傳這個區間的幀率並開始下一個區間；幀太少算不出來時回傳 None。
    fn take(&mut self) -> Option<f32> {
        let fps = match self.first {
            Some(first) if self.count >= 2 && self.last > first => {
                Some((self.count - 1) as f32 * 1000.0 / (self.last - first) as f32)
            }
            _ => None,
        };
        // 下一個區間從這一幀接著算，區間之間的那一段才不會漏掉
        if self.count > 0 {
            *self = FrameRate { count: 1, first: Some(self.last), last: self.last };
        }
        fps
    }
}

#[cfg(test)]
mod tests {
    use super::FrameRate;

    #[test]
    fn frame_rate_from_timestamps() {
        let mut r = FrameRate::default();
        assert_eq!(r.take(), None);
        // 30 fps，時間戳每幀約 33 ms；第一個區間一次收到 2 秒的量也照算
        for i in 0..61 {
            r.push(i * 1000 / 30);
        }
        assert_eq!(r.take().map(|f| f.round()), Some(30.0));
        for i in 61..76 {
            r.push(i * 1000 / 30);
        }
        assert_eq!(r.take().map(|f| f.round()), Some(30.0));
        // 這一秒沒收到任何幀
        assert_eq!(r.take(), None);
    }
}
