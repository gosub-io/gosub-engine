//! Gosub metrics CLI — fetches and displays timing stats from a running engine.
//!
//! Usage:
//!   cargo run --example metrics_cli                   # defaults: 127.0.0.1:9090
//!   cargo run --example metrics_cli -- --port 9091
//!   cargo run --example metrics_cli -- --reset        # clear counters
//!   cargo run --example metrics_cli -- --json         # raw JSON output
//!   cargo run --example metrics_cli -- --host 0.0.0.0 --port 9090
//!   cargo run --example metrics_cli -- --watch 2      # poll every 2 seconds

use std::collections::BTreeMap;
use std::time::Duration;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut host = "127.0.0.1".to_string();
    let mut port: u16 = 9090;
    let mut reset = false;
    let mut json_output = false;
    let mut watch_interval: Option<u64> = None;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--host" => {
                if let Some(v) = args.next() {
                    host = v;
                }
            }
            "--port" => {
                if let Some(v) = args.next() {
                    port = v.parse().unwrap_or(port);
                }
            }
            "--reset" => reset = true,
            "--json" => json_output = true,
            "--watch" => {
                watch_interval = args.next().and_then(|s| s.parse().ok()).or(Some(1));
            }
            _ => {}
        }
    }

    let base = format!("http://{host}:{port}");

    loop {
        let path = if reset { "/metrics/reset" } else { "/metrics" };
        let url = format!("{base}{path}");

        match fetch(&url).await {
            Ok(body) => {
                if json_output || reset {
                    println!("{body}");
                } else {
                    print_table(&body);
                }
            }
            Err(e) => {
                eprintln!("error: could not reach {url}: {e}");
                eprintln!("  Is the engine running with metrics::start({port}) called?");
            }
        }

        if reset {
            break;
        }

        match watch_interval {
            Some(secs) => {
                tokio::time::sleep(Duration::from_secs(secs)).await;
                // Clear screen for watch mode
                print!("\x1B[2J\x1B[1;1H");
            }
            None => break,
        }
    }

    Ok(())
}

async fn fetch(url: &str) -> anyhow::Result<String> {
    let resp = reqwest::get(url).await?.text().await?;
    Ok(resp)
}

fn print_table(json: &str) {
    let v: serde_json::Value = match serde_json::from_str(json) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("failed to parse response: {e}");
            eprintln!("{json}");
            return;
        }
    };

    let Some(namespaces) = v["namespaces"].as_object() else {
        println!("(no metrics recorded yet)");
        return;
    };

    if namespaces.is_empty() {
        println!("(no metrics recorded yet)");
        return;
    }

    // Sort alphabetically for stable output
    let sorted: BTreeMap<_, _> = namespaces.iter().collect();

    let hdr = format!(
        "{:<30} {:>8} {:>10} {:>10} {:>10} {:>10} {:>10} {:>10} {:>10} {:>10}",
        "Namespace", "Count", "Total", "Min", "Max", "Avg", "p50", "p75", "p95", "p99"
    );
    let sep = "-".repeat(hdr.len());
    println!("{hdr}");
    println!("{sep}");

    for (ns, stats) in &sorted {
        let count = stats["count"].as_u64().unwrap_or(0);
        let total = fmt_us(stats["total_us"].as_u64().unwrap_or(0));
        let min = fmt_us(stats["min_us"].as_u64().unwrap_or(0));
        let max = fmt_us(stats["max_us"].as_u64().unwrap_or(0));
        let avg = fmt_us(stats["avg_us"].as_u64().unwrap_or(0));
        let p50 = fmt_us(stats["p50_us"].as_u64().unwrap_or(0));
        let p75 = fmt_us(stats["p75_us"].as_u64().unwrap_or(0));
        let p95 = fmt_us(stats["p95_us"].as_u64().unwrap_or(0));
        let p99 = fmt_us(stats["p99_us"].as_u64().unwrap_or(0));

        println!(
            "{:<30} {:>8} {:>10} {:>10} {:>10} {:>10} {:>10} {:>10} {:>10} {:>10}",
            ns, count, total, min, max, avg, p50, p75, p95, p99
        );
    }
}

fn fmt_us(us: u64) -> String {
    if us < 1_000 {
        format!("{us}µs")
    } else if us < 1_000_000 {
        format!("{:.1}ms", us as f64 / 1_000.0)
    } else {
        format!("{:.2}s", us as f64 / 1_000_000.0)
    }
}
