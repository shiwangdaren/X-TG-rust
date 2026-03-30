//! XTG Web 管理端：REST API + SSE 日志 + 静态前端。

use axum::body::Body;
use axum::extract::State;
use axum::http::{header::AUTHORIZATION, Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Json;
use axum::Router;
use futures_util::stream::StreamExt;
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio_stream::wrappers::BroadcastStream;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;
use xtg_service::{AppSettings, XtgService};

#[derive(Clone)]
struct AppState {
    service: Arc<XtgService>,
    admin_token: Option<String>,
}

#[derive(Serialize)]
struct StatusResponse {
    tg_connected: bool,
    poll_running: bool,
    pending_2fa: bool,
}

#[derive(Deserialize)]
struct SignInBody {
    code: String,
}

#[derive(Deserialize)]
struct TwoFaBody {
    password: String,
}

async fn auth_middleware(
    State(state): State<Arc<AppState>>,
    req: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    if req.uri().path() == "/api/health" {
        return Ok(next.run(req).await);
    }
    if let Some(expected) = &state.admin_token {
        let ok = req
            .headers()
            .get(AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|a| a.strip_prefix("Bearer "))
            .map(|t| t == expected)
            .unwrap_or(false);
        if !ok {
            return Err(StatusCode::UNAUTHORIZED);
        }
    }
    Ok(next.run(req).await)
}

async fn health() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

async fn get_settings(State(state): State<Arc<AppState>>) -> Json<AppSettings> {
    Json(state.service.settings())
}

async fn put_settings(
    State(state): State<Arc<AppState>>,
    Json(s): Json<AppSettings>,
) -> Result<Json<AppSettings>, (StatusCode, String)> {
    state.service.set_settings(s);
    state
        .service
        .save_settings()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(state.service.settings()))
}

async fn tg_connect(State(state): State<Arc<AppState>>) -> Result<StatusCode, (StatusCode, String)> {
    state.service.connect_tg_pool();
    Ok(StatusCode::NO_CONTENT)
}

async fn tg_request_code(State(state): State<Arc<AppState>>) -> Result<StatusCode, (StatusCode, String)> {
    state
        .service
        .request_code_async()
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn tg_sign_in(
    State(state): State<Arc<AppState>>,
    Json(body): Json<SignInBody>,
) -> Result<StatusCode, (StatusCode, String)> {
    state
        .service
        .submit_login_async(&body.code)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn tg_2fa(
    State(state): State<Arc<AppState>>,
    Json(body): Json<TwoFaBody>,
) -> Result<StatusCode, (StatusCode, String)> {
    state
        .service
        .submit_2fa_async(&body.password)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn poll_start(State(state): State<Arc<AppState>>) -> Result<StatusCode, (StatusCode, String)> {
    state.service.start_poll().map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn poll_stop(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    state.service.stop_poll();
    StatusCode::NO_CONTENT
}

async fn api_status(State(state): State<Arc<AppState>>) -> Json<StatusResponse> {
    let tg = state
        .service
        .tg_client_arc()
        .lock()
        .await
        .is_some();
    Json(StatusResponse {
        tg_connected: tg,
        poll_running: state.service.poll_running(),
        pending_2fa: state.service.has_pending_2fa(),
    })
}

async fn logs_stream(
    State(state): State<Arc<AppState>>,
) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    let rx = state.service.subscribe_logs();
    let stream = BroadcastStream::new(rx).map(|item| {
        let line = match item {
            Ok(s) => s,
            Err(_) => return Ok(Event::default().comment("lag")),
        };
        Ok(Event::default().data(line))
    });
    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    )
}

fn api_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/settings", get(get_settings).put(put_settings))
        .route("/tg/connect", post(tg_connect))
        .route("/tg/request-code", post(tg_request_code))
        .route("/tg/sign-in", post(tg_sign_in))
        .route("/tg/2fa", post(tg_2fa))
        .route("/poll/start", post(poll_start))
        .route("/poll/stop", post(poll_stop))
        .route("/status", get(api_status))
        .route("/logs/stream", get(logs_stream))
        .with_state(state)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let admin_token = std::env::var("XTG_ADMIN_TOKEN").ok().filter(|s| !s.trim().is_empty());
    if admin_token.is_none() {
        tracing::warn!("XTG_ADMIN_TOKEN 未设置：API 不校验 Bearer（仅适合内网或反向代理保护）");
    }

    let service = Arc::new(XtgService::new(None));
    let app_state = Arc::new(AppState {
        service: Arc::clone(&service),
        admin_token,
    });

    let static_dir = std::env::var("XTG_STATIC_DIR").unwrap_or_else(|_| {
        if Path::new("crates/xtg-server/static/index.html").exists() {
            "crates/xtg-server/static".to_string()
        } else {
            "static".to_string()
        }
    });
    let index_path = PathBuf::from(&static_dir).join("index.html");

    let api = api_router(app_state.clone()).layer(middleware::from_fn_with_state(
        app_state.clone(),
        auth_middleware,
    ));

    let static_service = ServeDir::new(&static_dir).fallback(ServeFile::new(index_path));

    let app = Router::new()
        .nest("/api", api)
        .fallback_service(static_service)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http());

    let listen = std::env::var("XTG_LISTEN").unwrap_or_else(|_| "0.0.0.0:8080".to_string());
    let addr: SocketAddr = listen.parse()?;
    tracing::info!("listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
