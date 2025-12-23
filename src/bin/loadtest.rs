use reqwest::Client;
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

const SERVER_URL: &str = "http://localhost:3000/scrape";
const TOTAL_REQUESTS: usize = 100;
const CONCURRENT_LIMIT: usize = 50;

const TEST_URLS: &[&str] = &[
    "https://example.com",
    "https://httpbin.org/html",
    "https://www.rust-lang.org",
    "https://docs.rs",
    "https://crates.io",
];

#[derive(Debug, Default)]
struct Stats {
    success: AtomicUsize,
    failed: AtomicUsize,
    timeout: AtomicUsize,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let api_key = std::env::var("API_KEY").unwrap_or_else(|_| "changeme".to_string());

    println!("===========================================");
    println!("   Distill Load Test");
    println!("===========================================");
    println!("Target: {}", SERVER_URL);
    println!("Total requests: {}", TOTAL_REQUESTS);
    println!("Concurrent limit: {}", CONCURRENT_LIMIT);
    println!("API Key: {}...", &api_key[..api_key.len().min(8)]);
    println!("-------------------------------------------\n");

    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;

    print!("Checking server health... ");
    match client.get("http://localhost:3000/health").send().await {
        Ok(resp) if resp.status().is_success() => {
            println!("OK");
            if let Ok(body) = resp.text().await {
                println!("Server status: {}\n", body);
            }
        }
        Ok(resp) => {
            println!("FAILED (status: {})", resp.status());
            return Ok(());
        }
        Err(e) => {
            println!("FAILED");
            println!("Error: {}", e);
            println!("\nMake sure the server is running: cargo run");
            return Ok(());
        }
    }

    let stats = Arc::new(Stats::default());
    let semaphore = Arc::new(Semaphore::new(CONCURRENT_LIMIT));
    let start_time = Instant::now();

    println!("Starting load test...\n");

    let mut handles = Vec::with_capacity(TOTAL_REQUESTS);

    for i in 0..TOTAL_REQUESTS {
        let client = client.clone();
        let stats = stats.clone();
        let semaphore = semaphore.clone();
        let url = TEST_URLS[i % TEST_URLS.len()].to_string();
        let api_key = api_key.clone();

        let handle = tokio::spawn(async move {
            let _permit = semaphore.acquire().await.unwrap();
            let request_start = Instant::now();

            let payload = json!({
                "url": url,
                "output_format": "markdown"
            });

            let result = client
                .post(SERVER_URL)
                .header("x-api-key", &api_key)
                .json(&payload)
                .send()
                .await;

            let elapsed = request_start.elapsed();

            match result {
                Ok(resp) => {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();

                    if status.is_success() {
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) {
                            if json.get("success").and_then(|v| v.as_bool()) == Some(true) {
                                stats.success.fetch_add(1, Ordering::Relaxed);
                                println!(
                                    "[{:3}] OK      {:>6.2}s  {}",
                                    i + 1,
                                    elapsed.as_secs_f64(),
                                    url
                                );
                            } else {
                                let error_code = json
                                    .get("error")
                                    .and_then(|e| e.get("code"))
                                    .and_then(|c| c.as_str())
                                    .unwrap_or("UNKNOWN");
                                let error_msg = json
                                    .get("error")
                                    .and_then(|e| e.get("message"))
                                    .and_then(|m| m.as_str())
                                    .unwrap_or("Unknown error");

                                if error_code == "TIMEOUT_EXCEEDED" {
                                    stats.timeout.fetch_add(1, Ordering::Relaxed);
                                    println!(
                                        "[{:3}] TIMEOUT {:>6.2}s  {}",
                                        i + 1,
                                        elapsed.as_secs_f64(),
                                        url
                                    );
                                } else {
                                    stats.failed.fetch_add(1, Ordering::Relaxed);
                                    println!(
                                        "[{:3}] FAIL    {:>6.2}s  {}\n      └─ [{}] {}",
                                        i + 1,
                                        elapsed.as_secs_f64(),
                                        url,
                                        error_code,
                                        error_msg
                                    );
                                }
                            }
                        } else {
                            stats.success.fetch_add(1, Ordering::Relaxed);
                            println!(
                                "[{:3}] OK      {:>6.2}s  {}",
                                i + 1,
                                elapsed.as_secs_f64(),
                                url
                            );
                        }
                    } else {
                        let error_info = if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) {
                            let code = json
                                .get("error")
                                .and_then(|e| e.get("code"))
                                .and_then(|c| c.as_str())
                                .unwrap_or("UNKNOWN");
                            let msg = json
                                .get("error")
                                .and_then(|e| e.get("message"))
                                .and_then(|m| m.as_str())
                                .unwrap_or(&body);
                            (code.to_string(), msg.to_string())
                        } else {
                            ("HTTP_ERROR".to_string(), body.chars().take(100).collect())
                        };

                        if error_info.0 == "TIMEOUT_EXCEEDED" {
                            stats.timeout.fetch_add(1, Ordering::Relaxed);
                            println!(
                                "[{:3}] TIMEOUT {:>6.2}s  {}",
                                i + 1,
                                elapsed.as_secs_f64(),
                                url
                            );
                        } else {
                            stats.failed.fetch_add(1, Ordering::Relaxed);
                            println!(
                                "[{:3}] FAIL    {:>6.2}s  {} (HTTP {})\n      └─ [{}] {}",
                                i + 1,
                                elapsed.as_secs_f64(),
                                url,
                                status.as_u16(),
                                error_info.0,
                                error_info.1
                            );
                        }
                    }
                }
                Err(e) => {
                    let error_kind = if e.is_timeout() {
                        stats.timeout.fetch_add(1, Ordering::Relaxed);
                        "TIMEOUT"
                    } else if e.is_connect() {
                        stats.failed.fetch_add(1, Ordering::Relaxed);
                        "CONNECTION"
                    } else if e.is_request() {
                        stats.failed.fetch_add(1, Ordering::Relaxed);
                        "REQUEST"
                    } else {
                        stats.failed.fetch_add(1, Ordering::Relaxed);
                        "NETWORK"
                    };

                    println!(
                        "[{:3}] {}  {:>6.2}s  {}\n      └─ {}",
                        i + 1,
                        format!("{:<7}", error_kind),
                        elapsed.as_secs_f64(),
                        url,
                        e
                    );
                }
            }
        });

        handles.push(handle);
    }

    for handle in handles {
        let _ = handle.await;
    }

    let total_elapsed = start_time.elapsed();

    let success = stats.success.load(Ordering::Relaxed);
    let failed = stats.failed.load(Ordering::Relaxed);
    let timeout = stats.timeout.load(Ordering::Relaxed);

    println!("\n===========================================");
    println!("   Results");
    println!("===========================================");
    println!("Total time:     {:.2}s", total_elapsed.as_secs_f64());
    println!("Requests/sec:   {:.2}", TOTAL_REQUESTS as f64 / total_elapsed.as_secs_f64());
    println!("-------------------------------------------");
    println!("Success:        {} ({:.1}%)", success, success as f64 / TOTAL_REQUESTS as f64 * 100.0);
    println!("Failed:         {} ({:.1}%)", failed, failed as f64 / TOTAL_REQUESTS as f64 * 100.0);
    println!("Timeout:        {} ({:.1}%)", timeout, timeout as f64 / TOTAL_REQUESTS as f64 * 100.0);
    println!("===========================================");

    Ok(())
}
