//! 視窗層的原生材質：這是兩個平台唯一刻意不同的地方，視窗內的介面完全共用。

use tauri::WebviewWindow;

/// 回傳實際套用的材質，介面據此決定要不要自己鋪底色。
/// `transparent_ok` 為 false（例如上架版關掉 macOSPrivateApi）時一律不套用。
pub fn apply(win: &WebviewWindow, transparent_ok: bool) -> &'static str {
    #[cfg(target_os = "macos")]
    if transparent_ok {
        use window_vibrancy::{apply_liquid_glass, apply_vibrancy, LiquidGlassOptions, NSVisualEffectMaterial};
        // macOS 26 以上用液態玻璃，舊版退回 vibrancy
        if apply_liquid_glass(win, LiquidGlassOptions::default()).is_ok() {
            return "liquid-glass";
        }
        if apply_vibrancy(win, NSVisualEffectMaterial::Sidebar, None, None).is_ok() {
            return "vibrancy";
        }
    }
    #[cfg(target_os = "windows")]
    {
        let _ = transparent_ok;
        // Windows 11 才有 Mica；Windows 10 會失敗，介面改用純色底
        if window_vibrancy::apply_mica(win, None).is_ok() {
            return "mica";
        }
    }
    let _ = (win, transparent_ok);
    "none"
}
