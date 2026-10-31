//! SupaLive 桌面版：介面在 ../ui，錄製與封裝全部交給 supalive-core。

mod cover;
mod douyin;
mod files;
mod material;
mod player;
mod store;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::json;
use supalive_core::manager::{room_key_from_url, Manager, RecordingId, StartRequest};
use supalive_core::session::{self, SessionDirs};
use supalive_core::{Event, EventFn, Format};
use tauri::{AppHandle, Emitter, Manager as _, State};

use store::{Data, RecStatus, Recording, Room, Settings};
use supalive_core::tr;

pub struct AppState {
    data: Mutex<Data>,
    path: PathBuf,
    manager: Manager,
    /// 錄製編號 -> 直播間編號
    active: Mutex<HashMap<RecordingId, u64>>,
    material: Mutex<&'static str>,
}

impl AppState {
    fn save(&self) {
        self.data.lock().unwrap().save(&self.path);
    }

    fn room_of(&self, rec: RecordingId) -> Option<u64> {
        self.active.lock().unwrap().get(&rec).copied()
    }

    fn recording_of(&self, room: u64) -> Option<RecordingId> {
        self.active.lock().unwrap().iter().find(|(_, r)| **r == room).map(|(id, _)| *id)
    }
}

type Res<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn changed(app: &AppHandle) {
    let _ = app.emit("rooms-changed", ());
}

// ---------------------------------------------------------------- 狀態

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    settings: Settings,
    rooms: Vec<Room>,
    platform: &'static str,
    material: &'static str,
    private_api: bool,
    /// 直播間編號 -> 每次錄製的封面，和 `recordings` 一一對應。不存進 supalive.json
    covers: HashMap<u64, Vec<Option<cover::Cover>>>,
    /// 直播間編號 -> 每次錄製的檔案還在不在，和 `recordings` 一一對應
    present: HashMap<u64, Vec<Present>>,
}

/// 一次錄製的檔案還在不在。使用者可能在 App 外面搬走或刪掉，每次讀狀態時重新檢查。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Present {
    /// 錄製資料夾還在
    dir: bool,
    /// 紀錄裡有、但找不到的分段和合併檔
    missing: Vec<PathBuf>,
}

fn present(x: &Recording) -> Present {
    let dir = x.dir.is_dir();
    let missing = x.clips.iter().chain(&x.merged).filter(|p| !dir || !p.is_file()).cloned().collect();
    Present { dir, missing }
}

#[tauri::command]
fn get_state(app: AppHandle, state: State<'_, AppState>) -> Snapshot {
    let d = state.data.lock().unwrap();
    Snapshot {
        settings: d.settings.clone(),
        rooms: d.rooms.clone(),
        platform: std::env::consts::OS,
        material: *state.material.lock().unwrap(),
        private_api: app.config().app.macos_private_api,
        covers: d.rooms.iter().map(|r| (r.id, r.recordings.iter().map(|x| cover::find(&x.dir)).collect())).collect(),
        present: d.rooms.iter().map(|r| (r.id, r.recordings.iter().map(present).collect())).collect(),
    }
}

/// 存直播間最近一次錄製的封面。什麼時候存由介面決定（停止播放時，並限制頻率）。
#[tauri::command]
fn save_cover(state: State<'_, AppState>, id: u64, data: Vec<u8>) -> Res<()> {
    let dir = {
        let d = state.data.lock().unwrap();
        let room = d.rooms.iter().find(|r| r.id == id).ok_or_else(|| tr!("找不到直播間", "Room not found"))?;
        room.recordings.last().ok_or_else(|| tr!("還沒有錄製", "Nothing recorded yet"))?.dir.clone()
    };
    cover::write(&dir, &data).map_err(err)
}

/// 介面在啟動和切換語言時告訴後端，之後產生的錯誤和狀態訊息用這個語言。
#[tauri::command]
fn set_locale(lang: String) {
    supalive_core::i18n::set_english(lang == "en");
}

#[tauri::command]
fn save_settings(app: AppHandle, state: State<'_, AppState>, settings: Settings) -> Res<()> {
    let _ = app.asset_protocol_scope().allow_directory(&settings.root, true);
    state.manager.set_buffer_total(settings.buffer_total_bytes());
    state.data.lock().unwrap().settings = settings;
    state.save();
    Ok(())
}

// ---------------------------------------------------------------- 新增直播間

pub(crate) fn query_param<'a>(url: &'a str, key: &str) -> Option<&'a str> {
    let q = url.split_once('?')?.1;
    q.split('&').find_map(|kv| kv.split_once('=').filter(|(k, _)| *k == key).map(|(_, v)| v))
}

pub(crate) fn describe_quality(url: &str) -> String {
    match query_param(url, "biz_quality") {
        Some("origin") => return "原畫".into(),
        Some("uhd") => return "藍光".into(),
        Some("hd") => return "超清".into(),
        Some("sd") => return "高清".into(),
        Some("ld") => return "標清".into(),
        _ => {}
    }
    let stem = url.split('?').next().unwrap_or(url);
    if stem.contains("_or4") || stem.contains("origin") {
        "原畫".into()
    } else if stem.contains("_uhd") {
        "藍光".into()
    } else if stem.contains("_hd") {
        "超清".into()
    } else if stem.contains("_sd") {
        "高清".into()
    } else if stem.contains("_ld") {
        "標清".into()
    } else {
        "預設".into()
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NewRoom {
    url: String,
    #[serde(default)]
    headers: Vec<(String, String)>,
    #[serde(default)]
    name: String,
    #[serde(default)]
    anchor: String,
    #[serde(default)]
    web_rid: Option<String>,
}

fn room_key(r: &Room) -> String {
    r.web_rid.clone().unwrap_or_else(|| room_key_from_url(&r.url))
}

/// 同一個直播間（web_rid 或拉流網址相同）只保留一筆，再加入時更新它的網址。
fn upsert_room(state: &AppState, new: NewRoom) -> Room {
    let mut d = state.data.lock().unwrap();
    let key = new.web_rid.clone().unwrap_or_else(|| room_key_from_url(&new.url));
    if let Some(r) = d.rooms.iter_mut().find(|r| room_key(r) == key) {
        r.url = new.url.clone();
        r.quality = describe_quality(&new.url);
        if !new.headers.is_empty() {
            r.headers = new.headers;
        }
        if !new.name.is_empty() {
            r.name = new.name;
        }
        if !new.anchor.is_empty() {
            r.anchor = new.anchor;
        }
        let r = r.clone();
        drop(d);
        state.save();
        return r;
    }
    let id = d.next_id;
    d.next_id += 1;
    let room = Room {
        id,
        quality: describe_quality(&new.url),
        name: new.name,
        anchor: new.anchor,
        url: new.url,
        headers: new.headers,
        web_rid: new.web_rid,
        added_at: now_ms(),
        recordings: vec![],
    };
    d.rooms.push(room.clone());
    drop(d);
    state.save();
    room
}

#[tauri::command]
fn add_room(app: AppHandle, state: State<'_, AppState>, room: NewRoom) -> Room {
    let r = upsert_room(&state, room);
    changed(&app);
    r
}

#[tauri::command]
fn update_room(app: AppHandle, state: State<'_, AppState>, id: u64, name: String) -> Res<()> {
    state.data.lock().unwrap().room_mut(id).ok_or_else(|| tr!("找不到直播間", "Room not found"))?.name = name;
    state.save();
    changed(&app);
    Ok(())
}

/// 刪除直播間和它所有錄製的檔案。`permanent` 為假時移到廢紙簍。
/// 中途刪失敗時，已經刪掉的錄製從清單拿掉、其餘留著，清單和磁碟保持一致。
#[tauri::command]
async fn remove_room(app: AppHandle, id: u64, permanent: bool) -> Res<()> {
    let state = app.state::<AppState>();
    if state.recording_of(id).is_some() {
        return Err(tr!("請先停止錄製", "Stop the recording first"));
    }
    let (dirs, root) = {
        let d = state.data.lock().unwrap();
        let room = d.rooms.iter().find(|r| r.id == id).ok_or_else(|| tr!("找不到直播間", "Room not found"))?;
        if room.recordings.iter().any(|r| r.status != RecStatus::Done) {
            return Err(tr!("這個直播間正在合併，結束後再刪除", "This room is merging; delete it after that finishes"));
        }
        (room.recordings.iter().map(|r| r.dir.clone()).collect::<Vec<_>>(), d.settings.root.clone())
    };
    let mut failed = None;
    let mut deleted = vec![];
    for dir in dirs {
        let (d2, r2) = (dir.clone(), root.clone());
        match tokio::task::spawn_blocking(move || files::remove_recording_dir(&d2, &r2, permanent))
            .await
            .map_err(err)?
        {
            Ok(()) => deleted.push(dir),
            Err(e) => {
                failed = Some(e);
                break;
            }
        }
    }
    {
        let mut d = state.data.lock().unwrap();
        if failed.is_none() {
            d.rooms.retain(|r| r.id != id);
        } else if let Some(r) = d.room_mut(id) {
            r.recordings.retain(|x| !deleted.contains(&x.dir));
        }
    }
    state.save();
    changed(&app);
    failed.map_or(Ok(()), Err)
}

// ---------------------------------------------------------------- 錄製

// async：錄製任務要在 Tokio 執行緒上啟動
#[tauri::command]
async fn start_recording(app: AppHandle, state: State<'_, AppState>, id: u64) -> Res<()> {
    if state.recording_of(id).is_some() {
        return Err(tr!("這個直播間已經在錄製中", "This room is already being recorded"));
    }
    let (room, settings) = {
        let d = state.data.lock().unwrap();
        (
            d.rooms.iter().find(|r| r.id == id).cloned().ok_or_else(|| tr!("找不到直播間", "Room not found"))?,
            d.settings.clone(),
        )
    };
    let mut headers = room.headers.clone();
    if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("referer")) {
        headers.push(("Referer".into(), "https://live.douyin.com/".into()));
    }
    let name = if room.name.is_empty() { room.anchor.trim_start_matches('@').to_string() } else { room.name.clone() };
    let req = StartRequest {
        url: room.url.clone(),
        headers,
        room_name: Some(name).filter(|n| !n.is_empty()),
        room_key: Some(room_key(&room)),
        base_dir: settings.root.clone(),
        segment_secs: settings.seg_min * 60,
        format: Format::Mp4,
        merge: settings.auto_merge,
        keep_clips: settings.keep_clips,
        retries: 10,
        buffer: settings.buffer_bytes(),
    };
    // 先登記再開始，避免第一個事件到達時還查不到是哪個直播間
    let mut active = state.active.lock().unwrap();
    let rec_id = state.manager.start(req).map_err(err)?;
    active.insert(rec_id, id);
    drop(active);
    let dir = state.manager.active().into_iter().find(|r| r.id == rec_id).map(|r| r.dir).unwrap_or_default();
    let _ = app.asset_protocol_scope().allow_directory(&dir, true);
    {
        let mut d = state.data.lock().unwrap();
        if let Some(r) = d.room_mut(id) {
            r.recordings.push(Recording { dir, started: now_ms(), status: RecStatus::Rec, ..Default::default() });
        }
    }
    state.save();
    changed(&app);
    Ok(())
}

#[tauri::command]
fn stop_recording(state: State<'_, AppState>, id: u64) -> Res<()> {
    let rec = state.recording_of(id).ok_or_else(|| tr!("這個直播間沒有在錄製", "This room is not recording"))?;
    state.manager.stop(rec);
    Ok(())
}

/// `index` 是直播間底下第幾次錄製（從 0 起算，舊的在前），沒給就合併最近一次。
#[tauri::command]
async fn merge_recording(app: AppHandle, id: u64, index: Option<usize>) -> Res<()> {
    let state = app.state::<AppState>();
    let (rec_index, rec, keep) = {
        let d = state.data.lock().unwrap();
        let room = d.rooms.iter().find(|r| r.id == id).ok_or_else(|| tr!("找不到直播間", "Room not found"))?;
        let i = match index {
            Some(i) => i,
            None => room.recordings.len().checked_sub(1).ok_or_else(|| tr!("還沒有錄製", "Nothing recorded yet"))?,
        };
        let rec = room.recordings.get(i).ok_or_else(|| tr!("找不到這次錄製", "Recording not found"))?.clone();
        (i, rec, d.settings.keep_clips)
    };
    if rec.status != RecStatus::Done {
        return Err(tr!("錄製還沒結束", "The recording has not finished"));
    }
    set_status(&app, id, rec_index, RecStatus::Merging);
    let events = room_events(app.clone(), id, rec_index);
    // 使用者可能在 App 外面刪掉了部分分段，只合併還在的
    let clips: Vec<PathBuf> = rec.clips.iter().filter(|p| p.is_file()).cloned().collect();
    if clips.is_empty() {
        return Err(tr!("找不到可以合併的分段", "No segments left to merge"));
    }
    let dirs = SessionDirs::from_root(rec.dir.clone());
    let result = tokio::task::spawn_blocking(move || session::finish(&dirs, &clips, Format::Mp4, keep, &events))
        .await
        .map_err(err)?;
    set_status(&app, id, rec_index, RecStatus::Done);
    result.map(|_| ()).map_err(err)
}

/// 刪除一次錄製的檔案並從清單拿掉。`permanent` 為假時移到廢紙簍。
#[tauri::command]
async fn remove_recording(app: AppHandle, id: u64, index: usize, permanent: bool) -> Res<()> {
    let state = app.state::<AppState>();
    let (dir, root) = {
        let d = state.data.lock().unwrap();
        let room = d.rooms.iter().find(|r| r.id == id).ok_or_else(|| tr!("找不到直播間", "Room not found"))?;
        // 錄製和合併的事件用位置找紀錄，進行中時刪除會讓後面的位置錯開
        if room.recordings.iter().any(|r| r.status != RecStatus::Done) {
            return Err(tr!(
                "這個直播間正在錄製或合併，結束後再刪除",
                "This room is recording or merging; delete it after that finishes"
            ));
        }
        let rec = room.recordings.get(index).ok_or_else(|| tr!("找不到這次錄製", "Recording not found"))?;
        (rec.dir.clone(), d.settings.root.clone())
    };
    let d2 = dir.clone();
    tokio::task::spawn_blocking(move || files::remove_recording_dir(&d2, &root, permanent)).await.map_err(err)??;
    // 刪檔期間清單可能變過，用資料夾找回這一筆
    if let Some(r) = state.data.lock().unwrap().room_mut(id) {
        r.recordings.retain(|x| x.dir != dir);
    }
    state.save();
    changed(&app);
    Ok(())
}

fn set_status(app: &AppHandle, room: u64, index: usize, status: RecStatus) {
    let state = app.state::<AppState>();
    if let Some(rec) = state.data.lock().unwrap().room_mut(room).and_then(|r| r.recordings.get_mut(index)) {
        rec.status = status;
    }
    state.save();
    changed(app);
}

/// 手動合併時沿用同一套事件處理。
fn room_events(app: AppHandle, room: u64, index: usize) -> EventFn {
    Arc::new(move |e| apply_event(&app, room, Some(index), e))
}

/// 把核心的事件寫進直播間的錄製紀錄，並轉發給介面。
fn apply_event(app: &AppHandle, room: u64, index: Option<usize>, e: Event) {
    let state = app.state::<AppState>();
    let mut persist = true;
    let payload = {
        let mut d = state.data.lock().unwrap();
        let auto_merge = d.settings.auto_merge;
        let Some(r) = d.room_mut(room) else { return };
        let i = index.unwrap_or(r.recordings.len().saturating_sub(1));
        let Some(rec) = r.recordings.get_mut(i) else { return };
        match &e {
            Event::Stats { bytes, fps } => {
                rec.bytes = *bytes;
                persist = false;
                json!({ "kind": "stats", "bytes": bytes, "fps": fps })
            }
            Event::Stream { codec, width, height } => {
                rec.codec = format!("{codec:?}").replace("Avc", "H.264").replace("Hevc", "HEVC");
                rec.width = *width;
                rec.height = *height;
                json!({ "kind": "stream" })
            }
            Event::FileClosed(p) => {
                rec.clips.push(p.clone());
                json!({ "kind": "clip", "path": p })
            }
            Event::FileRemoved(p) => {
                rec.clips.retain(|c| c != p);
                json!({ "kind": "removed" })
            }
            Event::Merged(p) => {
                rec.merged.push(p.clone());
                json!({ "kind": "merged", "path": p })
            }
            Event::Disconnected { reason, failures, max } => {
                rec.note = reason.clone();
                json!({ "kind": "disconnected", "reason": reason, "failures": failures, "max": max })
            }
            Event::Warning(w) => {
                rec.note = w.clone();
                json!({ "kind": "warning", "message": w })
            }
            Event::Connected => {
                rec.note.clear();
                json!({ "kind": "connected" })
            }
            Event::Finished => {
                rec.ended = Some(now_ms());
                rec.status = if auto_merge && !rec.clips.is_empty() { RecStatus::Merging } else { RecStatus::Done };
                json!({ "kind": "finished" })
            }
            Event::Done => {
                rec.status = RecStatus::Done;
                json!({ "kind": "done" })
            }
            _ => {
                persist = false;
                serde_json::Value::Null
            }
        }
    };
    if persist {
        state.save();
        changed(app);
    }
    if !payload.is_null() {
        let mut p = payload;
        p["roomId"] = json!(room);
        let _ = app.emit("rec-event", p);
    }
}

#[tauri::command]
fn reveal(app: AppHandle, path: PathBuf) -> Res<()> {
    use tauri_plugin_opener::OpenerExt;
    if path.is_dir() {
        app.opener().open_path(path.to_string_lossy(), None::<&str>).map_err(err)
    } else {
        app.opener().reveal_item_in_dir(path).map_err(err)
    }
}

// ---------------------------------------------------------------- 啟動

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let path = app.path().app_config_dir()?.join("supalive.json");
            let data = Data::load(&path);
            let _ = app.asset_protocol_scope().allow_directory(&data.settings.root, true);
            for r in &data.rooms {
                for rec in &r.recordings {
                    let _ = app.asset_protocol_scope().allow_directory(&rec.dir, true);
                }
            }
            let handle = app.handle().clone();
            let manager = Manager::new(Arc::new(move |rec_id, e| {
                let state = handle.state::<AppState>();
                let Some(room) = state.room_of(rec_id) else { return };
                let done = matches!(e, Event::Done);
                apply_event(&handle, room, None, e);
                if done {
                    state.active.lock().unwrap().remove(&rec_id);
                    changed(&handle);
                }
            }));
            manager.set_buffer_total(data.settings.buffer_total_bytes());
            app.manage(player::Watches::default());
            app.manage(douyin::Streams::default());
            // 收到 SIGTERM／SIGINT（系統關機、`kill`、終端機 Ctrl+C）時，也先收好錄製再結束
            let term_app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                supalive_core::manager::termination().await;
                if !finish_then_exit(&term_app) {
                    term_app.exit(0);
                }
            });
            app.manage(AppState {
                data: Mutex::new(data),
                path,
                manager,
                active: Mutex::default(),
                material: Mutex::new("none"),
            });
            if let Some(win) = app.get_webview_window("main") {
                let applied = material::apply(&win, app.config().app.macos_private_api);
                *app.state::<AppState>().material.lock().unwrap() = applied;
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" && finish_then_exit(window.app_handle()) {
                    api.prevent_close();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_state,
            save_settings,
            set_locale,
            add_room,
            update_room,
            remove_room,
            start_recording,
            stop_recording,
            merge_recording,
            remove_recording,
            save_cover,
            reveal,
            player::watch,
            player::unwatch,
            douyin::douyin_open,
            douyin::douyin_close,
            douyin::douyin_hide,
            douyin::douyin_bounds,
            douyin::douyin_back,
            douyin::douyin_reload,
            douyin::douyin_stream,
            douyin::douyin_streams,
        ])
        .build(tauri::generate_context!())
        .expect("SupaLive 啟動失敗")
        .run(|app, event| {
            // Cmd+Q、從 Dock 結束、系統登出或關機都會走到這裡。
            // 收尾完成後呼叫 app.exit(0) 也會再來一次，那時 code 有值，直接放行
            if let tauri::RunEvent::ExitRequested { code: None, api, .. } = event {
                if finish_then_exit(app) {
                    api.prevent_exit();
                }
            }
        });
}

/// 結束程式前先停止所有錄製，寫出緩衝並封裝（含自動合併），完成後才真的結束。
/// 回傳 true 表示已經接手，呼叫端要擋下這次結束。
fn finish_then_exit(app: &AppHandle) -> bool {
    static STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if STARTED.load(std::sync::atomic::Ordering::SeqCst) {
        return true;
    }
    let state = app.state::<AppState>();
    if state.manager.active().is_empty() {
        return false;
    }
    STARTED.store(true, std::sync::atomic::Ordering::SeqCst);
    let _ = app.emit("closing", ());
    state.manager.stop_all();
    let manager = state.manager.clone();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        manager.wait_all().await;
        app.exit(0);
    });
    true
}
