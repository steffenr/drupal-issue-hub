use crate::drupal::err_chain;
use std::time::Duration;

/// Send a request, retrying once after a short pause on transport-level
/// failures ("error sending request"). These are typically stale pooled
/// keep-alive connections or transient network-state changes (e.g. right
/// after a system sleep/wake); both CDN hosts (drupal.org and
/// git.drupalcode.org sit on the same Fastly edge) close idle connections
/// aggressively, so an immediate retry on a fresh connection succeeds.
pub async fn send_retry(
    builder: reqwest::RequestBuilder,
) -> Result<reqwest::Response, String> {
    match builder.try_clone() {
        Some(clonable) => match builder.send().await {
            Ok(resp) => Ok(resp),
            Err(first) => {
                tokio_sleep(Duration::from_secs(1)).await;
                clonable.send().await.map_err(|second| {
                    format!(
                        "request failed after retry: {} first attempt: {}",
                        err_chain(&second),
                        err_chain(&first)
                    )
                })
            }
        },
        None => builder
            .send()
            .await
            .map_err(|e| format!("request failed: {}", err_chain(&e))),
    }
}

async fn tokio_sleep(d: Duration) {
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    std::thread::spawn(move || {
        std::thread::sleep(d);
        let _ = tx.send(());
    });
    let _ = tauri::async_runtime::spawn_blocking(move || rx.recv()).await;
}
