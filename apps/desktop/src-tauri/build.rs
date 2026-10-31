fn main() {
    // 介面檔在編譯時嵌入，cargo 預設不知道它們是輸入；不加這行，改了介面不會重新編譯
    println!("cargo:rerun-if-changed=../ui");
    // 列出自訂指令，讓 capabilities 可以逐一授權（抖音側欄只拿得到 douyin_stream）
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(&[
        "get_state",
        "save_settings",
        "set_locale",
        "add_room",
        "update_room",
        "remove_room",
        "start_recording",
        "stop_recording",
        "merge_recording",
        "remove_recording",
        "save_cover",
        "reveal",
        "watch",
        "unwatch",
        "douyin_open",
        "douyin_close",
        "douyin_hide",
        "douyin_bounds",
        "douyin_back",
        "douyin_reload",
        "douyin_stream",
        "douyin_streams",
    ])))
    .expect("tauri build");
}
