//! 抖音側欄：主視窗右側嵌一個子 WebView 開抖音網頁版。使用者在裡面登入、
//! 進直播間，注入的腳本讀出頁面自己請求的拉流網址回報給主介面。
//! 側欄的位置和大小由介面量好傳進來，Rust 端不寫死版面。
//!
//! 抓到的拉流網址不會直接拿去錄：同一個直播間的各種畫質都先暫存起來（使用者可能在頁面上切換畫質），
//! 由介面列出來讓使用者選定後，再用一般的 add_room / start_recording 開始錄。

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use supalive_core::tr;
use tauri::webview::NewWindowResponse;
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Url, WebviewBuilder, WebviewUrl};

pub const LABEL: &str = "douyin";
const HOME: &str = "https://live.douyin.com/";
const SCRIPT: &str = include_str!("../inject/douyin.js");

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Bounds {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

/// 從使用者貼上的文字裡找出抖音網址。可以是直播間網址，也可以是 App「分享」複製出來的整段文字，
/// 例如「8- 長按複製此條消息…… https://v.douyin.com/iAbCdEf/ 」。
pub fn find_douyin_url(text: &str) -> Option<Url> {
    text.split(|c: char| c.is_whitespace() || "，。！「」【】\"'<>".contains(c))
        .filter_map(|w| w.find("http").map(|i| &w[i..]))
        .filter_map(|w| Url::parse(w).ok())
        .find(allowed)
}

/// 打開側欄。`url` 有值時跳到那個網址（直播間網址或分享文字），已經打開的側欄也會跳過去。
#[tauri::command]
pub fn douyin_open(app: AppHandle, bounds: Bounds, url: Option<String>) -> Result<(), String> {
    let target = match url.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        Some(text) => Some(find_douyin_url(text).ok_or_else(|| {
            tr!(
                "找不到抖音直播間的網址。請貼上 live.douyin.com 的網址，或抖音 App 分享出來的連結。",
                "No Douyin room link found. Paste a live.douyin.com link, or the link shared from the Douyin app."
            )
        })?),
        None => None,
    };
    if let Some(wv) = app.get_webview(LABEL) {
        wv.show().map_err(|e| e.to_string())?;
        if let Some(target) = target {
            wv.navigate(target).map_err(|e| e.to_string())?;
        }
        return douyin_bounds(app, bounds);
    }
    let window = app.get_window("main").ok_or_else(|| tr!("找不到主視窗", "Main window not found"))?;
    let nav_app = app.clone();
    let new_app = app.clone();
    let start = target.unwrap_or_else(|| Url::parse(HOME).unwrap());
    let builder = WebviewBuilder::new(LABEL, WebviewUrl::External(start))
        .initialization_script(SCRIPT)
        .on_page_load(move |wv, payload| {
            let _ = nav_app.emit("douyin-nav", payload.url().as_str());
            let _ = wv;
        })
        // 只允許在抖音與它的登入頁面之間導航，其他連結不在側欄裡開
        .on_navigation(navigable)
        // 抖音的直播間卡片用開新分頁的方式打開，注入腳本已經改成在本頁打開。
        // 這裡是漏網的備援：WKWebView 不處理新視窗請求時會直接忽略，點了沒反應。
        .on_new_window(move |url, _features| {
            if allowed(&url) {
                let app = new_app.clone();
                // 不在回呼裡直接導航，排到主執行緒下一輪，避免重入 WebView 的委派
                let _ = new_app.run_on_main_thread(move || {
                    if let Some(wv) = app.get_webview(LABEL) {
                        let _ = wv.navigate(url);
                    }
                });
            }
            NewWindowResponse::Deny
        });
    window
        .add_child(
            builder,
            LogicalPosition::new(bounds.x, bounds.y),
            LogicalSize::new(bounds.w.max(1.0), bounds.h.max(1.0)),
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// 導航的檢查對每個框架都會跑，不只主頁面。頁面內部的空白框架、blob、data 網址也要放行，
/// 否則抖音自己的腳本會壞掉。
fn navigable(url: &Url) -> bool {
    matches!(url.scheme(), "about" | "blob" | "data") || allowed(url)
}

/// 側欄裡可以打開的網址：抖音、登入頁面與它們用到的位元組跳動網域
fn allowed(url: &Url) -> bool {
    let host = url.host_str().unwrap_or("");
    url.scheme() == "https"
        && [
            "douyin.com",
            "douyinpic.com",
            "bytedance.com",
            "snssdk.com",
            "amemv.com",
            "toutiao.com",
            "zijieapi.com",
            "bytegecko.com",
        ]
        .iter()
        .any(|d| host == *d || host.ends_with(&format!(".{d}")))
}

#[tauri::command]
pub fn douyin_close(app: AppHandle) -> Result<(), String> {
    // 使用者關掉側欄時真的關閉，直播的聲音和資源才會停。登入狀態存在系統 WebView 的資料裡，不受影響
    if let Some(wv) = app.get_webview(LABEL) {
        wv.close().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 對話框打開時暫時藏起來：原生的子 WebView 永遠在最上層，會蓋住對話框。頁面照常跑，關掉對話框再用 douyin_open 顯示。
#[tauri::command]
pub fn douyin_hide(app: AppHandle) -> Result<(), String> {
    if let Some(wv) = app.get_webview(LABEL) {
        wv.hide().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn douyin_bounds(app: AppHandle, bounds: Bounds) -> Result<(), String> {
    if let Some(wv) = app.get_webview(LABEL) {
        wv.set_position(LogicalPosition::new(bounds.x, bounds.y)).map_err(|e| e.to_string())?;
        wv.set_size(LogicalSize::new(bounds.w.max(1.0), bounds.h.max(1.0))).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn douyin_back(app: AppHandle) -> Result<(), String> {
    if let Some(wv) = app.get_webview(LABEL) {
        wv.eval("history.back()").map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn douyin_reload(app: AppHandle) -> Result<(), String> {
    if let Some(wv) = app.get_webview(LABEL) {
        wv.reload().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamInfo {
    url: String,
    #[serde(default)]
    source: String,
    #[serde(default)]
    quality: Option<String>,
    #[serde(default)]
    resolution: Option<String>,
    #[serde(default)]
    web_rid: Option<String>,
    #[serde(default)]
    anchor: String,
    #[serde(default)]
    page_url: String,
    #[serde(default)]
    user_agent: String,
}

/// 一個直播間目前抓到的各種畫質，給介面列出來讓使用者選。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomStreams {
    web_rid: Option<String>,
    anchor: String,
    page_url: String,
    user_agent: String,
    /// 由高到低排好
    streams: Vec<Candidate>,
    /// 毫秒時間戳，最後一次有新回報的時間
    updated: i64,
    /// 這個直播間的串流名稱（見 `stream_base`），不同名稱的網址是別的直播間的
    #[serde(skip)]
    base: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    url: String,
    /// 抖音的畫質代號（origin、FULL_HD1、HD1、SD1、SD2…），頁面播放器實際請求的網址可能沒有
    quality: Option<String>,
    /// 給人看的畫質名稱：原畫、藍光、超清、高清、標清
    label: String,
    resolution: Option<String>,
    /// api：直播間資料裡列出的；fetch / xhr / html：頁面播放器實際在用的
    source: String,
    /// 頁面現在播放的是不是這一路（使用者在頁面上切畫質時會跟著換）
    playing: bool,
    /// 秒級時間戳，網址的到期時間
    expire: Option<i64>,
}

/// 暫存的候選網址，只放在記憶體裡。
#[derive(Default)]
pub struct Streams {
    rooms: Mutex<HashMap<String, RoomStreams>>,
}

/// 暫存多久沒有新回報就丟掉。拉流網址本身也會過期，留太久沒有用。
const KEEP_MS: i64 = 30 * 60 * 1000;

fn quality_rank(c: &Candidate) -> usize {
    ["原畫", "藍光", "超清", "高清", "標清"].iter().position(|l| *l == c.label).unwrap_or(5)
}

fn label_of(quality: Option<&str>, url: &str) -> String {
    match quality {
        Some("origin") => "原畫".into(),
        Some("FULL_HD1" | "uhd") => "藍光".into(),
        Some("HD1" | "hd") => "超清".into(),
        Some("SD1" | "sd") => "高清".into(),
        Some("SD2" | "ld") => "標清".into(),
        _ => crate::describe_quality(url),
    }
}

/// 同一路串流的網址每次簽名不同，用去掉參數的路徑辨識。
fn stream_key(url: &str) -> &str {
    url.split('?').next().unwrap_or(url)
}

/// 串流名稱：網址最後一段去掉副檔名和畫質後綴，例如 `stream-123_or4.flv` 是 `stream-123`。
/// 同一個直播間的各種畫質名稱相同，別的直播間不同。
fn stream_base(url: &str) -> &str {
    let name = stream_key(url).rsplit('/').next().unwrap_or("");
    let name = name.rsplit_once('.').map_or(name, |(n, _)| n);
    name.rsplit_once('_').map_or(name, |(n, _)| n)
}

fn room_id(info: &StreamInfo) -> String {
    info.web_rid.clone().unwrap_or_else(|| info.page_url.clone())
}

impl Streams {
    fn add(&self, info: &StreamInfo) -> RoomStreams {
        let now = chrono::Utc::now().timestamp_millis();
        let mut rooms = self.rooms.lock().unwrap();
        rooms.retain(|_, r| now - r.updated < KEEP_MS);
        let room = rooms.entry(room_id(info)).or_insert_with(|| RoomStreams {
            web_rid: info.web_rid.clone(),
            anchor: String::new(),
            page_url: String::new(),
            user_agent: String::new(),
            streams: vec![],
            updated: now,
            base: None,
        });
        room.updated = now;
        if !info.anchor.is_empty() {
            room.anchor = info.anchor.clone();
        }
        room.page_url = info.page_url.clone();
        room.user_agent = info.user_agent.clone();
        let expire = crate::query_param(&info.url, "expire").and_then(|v| v.parse().ok());
        room.streams.retain(|c| c.expire.is_none_or(|e| e * 1000 > now));
        // 頁面播放器實際在拉的（fetch、xhr）才算正在播；從頁面文字裡掃到的不算
        let playing = info.source == "fetch" || info.source == "xhr";
        // 抖音是單頁應用，從首頁點進直播間後，頁面資料裡還有一堆推薦直播間的網址。
        // 以頁面正在播的串流為準，還沒開始播時先用直播間資料 API 給的第一個；頁面文字掃到的只用來補畫質
        let base = stream_base(&info.url);
        if playing && room.base.as_deref() != Some(base) {
            room.base = Some(base.to_string());
            room.streams.retain(|c| stream_base(&c.url) == base);
        } else if room.base.is_none() && info.source == "api" {
            room.base = Some(base.to_string());
        }
        if room.base.as_deref() != Some(base) {
            return room.clone();
        }
        if playing {
            for c in &mut room.streams {
                c.playing = false;
            }
        }
        let key = stream_key(&info.url);
        match room.streams.iter_mut().find(|c| stream_key(&c.url) == key) {
            Some(c) => {
                c.url = info.url.clone();
                c.expire = expire;
                c.playing |= playing;
                // 介面 API 給的畫質代號和解析度比較完整，有就保留
                if c.quality.is_none() {
                    c.quality = info.quality.clone();
                    c.label = label_of(c.quality.as_deref(), &c.url);
                }
                if c.resolution.is_none() {
                    c.resolution = info.resolution.clone();
                }
            }
            None => room.streams.push(Candidate {
                url: info.url.clone(),
                quality: info.quality.clone(),
                label: label_of(info.quality.as_deref(), &info.url),
                resolution: info.resolution.clone(),
                source: info.source.clone(),
                playing,
                expire,
            }),
        }
        room.streams.sort_by_key(quality_rank);
        room.clone()
    }
}

/// 由抖音側欄裡的注入腳本呼叫。內容來自網頁，只做格式檢查後暫存，並轉給主介面，由使用者決定要不要錄。
#[tauri::command]
pub fn douyin_stream(app: AppHandle, webview: tauri::Webview, info: StreamInfo) -> Result<(), String> {
    if webview.label() != LABEL {
        return Err("只接受抖音側欄的回報".into());
    }
    let ok = Url::parse(&info.url).is_ok_and(|u| u.scheme() == "https" || u.scheme() == "http");
    let short = |s: &Option<String>| s.as_ref().is_none_or(|s| s.len() <= 64);
    if !ok || info.url.len() > 4096 || info.anchor.len() > 200 || !short(&info.quality) || !short(&info.resolution) {
        return Err(tr!("無效的網址", "Invalid URL"));
    }
    let room = app.state::<Streams>().add(&info);
    app.emit_to("main", "douyin-streams", room).map_err(|e| e.to_string())
}

/// 讀取暫存的候選網址。`web_rid` 沒給時回傳最近有回報的直播間。
#[tauri::command]
pub fn douyin_streams(app: AppHandle, web_rid: Option<String>) -> Option<RoomStreams> {
    let streams = app.state::<Streams>();
    let rooms = streams.rooms.lock().unwrap();
    match web_rid {
        Some(id) => rooms.get(&id).cloned(),
        None => rooms.values().max_by_key(|r| r.updated).cloned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_url_in_share_text() {
        let t = "8- 長按複製此條消息，打開抖音搜索，查看TA的更多作品。 https://v.douyin.com/iAbCdEf/ 9@0.com";
        assert_eq!(find_douyin_url(t).unwrap().as_str(), "https://v.douyin.com/iAbCdEf/");
        assert_eq!(find_douyin_url("live.douyin.com/123456").map(|u| u.to_string()), None);
        assert_eq!(find_douyin_url(" https://live.douyin.com/123456 ").unwrap().path(), "/123456");
        assert!(find_douyin_url("https://example.com/live.douyin.com").is_none());
    }

    fn info(url: &str, source: &str, quality: Option<&str>) -> StreamInfo {
        StreamInfo {
            url: url.into(),
            source: source.into(),
            quality: quality.map(Into::into),
            resolution: None,
            web_rid: Some("123456".into()),
            anchor: "主播".into(),
            page_url: "https://live.douyin.com/123456".into(),
            user_agent: String::new(),
        }
    }

    #[test]
    fn keeps_every_quality_and_tracks_playing() {
        let s = Streams::default();
        s.add(&info("https://pull-flv.douyincdn.com/stage/stream-1_sd.flv?a=1", "api", Some("SD1")));
        s.add(&info("https://pull-flv.douyincdn.com/stage/stream-1_or4.flv?a=1", "api", Some("origin")));
        // 頁面播放器拉的網址參數不同，但是同一路
        let r = s.add(&info("https://pull-flv.douyincdn.com/stage/stream-1_sd.flv?a=2", "fetch", None));
        assert_eq!(r.streams.len(), 2);
        assert_eq!(r.streams[0].label, "原畫");
        assert_eq!(r.streams[1].label, "高清");
        assert!(r.streams[1].playing && !r.streams[0].playing);
        assert!(r.streams[1].url.ends_with("a=2"));
        assert_eq!(r.streams[1].quality.as_deref(), Some("SD1"));
        // 在頁面上切到原畫
        let r = s.add(&info("https://pull-flv.douyincdn.com/stage/stream-1_or4.flv?a=3", "fetch", None));
        assert!(r.streams[0].playing && !r.streams[1].playing);
    }

    #[test]
    fn ignores_other_rooms_on_the_page() {
        let s = Streams::default();
        // 頁面文字裡掃到的推薦直播間，還不知道這個直播間是哪一路時不收
        let r = s.add(&info("https://pull-flv.douyincdn.com/stage/stream-9_or4.flv", "html", None));
        assert!(r.streams.is_empty());
        // API 先回報了別的直播間，之後頁面播放器拉的才是這個直播間
        s.add(&info("https://pull-flv.douyincdn.com/stage/stream-8_or4.flv", "api", Some("origin")));
        s.add(&info("https://pull-flv.douyincdn.com/stage/stream-1_hd.flv", "fetch", None));
        s.add(&info("https://pull-flv.douyincdn.com/stage/stream-7_hd.flv", "api", Some("HD1")));
        let r = s.add(&info("https://pull-flv.douyincdn.com/stage/stream-1_or4.flv", "html", None));
        let names: Vec<_> = r.streams.iter().map(|c| stream_base(&c.url)).collect();
        assert_eq!(names, ["stream-1", "stream-1"]);
        assert!(!r.streams[0].playing && r.streams[1].playing);
        assert_eq!(stream_base("https://x/stage/stream-1_or4.flv?a=1"), "stream-1");
        assert_eq!(stream_base("https://x/stage/stream-1.flv"), "stream-1");
    }
}
