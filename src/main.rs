use actix_web::{web, App, HttpServer};
use dotenv::dotenv;
use std::env;
use bulk_price_api::{AppState, routes};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use std::collections::HashMap;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    dotenv().ok();
    env_logger::init();

    let app_state = web::Data::new(AppState {
        price_cache: Arc::new(RwLock::new(HashMap::new())),
        last_fetch: Arc::new(RwLock::new(HashMap::new())),
        update_tasks: Arc::new(Mutex::new(HashMap::new())),
        active_connections: Arc::new(RwLock::new(HashMap::new())),
    });

    let ip = env::var("IP_ADDRESS").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port = env::var("PORT").unwrap_or_else(|_| "8080".to_string());

    HttpServer::new(move || {
        App::new()
            .app_data(app_state.clone())
            .service(routes::websocket)
            .service(routes::health_check)
    })
    .bind(format!("{}:{}", ip, port))?
    .run()
    .await
}