//! FLV 串流拆包。支援 H.264（codec 7）、抖音非標準 HEVC（codec 12）、
//! Enhanced FLV（avc1 / hvc1）與 AAC。

use anyhow::{bail, Result};
use bytes::{Buf, Bytes, BytesMut};

use crate::tr;

pub const TAG_AUDIO: u8 = 8;
pub const TAG_VIDEO: u8 = 9;
pub const TAG_SCRIPT: u8 = 18;

#[derive(Debug, Clone)]
pub struct Tag {
    pub kind: u8,
    /// 毫秒，FLV 裡這是 DTS
    pub timestamp: u32,
    pub data: Bytes,
}

/// 增量解析：每收到一段網路資料就 `push`，再反覆呼叫 `next_tag`。
#[derive(Default)]
pub struct Demuxer {
    buf: BytesMut,
    header_done: bool,
}

impl Demuxer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }

    pub fn next_tag(&mut self) -> Result<Option<Tag>> {
        if !self.header_done {
            if self.buf.len() < 9 {
                return Ok(None);
            }
            if &self.buf[..3] != b"FLV" {
                bail!(tr!("回應不是 FLV 串流", "The response is not an FLV stream"));
            }
            let header_len = u32::from_be_bytes([self.buf[5], self.buf[6], self.buf[7], self.buf[8]]) as usize;
            if self.buf.len() < header_len + 4 {
                return Ok(None);
            }
            self.buf.advance(header_len + 4); // 檔頭 + PreviousTagSize0
            self.header_done = true;
        }
        loop {
            if self.buf.len() < 11 {
                return Ok(None);
            }
            let size = u32::from_be_bytes([0, self.buf[1], self.buf[2], self.buf[3]]) as usize;
            if self.buf.len() < 11 + size + 4 {
                return Ok(None);
            }
            let kind = self.buf[0] & 0x1f;
            let timestamp = u32::from_be_bytes([self.buf[7], self.buf[4], self.buf[5], self.buf[6]]);
            let encrypted = self.buf[0] & 0x20 != 0;
            self.buf.advance(11);
            let data = self.buf.split_to(size).freeze();
            self.buf.advance(4);
            if encrypted || data.is_empty() {
                continue;
            }
            return Ok(Some(Tag { kind, timestamp, data }));
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoCodec {
    Avc,
    Hevc,
}

#[derive(Debug, Clone)]
pub enum Video {
    /// avcC / hvcC 解碼器設定
    Config {
        codec: VideoCodec,
        record: Bytes,
    },
    /// 長度前綴（4 位元組）的 NAL 單元
    Frame {
        codec: VideoCodec,
        key: bool,
        cts: i32,
        data: Bytes,
    },
    End,
    Unsupported,
}

fn i24(b: &[u8]) -> i32 {
    ((b[0] as i32) << 24 | (b[1] as i32) << 16 | (b[2] as i32) << 8) >> 8
}

pub fn parse_video(data: &Bytes) -> Video {
    if data.len() < 5 {
        return Video::Unsupported;
    }
    let b0 = data[0];
    let frame_type = (b0 >> 4) & 0x07;
    if frame_type == 5 {
        return Video::Unsupported; // 資訊 / 指令幀
    }
    let key = frame_type == 1;
    if b0 & 0x80 != 0 {
        // Enhanced FLV
        let codec = match &data[1..5] {
            b"avc1" => VideoCodec::Avc,
            b"hvc1" => VideoCodec::Hevc,
            _ => return Video::Unsupported,
        };
        return match b0 & 0x0f {
            0 => Video::Config { codec, record: data.slice(5..) },
            1 if data.len() >= 8 => Video::Frame { codec, key, cts: i24(&data[5..8]), data: data.slice(8..) },
            3 => Video::Frame { codec, key, cts: 0, data: data.slice(5..) },
            2 => Video::End,
            _ => Video::Unsupported,
        };
    }
    let codec = match b0 & 0x0f {
        7 => VideoCodec::Avc,
        12 => VideoCodec::Hevc, // 抖音 / 國內 CDN 的非標準擴展
        _ => return Video::Unsupported,
    };
    match data[1] {
        0 => Video::Config { codec, record: data.slice(5..) },
        1 => Video::Frame { codec, key, cts: i24(&data[2..5]), data: data.slice(5..) },
        2 => Video::End,
        _ => Video::Unsupported,
    }
}

#[derive(Debug, Clone)]
pub enum Audio {
    /// AudioSpecificConfig
    Config(Bytes),
    /// 一個 raw AAC 幀
    Frame(Bytes),
    Unsupported,
}

pub fn parse_audio(data: &Bytes) -> Audio {
    if data.len() < 2 || data[0] >> 4 != 10 {
        return Audio::Unsupported;
    }
    match data[1] {
        0 => Audio::Config(data.slice(2..)),
        1 => Audio::Frame(data.slice(2..)),
        _ => Audio::Unsupported,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AacInfo {
    pub sample_rate: u32,
    pub channels: u16,
}

pub fn parse_asc(asc: &[u8]) -> Option<AacInfo> {
    const RATES: [u32; 13] = [96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350];
    let mut bits: u64 = 0;
    for (i, &b) in asc.iter().take(8).enumerate() {
        bits |= (b as u64) << (56 - 8 * i);
    }
    let mut pos = 0;
    let mut take = |n: u32| {
        let v = (bits << pos) >> (64 - n);
        pos += n;
        v as u32
    };
    if take(5) == 31 {
        take(6);
    }
    let idx = take(4);
    let sample_rate = if idx == 15 { take(24) } else { *RATES.get(idx as usize)? };
    let channels = match take(4) {
        0 => 2,
        7 => 8,
        c => c as u16,
    };
    Some(AacInfo { sample_rate, channels })
}

/// 原始 FLV 模式用：把 codec 12 改寫成 Enhanced FLV 'hvc1'，其他格式原樣返回。
pub fn to_enhanced_flv(data: &Bytes) -> Bytes {
    if data.len() < 5 || data[0] & 0x80 != 0 || data[0] & 0x0f != 12 {
        return data.clone();
    }
    let frame_type = (data[0] >> 4) & 0x07;
    let mut out = BytesMut::with_capacity(data.len() + 4);
    match data[1] {
        0 => {
            out.extend_from_slice(&[0x80 | frame_type << 4]);
            out.extend_from_slice(b"hvc1");
            out.extend_from_slice(&data[5..]);
        }
        1 => {
            out.extend_from_slice(&[0x80 | frame_type << 4 | 1]);
            out.extend_from_slice(b"hvc1");
            out.extend_from_slice(&data[2..]);
        }
        _ => {
            out.extend_from_slice(&[0x80 | frame_type << 4 | 2]);
            out.extend_from_slice(b"hvc1");
        }
    }
    out.freeze()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(kind: u8, ts: u32, data: &[u8]) -> Vec<u8> {
        let mut v = vec![kind];
        v.extend_from_slice(&(data.len() as u32).to_be_bytes()[1..]);
        v.extend_from_slice(&ts.to_be_bytes()[1..]);
        v.push((ts >> 24) as u8);
        v.extend_from_slice(&[0, 0, 0]);
        v.extend_from_slice(data);
        v.extend_from_slice(&(11 + data.len() as u32).to_be_bytes());
        v
    }

    #[test]
    fn demux_in_small_chunks() {
        let mut stream = b"FLV\x01\x05\x00\x00\x00\x09\x00\x00\x00\x00".to_vec();
        stream.extend(tag(9, 0x0100_0010, &[0x17, 0, 0, 0, 0, 1, 2]));
        stream.extend(tag(8, 20, &[0xaf, 1, 9]));
        let mut d = Demuxer::new();
        let mut tags = vec![];
        for chunk in stream.chunks(3) {
            d.push(chunk);
            while let Some(t) = d.next_tag().unwrap() {
                tags.push(t);
            }
        }
        assert_eq!(tags.len(), 2);
        assert_eq!(tags[0].timestamp, 0x0100_0010);
        assert!(matches!(parse_video(&tags[0].data), Video::Config { codec: VideoCodec::Avc, .. }));
        assert!(matches!(parse_audio(&tags[1].data), Audio::Frame(_)));
    }

    #[test]
    fn rejects_non_flv() {
        let mut d = Demuxer::new();
        d.push(b"<html>forbidden</html>");
        assert!(d.next_tag().is_err());
    }

    #[test]
    fn codec12_frame_with_negative_cts() {
        let data = Bytes::from_static(&[0x2c, 1, 0xff, 0xff, 0xfe, 0, 0, 0, 1, 0x02]);
        match parse_video(&data) {
            Video::Frame { codec, key, cts, data } => {
                assert_eq!(codec, VideoCodec::Hevc);
                assert!(!key);
                assert_eq!(cts, -2);
                assert_eq!(data.len(), 5);
            }
            other => panic!("{other:?}"),
        }
        let enhanced = to_enhanced_flv(&data);
        assert_eq!(&enhanced[..5], &[0xa1, b'h', b'v', b'c', b'1']);
        assert!(matches!(parse_video(&enhanced), Video::Frame { cts: -2, .. }));
    }

    #[test]
    fn asc() {
        assert_eq!(parse_asc(&[0x12, 0x10]), Some(AacInfo { sample_rate: 44100, channels: 2 }));
        assert_eq!(parse_asc(&[0x11, 0x90]), Some(AacInfo { sample_rate: 48000, channels: 2 }));
    }
}
