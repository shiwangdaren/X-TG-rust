//! 为 egui 注册含 CJK 的系统字体；默认内置字体无中文，会显示为方块。

use egui::{FontData, FontDefinitions, FontFamily};
use std::borrow::Cow;

/// 在应用启动时调用：将支持中文的字体插到 Proportional / Monospace 族的最前面。
pub fn setup_cjk_fonts(ctx: &egui::Context) {
    let Some((bytes, index)) = try_load_system_cjk_font() else {
        return;
    };

    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "cjk-ui".to_owned(),
        FontData {
            font: Cow::Owned(bytes),
            index,
            tweak: Default::default(),
        },
    );
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        if let Some(v) = fonts.families.get_mut(&family) {
            v.insert(0, "cjk-ui".to_owned());
        }
    }
    ctx.set_fonts(fonts);
}

fn try_load_system_cjk_font() -> Option<(Vec<u8>, u32)> {
    #[cfg(windows)]
    {
        let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".to_string());
        let paths = [
            format!(r"{windir}\Fonts\msyh.ttc"),
            format!(r"{windir}\Fonts\msyhbd.ttc"),
            format!(r"{windir}\Fonts\simhei.ttf"),
            format!(r"{windir}\Fonts\SIMHEI.TTF"),
        ];
        for p in paths {
            if let Some(pair) = try_font_file(&p) {
                return Some(pair);
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        const CANDIDATES: &[&str] = &[
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
        ];
        for p in CANDIDATES {
            if let Some(pair) = try_font_file(p) {
                return Some(pair);
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        const CANDIDATES: &[&str] = &[
            "/System/Library/Fonts/PingFang.ttc",
            "/System/Library/Fonts/STHeiti Light.ttc",
            "/Library/Fonts/Arial Unicode.ttf",
        ];
        for p in CANDIDATES {
            if let Some(pair) = try_font_file(p) {
                return Some(pair);
            }
        }
    }
    None
}

fn try_font_file(path: &str) -> Option<(Vec<u8>, u32)> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() < 100 {
        return None;
    }
    for idx in 0..8u32 {
        if ab_glyph::FontRef::try_from_slice_and_index(&bytes, idx).is_ok() {
            return Some((bytes, idx));
        }
    }
    None
}
