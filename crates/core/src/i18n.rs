//! 給人看的訊息用哪種語言：正體中文（預設）或英文。
//! 桌面版依介面的語言設定呼叫 `set_english`；命令列不設定，維持中文。

use std::sync::atomic::{AtomicBool, Ordering};

static ENGLISH: AtomicBool = AtomicBool::new(false);

pub fn set_english(on: bool) {
    ENGLISH.store(on, Ordering::Relaxed);
}

pub fn is_english() -> bool {
    ENGLISH.load(Ordering::Relaxed)
}

/// 依目前的語言選一句，回傳 `String`：`tr!("找不到 {p}", "Cannot find {p}")`。
/// 兩句的格式參數要一樣。
#[macro_export]
macro_rules! tr {
    ($zh:literal, $en:literal $(, $($arg:tt)*)?) => {
        if $crate::i18n::is_english() { format!($en $(, $($arg)*)?) } else { format!($zh $(, $($arg)*)?) }
    };
}
