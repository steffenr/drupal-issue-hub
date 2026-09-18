// Diagnostic probe: test the stale-keep-alive hypothesis.
// Client A pins pool_idle_timeout to 1h (keeps the server-closed connection),
// Client B uses a short pool idle timeout (the candidate fix).
//   cargo run --example probe --  (first request, then idle 3 min, then retry)
use std::error::Error;
use std::time::Duration;

fn chain(e: &dyn Error) -> String {
    let mut s = e.to_string();
    let mut src = e.source();
    while let Some(inner) = src {
        s.push_str(&format!("\n  caused by: {inner}"));
        src = inner.source();
    }
    s
}

fn main() {
    tauri::async_runtime::block_on(run());
}

async fn run() {
    let url = "https://www.drupal.org/api-d7/node.json?type=project_issue&field_project=2130961&sort=changed&direction=DESC&limit=100";

    let stale_pool = reqwest::Client::builder()
        .user_agent("drupal-issue-hub/0.1 (+https://www.drupal.org)")
        .pool_idle_timeout(Duration::from_secs(3600))
        .build()
        .unwrap();

    let fresh_pool = reqwest::Client::builder()
        .user_agent("drupal-issue-hub/0.1 (+https://www.drupal.org)")
        .pool_idle_timeout(Duration::from_secs(30))
        .build()
        .unwrap();

    println!("initial requests…");
    for (name, client) in [("stale-pool", &stale_pool), ("fresh-pool", &fresh_pool)] {
        match client.get(url).send().await {
            Ok(r) => println!("  [{name}] HTTP {}", r.status()),
            Err(e) => println!("  [{name}] FAILED: {}", chain(&e as &dyn Error)),
        }
    }

    println!("idling 180s to let the server close the keep-alive connection…");
    tokio_sleep(Duration::from_secs(180)).await;

    for (name, client) in [("stale-pool", &stale_pool), ("fresh-pool", &fresh_pool)] {
        match client.get(url).send().await {
            Ok(r) => println!("after idle [{name}]: HTTP {}", r.status()),
            Err(e) => println!("after idle [{name}]: FAILED: {}", chain(&e as &dyn Error)),
        }
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
