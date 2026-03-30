use eframe::egui;
use std::sync::Arc;
use xtg_service::XtgService;

pub struct XtgApp {
    service: XtgService,
    log_rx: tokio::sync::broadcast::Receiver<String>,
    logs: Vec<String>,
    login_code: String,
    password_2fa: String,
}

impl XtgApp {
    pub fn new(cc: &eframe::CreationContext<'_>, rt: Arc<tokio::runtime::Runtime>) -> Self {
        crate::fonts::setup_cjk_fonts(&cc.egui_ctx);

        let service = XtgService::new(Some(rt));
        let log_rx = service.subscribe_logs();

        Self {
            service,
            log_rx,
            logs: vec!["就绪。填写 API ID/Hash，点「连接 TG」，再请求验证码登录。".into()],
            login_code: String::new(),
            password_2fa: String::new(),
        }
    }

    fn push_log(&mut self, s: String) {
        self.logs.push(s);
        if self.logs.len() > 500 {
            self.logs.drain(0..100);
        }
    }
}

impl eframe::App for XtgApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(line) = self.log_rx.try_recv() {
            self.push_log(line);
        }

        let mut settings = self.service.settings();

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("X → Telegram 追踪（用户态）");
            ui.separator();

            ui.horizontal(|ui| {
                ui.label("API ID");
                ui.text_edit_singleline(&mut settings.api_id);
                ui.label("API Hash");
                ui.text_edit_singleline(&mut settings.api_hash);
            });
            ui.horizontal(|ui| {
                ui.label("手机号 (+86…)");
                ui.text_edit_singleline(&mut settings.phone);
            });
            ui.horizontal(|ui| {
                ui.label("TG session 文件");
                ui.text_edit_singleline(&mut settings.tg_session_path);
            });

            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("连接 TG（启动 MTProto）").clicked() {
                    self.service.set_settings(settings.clone());
                    self.service.connect_tg_pool();
                }
                if ui.button("请求验证码").clicked() {
                    self.service.set_settings(settings.clone());
                    match self.service.request_code_v2() {
                        Ok(()) => self.push_log("验证码已发送，请输入后点「提交登录」".into()),
                        Err(e) => self.push_log(format!("请求验证码失败: {e}")),
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label("验证码");
                ui.text_edit_singleline(&mut self.login_code);
                if ui.button("提交登录").clicked() {
                    self.service.set_settings(settings.clone());
                    match self.service.submit_login(&self.login_code) {
                        Ok(()) => {
                            if self.service.has_pending_2fa() {
                                self.push_log("需要二步验证密码，填写后点「提交 2FA」".into());
                            } else {
                                self.push_log("Telegram 登录成功".into());
                            }
                        }
                        Err(e) => self.push_log(format!("登录失败: {e}")),
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label("2FA 密码");
                ui.add(egui::TextEdit::singleline(&mut self.password_2fa).password(true));
                if ui.button("提交 2FA").clicked() {
                    self.service.set_settings(settings.clone());
                    match self.service.submit_2fa(&self.password_2fa) {
                        Ok(()) => self.push_log("二步验证完成，已登录".into()),
                        Err(e) => self.push_log(format!("2FA 失败: {e}")),
                    }
                }
            });

            ui.separator();
            ui.label("TG 目标（每行一个：群组/频道 @username 或数字 id，同一帖会发往每一行）");
            ui.text_edit_multiline(&mut settings.tg_targets);
            ui.label("X 账号（每行一个 handle，不含 @）");
            ui.text_edit_multiline(&mut settings.x_handles);
            ui.label("轮询间隔（秒，最小 0.1；可用滑块、快捷按钮或直接改数字）");
            ui.horizontal(|ui| {
                for (label, v) in [
                    ("0.1s", 0.1_f64),
                    ("1s", 1.0),
                    ("5s", 5.0),
                    ("15s", 15.0),
                    ("60s", 60.0),
                ] {
                    if ui.small_button(label).clicked() {
                        settings.poll_interval_secs = v;
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label("滑块");
                ui.add(
                    egui::Slider::new(&mut settings.poll_interval_secs, 0.1..=3600.0)
                        .logarithmic(true)
                        .text("秒"),
                );
            });
            ui.horizontal(|ui| {
                ui.label("数值");
                ui.add(
                    egui::DragValue::new(&mut settings.poll_interval_secs)
                        .range(0.1..=3600.0)
                        .speed(0.05)
                        .min_decimals(1)
                        .max_decimals(4),
                );
                ui.label("最大媒体 MB");
                ui.add(egui::DragValue::new(&mut settings.max_media_mb).range(1..=200));
            });
            ui.horizontal(|ui| {
                ui.checkbox(&mut settings.use_fake_x, "使用假 X 数据（测试 pipeline）");
            });
            ui.horizontal(|ui| {
                ui.label("X API Base");
                ui.text_edit_singleline(&mut settings.x_api_base);
            });
            ui.horizontal(|ui| {
                ui.label("X Bearer Token");
                ui.add(
                    egui::TextEdit::singleline(&mut settings.x_bearer_token).password(true),
                );
            });
            ui.label(egui::RichText::new("X 抓取仅使用 Twitter API v2（Bearer）。在 X Developer Portal 创建应用并获取 Bearer；需具备用户时间线等权限。未勾选假数据时须填写 Token。").small());

            ui.separator();
            ui.label("汉化（xAI Grok，POST …/v1/chat/completions）");
            ui.horizontal(|ui| {
                ui.checkbox(&mut settings.ai_enabled, "启用 AI 汉化");
            });
            ui.label(egui::RichText::new("API Base 留空或误填 api.openai.com 时会用 https://api.x.ai/v1；API Key 只填密钥本身，不要带「Bearer 」前缀。Model 留空则用 grok-3-mini。").small());
            ui.horizontal(|ui| {
                ui.label("API Base");
                ui.text_edit_singleline(&mut settings.ai_api_base);
            });
            ui.horizontal(|ui| {
                ui.label("API Key");
                ui.add(
                    egui::TextEdit::singleline(&mut settings.ai_api_key).password(true),
                );
            });
            ui.horizontal(|ui| {
                ui.label("Model");
                ui.text_edit_singleline(&mut settings.ai_model);
            });
            ui.label(egui::RichText::new("群内 @本账号 并发送「最新」可拉取各 X 账号最近一条推文（需已连接 TG）").small());

            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("保存配置").clicked() {
                    self.service.set_settings(settings.clone());
                    if let Err(e) = self.service.save_settings() {
                        self.push_log(format!("保存配置失败: {e}"));
                    }
                }
                if ui.button("开始轮询").clicked() {
                    self.service.set_settings(settings.clone());
                    if let Err(e) = self.service.start_poll() {
                        self.push_log(format!("启动轮询: {e}"));
                    }
                }
                if ui.button("停止轮询").clicked() {
                    self.service.set_settings(settings.clone());
                    self.service.stop_poll();
                }
            });

            ui.separator();
            egui::ScrollArea::vertical().show(ui, |ui| {
                for line in &self.logs {
                    ui.label(egui::RichText::new(line).monospace());
                }
            });
        });

        self.service.set_settings(settings);

        ctx.request_repaint_after(std::time::Duration::from_millis(500));
    }
}
