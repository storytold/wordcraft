//! `wordcraft-collab-server`: the authoritative sync server.
//!
//! ```text
//! COLLAB_BIND=127.0.0.1:8087 \
//! COLLAB_JWT_SECRET=dev-secret \
//! wordcraft-collab-server
//! ```
//!
//! Endpoints: `GET /healthz`, `GET /metrics`, `GET /ws` (WebSocket).
//! Tokens: `COLLAB_JWT_SECRET` (HS256, local development) or `COLLAB_JWT_PEM`
//! (ES256 public key, production — ADR-0008). No token ever reaches the logs.

use std::process::ExitCode;

use wordcraft_collab_server::{AppState, Config, app};

#[tokio::main]
async fn main() -> ExitCode {
    let cfg = match Config::from_env() {
        Ok(cfg) => cfg,
        Err(message) => {
            eprintln!("wordcraft-collab-server: {message}");
            return ExitCode::from(2);
        }
    };
    let bind = cfg.bind.clone();
    let state = AppState::new(cfg);
    let serving = state.clone();
    let listener = match tokio::net::TcpListener::bind(&bind).await {
        Ok(listener) => listener,
        Err(err) => {
            eprintln!("wordcraft-collab-server: cannot bind {bind}: {err}");
            return ExitCode::from(2);
        }
    };
    eprintln!("wordcraft-collab-server: listening on {bind}");
    let result = axum::serve(listener, app(serving)).with_graceful_shutdown(shutdown_signal()).await;
    // SIGTERM from the container runtime: write every open room before exit.
    if let Err(message) = state.persist_all() {
        eprintln!("wordcraft-collab-server: final snapshot failed: {message}");
    }
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("wordcraft-collab-server: stopped: {err}");
            ExitCode::FAILURE
        }
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            signal.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
}
