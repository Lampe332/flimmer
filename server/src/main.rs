//! Standalone Flimmer server for browser access. It has no stream sniffer; that only exists
//! inside the desktop app (`desktop/`).

use std::{path::PathBuf, process::ExitCode};

struct Args {
    root: PathBuf,
    home: Option<PathBuf>,
    port: u16,
    open: bool,
    dev: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut root = None;
    let mut home = None;
    let mut port = flimmer::DEFAULT_PORT;
    let mut open = false;
    let mut dev = false;
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--root" => root = Some(PathBuf::from(it.next().ok_or("--root needs a path")?)),
            "--data" => home = Some(PathBuf::from(it.next().ok_or("--data needs a path")?)),
            "--port" => {
                port = it
                    .next()
                    .ok_or("--port needs a number")?
                    .parse()
                    .map_err(|_| "--port must be a number between 1 and 65535")?;
            }
            "--open" => open = true,
            "--dev" => dev = true,
            "-h" | "--help" => {
                println!("Usage: flimmer-server [--root <path>] [--data <path>] [--port <port>] [--open] [--dev]");
                println!("  --open  opens the UI in the browser");
                println!("  --dev   also accepts the Vite dev server (localhost:5173) as host and origin");
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    let root = match root {
        Some(r) => r,
        None => std::env::current_dir().map_err(|e| format!("cannot read current directory: {e}"))?,
    };
    Ok(Args { root, home, port, open, dev })
}

fn open_browser(url: &str) {
    #[cfg(windows)]
    let res = std::process::Command::new("cmd").args(["/C", "start", "", url]).spawn();
    #[cfg(target_os = "macos")]
    let res = std::process::Command::new("open").arg(url).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let res = std::process::Command::new("xdg-open").arg(url).spawn();
    if let Err(e) = res {
        eprintln!("could not open browser: {e}");
    }
}

async fn run() -> Result<(), String> {
    let args = parse_args()?;
    // Without --data the data folder is `<root>/data`, as before.
    let home = args.home.unwrap_or_else(|| args.root.clone());
    let server = flimmer::bind(flimmer::Options { root: args.root, home, port: args.port, sniffer: None, dev: args.dev }).await?;
    println!("Flimmer running at {}", server.url);
    // The API only answers a page that came in through a start link (or a paired device).
    if args.open {
        open_browser(&server.entry_url(std::time::Duration::from_secs(60)));
    } else {
        println!("Open this link within 10 minutes (it works once): {}", server.entry_url(std::time::Duration::from_secs(600)));
    }
    server
        .serve(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("flimmer: {e}");
            ExitCode::FAILURE
        }
    }
}
