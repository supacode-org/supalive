//! 從 H.264 / HEVC 的 SPS 讀出畫面寬高。MP4 的 tkhd 與 sample entry 需要寬高，
//! 而 FLV 的 onMetaData 不一定可靠。

use anyhow::{bail, Context, Result};

use crate::tr;

/// 去掉防競爭位元組（00 00 03 -> 00 00）。
fn unescape(nal: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(nal.len());
    let mut zeros = 0;
    for &b in nal {
        if zeros >= 2 && b == 3 {
            zeros = 0;
            continue;
        }
        zeros = if b == 0 { zeros + 1 } else { 0 };
        out.push(b);
    }
    out
}

struct BitReader {
    data: Vec<u8>,
    pos: usize,
}

impl BitReader {
    fn new(data: Vec<u8>) -> Self {
        Self { data, pos: 0 }
    }

    fn bit(&mut self) -> Result<u32> {
        let byte = *self.data.get(self.pos / 8).with_context(|| tr!("SPS 資料不足", "SPS data is too short"))?;
        let v = (byte >> (7 - self.pos % 8)) & 1;
        self.pos += 1;
        Ok(v as u32)
    }

    fn bits(&mut self, n: u32) -> Result<u32> {
        let mut v = 0;
        for _ in 0..n {
            v = (v << 1) | self.bit()?;
        }
        Ok(v)
    }

    fn skip(&mut self, n: usize) {
        self.pos += n;
    }

    fn ue(&mut self) -> Result<u32> {
        let mut zeros = 0;
        while self.bit()? == 0 {
            zeros += 1;
            if zeros > 31 {
                bail!(tr!("SPS 中的 Exp-Golomb 編碼無效", "Invalid Exp-Golomb code in SPS"));
            }
        }
        Ok(((1u64 << zeros) - 1 + self.bits(zeros)? as u64) as u32)
    }

    fn se(&mut self) -> Result<i32> {
        let v = self.ue()?;
        Ok(if v & 1 == 1 { (v as i32 + 1) / 2 } else { -((v / 2) as i32) })
    }
}

/// H.264 SPS（含 1 位元組 NAL 頭）。
pub fn h264_dimensions(nal: &[u8]) -> Result<(u32, u32)> {
    let mut r = BitReader::new(unescape(nal));
    r.skip(8); // NAL 頭
    let profile = r.bits(8)?;
    r.skip(16); // constraint flags + level
    r.ue()?; // sps id
    let mut chroma = 1;
    let mut separate_planes = false;
    if matches!(profile, 100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128 | 138 | 139 | 134 | 135) {
        chroma = r.ue()?;
        if chroma == 3 {
            separate_planes = r.bit()? == 1;
        }
        r.ue()?; // bit depth luma
        r.ue()?; // bit depth chroma
        r.bit()?; // qpprime_y_zero_transform_bypass
        if r.bit()? == 1 {
            let lists = if chroma == 3 { 12 } else { 8 };
            for i in 0..lists {
                if r.bit()? == 1 {
                    let size = if i < 6 { 16 } else { 64 };
                    let (mut last, mut next) = (8i32, 8i32);
                    for _ in 0..size {
                        if next != 0 {
                            next = (last + r.se()? + 256) % 256;
                        }
                        if next != 0 {
                            last = next;
                        }
                    }
                }
            }
        }
    }
    r.ue()?; // log2_max_frame_num
    match r.ue()? {
        0 => {
            r.ue()?;
        }
        1 => {
            r.bit()?;
            r.se()?;
            r.se()?;
            for _ in 0..r.ue()? {
                r.se()?;
            }
        }
        _ => {}
    }
    r.ue()?; // max_num_ref_frames
    r.bit()?; // gaps_in_frame_num_allowed
    let w_mbs = r.ue()? + 1;
    let h_units = r.ue()? + 1;
    let frame_mbs_only = r.bit()?;
    if frame_mbs_only == 0 {
        r.bit()?;
    }
    r.bit()?; // direct_8x8_inference
    let (mut cl, mut cr, mut ct, mut cb) = (0, 0, 0, 0);
    if r.bit()? == 1 {
        cl = r.ue()?;
        cr = r.ue()?;
        ct = r.ue()?;
        cb = r.ue()?;
    }
    let array_type = if separate_planes { 0 } else { chroma };
    let (sub_w, sub_h) = match array_type {
        1 => (2, 2),
        2 => (2, 1),
        _ => (1, 1),
    };
    let (unit_x, unit_y) =
        if array_type == 0 { (1, 2 - frame_mbs_only) } else { (sub_w, sub_h * (2 - frame_mbs_only)) };
    let width = w_mbs * 16 - (cl + cr) * unit_x;
    let height = (2 - frame_mbs_only) * h_units * 16 - (ct + cb) * unit_y;
    Ok((width, height))
}

/// HEVC SPS（含 2 位元組 NAL 頭）。
pub fn hevc_dimensions(nal: &[u8]) -> Result<(u32, u32)> {
    let mut r = BitReader::new(unescape(nal));
    r.skip(16); // NAL 頭
    r.skip(4); // vps id
    let max_sub_layers_minus1 = r.bits(3)?;
    r.skip(1); // temporal_id_nesting
               // profile_tier_level：general 部分 88 位 + level 8 位
    r.skip(96);
    let mut profile_present = [false; 8];
    let mut level_present = [false; 8];
    for i in 0..max_sub_layers_minus1 as usize {
        profile_present[i] = r.bit()? == 1;
        level_present[i] = r.bit()? == 1;
    }
    if max_sub_layers_minus1 > 0 {
        r.skip(2 * (8 - max_sub_layers_minus1 as usize));
    }
    for i in 0..max_sub_layers_minus1 as usize {
        if profile_present[i] {
            r.skip(88);
        }
        if level_present[i] {
            r.skip(8);
        }
    }
    r.ue()?; // sps id
    let chroma = r.ue()?;
    let mut separate_planes = false;
    if chroma == 3 {
        separate_planes = r.bit()? == 1;
    }
    let width = r.ue()?;
    let height = r.ue()?;
    let (mut cl, mut cr, mut ct, mut cb) = (0, 0, 0, 0);
    if r.bit()? == 1 {
        cl = r.ue()?;
        cr = r.ue()?;
        ct = r.ue()?;
        cb = r.ue()?;
    }
    let (sub_w, sub_h) = match (chroma, separate_planes) {
        (1, _) => (2, 2),
        (2, _) => (2, 1),
        _ => (1, 1),
    };
    Ok((width - sub_w * (cl + cr), height - sub_h * (ct + cb)))
}

/// 從 AVCDecoderConfigurationRecord 取第一個 SPS。
pub fn sps_from_avcc(record: &[u8]) -> Option<&[u8]> {
    if record.len() < 8 || record[5] & 0x1f == 0 {
        return None;
    }
    let len = u16::from_be_bytes([record[6], record[7]]) as usize;
    record.get(8..8 + len)
}

/// 從 HEVCDecoderConfigurationRecord 取第一個 SPS（NAL type 33）。
pub fn sps_from_hvcc(record: &[u8]) -> Option<&[u8]> {
    let arrays = *record.get(22)?;
    let mut p = 23;
    for _ in 0..arrays {
        let nal_type = record.get(p)? & 0x3f;
        let count = u16::from_be_bytes([*record.get(p + 1)?, *record.get(p + 2)?]);
        p += 3;
        for _ in 0..count {
            let len = u16::from_be_bytes([*record.get(p)?, *record.get(p + 1)?]) as usize;
            p += 2;
            let nal = record.get(p..p + len)?;
            if nal_type == 33 {
                return Some(nal);
            }
            p += len;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unescape_removes_emulation_prevention() {
        assert_eq!(unescape(&[0, 0, 3, 1, 0, 0, 3]), vec![0, 0, 1, 0, 0]);
    }

    #[test]
    fn h264_1080p() {
        // x264 1920x1080 High profile SPS
        let sps = [
            0x67, 0x64, 0x00, 0x28, 0xac, 0xd9, 0x40, 0x78, 0x02, 0x27, 0xe5, 0xc0, 0x44, 0x00, 0x00, 0x03, 0x00, 0x04,
            0x00, 0x00, 0x03, 0x00, 0xf0, 0x3c, 0x60, 0xc6, 0x58,
        ];
        assert_eq!(h264_dimensions(&sps).unwrap(), (1920, 1080));
    }
}
