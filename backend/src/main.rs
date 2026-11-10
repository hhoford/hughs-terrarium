//! Authoritative hive sandbox HTTP server. Physics: Rapier/Parry. Maps: MapLibre.
mod bake;
mod distribute;
mod envelope;
mod geography;
mod hex_cache;
mod hollows;
mod lab;
mod land;
mod physics;
mod radar_field;
mod topo;
mod types;
use axum::{
    routing::{get, post},
    Json, Router,
};
use serde::Serialize;
use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
};
use tower_http::{
    cors::CorsLayer,
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};
#[derive(Serialize)]
struct Health {
    status: &'static str,
    service: &'static str,
    version: &'static str,
}
pub fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

async fn health() -> Json<Health> {
    Json(Health {
        status: "ok",
        service: "hug-gamer-backend",
        version: env!("CARGO_PKG_VERSION"),
    })
}

/// Bundled application state: the tile renderer + the authoritative hive game.
#[derive(Clone)]
struct AppState {
    land: Arc<land::Land>,
    hollows: Arc<hollows::Tiles>,
    renderer: Arc<topo::TerrainSource>,
    lab: lab::Store,
    persistence: Arc<lab::persistence::Persistence>,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "hug_gamer_backend=info,tower_http=info".into()),
        )
        .init();

    if std::env::args().nth(1).as_deref() == Some("--geography") {
        if let Err(e) = bake::run().await {
            eprintln!("{e}");
            std::process::exit(1)
        }
        return;
    }
    geography::Store::start_maintenance();
    let renderer = Arc::new(topo::TerrainSource::new());

    // Serve the built React client (frontend/dist). HUG_GAMER_STATIC overrides
    // the directory; in the container it points at /app/static.
    let static_dir = std::env::var("HUG_GAMER_STATIC").unwrap_or_else(|_| "static".to_string());
    let index_file = format!("{static_dir}/index.html");

    let persistence =
        Arc::new(lab::persistence::Persistence::open_env().expect("open persistent world"));
    let (mut initial, centres) = persistence
        .restore()
        .expect("restore persistent world")
        .unwrap_or_else(|| (lab::new(), Vec::new()));
    initial.journal = persistence.journal.clone();
    let state = AppState {
        land: Arc::new(land::Land::default()),
        hollows: Arc::new(hollows::Tiles::new()),
        renderer,
        lab: Arc::new(Mutex::new(initial)),
        persistence,
    };
    lab::restore_geography(&state, centres)
        .await
        .expect("prepare saved world geography");
    {
        let g = state.lab.lock().unwrap();
        g.journal.emit(
            g.epoch,
            g.time,
            "server_started",
            None,
            None,
            serde_json::json!({"units":g.units.len()}),
        );
    }
    state
        .persistence
        .save(&state.lab, true)
        .expect("save initial world");
    let checkpoint_task = lab::persistence::start(state.clone());
    let simulation = lab::start(state.clone());
    let shutdown_state = state.clone();
    let flush_task = tokio::spawn(async {
        let mesh = envelope::Envelope::shared();
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            if let Err(e) = mesh.flush_cache().await {
                tracing::warn!(%e,"Gaussian cache flush failed");
            }
        }
    });
    let app = Router::new()
        .route("/api/health", get(health))
        .route("/api/geography/status", get(geography_status))
        .route("/api/geography/water/{z}/{x}/{y}", get(water_tile))
        .route("/api/lab/state", get(lab::state))
        .route("/api/lab/ai", get(lab::ai_status))
        .route("/api/lab/monitor", get(lab::persistence::monitor))
        .route("/api/lab/events", get(lab::persistence::events))
        .route("/api/lab/save", post(lab::persistence::save_now))
        .route("/api/lab/tags", post(lab::tag))
        .route("/api/lab/reset", post(lab::reset))
        .route("/api/lab/feed", post(lab::feed))
        .route("/api/lab/command", post(lab::command))
        .route("/api/lab/input", post(lab::input))
        .route("/api/lab/terrain", get(lab::terrain))
        .route("/api/lab/envelope", get(lab::envelope_at))
        .route("/api/lab/hollows", get(lab::hollows))
        .route("/api/lab/hollows/{z}/{x}/{y}", get(hollows::tile))
        .route("/api/lab/dem/{z}/{x}/{y}", get(lab::dem))
        .route("/api/game/{*rest}", get(lab::retired).post(lab::retired))
        .with_state(state)
        .fallback_service(ServeDir::new(&static_dir).not_found_service(ServeFile::new(&index_file)))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http());

    let addr: SocketAddr = std::env::var("HUG_GAMER_BIND")
        .unwrap_or_else(|_| "127.0.0.1:8787".to_string())
        .parse()
        .expect("HUG_GAMER_BIND must be a host:port");
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("bind failed");
    tracing::info!(%addr, "hug-gamer backend listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let mut terminate =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .expect("SIGTERM");
            tokio::select! { _=tokio::signal::ctrl_c()=>{}, _=terminate.recv()=>{} }
            for task in simulation {
                task.abort();
                let _ = task.await;
            }
        })
        .await
        .expect("server error");
    // Finish in-flight HTTP requests first; no producer can dirty a block after this drain.
    checkpoint_task.abort();
    let _ = checkpoint_task.await;
    {
        let g = shutdown_state.lab.lock().unwrap();
        g.journal.emit(
            g.epoch,
            g.time,
            "server_stopped",
            None,
            None,
            serde_json::json!({}),
        );
    }
    if let Err(error) = shutdown_state.persistence.save(&shutdown_state.lab, true) {
        tracing::error!(%error,"Final world checkpoint failed");
    }
    flush_task.abort();
    let _ = flush_task.await;
    if let Err(e) = envelope::Envelope::shared().flush_cache().await {
        tracing::error!(%e,"Final Gaussian cache flush failed");
    }
    if let Err(e) = geography::Store::shared().checkpoint().await {
        tracing::error!(%e,"Final source checkpoint failed");
    }
}

async fn geography_status() -> Json<serde_json::Value> {
    let s = geography::Store::shared();
    let mesh = envelope::Envelope::shared();
    Json(
        serde_json::json!({"version":geography::VERSION,"mode":"cache","store":s.stats(),"hexagons":mesh.hex_stats(),"gaussian":mesh.stats()}),
    )
}

async fn water_tile(
    axum::extract::Path((z, x, y)): axum::extract::Path<(u32, u32, u32)>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    if z > 14 || x >= 1u32 << z || y >= 1u32 << z {
        return (axum::http::StatusCode::BAD_REQUEST, "Invalid tile").into_response();
    }
    match land::Land::default().vector_tile(z, x, y).await {
        Ok(v) => (
            [
                (axum::http::header::CONTENT_TYPE, "application/x-protobuf"),
                (
                    axum::http::header::CACHE_CONTROL,
                    "public,max-age=31536000,immutable",
                ),
            ],
            crate::geography::response_bytes(v),
        )
            .into_response(),
        Err(e) => (axum::http::StatusCode::BAD_GATEWAY, e).into_response(),
    }
}
