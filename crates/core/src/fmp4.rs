//! 分段式 MP4（ISO BMFF fragmented）封裝。只寫容器，不碰位元流內容。

use bytes::Bytes;

use crate::flv::VideoCodec;

pub const VIDEO_TRACK: u32 = 1;
pub const AUDIO_TRACK: u32 = 2;
pub const VIDEO_TIMESCALE: u32 = 1000; // FLV 時間戳就是毫秒

#[derive(Debug, Clone)]
pub struct VideoTrack {
    pub codec: VideoCodec,
    pub record: Bytes,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone)]
pub struct AudioTrack {
    pub asc: Bytes,
    pub sample_rate: u32,
    pub channels: u16,
}

#[derive(Debug, Clone)]
pub struct Sample {
    pub duration: u32,
    pub cts: i32,
    pub key: bool,
    pub data: Bytes,
}

pub struct Fragment<'a> {
    pub track_id: u32,
    pub base_time: u64,
    pub samples: &'a [Sample],
}

struct W(Vec<u8>);

impl W {
    fn new() -> Self {
        W(Vec::new())
    }
    fn u8(&mut self, v: u8) -> &mut Self {
        self.0.push(v);
        self
    }
    fn u16(&mut self, v: u16) -> &mut Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    fn u32(&mut self, v: u32) -> &mut Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    fn u64(&mut self, v: u64) -> &mut Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    fn bytes(&mut self, v: &[u8]) -> &mut Self {
        self.0.extend_from_slice(v);
        self
    }
    fn zeros(&mut self, n: usize) -> &mut Self {
        self.0.resize(self.0.len() + n, 0);
        self
    }
    /// 寫一個 box，內容由閉包產生，長度自動回填。
    fn bx(&mut self, kind: &[u8; 4], f: impl FnOnce(&mut W)) -> &mut Self {
        let start = self.0.len();
        self.u32(0).bytes(kind);
        f(self);
        let size = (self.0.len() - start) as u32;
        self.0[start..start + 4].copy_from_slice(&size.to_be_bytes());
        self
    }
    fn full(&mut self, kind: &[u8; 4], version: u8, flags: u32, f: impl FnOnce(&mut W)) -> &mut Self {
        self.bx(kind, |w| {
            w.u32((version as u32) << 24 | flags);
            f(w);
        })
    }
}

const MATRIX: [u32; 9] = [0x0001_0000, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000];

fn matrix(w: &mut W) {
    for v in MATRIX {
        w.u32(v);
    }
}

/// RFC 6381 編碼字串，例如 `hvc1.1.6.L120.90` 或 `avc1.64001f`。
pub fn video_codec_string(v: &VideoTrack) -> String {
    let r = &v.record;
    match v.codec {
        VideoCodec::Avc if r.len() >= 4 => format!("avc1.{:02x}{:02x}{:02x}", r[1], r[2], r[3]),
        VideoCodec::Avc => "avc1.64001f".into(),
        VideoCodec::Hevc if r.len() >= 13 => {
            let space = ["", "A", "B", "C"][(r[1] >> 6) as usize];
            let tier = if r[1] & 0x20 != 0 { 'H' } else { 'L' };
            let profile = r[1] & 0x1f;
            let compat = u32::from_be_bytes([r[2], r[3], r[4], r[5]]).reverse_bits();
            let mut s = format!("hvc1.{space}{profile}.{compat:x}.{tier}{}", r[12]);
            let constraints = &r[6..12];
            let used = constraints.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
            for b in &constraints[..used] {
                s.push_str(&format!(".{b:x}"));
            }
            s
        }
        VideoCodec::Hevc => "hvc1.1.6.L120.90".into(),
    }
}

pub fn audio_codec_string(a: &AudioTrack) -> String {
    let aot = a.asc.first().map_or(2, |b| b >> 3);
    format!("mp4a.40.{aot}")
}

pub fn init_segment(video: Option<&VideoTrack>, audio: Option<&AudioTrack>) -> Vec<u8> {
    let mut w = W::new();
    w.bx(b"ftyp", |w| {
        w.bytes(b"isom").u32(0x200).bytes(b"isom").bytes(b"iso6").bytes(b"iso5").bytes(b"mp41");
    });
    w.bx(b"moov", |w| {
        w.full(b"mvhd", 0, 0, |w| {
            w.u32(0).u32(0).u32(1000).u32(0).u32(0x0001_0000).u16(0x0100).zeros(10);
            matrix(w);
            w.zeros(24).u32(3);
        });
        if let Some(v) = video {
            video_trak(w, v);
        }
        if let Some(a) = audio {
            audio_trak(w, a);
        }
        w.bx(b"mvex", |w| {
            for id in [video.map(|_| VIDEO_TRACK), audio.map(|_| AUDIO_TRACK)].into_iter().flatten() {
                w.full(b"trex", 0, 0, |w| {
                    w.u32(id).u32(1).u32(0).u32(0).u32(0);
                });
            }
        });
    });
    w.0
}

fn tkhd(w: &mut W, id: u32, audio: bool, width: u32, height: u32) {
    w.full(b"tkhd", 0, 3, |w| {
        w.u32(0).u32(0).u32(id).u32(0).u32(0).zeros(8);
        w.u16(0).u16(0).u16(if audio { 0x0100 } else { 0 }).u16(0);
        matrix(w);
        w.u32(width << 16).u32(height << 16);
    });
}

fn mdia(
    w: &mut W,
    timescale: u32,
    handler: &[u8; 4],
    name: &str,
    header: impl FnOnce(&mut W),
    entry: impl FnOnce(&mut W),
) {
    w.bx(b"mdia", |w| {
        w.full(b"mdhd", 0, 0, |w| {
            w.u32(0).u32(0).u32(timescale).u32(0).u16(0x55c4).u16(0); // und
        });
        w.full(b"hdlr", 0, 0, |w| {
            w.u32(0).bytes(handler).zeros(12).bytes(name.as_bytes()).u8(0);
        });
        w.bx(b"minf", |w| {
            header(w);
            w.bx(b"dinf", |w| {
                w.full(b"dref", 0, 0, |w| {
                    w.u32(1).full(b"url ", 0, 1, |_| {});
                });
            });
            w.bx(b"stbl", |w| {
                w.full(b"stsd", 0, 0, |w| {
                    w.u32(1);
                    entry(w);
                });
                w.full(b"stts", 0, 0, |w| {
                    w.u32(0);
                });
                w.full(b"stsc", 0, 0, |w| {
                    w.u32(0);
                });
                w.full(b"stsz", 0, 0, |w| {
                    w.u32(0).u32(0);
                });
                w.full(b"stco", 0, 0, |w| {
                    w.u32(0);
                });
            });
        });
    });
}

fn video_trak(w: &mut W, v: &VideoTrack) {
    w.bx(b"trak", |w| {
        tkhd(w, VIDEO_TRACK, false, v.width, v.height);
        mdia(
            w,
            VIDEO_TIMESCALE,
            b"vide",
            "VideoHandler",
            |w| {
                w.full(b"vmhd", 0, 1, |w| {
                    w.zeros(8);
                });
            },
            |w| {
                let (entry, config) = match v.codec {
                    VideoCodec::Avc => (b"avc1", b"avcC"),
                    // hvc1：參數集放在 hvcC 裡，QuickTime 只認這個標記
                    VideoCodec::Hevc => (b"hvc1", b"hvcC"),
                };
                w.bx(entry, |w| {
                    w.zeros(6).u16(1).zeros(16);
                    w.u16(v.width as u16).u16(v.height as u16);
                    w.u32(0x0048_0000).u32(0x0048_0000).u32(0).u16(1);
                    w.zeros(32).u16(0x0018).u16(0xffff);
                    w.bx(config, |w| {
                        w.bytes(&v.record);
                    });
                });
            },
        );
    });
}

/// MPEG-4 描述子長度，固定用 4 位元組形式。
fn descriptor(w: &mut W, tag: u8, body: &[u8]) {
    let n = body.len() as u32;
    w.u8(tag)
        .u8(0x80 | (n >> 21) as u8 & 0x7f)
        .u8(0x80 | (n >> 14) as u8 & 0x7f)
        .u8(0x80 | (n >> 7) as u8 & 0x7f)
        .u8(n as u8 & 0x7f)
        .bytes(body);
}

fn audio_trak(w: &mut W, a: &AudioTrack) {
    w.bx(b"trak", |w| {
        tkhd(w, AUDIO_TRACK, true, 0, 0);
        mdia(
            w,
            a.sample_rate,
            b"soun",
            "SoundHandler",
            |w| {
                w.full(b"smhd", 0, 0, |w| {
                    w.u32(0);
                });
            },
            |w| {
                w.bx(b"mp4a", |w| {
                    w.zeros(6).u16(1).zeros(8);
                    w.u16(a.channels).u16(16).u16(0).u16(0);
                    // 取樣率 > 65535 時 16.16 欄位放不下，播放器以 esds 為準
                    w.u32((a.sample_rate.min(65535)) << 16);
                    w.full(b"esds", 0, 0, |w| {
                        let mut dsi = W::new();
                        descriptor(&mut dsi, 0x05, &a.asc);
                        let mut dcd = W::new();
                        dcd.u8(0x40).u8(0x15).zeros(3).u32(0).u32(0).bytes(&dsi.0);
                        let mut es = W::new();
                        es.u16(AUDIO_TRACK as u16).u8(0);
                        descriptor(&mut es, 0x04, &dcd.0);
                        descriptor(&mut es, 0x06, &[0x02]);
                        descriptor(w, 0x03, &es.0);
                    });
                });
            },
        );
    });
}

const FLAGS_KEY: u32 = 0x0200_0000; // sample_depends_on = 2（不依賴其他幀）
const FLAGS_NON_KEY: u32 = 0x0101_0000; // depends_on = 1，is_non_sync

/// 一組 moof + mdat。
pub fn media_segment(sequence: u32, fragments: &[Fragment]) -> Vec<u8> {
    let mut w = W::new();
    let mut offset_slots = Vec::new();
    w.bx(b"moof", |w| {
        w.full(b"mfhd", 0, 0, |w| {
            w.u32(sequence);
        });
        for f in fragments {
            w.bx(b"traf", |w| {
                // default-base-is-moof：trun 的 data_offset 從 moof 開頭算
                w.full(b"tfhd", 0, 0x02_0000, |w| {
                    w.u32(f.track_id);
                });
                w.full(b"tfdt", 1, 0, |w| {
                    w.u64(f.base_time);
                });
                // data_offset | duration | size | flags | composition offset，version 1 允許負的 CTS
                w.full(b"trun", 1, 0x0001 | 0x0100 | 0x0200 | 0x0400 | 0x0800, |w| {
                    w.u32(f.samples.len() as u32);
                    offset_slots.push(w.0.len());
                    w.u32(0);
                    for s in f.samples {
                        w.u32(s.duration).u32(s.data.len() as u32);
                        w.u32(if s.key { FLAGS_KEY } else { FLAGS_NON_KEY });
                        w.u32(s.cts as u32);
                    }
                });
            });
        }
    });
    let moof_len = w.0.len();
    let mut data_offset = moof_len + 8;
    for (slot, f) in offset_slots.iter().zip(fragments) {
        w.0[*slot..*slot + 4].copy_from_slice(&(data_offset as u32).to_be_bytes());
        data_offset += f.samples.iter().map(|s| s.data.len()).sum::<usize>();
    }
    w.bx(b"mdat", |w| {
        for f in fragments {
            for s in f.samples {
                w.bytes(&s.data);
            }
        }
    });
    w.0
}
