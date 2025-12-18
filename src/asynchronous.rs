//! # asynchronous.rs
//!
//! This module provides asynchronous versions of the SSH connection classes.
//! It uses the `russh` library to provide async Python-friendly interfaces for SSH operations.
//!
//! ## Classes
//!
//! ### AsyncConnection
//! An asynchronous class that represents an SSH connection. It provides methods for executing commands asynchronously, reading and writing files over SFTP, and creating an interactive shell.
//!
//! ### AsyncInteractiveShell
//! An asynchronous class that represents an interactive shell over an SSH connection. It includes methods for sending commands and reading the output.
//!
//! ### AsyncFileTailer
//! An asynchronous class for tailing remote files, allowing reading from a specified position in a file.
//!
//! ## Usage
//!
//! To use this module, create an `AsyncConnection` instance with the necessary connection details. Then, use the async methods on the `AsyncConnection` instance to perform SSH operations.
//!
//! ```python
//! import asyncio
//! from hussh.aio import AsyncConnection
//!
//! async def main():
//!     async with AsyncConnection("my.test.server", username="user", password="pass") as conn:
//!         result = await conn.execute("ls")
//!         print(result.stdout)
//!
//! asyncio.run(main())
//! ```
//!
//! Multiple forms of authentication are supported. You can use a password, a private key, or default SSH key files (ssh-agent is not supported in async connections).
//!
//! ```python
//! async with AsyncConnection("my.test.server", username="user", key_path="~/.ssh/id_rsa") as conn:
//!     result = await conn.execute("ls")
//!
//! async with AsyncConnection("my.test.server", username="user", password="pass") as conn:
//!     result = await conn.execute("ls")
//!
//! async with AsyncConnection("my.test.server", username="user") as conn:  # Uses default SSH keys
//!     result = await conn.execute("ls")
//! ```
//!
//! When no password or key_path is specified, the connection will attempt authentication using default SSH key files:
//! - ~/.ssh/id_rsa (RSA keys, most common)
//! - ~/.ssh/id_ed25519 (Ed25519 keys, modern and secure)
//! - ~/.ssh/id_ecdsa (ECDSA keys)
//! - ~/.ssh/id_dsa (DSA keys, legacy)
//!
//! To use the interactive shell, it is recommended to use the shell() context manager from the AsyncConnection class.
//! You can send commands to the shell using the `send` method, then get the results from the `read` method.
//!
//! ```python
//! async with conn.shell() as shell:
//!     await shell.send("ls")
//!     await shell.send("pwd")
//!     await shell.send("whoami")
//!
//! result = await shell.read()
//! print(result.stdout)
//! ```
//!
//! For SFTP operations:
//!
//! ```python
//! # Write a local file to the remote server
//! await conn.sftp_write("/path/to/local/file", "/remote/path/file")
//!
//! # Write string data directly to a remote file
//! await conn.sftp_write_data("Hello there!", "/remote/path/file")
//!
//! # Read a remote file to a local file
//! await conn.sftp_read("/remote/path/file", "/local/path/file")
//!
//! # Read a remote file's contents as a string
//! contents = await conn.sftp_read("/remote/path/file")
//!
//! # List directory contents
//! files = await conn.sftp_list("/remote/path")
//! ```
//!
//! For file tailing:
//!
//! ```python
//! async with conn.tail("remote_file.log") as tailer:
//!     await asyncio.sleep(5)  # wait or perform other operations
//!     content = await tailer.read()
//!     print(content)
//!
//! print(tailer.contents)
//! ```

use crate::connection::SSHResult;
use pyo3::exceptions::{PyRuntimeError, PyTimeoutError};
use pyo3::prelude::*;
use russh::client::{Config, Handle, Handler};
use russh::keys::{HashAlg, PrivateKeyWithHashAlg};
use russh_sftp::client::SftpSession;
use std::path::Path;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;

/// Create a PrivateKeyWithHashAlg with the appropriate hash algorithm for the key type.
/// For RSA keys, we use SHA-256 for modern server compatibility.
/// For other key types (Ed25519, ECDSA), the hash algorithm is determined by the key type.
fn create_key_with_hash(key: russh::keys::PrivateKey) -> PrivateKeyWithHashAlg {
    let hash_alg = if key.algorithm().is_rsa() {
        Some(HashAlg::Sha256)
    } else {
        None
    };
    PrivateKeyWithHashAlg::new(Arc::new(key), hash_alg)
}

/// Helper function to try default SSH key files
/// Note: password parameter is reserved for future support of password-protected default keys
async fn try_default_keys(
    session: &mut russh::client::Handle<ClientHandler>,
    username: &str,
    password: Option<&str>,
) -> Result<bool, russh::Error> {
    let default_keys = [
        "~/.ssh/id_rsa",     // RSA keys (most common)
        "~/.ssh/id_ed25519", // Ed25519 keys (modern, secure)
        "~/.ssh/id_ecdsa",   // ECDSA keys
        "~/.ssh/id_dsa",     // DSA keys (legacy, deprecated but included for compatibility)
    ];

    for key_path in &default_keys {
        let expanded_key_path = shellexpand::tilde(key_path).into_owned();
        let path = Path::new(&expanded_key_path);

        if path.exists() {
            // Try to load and use this key
            if let Ok(key_pair) = russh::keys::load_secret_key(path, password) {
                let key_with_hash = create_key_with_hash(key_pair);
                match session
                    .authenticate_publickey(username, key_with_hash)
                    .await
                {
                    Ok(auth_result) if auth_result.success() => return Ok(true),
                    // Authentication failed with this key, try next one
                    Ok(_) | Err(_) => continue,
                }
            }
        }
    }
    Ok(false) // No default keys worked
}

#[derive(Clone)]
struct ClientHandler;

impl From<(russh::ChannelId, russh::ChannelMsg)> for ClientHandler {
    fn from(_: (russh::ChannelId, russh::ChannelMsg)) -> Self {
        ClientHandler
    }
}

impl Handler for ClientHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _server_public_key: &russh::keys::PublicKey,
    ) -> Result<bool, Self::Error> {
        // For now, we blindly accept keys (MVP).
        Ok(true)
    }
}

/// # AsyncConnection
///
/// `AsyncConnection` is an asynchronous class that represents an SSH connection. It provides methods for executing commands asynchronously, reading and writing files over SFTP, and creating an interactive shell.
///
/// ## Attributes
///
/// * `host`: The host to connect to.
/// * `port`: The port to connect to.
/// * `username`: The username to use for authentication.
/// * `password`: The password to use for authentication.
/// * `key_path`: The path to the private key to use for authentication.
/// * `timeout`: The timeout (in seconds) for the SSH session.
/// * `keepalive_interval`: The interval (in seconds) for sending keepalive messages.
///
/// ## Methods
///
/// ### `execute`
///
/// Executes a command over the SSH connection asynchronously and returns the result. It takes the following parameters:
///
/// * `command`: The command to execute.
/// * `timeout`: Optional timeout for the command execution.
///
/// ### `sftp_read`
///
/// Reads a file over SFTP and returns the contents. It takes the following parameters:
///
/// * `remote_path`: The path to the file on the remote system.
/// * `local_path`: The path to save the file on the local system. If not provided, the contents of the file are returned.
///
/// ### `sftp_write`
///
/// Writes a file over SFTP. It takes the following parameters:
///
/// * `local_path`: The path to the file on the local system.
/// * `remote_path`: The path to save the file on the remote system. If not provided, the local path is used.
///
/// ### `sftp_write_data`
///
/// Writes data over SFTP. It takes the following parameters:
///
/// * `data`: The data to write.
/// * `remote_path`: The path to save the data on the remote system.
///
/// ### `sftp_list`
///
/// Lists the contents of a remote directory. It takes the following parameter:
///
/// * `path`: The path to the remote directory to list.
///
/// ### `shell`
///
/// Creates an `AsyncInteractiveShell` instance. It takes the following parameter:
///
/// * `pty`: Whether to request a pseudo-terminal for the shell.
///
/// ### `tail`
///
/// Creates an `AsyncFileTailer` instance for tailing a remote file. It takes the following parameter:
///
/// * `remote_file`: The path to the remote file to tail.
///
/// ### `connect`
///
/// Explicitly connects to the SSH server. Usually not needed as connection happens automatically in context managers.
///
/// ### `close`
///
/// Closes the SSH connection.
///
#[pyclass]
#[derive(Clone)]
pub struct AsyncConnection {
    host: String,
    port: u16,
    username: Option<String>,
    password: Option<String>,
    key_path: Option<String>,
    // Use Arc<Mutex<>> to allow updating the session from the async block
    // Wrap Handle in Arc because it might not be Clone
    session: Arc<Mutex<Option<Arc<Handle<ClientHandler>>>>>,
    // Lazily initialized SFTP session, cached for reuse
    sftp_session: Arc<Mutex<Option<Arc<SftpSession>>>>,
    config: Arc<Config>,
    timeout: u64,
}

// Private helper methods for AsyncConnection
impl AsyncConnection {
    /// Lazily initializes and returns the SFTP session, caching it for reuse
    async fn get_or_init_sftp(
        session_arc: Arc<Mutex<Option<Arc<Handle<ClientHandler>>>>>,
        sftp_arc: Arc<Mutex<Option<Arc<SftpSession>>>>,
    ) -> PyResult<Arc<SftpSession>> {
        // Check if we already have an SFTP session
        {
            let sftp_guard = sftp_arc.lock().await;
            if let Some(sftp) = sftp_guard.as_ref() {
                return Ok(sftp.clone());
            }
        }

        // Need to create a new SFTP session
        let session_guard = session_arc.lock().await;
        let session = session_guard
            .as_ref()
            .ok_or_else(|| PyRuntimeError::new_err("Not connected"))?
            .clone();
        drop(session_guard);

        let channel = session
            .channel_open_session()
            .await
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to open channel: {}", e)))?;

        channel.request_subsystem(true, "sftp").await.map_err(|e| {
            PyRuntimeError::new_err(format!("Failed to request SFTP subsystem: {}", e))
        })?;

        let sftp = SftpSession::new(channel.into_stream()).await.map_err(|e| {
            PyRuntimeError::new_err(format!("Failed to create SFTP session: {}", e))
        })?;

        let sftp = Arc::new(sftp);

        // Cache the SFTP session
        {
            let mut sftp_guard = sftp_arc.lock().await;
            *sftp_guard = Some(sftp.clone());
        }

        Ok(sftp)
    }
}

#[pymethods]
impl AsyncConnection {
    #[new]
    #[pyo3(signature = (host, username=None, password=None, key_path=None, port=22, keepalive_interval=0, timeout=0))]
    fn new(
        host: String,
        username: Option<String>,
        password: Option<String>,
        key_path: Option<String>,
        port: u16,
        keepalive_interval: u64,
        timeout: u64,
    ) -> Self {
        let mut config = Config::default();
        if keepalive_interval > 0 {
            config.keepalive_interval = Some(std::time::Duration::from_secs(keepalive_interval));
        }

        AsyncConnection {
            host,
            port,
            username,
            password,
            key_path,
            session: Arc::new(Mutex::new(None)),
            sftp_session: Arc::new(Mutex::new(None)),
            config: Arc::new(config),
            timeout,
        }
    }

    #[pyo3(signature = (timeout=None))]
    fn connect<'p>(&self, py: Python<'p>, timeout: Option<u64>) -> PyResult<Bound<'p, PyAny>> {
        let config = self.config.clone();
        let host = self.host.clone();
        let port = self.port;
        let username = self.username.clone().unwrap_or("".to_string());
        let password = self.password.clone();
        let key_path = self.key_path.clone();
        let session_arc = self.session.clone();
        let timeout = timeout.unwrap_or(self.timeout);

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let run = async {
                let handler = ClientHandler {};
                let mut session = russh::client::connect(config, (host.as_str(), port), handler)
                    .await
                    .map_err(|e| PyRuntimeError::new_err(format!("Connection failed: {}", e)))?;

                // Authentication
                let auth_res = if let Some(key_p) = key_path.clone() {
                    let key_p = shellexpand::full(&key_p)
                        .map(|p| p.into_owned())
                        .unwrap_or(key_p);
                    let key_path = Path::new(&key_p);
                    let key_pair = russh::keys::load_secret_key(key_path, password.as_deref())
                        .map_err(|e| {
                            PyRuntimeError::new_err(format!("Failed to load key: {}", e))
                        })?;
                    let key_with_hash = create_key_with_hash(key_pair);
                    session
                        .authenticate_publickey(&username, key_with_hash)
                        .await
                        .map(|r| r.success())
                } else if let Some(pwd) = password.clone() {
                    session
                        .authenticate_password(&username, pwd)
                        .await
                        .map(|r| r.success())
                } else {
                    // If no authentication method provided, try default SSH key files
                    try_default_keys(&mut session, &username, None).await
                };

                match auth_res {
                    Ok(true) => {
                        // Authentication succeeded
                    }
                    Ok(false) => {
                        return Err(PyRuntimeError::new_err(
                            if key_path.is_some() || password.is_some() {
                                "Authentication failed"
                            } else {
                                "Failed to authenticate with default SSH keys"
                            },
                        ));
                    }
                    Err(e) => {
                        return Err(PyRuntimeError::new_err(format!(
                            "Authentication failed: {}",
                            e
                        )));
                    }
                }

                let mut guard = session_arc.lock().await;
                *guard = Some(Arc::new(session));

                Ok(())
            };

            if timeout > 0 {
                match tokio::time::timeout(std::time::Duration::from_secs(timeout), run).await {
                    Ok(res) => res,
                    Err(_) => Err(PyTimeoutError::new_err("Connection timed out")),
                }
            } else {
                run.await
            }
        })
    }

    #[pyo3(signature = (command, timeout=None))]
    fn execute<'p>(
        &self,
        py: Python<'p>,
        command: String,
        timeout: Option<u64>,
    ) -> PyResult<Bound<'p, PyAny>> {
        let session_arc = self.session.clone();
        let timeout = timeout.unwrap_or(0);

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let run = async {
                let guard = session_arc.lock().await;
                let session = guard
                    .as_ref()
                    .ok_or_else(|| PyRuntimeError::new_err("Not connected"))?;

                // Clone the Arc<Handle>
                let session = session.clone();
                drop(guard);

                let mut channel = session
                    .channel_open_session()
                    .await
                    .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;

                channel
                    .exec(true, command)
                    .await
                    .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;

                let mut stdout = Vec::new();
                let mut stderr = Vec::new();
                let mut exit_code = 0;

                while let Some(msg) = channel.wait().await {
                    match msg {
                        russh::ChannelMsg::Data { ref data } => {
                            stdout.extend_from_slice(data);
                        }
                        russh::ChannelMsg::ExtendedData { ref data, ext } => {
                            if ext == 1 {
                                stderr.extend_from_slice(data);
                            }
                        }
                        russh::ChannelMsg::ExitStatus { exit_status } => {
                            exit_code = exit_status;
                        }
                        _ => {}
                    }
                }

                let stdout_str = String::from_utf8_lossy(&stdout).to_string();
                let stderr_str = String::from_utf8_lossy(&stderr).to_string();

                Ok(SSHResult {
                    stdout: stdout_str,
                    stderr: stderr_str,
                    status: exit_code as i32,
                })
            };

            if timeout > 0 {
                match tokio::time::timeout(std::time::Duration::from_secs(timeout), run).await {
                    Ok(res) => res,
                    Err(_) => Err(PyTimeoutError::new_err("Command timed out")),
                }
            } else {
                run.await
            }
        })
    }

    fn close<'p>(&self, py: Python<'p>) -> PyResult<Bound<'p, PyAny>> {
        let session_arc = self.session.clone();
        let sftp_arc = self.sftp_session.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut sftp_guard = sftp_arc.lock().await;
            *sftp_guard = None;
            let mut guard = session_arc.lock().await;
            *guard = None;
            Ok(())
        })
    }

    fn __aenter__<'p>(slf: PyRef<'p, Self>, py: Python<'p>) -> PyResult<Bound<'p, PyAny>> {
        let config = slf.config.clone();
        let host = slf.host.clone();
        let port = slf.port;
        let username = slf.username.clone().unwrap_or("".to_string());
        let password = slf.password.clone();
        let key_path = slf.key_path.clone();
        let session_arc = slf.session.clone();
        let timeout = slf.timeout;

        let py_self = Bound::new(py, (*slf).clone())?.into_any().unbind();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let run = async {
                let handler = ClientHandler {};
                let mut session = russh::client::connect(config, (host.as_str(), port), handler)
                    .await
                    .map_err(|e| PyRuntimeError::new_err(format!("Connection failed: {}", e)))?;

                // Auth ...
                let auth_res = if let Some(key_p) = key_path.clone() {
                    let key_p = shellexpand::full(&key_p)
                        .map(|p| p.into_owned())
                        .unwrap_or(key_p);
                    let key_path = Path::new(&key_p);
                    let key_pair = russh::keys::load_secret_key(key_path, password.as_deref())
                        .map_err(|e| {
                            PyRuntimeError::new_err(format!("Failed to load key: {}", e))
                        })?;
                    let key_with_hash = create_key_with_hash(key_pair);
                    session
                        .authenticate_publickey(&username, key_with_hash)
                        .await
                        .map(|r| r.success())
                } else if let Some(pwd) = password.clone() {
                    session
                        .authenticate_password(&username, pwd)
                        .await
                        .map(|r| r.success())
                } else {
                    // If no authentication method provided, try default SSH key files
                    try_default_keys(&mut session, &username, None).await
                };

                match auth_res {
                    Ok(true) => {
                        // Authentication succeeded
                    }
                    Ok(false) => {
                        return Err(PyRuntimeError::new_err(
                            if key_path.is_some() || password.is_some() {
                                "Authentication failed"
                            } else {
                                "Failed to authenticate with default SSH keys"
                            },
                        ));
                    }
                    Err(e) => {
                        return Err(PyRuntimeError::new_err(format!(
                            "Authentication failed: {}",
                            e
                        )));
                    }
                }

                let mut guard = session_arc.lock().await;
                *guard = Some(Arc::new(session));

                Ok(py_self)
            };

            if timeout > 0 {
                match tokio::time::timeout(std::time::Duration::from_secs(timeout), run).await {
                    Ok(res) => res,
                    Err(_) => Err(PyTimeoutError::new_err("Connection timed out")),
                }
            } else {
                run.await
            }
        })
    }

    fn __aexit__<'p>(
        &self,
        py: Python<'p>,
        _exc_type: Option<Bound<'p, PyAny>>,
        _exc_value: Option<Bound<'p, PyAny>>,
        _traceback: Option<Bound<'p, PyAny>>,
    ) -> PyResult<Bound<'p, PyAny>> {
        self.close(py)
    }

    /// Reads a file over SFTP and returns the contents.
    /// If `local_path` is provided, the file is saved to the local system.
    /// Otherwise, the contents of the file are returned as a string.
    #[pyo3(signature = (remote_path, local_path=None))]
    fn sftp_read<'p>(
        &self,
        py: Python<'p>,
        remote_path: String,
        local_path: Option<String>,
    ) -> PyResult<Bound<'p, PyAny>> {
        let session_arc = self.session.clone();
        let sftp_arc = self.sftp_session.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let sftp = AsyncConnection::get_or_init_sftp(session_arc, sftp_arc).await?;

            let mut remote_file = sftp.open(&remote_path).await.map_err(|e| {
                PyRuntimeError::new_err(format!("Failed to open remote file: {}", e))
            })?;

            if let Some(local_p) = local_path {
                // Save to local file
                let mut local_file = tokio::fs::File::create(&local_p).await.map_err(|e| {
                    PyRuntimeError::new_err(format!("Failed to create local file: {}", e))
                })?;

                let mut buffer = vec![0u8; 65536]; // 64KB buffer to match sync version
                loop {
                    let n = remote_file.read(&mut buffer).await.map_err(|e| {
                        PyRuntimeError::new_err(format!("Failed to read remote file: {}", e))
                    })?;
                    if n == 0 {
                        break;
                    }
                    local_file.write_all(&buffer[..n]).await.map_err(|e| {
                        PyRuntimeError::new_err(format!("Failed to write local file: {}", e))
                    })?;
                }
                Ok("Ok".to_string())
            } else {
                // Return contents as string
                let mut buffer = Vec::new();
                remote_file.read_to_end(&mut buffer).await.map_err(|e| {
                    PyRuntimeError::new_err(format!("Failed to read remote file: {}", e))
                })?;
                Ok(String::from_utf8_lossy(&buffer).to_string())
            }
        })
    }

    /// Writes a file over SFTP.
    /// If `remote_path` is not provided, the local file is written to the same path on the remote system.
    #[pyo3(signature = (local_path, remote_path=None))]
    fn sftp_write<'p>(
        &self,
        py: Python<'p>,
        local_path: String,
        remote_path: Option<String>,
    ) -> PyResult<Bound<'p, PyAny>> {
        let session_arc = self.session.clone();
        let sftp_arc = self.sftp_session.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let sftp = AsyncConnection::get_or_init_sftp(session_arc, sftp_arc).await?;

            let remote_p = remote_path.unwrap_or_else(|| local_path.clone());

            let mut local_file = tokio::fs::File::open(&local_path).await.map_err(|e| {
                PyRuntimeError::new_err(format!("Failed to open local file: {}", e))
            })?;

            let mut remote_file = sftp.create(&remote_p).await.map_err(|e| {
                PyRuntimeError::new_err(format!("Failed to create remote file: {}", e))
            })?;

            let mut buffer = vec![0u8; 65536]; // 64KB buffer to match sync version
            loop {
                let n = local_file.read(&mut buffer).await.map_err(|e| {
                    PyRuntimeError::new_err(format!("Failed to read local file: {}", e))
                })?;
                if n == 0 {
                    break;
                }
                remote_file.write_all(&buffer[..n]).await.map_err(|e| {
                    PyRuntimeError::new_err(format!("Failed to write remote file: {}", e))
                })?
            }
            Ok(())
        })
    }

    /// Writes data over SFTP.
    fn sftp_write_data<'p>(
        &self,
        py: Python<'p>,
        data: String,
        remote_path: String,
    ) -> PyResult<Bound<'p, PyAny>> {
        let session_arc = self.session.clone();
        let sftp_arc = self.sftp_session.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let sftp = AsyncConnection::get_or_init_sftp(session_arc, sftp_arc).await?;

            let mut remote_file = sftp.create(&remote_path).await.map_err(|e| {
                PyRuntimeError::new_err(format!("Failed to create remote file: {}", e))
            })?;

            remote_file.write_all(data.as_bytes()).await.map_err(|e| {
                PyRuntimeError::new_err(format!("Failed to write remote file: {}", e))
            })?;

            Ok(())
        })
    }

    /// Lists the contents of a remote directory over SFTP.
    fn sftp_list<'p>(&self, py: Python<'p>, path: String) -> PyResult<Bound<'p, PyAny>> {
        let session_arc = self.session.clone();
        let sftp_arc = self.sftp_session.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let sftp = AsyncConnection::get_or_init_sftp(session_arc, sftp_arc).await?;

            let entries = sftp
                .read_dir(&path)
                .await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to list directory: {}", e)))?;

            let mut results = Vec::new();
            for entry in entries {
                results.push(entry.file_name());
            }
            Ok(results)
        })
    }

    #[pyo3(signature = (pty=None))]
    fn shell<'p>(&self, py: Python<'p>, pty: Option<bool>) -> PyResult<Bound<'p, PyAny>> {
        let session_arc = self.session.clone();
        let pty = pty.unwrap_or(false);
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let guard = session_arc.lock().await;
            let session = guard
                .as_ref()
                .ok_or_else(|| PyRuntimeError::new_err("Not connected"))?;
            let session = session.clone();
            drop(guard);

            let channel: russh::Channel<russh::client::Msg> = session
                .channel_open_session()
                .await
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;

            if pty {
                channel
                    .request_pty(true, "xterm", 80, 24, 0, 0, &[])
                    .await
                    .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
            }

            channel
                .request_shell(true)
                .await
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;

            Ok(AsyncInteractiveShell {
                channel: Arc::new(Mutex::new(channel)),
            })
        })
    }

    fn tail(&self, remote_file: String) -> AsyncFileTailer {
        AsyncFileTailer {
            conn_session: self.session.clone(),
            remote_file,
            state: Arc::new(Mutex::new(TailerState {
                sftp: None,
                init_pos: None,
                last_pos: 0,
                contents: None,
            })),
        }
    }
}

/// # AsyncInteractiveShell
///
/// `AsyncInteractiveShell` is an asynchronous class that represents an interactive shell over an SSH connection. It includes methods for sending commands and reading the output.
///
/// ## Methods
///
/// ### `send`
///
/// Sends a command to the shell. It takes the following parameters:
///
/// * `data`: The command to send.
/// * `add_newline`: Whether to add a newline at the end of the command (default: true).
///
/// ### `read`
///
/// Reads the output from the shell and returns an `SSHResult`.
///
/// ### `close`
///
/// Closes the shell.
///
#[pyclass]
#[derive(Clone)]
pub struct AsyncInteractiveShell {
    channel: Arc<Mutex<russh::Channel<russh::client::Msg>>>,
}

#[pymethods]
impl AsyncInteractiveShell {
    #[pyo3(signature = (data, add_newline=None))]
    fn send<'p>(
        &self,
        py: Python<'p>,
        data: String,
        add_newline: Option<bool>,
    ) -> PyResult<Bound<'p, PyAny>> {
        let channel = self.channel.clone();
        let add_newline = add_newline.unwrap_or(true);
        let data = if add_newline && !data.ends_with('\n') {
            format!("{}\n", data)
        } else {
            data
        };

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let guard = channel.lock().await;
            guard
                .data(data.as_bytes())
                .await
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
            Ok(())
        })
    }

    fn read<'p>(&self, py: Python<'p>) -> PyResult<Bound<'p, PyAny>> {
        let channel = self.channel.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut guard = channel.lock().await;
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();

            // First, wait for at least one data packet (ignoring non-data packets)
            loop {
                match tokio::time::timeout(std::time::Duration::from_secs(2), guard.wait()).await {
                    Ok(Some(russh::ChannelMsg::Data { data })) => {
                        stdout.extend_from_slice(&data);
                        break;
                    }
                    Ok(Some(russh::ChannelMsg::ExtendedData { data, .. })) => {
                        stderr.extend_from_slice(&data);
                        break;
                    }
                    Ok(Some(_)) => continue, // Ignore other events
                    Ok(None) => {
                        return Ok(SSHResult {
                            stdout: String::new(),
                            stderr: String::new(),
                            status: 0,
                        })
                    } // Channel closed
                    Err(_) => {
                        return Ok(SSHResult {
                            stdout: String::new(),
                            stderr: String::new(),
                            status: 0,
                        })
                    } // Timeout waiting for first packet
                }
            }

            // Then, consume any subsequent packets that arrive quickly
            loop {
                match tokio::time::timeout(std::time::Duration::from_millis(50), guard.wait()).await
                {
                    Ok(Some(russh::ChannelMsg::Data { data })) => {
                        stdout.extend_from_slice(&data);
                    }
                    Ok(Some(russh::ChannelMsg::ExtendedData { data, .. })) => {
                        stderr.extend_from_slice(&data);
                    }
                    Ok(Some(_)) => continue,
                    Ok(None) => break,
                    Err(_) => break,
                }
            }

            Ok(SSHResult {
                stdout: String::from_utf8_lossy(&stdout).to_string(),
                stderr: String::from_utf8_lossy(&stderr).to_string(),
                status: 0,
            })
        })
    }

    fn close<'p>(&self, py: Python<'p>) -> PyResult<Bound<'p, PyAny>> {
        let channel = self.channel.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let guard = channel.lock().await;
            guard
                .close()
                .await
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
            Ok(())
        })
    }

    fn __aenter__<'p>(slf: PyRef<'p, Self>, py: Python<'p>) -> PyResult<Bound<'p, PyAny>> {
        let py_self = Bound::new(py, (*slf).clone())?.into_any().unbind();
        pyo3_async_runtimes::tokio::future_into_py(py, async move { Ok(py_self) })
    }

    fn __aexit__<'p>(
        &self,
        py: Python<'p>,
        _exc_type: Option<Bound<'p, PyAny>>,
        _exc_value: Option<Bound<'p, PyAny>>,
        _traceback: Option<Bound<'p, PyAny>>,
    ) -> PyResult<Bound<'p, PyAny>> {
        self.close(py)
    }
}

struct TailerState {
    sftp: Option<SftpSession>,
    init_pos: Option<u64>,
    last_pos: u64,
    contents: Option<String>,
}

/// # AsyncFileTailer
///
/// `AsyncFileTailer` is an asynchronous class for tailing remote files over SFTP. It maintains an SFTP connection and allows reading from a specified position in a remote file.
///
/// ## Attributes
///
/// * `remote_file`: The path to the remote file.
/// * `contents`: The contents read from the file (available after exiting the context manager).
///
/// ## Methods
///
/// ### `seek_end`
///
/// Seeks to the end of the remote file and returns the file size.
///
/// ### `read`
///
/// Reads the contents of the remote file from a given position. It takes the following parameter:
///
/// * `from_pos`: Optional position to start reading from (default: last read position).
///
#[pyclass]
#[derive(Clone)]
pub struct AsyncFileTailer {
    conn_session: Arc<Mutex<Option<Arc<Handle<ClientHandler>>>>>,
    remote_file: String,
    state: Arc<Mutex<TailerState>>,
}

impl AsyncFileTailer {
    async fn _seek_end(state: Arc<Mutex<TailerState>>, remote_file: String) -> PyResult<u64> {
        let mut guard = state.lock().await;
        if let Some(sftp) = &guard.sftp {
            let metadata = sftp
                .metadata(&remote_file)
                .await
                .map_err(|e| PyRuntimeError::new_err(format!("Stat error: {}", e)))?;
            let size = metadata.size.unwrap_or(0);
            guard.last_pos = size;
            if guard.init_pos.is_none() {
                guard.init_pos = Some(size);
            }
            Ok(size)
        } else {
            Err(PyRuntimeError::new_err("SFTP session not initialized"))
        }
    }
}

#[pymethods]
impl AsyncFileTailer {
    fn seek_end<'p>(&self, py: Python<'p>) -> PyResult<Bound<'p, PyAny>> {
        let state = self.state.clone();
        let remote_file = self.remote_file.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            AsyncFileTailer::_seek_end(state, remote_file).await
        })
    }

    #[pyo3(signature = (from_pos=None))]
    fn read<'p>(&self, py: Python<'p>, from_pos: Option<u64>) -> PyResult<Bound<'p, PyAny>> {
        let state = self.state.clone();
        let remote_file = self.remote_file.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut guard = state.lock().await;
            let from_pos = from_pos.unwrap_or(guard.last_pos);

            if let Some(sftp) = &guard.sftp {
                let mut file = sftp
                    .open(&remote_file)
                    .await
                    .map_err(|e| PyRuntimeError::new_err(format!("Open error: {}", e)))?;

                let mut buffer = Vec::new();
                file.read_to_end(&mut buffer)
                    .await
                    .map_err(|e| PyRuntimeError::new_err(format!("Read error: {}", e)))?;

                if from_pos > buffer.len() as u64 {
                    guard.last_pos = buffer.len() as u64;
                    return Ok(String::new());
                }

                let content = String::from_utf8_lossy(&buffer[from_pos as usize..]).to_string();
                guard.last_pos = buffer.len() as u64;
                Ok(content)
            } else {
                Err(PyRuntimeError::new_err("SFTP session not initialized"))
            }
        })
    }

    fn __aenter__<'p>(slf: PyRef<'p, Self>, py: Python<'p>) -> PyResult<Bound<'p, PyAny>> {
        let conn_session = slf.conn_session.clone();
        let state = slf.state.clone();
        let remote_file = slf.remote_file.clone();

        let py_self = Bound::new(py, (*slf).clone())?.into_any().unbind();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let guard = conn_session.lock().await;
            let session = guard
                .as_ref()
                .ok_or_else(|| PyRuntimeError::new_err("Not connected"))?
                .clone();
            drop(guard);

            let channel = session
                .channel_open_session()
                .await
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;

            channel
                .request_subsystem(true, "sftp")
                .await
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;

            let sftp = SftpSession::new(channel.into_stream())
                .await
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;

            {
                let mut state_guard = state.lock().await;
                state_guard.sftp = Some(sftp);
            }

            AsyncFileTailer::_seek_end(state, remote_file).await?;

            Ok(py_self)
        })
    }

    fn __aexit__<'p>(
        &self,
        py: Python<'p>,
        _exc_type: Option<Bound<'p, PyAny>>,
        _exc_value: Option<Bound<'p, PyAny>>,
        _traceback: Option<Bound<'p, PyAny>>,
    ) -> PyResult<Bound<'p, PyAny>> {
        let state = self.state.clone();
        let remote_file = self.remote_file.clone();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut guard = state.lock().await;
            let init_pos = guard.init_pos.unwrap_or(0);

            if let Some(sftp) = &guard.sftp {
                let mut file = sftp
                    .open(&remote_file)
                    .await
                    .map_err(|e| PyRuntimeError::new_err(format!("Open error: {}", e)))?;

                let mut buffer = Vec::new();
                file.read_to_end(&mut buffer)
                    .await
                    .map_err(|e| PyRuntimeError::new_err(format!("Read error: {}", e)))?;

                let content = String::from_utf8_lossy(&buffer[init_pos as usize..]).to_string();
                guard.contents = Some(content);
            }
            Ok(())
        })
    }

    #[getter]
    fn contents(&self) -> PyResult<Option<String>> {
        let guard = self.state.blocking_lock();
        Ok(guard.contents.clone())
    }
}
