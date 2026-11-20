//! Standalone Flimmer server for browser access. It has no stream sniffer; that only exists
//! inside the desktop app (`desktop/`).

use std::{path::PathBuf, process::ExitCode};

struct Args {
    root: PathBuf,
    home: Option<PathBuf>,
    port: u16,
    open: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut root = None;
    let mut home = None;
    let mut port = flimmer::DEFAULT_PORT;
    let mut open = false;
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
            "-h" | "--help" => {
                println!("Usage: flimmer-server [--root <path>] [--data <path>] [--port <port>] [--open]");
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    let root = match root {
        Some(r) => r,
        None => std::env::current_dir().map_err(|e| format!("cannot read current directory: {e}"))?,
    };
    Ok(Args { root, home, port, open })
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
    let server = flimmer::bind(flimmer::Options { root: args.root, home, port: args.port, sniffer: None }).await?;
    println!("Flimmer running at {}", server.url);
    if args.open {
        open_browser(&server.url);
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
