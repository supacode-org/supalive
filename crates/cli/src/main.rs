use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use supalive_core::manager::{list_files, Manager, StartRequest};
use supalive_core::session::{self, DEFAULT_SEGMENT_MINUTES};
use supalive_core::{curl, Event, EventFn, Format, SinkOptions};

/// SupaLive：抖音直播錄製，直接保存原始串流，不錄屏、不重新編碼、不需要 ffmpeg。
#[derive(Parser)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Clone, Copy, ValueEnum)]
enum OutFormat {
    /// 分段式 MP4，中斷也不會損壞
    Mp4,
    /// 原始 FLV（HEVC 改寫為 Enhanced FLV）
    Flv,
}

#[derive(clap::Args)]
struct OutputArgs {
    /// 根目錄，預設為 下載/SupaLive
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// 每個切片約多少分鐘（在關鍵幀處切開，不精確），0 表示不切
    #[arg(long, default_value_t = DEFAULT_SEGMENT_MINUTES)]
    segment: u64,
    #[arg(long, value_enum, default_value_t = OutFormat::Mp4)]
    format: OutFormat,
    /// 錄製結束後不自動合併切片
    #[arg(long)]
    no_merge: bool,
    /// 合併成功後刪除切片
    #[arg(long)]
    delete_clips: bool,
    /// 每路錄製的寫檔緩衝（MB），攢滿才寫進磁碟，減少頻繁寫入
    #[arg(long, default_value_t = 16)]
    buffer: usize,
    /// 所有錄製加起來的寫檔緩衝（MB）
    #[arg(long, default_value_t = 64)]
    buffer_total: usize,
}

#[derive(Subcommand)]
enum Cmd {
    /// 錄製直播：參數可以是拉流網址、完整 cURL 指令，或 - 表示從標準輸入讀取；可同時給多個
    Record {
        /// 可以給多個，同時錄製多個直播間
        #[arg(required = true)]
        inputs: Vec<String>,
        #[command(flatten)]
        out: OutputArgs,
        /// 直播間名稱，用於資料夾命名（<根目錄>/<直播間名>/<時間>/），依序對應每個輸入；不給就放在「未命名直播間」底下
        #[arg(short, long = "name")]
        names: Vec<String>,
        /// 額外請求頭，例如 -H "Cookie: a=b"
        #[arg(short = 'H', long = "header")]
        headers: Vec<String>,
        /// 斷線後連續重試次數，用完視為直播結束
        #[arg(long, default_value_t = 10)]
        retries: u32,
    },
    /// 把本機 FLV 檔轉成 MP4 切片並合併（測試或修復舊錄影用）
    Remux {
        input: PathBuf,
        #[command(flatten)]
        out: OutputArgs,
        /// 用於資料夾命名
        #[arg(short, long)]
        name: Option<String>,
    },
    /// 合併資料夾裡的 MP4 切片（依檔名排序）
    Merge {
        clips_dir: PathBuf,
        /// 輸出資料夾，預設為切片資料夾旁的 merged
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
}

fn log(msg: impl AsRef<str>) {
    eprintln!("[{}] {}", chrono::Local::now().format("%H:%M:%S"), msg.as_ref());
}

fn describe(e: Event) -> Option<String> {
    Some(match e {
        Event::Connecting { attempt } => format!("連線中…（第 {attempt} 次）"),
        Event::Connected => "已連線".into(),
        Event::Stream { codec, width, height } => format!("串流 {codec:?} {width}x{height}"),
        Event::FileOpened(p) => format!("開始寫入 {}", p.display()),
        Event::FileClosed(p) => format!("已完成 {}", p.display()),
        Event::FileRemoved(p) => format!("已刪除 {}", p.display()),
        Event::Merged(p) => format!("已合併 {}", p.display()),
        Event::Progress { bytes } => format!("錄製中，本次連線已接收 {:.1} MB", bytes as f64 / 1e6),
        Event::Disconnected { reason, failures, max } => format!("{reason}（{failures}/{max}）"),
        Event::Warning(w) => format!("注意：{w}"),
        Event::Finished => "錄製結束".into(),
        Event::Done | Event::Stats { .. } => return None,
    })
}

fn events() -> EventFn {
    Arc::new(|e| {
        if let Some(m) = describe(e) {
            log(m)
        }
    })
}

impl From<OutFormat> for Format {
    fn from(f: OutFormat) -> Self {
        match f {
            OutFormat::Mp4 => Format::Mp4,
            OutFormat::Flv => Format::Flv,
        }
    }
}

fn prepare(out: &OutputArgs, name: Option<&str>) -> Result<(session::SessionDirs, SinkOptions)> {
    let base = out.output.clone().unwrap_or_else(session::default_root);
    let dirs = session::session_dirs(&base, name)?;
    let sink = SinkOptions {
        dir: dirs.clips.clone(),
        segment_secs: out.segment * 60,
        keep: 0,
        format: out.format.into(),
        prefix: String::new(),
        tap: None,
        buffer: out.buffer << 20,
        budget: None,
    };
    log(format!("輸出目錄：{}", dirs.root.display()));
    Ok((dirs, sink))
}

fn finish(
    out: &OutputArgs,
    dirs: &session::SessionDirs,
    sink: &SinkOptions,
    clips: &[PathBuf],
    ev: &EventFn,
) -> Result<()> {
    if !out.no_merge {
        session::finish(dirs, clips, sink.format, !out.delete_clips, ev)?;
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let ev = events();
    match cli.cmd {
        Cmd::Record { inputs, out, names, headers, retries } => {
            let base = out.output.clone().unwrap_or_else(session::default_root);
            let multi = inputs.len() > 1;
            let manager = Manager::new(Arc::new(move |id, e| {
                if let Some(m) = describe(e) {
                    log(if multi { format!("#{id} {m}") } else { m })
                }
            }));
            manager.set_buffer_total(out.buffer_total << 20);
            for (i, input) in inputs.iter().enumerate() {
                let input = if input == "-" {
                    let mut s = String::new();
                    std::io::stdin().read_to_string(&mut s)?;
                    s
                } else {
                    input.clone()
                };
                let mut req = curl::parse(&input).context("無法解析輸入的網址或 cURL 指令")?;
                for h in &headers {
                    let (k, v) = h.split_once(':').context("請求頭格式應為 \"名稱: 值\"")?;
                    req.headers.push((k.trim().into(), v.trim().into()));
                }
                let start = StartRequest {
                    url: req.url,
                    headers: req.headers,
                    room_name: names.get(i).cloned(),
                    room_key: None,
                    base_dir: base.clone(),
                    segment_secs: out.segment * 60,
                    format: out.format.into(),
                    merge: !out.no_merge,
                    keep_clips: !out.delete_clips,
                    retries,
                    buffer: out.buffer << 20,
                };
                match manager.start(start) {
                    Ok(id) => {
                        let dir = manager.active().into_iter().find(|r| r.id == id).map(|r| r.dir).unwrap_or_default();
                        log(format!("#{id} 輸出目錄：{}", dir.display()));
                    }
                    Err(e) => log(format!("第 {} 個直播略過：{e:#}", i + 1)),
                }
            }
            if inputs.len() > 2 {
                log("注意：同時錄製超過 2 個直播間，可能觸發抖音風控");
            }
            let m = manager.clone();
            tokio::spawn(async move {
                supalive_core::manager::termination().await;
                log("收到停止訊號，正在寫出緩衝、封裝最後一個切片…");
                m.stop_all();
            });
            manager.wait_all().await;
        }
        Cmd::Remux { input, out, name } => {
            let (dirs, sink) = prepare(&out, name.as_deref())?;
            let clips = supalive_core::remux_file(&input, sink.clone(), ev.clone())?;
            finish(&out, &dirs, &sink, &clips, &ev)?;
        }
        Cmd::Merge { clips_dir, output } => {
            let clips = list_files(&clips_dir, Format::Mp4);
            let parent = clips_dir.parent().map(PathBuf::from).unwrap_or_default();
            let out_dir = output.unwrap_or_else(|| parent.join("merged"));
            let name = parent.file_name().and_then(|n| n.to_str()).unwrap_or("merged").to_string();
            for m in supalive_core::merge::merge_clips(&clips, &out_dir, &name)? {
                log(format!("已合併 {}", m.display()));
            }
        }
    }
    Ok(())
}
