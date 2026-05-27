// VyomaOS host-side CLI — vyoma
//
// Connects to a running VyomaOS supervisor management server over NDJSON/TCP
// (port 9090) or QEMU Unix socket and provides: ps, logs, push, monitor, exec.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

mod connection;
mod protocol;
pub mod commands;

use connection::VyomaConnection;

/// vyoma — VyomaOS host-side developer CLI
#[derive(Parser)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    /// Supervisor hostname or IP (TCP transport)
    #[arg(long, default_value = "localhost", global = true)]
    pub host: String,

    /// Supervisor management port (TCP transport)
    #[arg(long, default_value_t = 9090, global = true)]
    pub port: u16,

    /// Unix socket path (overrides --host/--port when set)
    #[arg(long, env = "VYOMA_SOCKET", global = true)]
    pub socket: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// List all running apps with status, uptime, and restart count
    Ps,

    /// Stream live log output from a named app
    Logs {
        /// App name to stream logs from
        app: String,
    },

    /// Transfer a WASM binary to the running VM via OTA push
    Push {
        /// Path to the .wasm file to push
        path: PathBuf,

        /// Target app name (defaults to file stem)
        #[arg(long)]
        name: Option<String>,

        /// Expected SHA-256 hex digest for integrity check
        #[arg(long)]
        sha256: Option<String>,
    },

    /// Show a live crossterm dashboard of all app heartbeats
    Monitor,

    /// Send an IPC message to a running app and print the reply
    Exec {
        /// Target app name
        app: String,

        /// IPC message payload
        message: String,

        /// Timeout in seconds (default: 5)
        #[arg(long, default_value_t = 5)]
        timeout: u64,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let mut conn = if let Some(ref socket_path) = cli.socket {
        VyomaConnection::connect_socket(socket_path)?
    } else {
        VyomaConnection::connect(&cli.host, cli.port)?
    };

    match &cli.command {
        Commands::Ps => {
            commands::ps::run(&mut conn)?;
        }
        Commands::Logs { app } => {
            commands::logs::run(&mut conn, app)?;
        }
        Commands::Push { path, name, sha256 } => {
            let app_name = name.clone().unwrap_or_else(|| {
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown")
                    .to_string()
            });
            commands::push::run(&mut conn, path, &app_name, sha256.as_deref())?;
        }
        Commands::Monitor => {
            commands::monitor::run(&mut conn, &cli.host, cli.port)?;
        }
        Commands::Exec { app, message, timeout } => {
            commands::exec::run(&mut conn, app, message, *timeout)?;
        }
    }

    Ok(())
}
