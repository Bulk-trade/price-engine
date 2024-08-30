use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use tokio::runtime::Runtime;
use bulk_price_engine::{AppState, PriceInfo, fetch_price, update_price_cache, WsSession, TextSender};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use std::collections::HashMap;
use actix_web::web;
use std::time::Duration;
use futures::future;
use tokio::time::sleep;

fn fetch_price_benchmark(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    
    c.bench_function("fetch_price", |b| {
        b.iter(|| {
            rt.block_on(async {
                black_box(fetch_price("So11111111111111111111111111111111111111112", 9).await)
            })
        })
    });
}

fn update_price_cache_benchmark(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    
    let app_state = web::Data::new(AppState {
        price_cache: Arc::new(RwLock::new(HashMap::new())),
        last_fetch: Arc::new(RwLock::new(HashMap::new())),
        update_tasks: Arc::new(Mutex::new(HashMap::new())),
        active_connections: Arc::new(RwLock::new(HashMap::new())),
    });

    c.bench_function("update_price_cache", |b| {
        b.iter(|| {
            rt.block_on(async {
                black_box(update_price_cache(
                    app_state.clone(),
                    "So11111111111111111111111111111111111111112".to_string(),
                    9
                ).await)
            })
        })
    });
}

fn concurrent_price_updates_parameterized(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("concurrent_price_updates");
    
    for concurrent_updates in [5, 10, 20, 50, 100].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(concurrent_updates), concurrent_updates, |b, &n| {
            let app_state = web::Data::new(AppState {
                price_cache: Arc::new(RwLock::new(HashMap::new())),
                last_fetch: Arc::new(RwLock::new(HashMap::new())),
                update_tasks: Arc::new(Mutex::new(HashMap::new())),
                active_connections: Arc::new(RwLock::new(HashMap::new())),
            });

            b.iter(|| {
                rt.block_on(async {
                    let handles: Vec<_> = (0..n).map(|_| {
                        let state_clone = app_state.clone();
                        tokio::spawn(async move {
                            update_price_cache(
                                state_clone,
                                "So11111111111111111111111111111111111111112".to_string(),
                                9
                            ).await
                        })
                    }).collect();
                    future::join_all(handles).await;
                })
            })
        });
    }
    group.finish();
}

fn multi_token_price_fetch_benchmark(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let tokens = [
        ("So11111111111111111111111111111111111111112", 9), // SOL
        ("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v", 6), // USDC
        ("Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB", 6), // USDT
        ("mSoLzYCxHdYgdzU16g5QSh3i5K3z3KZK7ytfqcJm7So", 9), // mSOL
    ];

    let mut group = c.benchmark_group("multi_token_price_fetch");
    for (token, decimals) in tokens.iter() {
        group.bench_with_input(BenchmarkId::from_parameter(token), token, |b, &token| {
            b.iter(|| {
                rt.block_on(async {
                    black_box(fetch_price(token, *decimals).await)
                })
            })
        });
    }
    group.finish();
}

// Mock context for WebSocket benchmarking
struct MockContext;

impl TextSender for MockContext {
    fn text(&mut self, _: String) {
        // Simulate sending text, do nothing in the mock
    }
}

fn websocket_connection_benchmark(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    
    let app_state = web::Data::new(AppState {
        price_cache: Arc::new(RwLock::new(HashMap::new())),
        last_fetch: Arc::new(RwLock::new(HashMap::new())),
        update_tasks: Arc::new(Mutex::new(HashMap::new())),
        active_connections: Arc::new(RwLock::new(HashMap::new())),
    });

    rt.block_on(async {
        app_state.price_cache.write().await.insert(
            "So11111111111111111111111111111111111111112".to_string(),
            PriceInfo {
                price: 1.0,
                timestamp: 12345,
            }
        );
    });

    c.bench_function("websocket_connection", |b| {
        b.iter(|| {
            rt.block_on(async {
                let ws_session = WsSession {
                    app_state: app_state.clone(),
                    token_mint: "So11111111111111111111111111111111111111112".to_string(),
                    token_decimal: 9,
                };
                
                // Simulate WebSocket session lifecycle
                // Instead of calling ensure_price_updates, we'll simulate its effect
                {
                    let mut update_tasks = ws_session.app_state.update_tasks.lock().await;
                    if !update_tasks.contains_key(&ws_session.token_mint) {
                        let task = tokio::spawn(update_price_cache(
                            ws_session.app_state.clone(),
                            ws_session.token_mint.clone(),
                            ws_session.token_decimal,
                        ));
                        update_tasks.insert(ws_session.token_mint.clone(), task);
                    }
                }

                // Simulate increment_connection_count
                {
                    let mut connections = ws_session.app_state.active_connections.write().await;
                    *connections.entry(ws_session.token_mint.clone()).or_insert(0) += 1;
                }

                // Simulate a few price updates
                for _ in 0..5 {
                    let mut mock_ctx = MockContext;
                    ws_session.stream_price(&mut mock_ctx);
                    sleep(Duration::from_millis(100)).await;
                }
                
                // Simulate decrement_connection_count
                {
                    let mut connections = ws_session.app_state.active_connections.write().await;
                    if let Some(count) = connections.get_mut(&ws_session.token_mint) {
                        *count -= 1;
                        if *count == 0 {
                            connections.remove(&ws_session.token_mint);
                            let mut update_tasks = ws_session.app_state.update_tasks.lock().await;
                            if let Some(task) = update_tasks.remove(&ws_session.token_mint) {
                                task.abort();
                            }
                            ws_session.app_state.price_cache.write().await.remove(&ws_session.token_mint);
                            ws_session.app_state.last_fetch.write().await.remove(&ws_session.token_mint);
                        }
                    }
                }
            })
        })
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default().measurement_time(Duration::from_secs(10));
    targets = fetch_price_benchmark, update_price_cache_benchmark, 
              concurrent_price_updates_parameterized, multi_token_price_fetch_benchmark, 
              websocket_connection_benchmark
}
criterion_main!(benches);