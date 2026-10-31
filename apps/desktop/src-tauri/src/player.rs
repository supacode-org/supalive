//! 邊錄邊播：把核心分發的分段 MP4 透過 IPC channel 送給介面的 MediaSource。
//!
//! 每則訊息是一段二進位：第 1 個位元組是種類（0 = init，1 = 片段，2 = 結束），
//! init 之後接 2 位元組的編碼字串長度與字串，最後是 MP4 資料。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use supalive_core::live::LivePacket;
use tauri::async_runtime::JoinHandle;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, Manager as _};
use tokio::sync::broadcast::error::RecvError;

use crate::AppState;
use supalive_core::tr;

/// 進行中的轉發。介面丟掉 channel 時 Rust 端不會知道，所以要由介面明確呼叫 `unwatch` 停掉，
/// 否則每個片段都還會繼續送進 WebView。
#[derive(Default)]
pub struct Watches {
    next: AtomicU64,
    tasks: Mutex<HashMap<u64, JoinHandle<()>>>,
}

fn frame(p: &LivePacket) -> Vec<u8> {
    match p {
        LivePacket::Init { data, codecs } => {
            let mut v = Vec::with_capacity(3 + codecs.len() + data.len());
            v.push(0);
            v.extend_from_slice(&(codecs.len() as u16).to_be_bytes());
            v.extend_from_slice(codecs.as_bytes());
            v.extend_from_slice(data);
            v
        }
        LivePacket::Fragment { data, .. } => {
            let mut v = Vec::with_capacity(1 + data.len());
            v.push(1);
            v.extend_from_slice(data);
            v
        }
    }
}

/// `tail` 有值時只送最近約這麼多秒的回看片段（例如總覽卡片的預覽），沒有就送完整的回看視窗。
/// 每個片段都從關鍵幀開始，所以從中間開始送也能直接播放。
#[tauri::command]
pub async fn watch(
    app: AppHandle,
    id: u64,
    tail: Option<f64>,
    channel: Channel<InvokeResponseBody>,
) -> Result<u64, String> {
    let state = app.state::<AppState>();
    let rec = state.recording_of(id).ok_or_else(|| tr!("這個直播間沒有在錄製", "This room is not recording"))?;
    let (mut head, mut rx) =
        state.manager.subscribe(rec).ok_or_else(|| tr!("錄製已結束", "The recording has ended"))?;
    if let Some(tail) = tail {
        let mut secs = 0.0;
        let mut keep = head.len();
        while keep > 0 && secs < tail {
            match &head[keep - 1] {
                LivePacket::Fragment { secs: s, .. } => secs += s,
                LivePacket::Init { .. } => break,
            }
            keep -= 1;
        }
        // 保留開頭的 init，只丟掉中間較舊的片段
        let init = head.iter().take_while(|p| matches!(p, LivePacket::Init { .. })).count();
        head.drain(init..keep.max(init));
    }
    for p in &head {
        if channel.send(InvokeResponseBody::Raw(frame(p))).is_err() {
            return Ok(0);
        }
    }
    let watches = app.state::<Watches>();
    let wid = watches.next.fetch_add(1, Ordering::Relaxed) + 1;
    let done_app = app.clone();
    // 先鎖住再開工作，工作很快結束時也會等登記完才移除，不會留下殘項
    let mut tasks = watches.tasks.lock().unwrap();
    let task = tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(p) => {
                    // WebView 關掉後 channel 會失效，就停止轉發
                    if channel.send(InvokeResponseBody::Raw(frame(&p))).is_err() {
                        break;
                    }
                }
                // 介面跟不上（例如視窗被縮小很久），讓它重新訂閱
                Err(RecvError::Lagged(_)) | Err(RecvError::Closed) => {
                    let _ = channel.send(InvokeResponseBody::Raw(vec![2]));
                    break;
                }
            }
        }
        done_app.state::<Watches>().tasks.lock().unwrap().remove(&wid);
    });
    tasks.insert(wid, task);
    Ok(wid)
}

/// 介面不再需要畫面時（關掉播放器、離開總覽）停止轉發。
#[tauri::command]
pub fn unwatch(app: AppHandle, watch: u64) {
    if let Some(task) = app.state::<Watches>().tasks.lock().unwrap().remove(&watch) {
        task.abort();
    }
}
