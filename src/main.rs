use std::path::PathBuf;
use std::time::Instant;
use tokio::sync::broadcast;
use tracing::{error, info};
use tracing_subscriber::fmt::time::OffsetTime;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use vine::backend::forwarding::ForwardingHelper;
use vine::config::{Config, ForwardingMode};
use vine::proxy::ProxyServer;

const USAGE: &str = "Usage: vine [--init] [CONFIG]

Starts the proxy with CONFIG (default: vine.toml), creating it if it does not exist.

Options:
  --init         Write the default files and exit without starting the proxy
  -h, --help     Print this help
  -V, --version  Print the version";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let started = Instant::now();

    let mut init_only = false;
    let mut config_path = None;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            "-V" | "--version" => {
                println!("Vine {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--init" => init_only = true,
            _ if arg.starts_with('-') || config_path.is_some() => {
                eprintln!("vine: unexpected argument '{arg}'\n\n{USAGE}");
                std::process::exit(2);
            }
            _ => config_path = Some(PathBuf::from(arg)),
        }
    }
    let config_path = config_path.unwrap_or_else(|| PathBuf::from("vine.toml"));

    // The local offset can only be read while the process has a single thread
    let offset = time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC);
    let timer = OffsetTime::new(
        offset,
        time::macros::format_description!("[hour]:[minute]:[second]"),
    );

    // Lines are written on a thread of their own, so a slow console never holds up the proxy;
    // the guard flushes what is left, so drop it before exiting
    let (log_writer, log_guard) = tracing_appender::non_blocking(std::io::stdout());
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(log_writer)
                .with_timer(timer)
                .with_target(false),
        )
        .init();

    // Exiting with the runtime still alive skips waiting on tasks that never finish
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let code = runtime.block_on(run(config_path, init_only, started));
    drop(log_guard);
    std::process::exit(code);
}

async fn run(config_path: PathBuf, init_only: bool, started: Instant) -> i32 {
    let created = !config_path.exists();
    info!("Loading configuration from '{}'...", config_path.display());
    let config = match Config::load_or_create(&config_path) {
        Ok(cfg) => cfg,
        Err(e) => {
            error!(
                "Failed to load configuration file '{}': {}",
                config_path.display(),
                e
            );
            return 1;
        }
    };
    if created {
        info!(
            "Created '{}' with example servers; point [servers] at your backends",
            config_path.display()
        );
    }

    let lang_dir = config_path
        .parent()
        .unwrap_or(std::path::Path::new(""))
        .join("lang");
    if let Err(e) = vine::i18n::init(&lang_dir, &config.server.default_locale) {
        error!(
            "Failed to load translations from '{}': {}",
            lang_dir.display(),
            e
        );
    }

    if init_only {
        info!(
            "Wrote the default files next to '{}'",
            config_path.display()
        );
        for (name, server) in &config.servers {
            if server.forwarding == ForwardingMode::Vine
                && let Some(key) = server
                    .secret
                    .as_deref()
                    .and_then(|secret| ForwardingHelper::get_vine_public_key_hex(secret).ok())
            {
                info!(
                    "Vine public key for '{name}': {key} (networking.proxy.vine.public_key on Pumpkin)"
                );
            }
        }
        return 0;
    }

    let server = match ProxyServer::new(config) {
        Ok(server) => server,
        Err(e) => {
            error!("Failed to start the proxy: {}", e);
            return 1;
        }
    };

    let (shutdown_tx, _) = broadcast::channel(1);
    let signal_tx = shutdown_tx.clone();

    tokio::spawn(async move {
        match shutdown_signal().await {
            Ok(signal) => {
                info!("Shutting down ({signal})");
                let _ = signal_tx.send(());
            }
            Err(err) => error!("Failed to listen for shutdown signal: {}", err),
        }
    });

    if let Err(e) = server.run(shutdown_tx, started).await {
        error!("Fatal proxy error: {}", e);
        return 1;
    }

    info!("Stopped");
    0
}

#[cfg(unix)]
async fn shutdown_signal() -> std::io::Result<&'static str> {
    use tokio::signal::unix::{SignalKind, signal};
    let mut terminate = signal(SignalKind::terminate())?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => result.map(|()| "Ctrl+C"),
        _ = terminate.recv() => Ok("SIGTERM"),
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() -> std::io::Result<&'static str> {
    tokio::signal::ctrl_c().await.map(|()| "Ctrl+C")
}
