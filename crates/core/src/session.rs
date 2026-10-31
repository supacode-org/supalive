//! 一次錄製的目錄結構：<根目錄>/<直播間名>/<時間>/{clips,merged}，以及錄完後的合併。
//! 合併檔仍然叫「直播間名_時間.mp4」，搬離資料夾後也看得出是哪場。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::tr;
use crate::{merge, Event, EventFn, Format};

pub const DEFAULT_SEGMENT_MINUTES: u64 = 15;

/// 讀不到直播間名稱時用的資料夾，跟著介面語言。
pub fn unnamed() -> &'static str {
    if crate::i18n::is_english() {
        "Unnamed room"
    } else {
        "未命名直播間"
    }
}

/// 預設根目錄：使用者的下載資料夾 / SupaLive。
pub fn default_root() -> PathBuf {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join("Downloads").join("SupaLive")
}

/// 檔名不允許的字元換成底線。直播間名稱現在單獨當一層資料夾，
/// 所以還要擋掉 `.`、`..` 和 Windows 不接受的結尾句點、保留裝置名稱。
fn sanitize(name: &str) -> String {
    let s: String =
        name.trim()
            .chars()
            .map(|c| {
                if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control() {
                    '_'
                } else {
                    c
                }
            })
            .collect();
    let s = s.trim_end_matches(['.', ' ']).to_string();
    let stem = s.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit());
    if reserved {
        format!("{s}_")
    } else {
        s
    }
}

/// 資料夾名稱是不是錄製開始的時間（yyyy-mm-dd-hh-mm，可能帶 _2 之類的後綴）。
fn is_time_dir(name: &str) -> bool {
    let b = name.as_bytes();
    b.len() >= 16
        && b[..16]
            .iter()
            .enumerate()
            .all(|(i, c)| if matches!(i, 4 | 7 | 10 | 13) { *c == b'-' } else { c.is_ascii_digit() })
}

pub struct SessionDirs {
    pub name: String,
    pub root: PathBuf,
    pub clips: PathBuf,
    pub merged: PathBuf,
}

impl SessionDirs {
    /// 由既有的錄製資料夾還原（例如手動合併舊錄影）。
    /// 新的「直播間名/時間」結構把兩層名稱接起來當合併檔名；舊的「直播間名_時間」直接用資料夾名稱。
    pub fn from_root(root: PathBuf) -> Self {
        let file_name = |p: &Path| p.file_name().map(|n| n.to_string_lossy().into_owned());
        let dir = file_name(&root).unwrap_or_else(|| "merged".into());
        let name = match root.parent().and_then(file_name) {
            Some(room) if is_time_dir(&dir) => format!("{room}_{dir}"),
            _ => dir,
        };
        Self { clips: root.join("clips"), merged: root.join("merged"), root, name }
    }
}

/// 建立並佔用這次錄製的資料夾：<base>/<直播間名>/<時間>/；`room` 讀不到時放在「未命名直播間」底下。
/// 同一分鐘開始的錄製（同名的直播間同時錄、或都沒有名稱）會自動加上 _2、_3，
/// 用 create_dir 的原子性避免兩個錄製搶到同一個資料夾。
pub fn session_dirs(base: &Path, room: Option<&str>) -> Result<SessionDirs> {
    let time = chrono::Local::now().format("%Y-%m-%d-%H-%M").to_string();
    let room = room.map(sanitize).filter(|r| !r.is_empty()).unwrap_or_else(|| unnamed().into());
    let parent = base.join(&room);
    std::fs::create_dir_all(&parent).with_context(|| tr!("無法建立 {}", "Cannot create {}", parent.display()))?;
    let mut n = 1;
    loop {
        let dir = if n == 1 { time.clone() } else { format!("{time}_{n}") };
        let root = parent.join(&dir);
        match std::fs::create_dir(&root) {
            Ok(()) => {
                let name = format!("{room}_{dir}");
                let dirs = SessionDirs { clips: root.join("clips"), merged: root.join("merged"), root, name };
                std::fs::create_dir_all(&dirs.clips)?;
                return Ok(dirs);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => n += 1,
            Err(e) => return Err(e).with_context(|| tr!("無法建立 {}", "Cannot create {}", root.display())),
        }
    }
}

/// 錄製結束後（不論原因）合併切片；`keep_clips` 為 false 時合併成功才刪切片。
pub fn finish(
    dirs: &SessionDirs,
    clips: &[PathBuf],
    format: Format,
    keep_clips: bool,
    events: &EventFn,
) -> Result<Vec<PathBuf>> {
    if clips.is_empty() {
        return Ok(vec![]);
    }
    if format != Format::Mp4 {
        events(Event::Warning(tr!(
            "目前只支援合併 MP4 切片，FLV 切片保持原樣",
            "Only MP4 segments can be merged; FLV segments are left as they are"
        )));
        return Ok(vec![]);
    }
    let merged = merge::merge_clips(clips, &dirs.merged, &dirs.name)?;
    for m in &merged {
        events(Event::Merged(m.clone()));
    }
    if !keep_clips && !merged.is_empty() {
        for c in clips {
            if std::fs::remove_file(c).is_ok() {
                events(Event::FileRemoved(c.clone()));
            }
        }
        let _ = std::fs::remove_dir(&dirs.clips);
    }
    Ok(merged)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_base(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("supalive-session-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        p
    }

    #[test]
    fn room_then_time() {
        let base = temp_base("layout");
        let a = session_dirs(&base, Some("小滿在唱歌")).unwrap();
        let b = session_dirs(&base, Some("小滿在唱歌")).unwrap();
        assert_eq!(a.root.parent().unwrap(), base.join("小滿在唱歌"));
        assert!(is_time_dir(&a.root.file_name().unwrap().to_string_lossy()));
        assert!(a.clips.is_dir());
        // 同一分鐘的第二場加上 _2（剛好跨到下一分鐘時就是新的時間），合併檔名帶直播間名
        let (an, bn) = (a.root.file_name().unwrap().to_string_lossy(), b.root.file_name().unwrap().to_string_lossy());
        assert_eq!(b.root.parent(), a.root.parent());
        assert!(bn == format!("{an}_2") || (bn != an && is_time_dir(&bn)));
        assert!(a.name.starts_with("小滿在唱歌_"));
        assert_eq!(SessionDirs::from_root(a.root.clone()).name, a.name);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn unnamed_and_unsafe_names_stay_inside_base() {
        let base = temp_base("unsafe");
        for room in [None, Some(""), Some(".."), Some(" . ")] {
            let d = session_dirs(&base, room).unwrap();
            assert_eq!(d.root.parent().unwrap(), base.join(unnamed()), "{room:?}");
        }
        assert_eq!(sanitize("a/b"), "a_b");
        assert_eq!(sanitize("主播..."), "主播");
        assert_eq!(sanitize("con"), "con_");
        assert_eq!(sanitize("COM1.txt"), "COM1.txt_");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn old_layout_keeps_folder_name() {
        assert_eq!(SessionDirs::from_root(PathBuf::from("/r/小滿_2026-10-25-23-12")).name, "小滿_2026-10-25-23-12");
    }
}
