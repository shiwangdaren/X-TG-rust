//! X 推文追踪并转发到 Telegram（用户态）— 桌面控制面板。

// Release 下双击运行 exe 时不弹出黑色控制台窗口（调试构建仍保留控制台便于看日志）。
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod fonts;

use app::XtgApp;
use eframe::egui;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

fn app_data_dir() -> PathBuf {
    xtg_service::paths::data_dir()
}

fn startup_log_path() -> PathBuf {
    app_data_dir().join("xtg-startup.log")
}

fn append_startup_log(line: &str) {
    let path = startup_log_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(f, "{}", line);
    }
}

#[cfg(windows)]
fn show_error_dialog(title: &str, body: &str) {
    let _ = msgbox::create(title, body, msgbox::IconType::Error);
}

#[cfg(not(windows))]
fn show_error_dialog(_title: &str, _body: &str) {}

fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        append_startup_log(&format!("[panic] {info}"));
    }));
}

fn main() -> eframe::Result<()> {
    install_panic_hook();
    append_startup_log("main: start");

    let log_result = (|| -> Result<(), String> {
        tracing_subscriber::fmt()
            .with_env_filter(
                EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
            )
            .try_init()
            .map_err(|e| e.to_string())
    })();
    if let Err(e) = log_result {
        append_startup_log(&format!("tracing init failed (non-fatal): {e}"));
    }

    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(r) => Arc::new(r),
        Err(e) => {
            append_startup_log(&format!("tokio runtime build failed: {e}"));
            let log = startup_log_path();
            show_error_dialog(
                "XTG 启动失败",
                &format!("无法创建异步运行时:\n{e}\n\n日志: {}", log.display()),
            );
            std::process::exit(1);
        }
    };
    append_startup_log("main: tokio ok");

    let mut options = eframe::NativeOptions::default();
    options.renderer = eframe::Renderer::Wgpu;
    options.viewport = egui::ViewportBuilder::default()
        .with_inner_size([920.0, 640.0])
        .with_title("X → Telegram 追踪");

    let out = eframe::run_native(
        "xtg-app",
        options,
        Box::new(move |cc| Ok(Box::new(XtgApp::new(cc, rt.clone())) as _)),
    );
    match &out {
        Ok(()) => append_startup_log("main: run_native Ok"),
        Err(e) => {
            append_startup_log(&format!("main: run_native Err: {e}"));
            let log = startup_log_path();
            show_error_dialog(
                "XTG 界面启动失败",
                &format!(
                    "{e}\n\n可尝试：更新显卡驱动、安装 VC++ x64 运行库、运行同目录 start_xtg_safe.bat（软件渲染）。\n日志: {}",
                    log.display()
                ),
            );
        }
    }
    out
}
