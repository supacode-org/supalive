//! 合併本程式產生的分段 MP4：沿用第一個切片的 ftyp/moov，把後續切片的
//! moof/mdat 接在後面，並改寫 tfdt 讓時間軸連續。不解碼、不重新封裝樣本。
//! 解碼器設定（解析度、編碼）不同的切片無法放進同一個檔案，會各自成為一個合併檔。

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::tr;

struct Clip {
    init: Vec<u8>,
    /// 依序排列的 moof+mdat 原始位元組
    body: Vec<u8>,
    /// 每個 moof 在 body 中的位置
    moofs: Vec<usize>,
}

fn boxes(data: &[u8]) -> Result<Vec<(usize, usize, [u8; 4])>> {
    let mut out = vec![];
    let mut p = 0;
    while p + 8 <= data.len() {
        let mut size = u32::from_be_bytes(data[p..p + 4].try_into().unwrap()) as usize;
        let kind: [u8; 4] = data[p + 4..p + 8].try_into().unwrap();
        if size == 1 && p + 16 <= data.len() {
            size = u64::from_be_bytes(data[p + 8..p + 16].try_into().unwrap()) as usize;
        } else if size == 0 {
            size = data.len() - p;
        }
        if size < 8 || p + size > data.len() {
            break; // 錄製中斷留下的不完整尾巴，丟掉
        }
        out.push((p, size, kind));
        p += size;
    }
    Ok(out)
}

fn read_clip(path: &Path) -> Result<Clip> {
    let mut data = vec![];
    File::open(path)?.read_to_end(&mut data)?;
    let mut init = vec![];
    let mut body = vec![];
    let mut moofs = vec![];
    let list = boxes(&data)?;
    let mut i = 0;
    while i < list.len() {
        let (p, size, kind) = list[i];
        match &kind {
            b"ftyp" | b"moov" => init.extend_from_slice(&data[p..p + size]),
            // moof 必須和緊接的 mdat 成對保留
            b"moof" if i + 1 < list.len() && &list[i + 1].2 == b"mdat" => {
                let (mp, msize, _) = list[i + 1];
                moofs.push(body.len());
                body.extend_from_slice(&data[p..p + size]);
                body.extend_from_slice(&data[mp..mp + msize]);
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }
    if init.is_empty() {
        bail!(tr!("{} 不是有效的 MP4", "{} is not a valid MP4", path.display()));
    }
    Ok(Clip { init, body, moofs })
}

/// 走訪一個 moof：回傳 (track_id, tfdt 欄位位置, tfdt 值, 本片段總長度)。
fn fragments(moof: &[u8]) -> Vec<(u32, Option<usize>, u64, u64)> {
    let mut out = vec![];
    let Ok(top) = boxes(&moof[8..]) else { return out };
    for (tp, tsize, kind) in top {
        if &kind != b"traf" {
            continue;
        }
        let traf = 8 + tp;
        let Ok(children) = boxes(&moof[traf + 8..traf + tsize]) else { continue };
        let (mut track, mut tfdt_pos, mut tfdt, mut total) = (0, None, 0u64, 0u64);
        for (cp, _, ck) in children {
            let c = traf + 8 + cp;
            let version = moof[c + 8];
            let body = c + 12;
            match &ck {
                b"tfhd" => track = u32::from_be_bytes(moof[body..body + 4].try_into().unwrap()),
                b"tfdt" => {
                    tfdt_pos = Some(body);
                    tfdt = if version == 1 {
                        u64::from_be_bytes(moof[body..body + 8].try_into().unwrap())
                    } else {
                        u32::from_be_bytes(moof[body..body + 4].try_into().unwrap()) as u64
                    };
                }
                b"trun" => {
                    let flags = u32::from_be_bytes(moof[c + 8..c + 12].try_into().unwrap()) & 0xff_ffff;
                    let count = u32::from_be_bytes(moof[body..body + 4].try_into().unwrap());
                    let mut q = body + 4;
                    if flags & 0x1 != 0 {
                        q += 4;
                    }
                    if flags & 0x4 != 0 {
                        q += 4;
                    }
                    let per = [0x100, 0x200, 0x400, 0x800].iter().filter(|f| flags & **f != 0).count() * 4;
                    if flags & 0x100 != 0 {
                        for s in 0..count as usize {
                            let at = q + s * per;
                            total += u32::from_be_bytes(moof[at..at + 4].try_into().unwrap()) as u64;
                        }
                    }
                }
                _ => {}
            }
        }
        out.push((track, tfdt_pos, tfdt, total));
    }
    out
}

/// 從 moov 讀出各軌的 timescale（track_id -> timescale）。
fn timescales(init: &[u8]) -> HashMap<u32, u32> {
    let mut out = HashMap::new();
    let find = |data: &[u8], kind: &[u8; 4]| -> Vec<(usize, usize)> {
        boxes(data).unwrap_or_default().into_iter().filter(|b| &b.2 == kind).map(|b| (b.0, b.1)).collect()
    };
    for (mp, msize) in find(init, b"moov") {
        let moov = &init[mp + 8..mp + msize];
        for (tp, tsize) in find(moov, b"trak") {
            let trak = &moov[tp + 8..tp + tsize];
            let mut id = None;
            let mut scale = None;
            for (hp, _) in find(trak, b"tkhd") {
                let v = trak[hp + 8];
                let at = hp + 12 + if v == 1 { 16 } else { 8 };
                id = Some(u32::from_be_bytes(trak[at..at + 4].try_into().unwrap()));
            }
            for (dp, dsize) in find(trak, b"mdia") {
                let mdia = &trak[dp + 8..dp + dsize];
                for (hp, _) in find(mdia, b"mdhd") {
                    let v = mdia[hp + 8];
                    let at = hp + 12 + if v == 1 { 16 } else { 8 };
                    scale = Some(u32::from_be_bytes(mdia[at..at + 4].try_into().unwrap()));
                }
            }
            if let (Some(i), Some(s)) = (id, scale) {
                out.insert(i, s);
            }
        }
    }
    out
}

/// 把 `clips` 依序合併到 `out_dir`，回傳產生的檔案。
pub fn merge_clips(clips: &[PathBuf], out_dir: &Path, name: &str) -> Result<Vec<PathBuf>> {
    std::fs::create_dir_all(out_dir).with_context(|| tr!("無法建立 {}", "Cannot create {}", out_dir.display()))?;
    let mut outputs = vec![];
    let mut current: Option<(Vec<u8>, BufWriter<File>)> = None;
    let mut merged_secs = 0f64;
    let mut sequence = 0u32;
    for path in clips {
        let mut clip = match read_clip(path) {
            Ok(c) if !c.moofs.is_empty() => c,
            _ => continue, // 空切片或壞檔直接略過
        };
        let same = current.as_ref().is_some_and(|(init, _)| *init == clip.init);
        if !same {
            if let Some((_, mut w)) = current.take() {
                w.flush()?;
            }
            let file_name =
                if outputs.is_empty() { format!("{name}.mp4") } else { format!("{name}_{}.mp4", outputs.len() + 1) };
            let out = out_dir.join(file_name);
            let mut w = BufWriter::with_capacity(
                1 << 20,
                File::create(&out).with_context(|| tr!("無法建立 {}", "Cannot create {}", out.display()))?,
            );
            w.write_all(&clip.init)?;
            outputs.push(out);
            current = Some((clip.init.clone(), w));
            merged_secs = 0.0;
            sequence = 0;
        }
        // 切片內各軌相對位置不變；整段接在上一段結尾（以秒計，各軌共用同一個接縫）
        let scales = timescales(&clip.init);
        let mut end_secs = merged_secs;
        for &m in &clip.moofs {
            let size = u32::from_be_bytes(clip.body[m..m + 4].try_into().unwrap()) as usize;
            let moof = &mut clip.body[m..m + size];
            sequence += 1;
            moof[20..24].copy_from_slice(&sequence.to_be_bytes()); // mfhd.sequence_number
            for (track, pos, tfdt, total) in fragments(moof) {
                let scale = *scales.get(&track).unwrap_or(&1000) as f64;
                let new = (merged_secs * scale).round() as u64 + tfdt;
                if let Some(p) = pos {
                    if moof[p - 4] == 1 {
                        moof[p..p + 8].copy_from_slice(&new.to_be_bytes());
                    } else {
                        moof[p..p + 4].copy_from_slice(&(new as u32).to_be_bytes());
                    }
                }
                end_secs = end_secs.max((new + total) as f64 / scale);
            }
        }
        merged_secs = end_secs;
        current.as_mut().unwrap().1.write_all(&clip.body)?;
    }
    if let Some((_, mut w)) = current {
        w.flush()?;
    }
    Ok(outputs)
}
