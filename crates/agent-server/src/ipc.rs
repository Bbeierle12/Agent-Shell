//! Unix socket / TCP IPC server for daemon mode.
//!
//! Listens on a Unix domain socket (`~/.agent-shell/daemon.sock`) on
//! Linux/macOS, falling back to a TCP socket on Windows. Each connection
//! speaks a simple line-delimited JSON protocol: one JSON object per line,
//! one JSON response per line.
//!
//! Ported from ShellVault's `shellvault-daemon::server`.

use crate::ipc_handlers::handle_message;
use crate::state::AppState;
use anyhow::Result;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::watch;
use tracing::{debug, error, info};

/// Maximum allowed size of a single IPC message (64 KiB).
const MAX_MESSAGE_BYTES: usize = 64 * 1024;

/// Default socket path: `~/.agent-shell/daemon.sock`.
pub fn default_socket_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".agent-shell")
        .join("daemon.sock")
}

/// Run the IPC server.
///
/// On Unix, listens on a Unix domain socket. On Windows, falls back to TCP.
/// Shuts down when `shutdown_rx` receives `true`.
pub async fn run_ipc_server(
    socket_path: PathBuf,
    state: AppState,
    shutdown_rx: watch::Receiver<bool>,
) -> Result<()> {
    #[cfg(unix)]
    {
        run_unix_socket_server(socket_path, state, shutdown_rx).await
    }

    #[cfg(windows)]
    {
        let _ = socket_path; // unused on Windows
        run_tcp_server(state, shutdown_rx).await
    }
}

/// Unix domain socket server.
#[cfg(unix)]
async fn run_unix_socket_server(
    socket_path: PathBuf,
    state: AppState,
    mut shutdown_rx: watch::Receiver<bool>,
) -> Result<()> {
    use tokio::net::UnixListener;

    // Ensure parent directory exists.
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Remove stale socket file.
    let _ = std::fs::remove_file(&socket_path);

    let listener = UnixListener::bind(&socket_path)?;
    set_unix_socket_permissions(&socket_path);
    info!("IPC server listening on {:?}", socket_path);

    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    info!("IPC server shutdown requested");
                    break;
                }
            }
            accept_result = listener.accept() => {
                match accept_result {
                    Ok((stream, _)) => {
                        let state = state.clone();
                        tokio::spawn(async move {
                            if let Err(e) = handle_unix_connection(stream, state).await {
                                debug!("IPC connection error: {}", e);
                            }
                        });
                    }
                    Err(e) => {
                        error!("IPC accept error: {}", e);
                    }
                }
            }
        }
    }

    // Cleanup socket file on shutdown.
    let _ = std::fs::remove_file(&socket_path);
    info!("IPC server stopped");
    Ok(())
}

/// Handle a single Unix socket connection.
#[cfg(unix)]
async fn handle_unix_connection(
    stream: tokio::net::UnixStream,
    state: AppState,
) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();

    while reader.read_line(&mut line).await? > 0 {
        if line.len() > MAX_MESSAGE_BYTES {
            writer
                .write_all(b"{\"status\":\"error\",\"message\":\"Message too large\"}\n")
                .await?;
            break;
        }
        let response = handle_message(&line, &state).await;
        writer.write_all(response.as_bytes()).await?;
        writer.write_all(b"\n").await?;
        line.clear();
    }

    Ok(())
}

/// TCP fallback server (for Windows or environments without Unix sockets).
#[cfg(windows)]
async fn run_tcp_server(
    state: AppState,
    mut shutdown_rx: watch::Receiver<bool>,
) -> Result<()> {
    use tokio::net::TcpListener;

    let host = std::env::var("AGENT_SHELL_IPC_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port = std::env::var("AGENT_SHELL_IPC_PORT").unwrap_or_else(|_| "51842".to_string());
    let addr = format!("{}:{}", host, port);
    let listener = TcpListener::bind(&addr).await?;
    info!("IPC server listening on {}", listener.local_addr()?);

    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    info!("IPC server shutdown requested");
                    break;
                }
            }
            accept_result = listener.accept() => {
                match accept_result {
                    Ok((stream, _)) => {
                        let state = state.clone();
                        tokio::spawn(async move {
                            if let Err(e) = handle_tcp_connection(stream, state).await {
                                debug!("IPC connection error: {}", e);
                            }
                        });
                    }
                    Err(e) => {
                        error!("IPC accept error: {}", e);
                    }
                }
            }
        }
    }

    info!("IPC server stopped");
    Ok(())
}

/// Handle a single TCP connection.
#[cfg(windows)]
async fn handle_tcp_connection(
    stream: tokio::net::TcpStream,
    state: AppState,
) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();

    while reader.read_line(&mut line).await? > 0 {
        if line.len() > MAX_MESSAGE_BYTES {
            writer
                .write_all(b"{\"status\":\"error\",\"message\":\"Message too large\"}\n")
                .await?;
            break;
        }
        let response = handle_message(&line, &state).await;
        writer.write_all(response.as_bytes()).await?;
        writer.write_all(b"\n").await?;
        line.clear();
    }

    Ok(())
}

/// Set restrictive permissions on the Unix socket and its parent directory.
#[cfg(unix)]
fn set_unix_socket_permissions(path: &Path) {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    if let Some(parent) = path.parent() {
        let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
    }
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_socket_path() {
        let path = default_socket_path();
        let path_str = path.to_string_lossy();
        assert!(path_str.contains(".agent-shell"));
        assert!(path_str.ends_with("daemon.sock"));
    }

    #[test]
    fn test_max_message_bytes() {
        // Ensure the constant is reasonable.
        assert_eq!(MAX_MESSAGE_BYTES, 64 * 1024);
    }
}
