//! 核心业务：TG 连接/登录、轮询、配置持久化、日志广播。

use crate::command_state::CommandState;
use crate::paths;
use crate::pipeline::{default_store_path, run_poll_round};
use crate::settings::AppSettings;
use crate::tg_commands;
use grammers_client::Client;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{broadcast, Mutex as TokioMutex};
use tokio::task::JoinHandle;
use xtg_core::JobConfig;
use xtg_tg_bridge::{
    is_authorized, request_login_code, sign_in_with_code, sign_in_with_password, GrammersPool,
    LoginToken, PasswordToken,
};
use xtg_tg_bridge::SignInError;
use xtg_x_session::build_x_source;

pub struct XtgService {
    pub runtime: Option<Arc<tokio::runtime::Runtime>>,
    inner: StdMutex<XtgServiceInner>,
}

struct XtgServiceInner {
    settings: AppSettings,
    config_path: PathBuf,
    data_dir: PathBuf,
    tg_client: Arc<TokioMutex<Option<Client>>>,
    _tg_runner: Arc<TokioMutex<Option<JoinHandle<()>>>>,
    login_token: Option<LoginToken>,
    pending_password: Option<PasswordToken>,
    poll_running: Arc<AtomicBool>,
    poll_task: Arc<StdMutex<Option<JoinHandle<()>>>>,
    log_tx: broadcast::Sender<String>,
    command_state: Arc<StdMutex<CommandState>>,
    tg_updates_started: Arc<AtomicBool>,
}

impl XtgService {
    /// `runtime`: 桌面端传入 `Some(Arc<Runtime>)`；纯 tokio 服务传 `None`（异步任务用 `tokio::spawn`）。
    pub fn new(runtime: Option<Arc<tokio::runtime::Runtime>>) -> Self {
        let data_dir = paths::data_dir();
        let config_path = paths::config_path();
        let settings = AppSettings::load(&config_path).unwrap_or_default();
        let cmd0 = CommandState::from_settings(&settings);
        let (log_tx, _) = broadcast::channel(1024);

        Self {
            runtime,
            inner: StdMutex::new(XtgServiceInner {
                settings,
                config_path,
                data_dir,
                tg_client: Arc::new(TokioMutex::new(None)),
                _tg_runner: Arc::new(TokioMutex::new(None)),
                login_token: None,
                pending_password: None,
                poll_running: Arc::new(AtomicBool::new(false)),
                poll_task: Arc::new(StdMutex::new(None)),
                log_tx,
                command_state: Arc::new(StdMutex::new(cmd0)),
                tg_updates_started: Arc::new(AtomicBool::new(false)),
            }),
        }
    }

    pub fn subscribe_logs(&self) -> broadcast::Receiver<String> {
        self.inner.lock().expect("xtg inner").log_tx.subscribe()
    }

    pub fn log_sender(&self) -> broadcast::Sender<String> {
        self.inner.lock().expect("xtg inner").log_tx.clone()
    }

    pub fn settings(&self) -> AppSettings {
        self.inner.lock().expect("xtg inner").settings.clone()
    }

    pub fn config_path(&self) -> PathBuf {
        self.inner.lock().expect("xtg inner").config_path.clone()
    }

    pub fn data_dir(&self) -> PathBuf {
        self.inner.lock().expect("xtg inner").data_dir.clone()
    }

    pub fn set_settings(&self, s: AppSettings) {
        self.inner.lock().expect("xtg inner").settings = s;
    }

    fn spawn_bg(&self, fut: impl std::future::Future<Output = ()> + Send + 'static) {
        if let Some(rt) = &self.runtime {
            rt.spawn(fut);
        } else {
            tokio::spawn(fut);
        }
    }

    pub fn connect_tg_pool(&self) {
        let (path, api_id, log_tx, tg_client, runner_slot, command_state, tg_updates_started, data_dir) = {
            let g = self.inner.lock().expect("xtg inner");
            let path = g.settings.tg_session_path_buf();
            let api_id: i32 = g.settings.api_id.parse().unwrap_or(0);
            if api_id == 0 {
                let _ = g.log_tx.send("API ID 无效".into());
                return;
            }
            if let Ok(mut cs) = g.command_state.lock() {
                *cs = CommandState::from_settings(&g.settings);
            }
            (
                path,
                api_id,
                g.log_tx.clone(),
                g.tg_client.clone(),
                g._tg_runner.clone(),
                g.command_state.clone(),
                g.tg_updates_started.clone(),
                g.data_dir.clone(),
            )
        };

        let http = reqwest::Client::new();

        self.spawn_bg(async move {
            let pool = match GrammersPool::connect(&path, api_id).await {
                Ok(p) => p,
                Err(e) => {
                    let _ = log_tx.send(format!("TG 连接失败: {e}"));
                    return;
                }
            };
            let GrammersPool {
                client,
                runner,
                updates,
            } = pool;
            let j = tokio::spawn(runner.run());
            *runner_slot.lock().await = Some(j);
            *tg_client.lock().await = Some(client.clone());
            let _ = log_tx.send("Telegram 传输层已连接".into());

            if !tg_updates_started.swap(true, Ordering::SeqCst) {
                let temp_base = data_dir.join("tg_cmd_media");
                let _ = tokio::fs::create_dir_all(&temp_base).await;
                let log2 = log_tx.clone();
                tokio::spawn(tg_commands::run_updates_task(
                    client,
                    updates,
                    command_state,
                    http,
                    temp_base,
                    log2,
                ));
            }
        });
    }

    pub fn request_code_v2(&self) -> Result<(), String> {
        let (phone, api_hash, tg, rt) = {
            let g = self.inner.lock().map_err(|_| "lock")?;
            (
                g.settings.phone.clone(),
                g.settings.api_hash.clone(),
                g.tg_client.clone(),
                self.runtime.clone(),
            )
        };
        let rt = rt.ok_or_else(|| "需要 Tokio Runtime（桌面端）".to_string())?;
        let (tx, rx) = std::sync::mpsc::channel::<Result<LoginToken, String>>();
        let client = match rt.block_on(async { tg.lock().await.clone() }) {
            Some(c) => c,
            None => return Err("请先点「连接 TG」".into()),
        };
        std::thread::spawn(move || {
            let r = rt
                .block_on(request_login_code(&client, &phone, &api_hash))
                .map_err(|e| e.to_string());
            let _ = tx.send(r);
        });
        match rx.recv() {
            Ok(Ok(tok)) => {
                self.inner.lock().expect("inner").login_token = Some(tok);
                Ok(())
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err("内部错误（验证码通道）".into()),
        }
    }

    pub fn submit_login(&self, code: &str) -> Result<(), String> {
        let token = match self.inner.lock().expect("inner").login_token.take() {
            Some(t) => t,
            None => return Err("请先请求验证码".into()),
        };
        let code = code.to_string();
        let tg = self.inner.lock().expect("inner").tg_client.clone();
        let rt = self
            .runtime
            .clone()
            .ok_or_else(|| "需要 Tokio Runtime".to_string())?;
        let (tx, rx) =
            std::sync::mpsc::channel::<Result<grammers_client::peer::User, SignInError>>();
        let client = match rt.block_on(async { tg.lock().await.clone() }) {
            Some(c) => c,
            None => return Err("请先点「连接 TG」".into()),
        };
        std::thread::spawn(move || {
            let r = rt.block_on(sign_in_with_code(&client, &token, &code));
            let _ = tx.send(r);
        });
        match rx.recv() {
            Ok(Ok(_user)) => {
                self.inner.lock().expect("inner").pending_password = None;
                Ok(())
            }
            Ok(Err(SignInError::PasswordRequired(t))) => {
                self.inner.lock().expect("inner").pending_password = Some(t);
                self.inner.lock().expect("inner").login_token = None;
                Ok(())
            }
            Ok(Err(e)) => Err(e.to_string()),
            Err(_) => Err("内部错误（登录通道）".into()),
        }
    }

    pub fn submit_2fa(&self, password: &str) -> Result<(), String> {
        let token = match self.inner.lock().expect("inner").pending_password.take() {
            Some(t) => t,
            None => return Err("当前不需要 2FA".into()),
        };
        let pwd: Vec<u8> = password.as_bytes().to_vec();
        let tg = self.inner.lock().expect("inner").tg_client.clone();
        let rt = self
            .runtime
            .clone()
            .ok_or_else(|| "需要 Tokio Runtime".to_string())?;
        let (tx, rx) =
            std::sync::mpsc::channel::<Result<grammers_client::peer::User, SignInError>>();
        let client = match rt.block_on(async { tg.lock().await.clone() }) {
            Some(c) => c,
            None => return Err("请先点「连接 TG」".into()),
        };
        std::thread::spawn(move || {
            let r = rt.block_on(sign_in_with_password(&client, token, &pwd));
            let _ = tx.send(r);
        });
        match rx.recv() {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(e.to_string()),
            Err(_) => Err("内部错误（2FA 通道）".into()),
        }
    }

    pub fn start_poll(&self) -> Result<(), String> {
        let g = self.inner.lock().map_err(|_| "lock")?;
        if g.poll_task.lock().map(|t| t.is_some()).unwrap_or(false) {
            g.poll_running.store(true, Ordering::SeqCst);
            let _ = g.log_tx.send("轮询已恢复".into());
            return Ok(());
        }

        g.poll_running.store(true, Ordering::SeqCst);
        let s = g.settings.clone();
        *g.command_state.lock().expect("cmd") = CommandState::from_settings(&s);

        let api_id: i32 = s.api_id.parse().unwrap_or(0);
        if api_id == 0 {
            let _ = g.log_tx.send("API ID 无效".into());
            g.poll_running.store(false, Ordering::SeqCst);
            return Err("API ID 无效".into());
        }

        let handles: Vec<String> = s
            .x_handles
            .lines()
            .map(|l| l.trim().trim_start_matches('@').to_string())
            .filter(|x| !x.is_empty())
            .collect();

        let tg_targets = s.tg_target_list();
        if tg_targets.is_empty() {
            let _ = g.log_tx.send("未配置 Telegram 目标（每行一个群组/频道）".into());
            g.poll_running.store(false, Ordering::SeqCst);
            return Err("未配置 TG 目标".into());
        }

        if !s.use_fake_x && s.x_bearer_token.trim().is_empty() {
            let _ = g.log_tx.send("未配置 X Bearer Token（或勾选「假 X 数据」用于测试）".into());
            g.poll_running.store(false, Ordering::SeqCst);
            return Err("未配置 X Bearer".into());
        }

        let poll_started_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        let job = JobConfig {
            x_handles: handles,
            tg_targets,
            poll_interval_secs: s.poll_interval_secs,
            max_media_bytes: s.max_media_mb * 1024 * 1024,
            poll_started_at_ms,
            ai: s.ai_job_config(),
        };

        let store_path = default_store_path(&g.data_dir);
        let tg = g.tg_client.clone();
        let log_tx = g.log_tx.clone();
        let use_fake = s.use_fake_x;
        let x_bearer = s.x_bearer_token.clone();
        let x_api_base = s.x_api_base.clone();
        let interval = job.poll_interval_secs.max(0.1);
        let running = g.poll_running.clone();
        let job = Arc::new(job);
        drop(g);

        let j = if let Some(rt) = &self.runtime {
            rt.spawn(async move {
                poll_loop(
                    tg,
                    log_tx,
                    use_fake,
                    x_bearer,
                    x_api_base,
                    store_path,
                    job,
                    running,
                    interval,
                )
                .await;
            })
        } else {
            tokio::spawn(async move {
                poll_loop(
                    tg,
                    log_tx,
                    use_fake,
                    x_bearer,
                    x_api_base,
                    store_path,
                    job,
                    running,
                    interval,
                )
                .await;
            })
        };

        self.inner
            .lock()
            .expect("inner")
            .poll_task
            .lock()
            .expect("poll")
            .replace(j);
        let _ = self
            .inner
            .lock()
            .expect("inner")
            .log_tx
            .send(format!("轮询已启动，间隔 {:.3} 秒", interval));
        Ok(())
    }

    pub fn stop_poll(&self) {
        let g = self.inner.lock().expect("inner");
        g.poll_running.store(false, Ordering::SeqCst);
        let _ = g.log_tx.send("已暂停轮询（任务仍在后台，可再次开始）".into());
    }

    pub fn save_settings(&self) -> Result<(), String> {
        let mut g = self.inner.lock().map_err(|_| "lock")?;
        g.settings.poll_interval_secs = g.settings.poll_interval_secs.clamp(0.1, 3600.0);
        let path = g.config_path.clone();
        let s = g.settings.clone();
        match s.save(&path) {
            Ok(()) => {
                *g.command_state.lock().expect("cmd") = CommandState::from_settings(&s);
                let _ = g.log_tx.send("配置已保存".into());
                Ok(())
            }
            Err(e) => {
                let _ = g.log_tx.send(format!("保存配置失败: {e}"));
                Err(e.to_string())
            }
        }
    }

    pub fn has_pending_2fa(&self) -> bool {
        self.inner
            .lock()
            .expect("inner")
            .pending_password
            .is_some()
    }

    pub fn poll_running(&self) -> bool {
        self.inner
            .lock()
            .expect("inner")
            .poll_running
            .load(Ordering::SeqCst)
    }

    pub fn tg_client_arc(&self) -> Arc<TokioMutex<Option<Client>>> {
        self.inner.lock().expect("inner").tg_client.clone()
    }

    /// 供 Web 服务使用（无独立 `Runtime` 时）。
    pub async fn request_code_async(&self) -> Result<(), String> {
        let (phone, api_hash, tg) = {
            let g = self.inner.lock().map_err(|_| "lock failed")?;
            (
                g.settings.phone.clone(),
                g.settings.api_hash.clone(),
                g.tg_client.clone(),
            )
        };
        let client = tg.lock().await.clone().ok_or_else(|| "请先连接 TG".to_string())?;
        let tok = request_login_code(&client, &phone, &api_hash)
            .await
            .map_err(|e| e.to_string())?;
        self.inner.lock().expect("inner").login_token = Some(tok);
        Ok(())
    }

    pub async fn submit_login_async(&self, code: &str) -> Result<(), String> {
        let token = match self.inner.lock().expect("inner").login_token.take() {
            Some(t) => t,
            None => return Err("请先请求验证码".into()),
        };
        let tg = self.inner.lock().expect("inner").tg_client.clone();
        let client = tg.lock().await.clone().ok_or_else(|| "请先连接 TG".to_string())?;
        match sign_in_with_code(&client, &token, code).await {
            Ok(_) => {
                self.inner.lock().expect("inner").pending_password = None;
                Ok(())
            }
            Err(SignInError::PasswordRequired(t)) => {
                self.inner.lock().expect("inner").pending_password = Some(t);
                self.inner.lock().expect("inner").login_token = None;
                Ok(())
            }
            Err(e) => Err(e.to_string()),
        }
    }

    pub async fn submit_2fa_async(&self, password: &str) -> Result<(), String> {
        let token = match self.inner.lock().expect("inner").pending_password.take() {
            Some(t) => t,
            None => return Err("当前不需要 2FA".into()),
        };
        let tg = self.inner.lock().expect("inner").tg_client.clone();
        let client = tg.lock().await.clone().ok_or_else(|| "请先连接 TG".to_string())?;
        sign_in_with_password(&client, token, password.as_bytes())
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

async fn poll_loop(
    tg: Arc<TokioMutex<Option<Client>>>,
    log_tx: broadcast::Sender<String>,
    use_fake: bool,
    x_bearer: String,
    x_api_base: String,
    store_path: PathBuf,
    job: Arc<JobConfig>,
    running: Arc<AtomicBool>,
    interval: f64,
) {
    let http = reqwest::Client::new();
    let temp_base = std::env::temp_dir().join("xtg-media");
    let _ = tokio::fs::create_dir_all(&temp_base).await;

    loop {
        if !running.load(Ordering::SeqCst) {
            tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
            continue;
        }

        let client = tg.lock().await.clone();
        let Some(client) = client else {
            let _ = log_tx.send("轮询跳过：未连接 TG".into());
            tokio::time::sleep(tokio::time::Duration::from_secs_f64(interval)).await;
            continue;
        };
        if !matches!(is_authorized(&client).await, Ok(true)) {
            let _ = log_tx.send("轮询跳过：TG 未授权".into());
            tokio::time::sleep(tokio::time::Duration::from_secs_f64(interval)).await;
            continue;
        }

        let x_fetcher = build_x_source(use_fake, &x_bearer, &x_api_base);

        run_poll_round(
            x_fetcher.as_ref(),
            &client,
            &http,
            &store_path,
            job.as_ref(),
            &temp_base,
            &log_tx,
        )
        .await;

        tokio::time::sleep(tokio::time::Duration::from_secs_f64(interval)).await;
    }
}
