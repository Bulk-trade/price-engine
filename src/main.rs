use actix::prelude::*;
use actix_web::{get, web, App, HttpRequest, HttpResponse, HttpServer, Responder};
use actix_web_actors::ws;
use anyhow::Error as AnyhowError;
use backoff::{Error as BackoffError, ExponentialBackoff};
use log::{error, info};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, RwLock};
use tokio::time::{interval, Instant};
use dotenv::dotenv;

const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

#[derive(Clone, Serialize, Deserialize)]
struct PriceInfo {
    price: f64,
    timestamp: u64,
}

struct AppState {
    price_cache: Arc<RwLock<HashMap<String, PriceInfo>>>,
    last_fetch: Arc<RwLock<HashMap<String, Instant>>>,
    update_tasks: Arc<Mutex<HashMap<String, tokio::task::JoinHandle<()>>>>,
    active_connections: Arc<RwLock<HashMap<String, usize>>>,
}

struct WsSession {
    app_state: web::Data<AppState>,
    token_mint: String,
    token_decimal: u8,
}

impl Actor for WsSession {
    type Context = ws::WebsocketContext<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        self.ensure_price_updates();
        self.increment_connection_count();
        self.stream_price(ctx);
    }

    fn stopped(&mut self, _ctx: &mut Self::Context) {
        self.decrement_connection_count();
    }
}

impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for WsSession {
    fn handle(&mut self, msg: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        match msg {
            Ok(ws::Message::Ping(msg)) => ctx.pong(&msg),
            Ok(ws::Message::Close(reason)) => {
                ctx.close(reason);
                ctx.stop();
            }
            _ => (),
        }
    }
}

impl WsSession {
    fn stream_price(&self, ctx: &mut ws::WebsocketContext<Self>) {
        let app_state = self.app_state.clone();
        let token_mint = self.token_mint.clone();
        let _token_decimal = self.token_decimal;
        ctx.run_interval(Duration::from_millis(3000), move |_, ctx| {
            let price_cache = app_state.price_cache.try_read().unwrap();
            if let Some(price_info) = price_cache.get(&token_mint) {
                ctx.text(serde_json::to_string(price_info).unwrap());
            }
        });
    }

    fn ensure_price_updates(&self) {
        let app_state = self.app_state.clone();
        let token_mint = self.token_mint.clone();
        let token_decimal = self.token_decimal.clone();
        actix_web::rt::spawn(async move {
            let mut update_tasks = app_state.update_tasks.lock().await;
            if !update_tasks.contains_key(&token_mint) {
                let task = tokio::spawn(update_price_cache(
                    app_state.clone(),
                    token_mint.clone(),
                    token_decimal,
                ));
                update_tasks.insert(token_mint, task);
            }
        });
    }

    fn increment_connection_count(&self) {
        let app_state = self.app_state.clone();
        let token = self.token_mint.clone();
        actix_web::rt::spawn(async move {
            let mut connections = app_state.active_connections.write().await;
            *connections.entry(token).or_insert(0) += 1;
        });
    }

    fn decrement_connection_count(&self) {
        let app_state = self.app_state.clone();
        let token = self.token_mint.clone();
        actix_web::rt::spawn(async move {
            let mut connections = app_state.active_connections.write().await;
            if let Some(count) = connections.get_mut(&token) {
                *count -= 1;
                if *count == 0 {
                    connections.remove(&token);
                    let mut update_tasks = app_state.update_tasks.lock().await;
                    if let Some(task) = update_tasks.remove(&token) {
                        task.abort();
                    }
                    app_state.price_cache.write().await.remove(&token);
                    app_state.last_fetch.write().await.remove(&token);
                    info!("Stopped price updates for token: {}", token);
                }
            }
        });
    }
}

fn convert_to_unit(amount: f64, token_decimal: u32) -> u64 {
    (amount * 10f64.powi(token_decimal as i32)).round() as u64
}

async fn fetch_price(token_mint: &str, token_decimal: u8) -> Result<f64, AnyhowError> {
    let base_url = env::var("JUPITER_API_URL").expect("JUPITER_API_URL must be set");
    let mut amount_in = convert_to_unit(1.0, token_decimal as u32); // Start with 1 unit
    let mut total_increase = 1.0; // Track the total increase

    loop {
    let url = format!(
        "{}?inputMint={}&outputMint={}&amount={}",
        base_url, token_mint, USDC_MINT, amount_in,
    );

    let response = reqwest::get(&url).await?;
    let body = response.text().await?;

    let json_response: serde_json::Value = serde_json::from_str(&body)?;

    // Check for error in the JSON response
        if let Some(error) = json_response.get("error") {
            if error.as_str().unwrap_or("").contains("Cannot compute other amount threshold") {
                // Increase the amount_in and retry
                amount_in *= 10;
                total_increase *= 10.0;
                continue;
            } else {
                return Err(AnyhowError::msg(format!(
                    "API error: {}",
                    error.as_str().unwrap_or("Unknown error")
                )));
            }
        }

    let out_amount = json_response["outAmount"]
        .as_str()
        .ok_or_else(|| AnyhowError::msg("outAmount not found"))?
        .parse::<f64>()?;

    return Ok(out_amount / (1_000_000.0 * total_increase));
  }
} 

async fn update_price_cache(app_state: web::Data<AppState>, token_mint: String, token_decimal: u8) {
    let mut interval = interval(Duration::from_millis(3000));
    loop {
        interval.tick().await;

        {
            let connections = app_state.active_connections.read().await;
            if !connections.contains_key(&token_mint) {
                info!(
                    "No active connections for {}. Stopping price updates.",
                    token_mint
                );
                break;
            }
        }

        {
            let last_fetch = app_state.last_fetch.read().await;
            if let Some(last_fetch_time) = last_fetch.get(&token_mint) {
                let time_since_last_fetch = Instant::now().duration_since(*last_fetch_time);
                if time_since_last_fetch < Duration::from_millis(3000) {
                    tokio::time::sleep(Duration::from_millis(3000) - time_since_last_fetch).await;
                }
            }
        }

        let fetch_result = backoff::future::retry(ExponentialBackoff::default(), || async {
            match fetch_price(&token_mint, token_decimal).await {
                Ok(price) => Ok(price),
                Err(e) => {
                    error!(
                        "Error fetching price for {}: {}. Retrying...",
                        token_mint, e
                    );
                    Err(BackoffError::transient(e))
                }
            }
        })
        .await;

        match fetch_result {
            Ok(price) => {
                let mut cache = app_state.price_cache.write().await;
                cache.insert(
                    token_mint.clone(),
                    PriceInfo {
                        price,
                        timestamp: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_secs(),
                    },
                );
                let mut last_fetch = app_state.last_fetch.write().await;
                last_fetch.insert(token_mint.clone(), Instant::now());
                info!("Updated price for {}: {}", token_mint, price);
            }
            Err(e) => error!(
                "Failed to fetch price for {} after retries: {}",
                token_mint, e
            ),
        }
    }
}

#[get("/ws/price")]
async fn websocket(
    req: HttpRequest,
    stream: web::Payload,
    query: web::Query<HashMap<String, String>>,
    app_state: web::Data<AppState>,
) -> impl Responder {
    let token_mint = query
        .get("token")
        .cloned()
        .unwrap_or_else(|| USDC_MINT.to_string());

    let token_decimal = query
        .get("token_decimal")
        .and_then(|d| d.parse().ok())
        .unwrap_or(6);
    ws::start(
        WsSession {
            app_state: app_state.clone(),
            token_mint,
            token_decimal,
        },
        &req,
        stream,
    )
}

#[get("/health")]
async fn health_check(app_state: web::Data<AppState>) -> impl Responder {
    let price_cache = app_state.price_cache.read().await;
    if price_cache.is_empty() {
        HttpResponse::ServiceUnavailable().json("No price data available")
    } else {
        HttpResponse::Ok().json("Healthy")
    }
}

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
            .service(websocket)
            .service(health_check)
    })
    .bind(format!("{}:{}", ip, port))?
    .run()
    .await
}