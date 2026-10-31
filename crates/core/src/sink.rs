//! 把 FLV tag 寫成檔案：分段 MP4（預設）或改寫過的原始 FLV。
//! 兩者都只在關鍵幀處切檔，切檔、重連、換解析度都會開新檔。

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use bytes::Bytes;

use crate::diskbuf::{BufferedFile, WriteBudget};
use crate::flv::{self, Audio, Tag, Video, VideoCodec, TAG_AUDIO, TAG_VIDEO};
use crate::fmp4::{self, AudioTrack, Fragment, Sample, VideoTrack, AUDIO_TRACK, VIDEO_TRACK};
use crate::sps;
use crate::tr;
use crate::Event;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Mp4,
    Flv,
}

impl Format {
    pub fn ext(self) -> &'static str {
        match self {
            Format::Mp4 => "mp4",
            Format::Flv => "flv",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SinkOptions {
    pub dir: PathBuf,
    /// 每個檔案的長度，0 表示不分段
    pub segment_secs: u64,
    /// 只保留最新 N 個檔案，0 表示全部保留
    pub keep: usize,
    pub format: Format,
    /// 檔名前綴，例如主播名
    pub prefix: String,
    /// 邊錄邊播：MP4 模式下把每個片段同步送出
    pub tap: Option<Arc<crate::live::LiveTap>>,
    /// 寫檔緩衝的上限（位元組），攢滿才寫進磁碟。見 `diskbuf`
    pub buffer: usize,
    /// 所有錄製共用的緩衝總量，沒有就只看 `buffer`
    pub budget: Option<Arc<WriteBudget>>,
}

/// FLV 時間戳在同一條連線內應該連續，超出這個範圍就當作斷層並切新檔。
const MAX_BACKWARD_MS: i64 = 1_000;
const MAX_FORWARD_MS: i64 = 10_000;

pub trait Sink {
    fn on_tag(&mut self, tag: &Tag) -> Result<()>;
    /// 封裝並關閉目前的檔案（斷線重連或停止時呼叫）。
    fn close(&mut self) -> Result<()>;
    /// 已完成且仍保留的檔案，依產生順序。
    fn created(&self) -> &[PathBuf];
}

pub fn new_sink(opts: SinkOptions, events: crate::EventFn) -> Box<dyn Sink + Send> {
    match opts.format {
        Format::Mp4 => Box::new(Mp4Sink::new(opts, events)),
        Format::Flv => Box::new(FlvSink::new(opts, events)),
    }
}

/// 管理檔名、輪替刪除這些兩種格式共用的部分。
struct Files {
    opts: SinkOptions,
    events: crate::EventFn,
    created: Vec<PathBuf>,
}

impl Files {
    fn open(&mut self) -> Result<(BufferedFile, PathBuf)> {
        std::fs::create_dir_all(&self.opts.dir)
            .with_context(|| tr!("無法建立輸出目錄 {}", "Cannot create output folder {}", self.opts.dir.display()))?;
        let stamp = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
        let base = if self.opts.prefix.is_empty() { stamp } else { format!("{}_{stamp}", self.opts.prefix) };
        // create_new：檔名已存在就換下一個，不會覆蓋別人的檔案
        let mut n = 1;
        let (file, path) = loop {
            let name = if n == 1 {
                format!("{base}.{}", self.opts.format.ext())
            } else {
                format!("{base}_{n}.{}", self.opts.format.ext())
            };
            let path = self.opts.dir.join(name);
            match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(f) => break (f, path),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => n += 1,
                Err(e) => return Err(e).with_context(|| tr!("無法建立 {}", "Cannot create {}", path.display())),
            }
        };
        (self.events)(Event::FileOpened(path.clone()));
        Ok((BufferedFile::new(file, self.opts.buffer, self.opts.budget.clone()), path))
    }

    fn closed(&mut self, mut w: BufferedFile, path: PathBuf) -> Result<()> {
        w.flush()?;
        drop(w);
        (self.events)(Event::FileClosed(path.clone()));
        self.created.push(path);
        self.prune();
        Ok(())
    }

    /// 只刪本次錄製自己產生的檔案，不碰目錄裡的其他東西。
    fn prune(&mut self) {
        let keep = self.opts.keep;
        if keep == 0 || self.created.len() <= keep {
            return;
        }
        let old: Vec<_> = self.created.drain(..self.created.len() - keep).collect();
        for p in old {
            if std::fs::remove_file(&p).is_ok() {
                (self.events)(Event::FileRemoved(p));
            }
        }
    }

    fn segment_due(&self, elapsed_ms: i64) -> bool {
        self.opts.segment_secs > 0 && elapsed_ms >= self.opts.segment_secs as i64 * 1000
    }
}

fn is_discontinuous(prev: Option<u32>, ts: u32) -> bool {
    prev.is_some_and(|p| {
        let d = ts as i64 - p as i64;
        !(-MAX_BACKWARD_MS..=MAX_FORWARD_MS).contains(&d)
    })
}

// ---------------------------------------------------------------- MP4

struct Mp4File {
    w: BufferedFile,
    path: PathBuf,
    /// 這個檔案時間軸的零點（FLV 毫秒）
    base_ms: u32,
    sequence: u32,
    has_audio: bool,
    sample_rate: u32,
    video: Vec<(i64, Sample)>,
    audio: Vec<(i64, Sample)>,
    last_video_duration: u32,
}

struct Mp4Sink {
    files: Files,
    video: Option<VideoTrack>,
    audio: Option<AudioTrack>,
    file: Option<Mp4File>,
    last_video_ts: Option<u32>,
}

impl Mp4Sink {
    fn new(opts: SinkOptions, events: crate::EventFn) -> Self {
        Self {
            files: Files { opts, events, created: vec![] },
            video: None,
            audio: None,
            file: None,
            last_video_ts: None,
        }
    }

    fn start_file(&mut self, base_ms: u32) -> Result<()> {
        let video = self.video.as_ref().expect("有視訊設定才會開檔");
        let (mut w, path) = self.files.open()?;
        let init = Bytes::from(fmp4::init_segment(Some(video), self.audio.as_ref()));
        w.write_all(&init)?;
        if let Some(tap) = &self.files.opts.tap {
            let mut codecs = fmp4::video_codec_string(video);
            if let Some(a) = &self.audio {
                codecs = format!("{codecs},{}", fmp4::audio_codec_string(a));
            }
            tap.init(init, codecs);
        }
        self.file = Some(Mp4File {
            w,
            path,
            base_ms,
            sequence: 0,
            has_audio: self.audio.is_some(),
            sample_rate: self.audio.as_ref().map_or(1, |a| a.sample_rate),
            video: vec![],
            audio: vec![],
            last_video_duration: 33,
        });
        Ok(())
    }

    /// 寫出佇列中的樣本。`next_video_dts` 是下一個關鍵幀的時間，用來算最後一幀的長度。
    fn flush(file: &mut Mp4File, next_video_dts: Option<i64>, tap: Option<&crate::live::LiveTap>) -> Result<()> {
        if file.video.is_empty() && file.audio.is_empty() {
            return Ok(());
        }
        let n = file.video.len();
        for i in 0..n {
            let next = if i + 1 < n { Some(file.video[i + 1].0) } else { next_video_dts };
            let d = match next {
                Some(t) => (t - file.video[i].0).clamp(0, u32::MAX as i64) as u32,
                None => file.last_video_duration,
            };
            file.video[i].1.duration = d;
            if d > 0 {
                file.last_video_duration = d;
            }
        }
        let video: Vec<Sample> = file.video.iter().map(|(_, s)| s.clone()).collect();
        let audio: Vec<Sample> = file.audio.iter().map(|(_, s)| s.clone()).collect();
        let mut frags = vec![];
        if let Some((dts, _)) = file.video.first() {
            frags.push(Fragment { track_id: VIDEO_TRACK, base_time: *dts as u64, samples: &video });
        }
        if let Some((dts, _)) = file.audio.first() {
            let base = (*dts as i128 * file.sample_rate as i128 / 1000) as u64;
            frags.push(Fragment { track_id: AUDIO_TRACK, base_time: base, samples: &audio });
        }
        file.sequence += 1;
        let segment = fmp4::media_segment(file.sequence, &frags);
        file.w.write_all(&segment)?;
        if let Some(tap) = tap {
            // 邊錄邊播直接拿記憶體裡的片段，不讀檔案，所以不用先寫進磁碟
            let secs = video.iter().map(|s| s.duration as f64).sum::<f64>() / 1000.0;
            tap.fragment(Bytes::from(segment), secs);
        }
        file.video.clear();
        file.audio.clear();
        Ok(())
    }

    fn on_video(&mut self, ts: u32, v: Video) -> Result<()> {
        match v {
            Video::Config { codec, record } => {
                let changed = self.video.as_ref().is_none_or(|old| old.codec != codec || old.record != record);
                if changed {
                    self.close()?;
                    self.video = Some(video_track(codec, record, &self.files.events));
                }
            }
            Video::Frame { key, cts, data, .. } => {
                if self.video.is_none() {
                    return Ok(());
                }
                if is_discontinuous(self.last_video_ts, ts) {
                    (self.files.events)(Event::Warning(tr!(
                        "時間戳跳變 {:?} -> {ts}，切新檔",
                        "Timestamp jumped {:?} -> {ts}; starting a new file",
                        self.last_video_ts
                    )));
                    self.close()?;
                }
                self.last_video_ts = Some(ts);
                if key {
                    let mut due = false;
                    if let Some(f) = self.file.as_mut() {
                        let dts = ts as i64 - f.base_ms as i64;
                        Self::flush(f, Some(dts), self.files.opts.tap.as_deref())?;
                        due = self.files.segment_due(dts);
                    }
                    if due {
                        self.close()?;
                        self.last_video_ts = Some(ts);
                    }
                    if self.file.is_none() {
                        self.start_file(ts)?;
                    }
                }
                let Some(f) = self.file.as_mut() else {
                    return Ok(()); // 還沒等到第一個關鍵幀
                };
                let dts = ts as i64 - f.base_ms as i64;
                f.video.push((dts, Sample { duration: 0, cts, key, data }));
            }
            Video::End | Video::Unsupported => {}
        }
        Ok(())
    }

    fn on_audio(&mut self, ts: u32, a: Audio) -> Result<()> {
        match a {
            Audio::Config(asc) => {
                let Some(info) = flv::parse_asc(&asc) else {
                    (self.files.events)(Event::Warning(tr!(
                        "無法解析 AAC 設定，略過音訊",
                        "Cannot parse the AAC config; skipping audio"
                    )));
                    return Ok(());
                };
                let changed = self.audio.as_ref().is_none_or(|old| old.asc != asc);
                if changed {
                    if self.file.as_ref().is_some_and(|f| f.has_audio) {
                        self.close()?;
                    }
                    self.audio = Some(AudioTrack { asc, sample_rate: info.sample_rate, channels: info.channels });
                }
            }
            Audio::Frame(data) => {
                let Some(f) = self.file.as_mut().filter(|f| f.has_audio) else {
                    return Ok(());
                };
                let dts = ts as i64 - f.base_ms as i64;
                if dts < 0 {
                    return Ok(());
                }
                f.audio.push((dts, Sample { duration: 1024, cts: 0, key: true, data }));
            }
            Audio::Unsupported => {}
        }
        Ok(())
    }
}

fn video_track(codec: VideoCodec, record: Bytes, events: &crate::EventFn) -> VideoTrack {
    let dims = match codec {
        VideoCodec::Avc => sps::sps_from_avcc(&record).map(sps::h264_dimensions),
        VideoCodec::Hevc => sps::sps_from_hvcc(&record).map(sps::hevc_dimensions),
    };
    let (width, height) = match dims {
        Some(Ok(d)) => d,
        _ => {
            events(Event::Warning(tr!(
                "無法從 SPS 讀出解析度，暫用 1920x1080",
                "Cannot read the resolution from SPS; assuming 1920x1080"
            )));
            (1920, 1080)
        }
    };
    events(Event::Stream { codec, width, height });
    VideoTrack { codec, record, width, height }
}

impl Sink for Mp4Sink {
    fn created(&self) -> &[PathBuf] {
        &self.files.created
    }

    fn on_tag(&mut self, tag: &Tag) -> Result<()> {
        match tag.kind {
            TAG_VIDEO => self.on_video(tag.timestamp, flv::parse_video(&tag.data)),
            TAG_AUDIO => self.on_audio(tag.timestamp, flv::parse_audio(&tag.data)),
            _ => Ok(()),
        }
    }

    fn close(&mut self) -> Result<()> {
        self.last_video_ts = None;
        if let Some(mut f) = self.file.take() {
            Self::flush(&mut f, None, self.files.opts.tap.as_deref())?;
            self.files.closed(f.w, f.path)?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- FLV

struct FlvFile {
    w: BufferedFile,
    path: PathBuf,
    base_ms: u32,
}

/// 原始 FLV：資料原樣保留，只把 codec 12 改寫成 Enhanced FLV，並丟掉會誤導播放器的 onMetaData。
struct FlvSink {
    files: Files,
    video_config: Option<Bytes>,
    audio_config: Option<Bytes>,
    file: Option<FlvFile>,
    last_video_ts: Option<u32>,
}

impl FlvSink {
    fn new(opts: SinkOptions, events: crate::EventFn) -> Self {
        Self {
            files: Files { opts, events, created: vec![] },
            video_config: None,
            audio_config: None,
            file: None,
            last_video_ts: None,
        }
    }

    fn write_tag(w: &mut impl Write, kind: u8, ts: u32, data: &[u8]) -> Result<()> {
        let size = data.len() as u32;
        w.write_all(&[kind])?;
        w.write_all(&size.to_be_bytes()[1..])?;
        w.write_all(&ts.to_be_bytes()[1..])?;
        w.write_all(&[(ts >> 24) as u8, 0, 0, 0])?;
        w.write_all(data)?;
        w.write_all(&(11 + size).to_be_bytes())?;
        Ok(())
    }

    fn start_file(&mut self, base_ms: u32) -> Result<()> {
        let (mut w, path) = self.files.open()?;
        let flags = if self.audio_config.is_some() { 0x05 } else { 0x01 };
        w.write_all(&[b'F', b'L', b'V', 1, flags, 0, 0, 0, 9, 0, 0, 0, 0])?;
        if let Some(c) = &self.video_config {
            Self::write_tag(&mut w, TAG_VIDEO, 0, c)?;
        }
        if let Some(c) = &self.audio_config {
            Self::write_tag(&mut w, TAG_AUDIO, 0, c)?;
        }
        self.file = Some(FlvFile { w, path, base_ms });
        Ok(())
    }
}

impl Sink for FlvSink {
    fn created(&self) -> &[PathBuf] {
        &self.files.created
    }

    fn on_tag(&mut self, tag: &Tag) -> Result<()> {
        let ts = tag.timestamp;
        match tag.kind {
            TAG_VIDEO => {
                let data = flv::to_enhanced_flv(&tag.data);
                match flv::parse_video(&tag.data) {
                    Video::Config { .. } => {
                        if self.video_config.as_ref() != Some(&data) {
                            self.close()?;
                            self.video_config = Some(data);
                        }
                        return Ok(());
                    }
                    Video::Frame { key, .. } => {
                        if is_discontinuous(self.last_video_ts, ts) {
                            self.close()?;
                        }
                        self.last_video_ts = Some(ts);
                        if key {
                            let due = self
                                .file
                                .as_ref()
                                .is_some_and(|f| self.files.segment_due(ts as i64 - f.base_ms as i64));
                            if due {
                                self.close()?;
                            }
                            if self.file.is_none() && self.video_config.is_some() {
                                self.start_file(ts)?;
                            }
                        }
                        if let Some(f) = self.file.as_mut() {
                            Self::write_tag(&mut f.w, TAG_VIDEO, ts.wrapping_sub(f.base_ms), &data)?;
                        }
                    }
                    _ => {}
                }
            }
            TAG_AUDIO => {
                if let Audio::Config(_) = flv::parse_audio(&tag.data) {
                    if self.audio_config.as_ref() != Some(&tag.data) {
                        self.audio_config = Some(tag.data.clone());
                        if let Some(f) = self.file.as_mut() {
                            Self::write_tag(&mut f.w, TAG_AUDIO, ts.saturating_sub(f.base_ms), &tag.data)?;
                        }
                    }
                    return Ok(());
                }
                if let Some(f) = self.file.as_mut() {
                    if ts >= f.base_ms {
                        Self::write_tag(&mut f.w, TAG_AUDIO, ts - f.base_ms, &tag.data)?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        self.last_video_ts = None;
        if let Some(f) = self.file.take() {
            self.files.closed(f.w, f.path)?;
        }
        Ok(())
    }
}

/// 離線轉換：把本機 FLV 檔走同一條封裝流程，方便測試與修復舊錄影。
pub fn remux_file(input: &Path, opts: SinkOptions, events: crate::EventFn) -> Result<Vec<PathBuf>> {
    use std::io::Read;
    let mut file = File::open(input).with_context(|| tr!("無法開啟 {}", "Cannot open {}", input.display()))?;
    let mut demux = flv::Demuxer::new();
    let mut sink = new_sink(opts, events);
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        demux.push(&buf[..n]);
        while let Some(tag) = demux.next_tag()? {
            sink.on_tag(&tag)?;
        }
    }
    sink.close()?;
    Ok(sink.created().to_vec())
}
