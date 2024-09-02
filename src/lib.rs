use actix::prelude::*;
use actix_web::{get, web, HttpRequest, HttpResponse, Responder};
use actix_web_actors::ws;
use anyhow::Error as AnyhowError;
use backoff::{Error as BackoffError, ExponentialBackoff};
use log::{error, info, debug};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, RwLock};
use tokio::time::{interval, Instant};
use futures::executor::block_on;

pub const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct PriceInfo {
    pub price: f64,
    pub timestamp: u64,
}

pub struct AppState {
    pub price_cache: Arc<RwLock<HashMap<String, PriceInfo>>>,
    pub last_fetch: Arc<RwLock<HashMap<String, Instant>>>,
    pub update_tasks: Arc<Mutex<HashMap<String, tokio::task::JoinHandle<()>>>>,
    pub active_connections: Arc<RwLock<HashMap<String, usize>>>,
}

pub struct WsSession {
    pub app_state: web::Data<AppState>,
    pub token_mint: String,
    pub token_decimal: u8,
}

impl WsSession {
    pub fn stream_price(&self, ctx: &mut ws::WebsocketContext<Self>) {
        let app_state = self.app_state.clone();
        let token_mint = self.token_mint.clone();

        // Send initial price if available
        {
            let price_cache = block_on(app_state.price_cache.read());
            if let Some(price_info) = price_cache.get(&token_mint) {
                if let Ok(json) = serde_json::to_string(price_info) {
                    ctx.text(json);
                    debug!("Sent initial price for {}: {:?}", token_mint, price_info);
                }
            }
        }

        // Set up interval for periodic updates
        ctx.run_interval(Duration::from_secs(3), move |_, ctx| {
            let price_cache = block_on(app_state.price_cache.read());
            if let Some(price_info) = price_cache.get(&token_mint) {
                if let Ok(json) = serde_json::to_string(price_info) {
                    ctx.text(json);
                    debug!("Sent price update for {}: {:?}", token_mint, price_info);
                }
            }
        });
    }

    pub fn ensure_price_updates(&self) {
        let app_state = self.app_state.clone();
        let token_mint = self.token_mint.clone();
        let token_decimal = self.token_decimal;
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

    pub fn increment_connection_count(&self) {
        let app_state = self.app_state.clone();
        let token = self.token_mint.clone();
        actix_web::rt::spawn(async move {
            let mut connections = app_state.active_connections.write().await;
            *connections.entry(token).or_insert(0) += 1;
        });
    }

    pub fn decrement_connection_count(&self) {
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

impl Actor for WsSession {
    type Context = ws::WebsocketContext<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        info!("WebSocket connection started for token: {}", self.token_mint);
        self.ensure_price_updates();
        self.increment_connection_count();
        self.stream_price(ctx);
    }

    fn stopped(&mut self, _ctx: &mut Self::Context) {
        info!("WebSocket connection closed for token: {}", self.token_mint);
        self.decrement_connection_count();
    }
}

impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for WsSession {
    fn handle(&mut self, msg: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        match msg {
            Ok(ws::Message::Ping(msg)) => {
                ctx.pong(&msg);
                debug!("Received ping, sent pong");
            },
            Ok(ws::Message::Text(text)) => {
                debug!("Received text message: {}", text);
            },
            Ok(ws::Message::Binary(bin)) => {
                debug!("Received binary message: {:?}", bin);
            },
            Ok(ws::Message::Close(reason)) => {
                info!("WebSocket closing: {:?}", reason);
                ctx.close(reason);
                ctx.stop();
            }
            _ => (),
        }
    }
}

#[inline]
pub fn convert_to_unit(amount: f64, token_decimal: u32) -> u64 {
    (amount * 10f64.powi(token_decimal as i32)).round() as u64
}

pub async fn fetch_price(token_mint: &str, token_decimal: u8) -> Result<f64, AnyhowError> {
    let base_url = env::var("JUPITER_API_URL").expect("JUPITER_API_URL must be set");
    let mut amount_in = convert_to_unit(1.0, token_decimal as u32);
    let mut total_increase = 1.0;

    loop {
        let url = format!(
            "{}?inputMint={}&outputMint={}&amount={}",
            base_url, token_mint, USDC_MINT, amount_in,
        );

        debug!("Fetching price from URL: {}", url);

        let response = reqwest::get(&url).await?;
        let status = response.status();
        let body = response.text().await?;

        if body.is_empty() {
            error!("Received empty response from API. Status: {}", status);
            return Err(AnyhowError::msg("Empty response from API"));
        }

        debug!("Received response: {}", body);

        let json_response: serde_json::Value = match serde_json::from_str(&body) {
            Ok(json) => json,
            Err(e) => {
                error!("Failed to parse JSON response: {}. Body: {}", e, body);
                return Err(AnyhowError::msg(format!("JSON parsing error: {}", e)));
            }
        };

        if let Some(error) = json_response.get("error") {
            let error_msg = error.as_str().unwrap_or("Unknown error");
            if error_msg.contains("Cannot compute other amount threshold") {
                amount_in *= 10;
                total_increase *= 10.0;
                debug!("Increasing amount and retrying. New amount: {}", amount_in);
                continue;
            } else {
                error!("API error: {}", error_msg);
                return Err(AnyhowError::msg(format!("API error: {}", error_msg)));
            }
        }

        let out_amount = json_response["outAmount"]
            .as_str()
            .ok_or_else(|| {
                error!("outAmount not found in response: {:?}", json_response);
                AnyhowError::msg("outAmount not found")
            })?
            .parse::<f64>()?;

        let price = out_amount / (1_000_000.0 * total_increase);
        debug!("Calculated price for {}: {}", token_mint, price);
        return Ok(price);
    }
}

pub async fn update_price_cache(app_state: web::Data<AppState>, token_mint: String, token_decimal: u8) {
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
                let price_info = PriceInfo {
                    price,
                    timestamp: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs(),
                };
                {
                    let mut cache = app_state.price_cache.write().await;
                    cache.insert(token_mint.clone(), price_info.clone());
                }
                {
                    let mut last_fetch = app_state.last_fetch.write().await;
                    last_fetch.insert(token_mint.clone(), Instant::now());
                }
                info!("Updated price for {}: {}", token_mint, price);
                debug!("New price available for {}: {:?}", token_mint, price_info);
            }
            Err(e) => error!(
                "Failed to fetch price for {} after retries: {}",
                token_mint, e
            ),
        }
    }
}

pub mod routes {
    use super::*;

    #[get("/ws/price")]
    pub async fn websocket(
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
    pub async fn health_check(app_state: web::Data<AppState>) -> impl Responder {
        let price_cache = app_state.price_cache.read().await;
        if price_cache.is_empty() {
            HttpResponse::ServiceUnavailable().json("No price data available")
        } else {
            HttpResponse::Ok().json("Healthy")
        }
    }
}