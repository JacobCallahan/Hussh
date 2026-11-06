//! # connection.rs
//!
//! This module provides a higher-level class that makes establishing and using ssh connections easier.
//! It uses the `ssh2` and `pyo3` libraries to provide a Python-friendly interface for SSH operations.
//!
//! ## Classes
//!
//! ### SSHResult
//! A class that represents the result of an SSH operation. It includes the standard output, standard error, and exit status of the operation.
//!
//! ### Connection
//! A class that represents an SSH connection. It includes methods for executing commands, reading and writing files over SCP and SFTP, and creating an interactive shell.
//!
//! ### InteractiveShell
//! A class that represents an interactive shell over an SSH connection. It includes methods for sending commands and reading the output.
//!
//! ## Functions
//!
//! ### read_from_channel
//! A helper function that reads the output from an SSH channel and returns an `SSHResult`.
//!
//! ## Usage
//!
//! To use this module, create a `Connection` instance with the necessary connection details. Then, use the methods on the `Connection` instance to perform SSH operations.
//!
//! ```python
//! conn = Connection("my.test.server", username="user", password="pass")
//! result = conn.execute("ls")
//! print(result.stdout)
//! ```
//!
//! Multiple forms of authentication are supported. You can use a password, a private key, or default SSH authentication (ssh-agent + default keys).
//!
//! ```python
//! conn = Connection("my.test.server", username="user", private_key="~/.ssh/id_rsa")
//! conn = Connection("my.test.server", username="user", password="pass")
//! conn = Connection("my.test.server", username="user")  # Uses ssh-agent, then tries default SSH keys
//! ````
//!
//! If you don't pass a port, the default SSH port (22) is used.
//! If you don't pass a username, "root" is used.
//!
//! When no password or private_key is specified, the connection will attempt authentication in this order:
//! 1. SSH agent (if available)
//! 2. Default SSH key files: ~/.ssh/id_rsa, ~/.ssh/id_ed25519, ~/.ssh/id_ecdsa, ~/.ssh/id_dsa
//!
//! To use the interactive shell, it is recommended to use the shell() context manager from the Connection class.
//! You can send commands to the shell using the `send` method, then get the results from result when you exit the context manager.
//! Due to the nature of reading from the shell, do not use the `read` method if you want to send more commands.
//!
//! ```python
//! with conn.shell() as shell:
//!    shell.send("ls")
//!    shell.send("pwd")
//!    shell.send("whoami")
//!
//! print(shell.result.stdout)
//! ```
//!
//! Note: The `read` method sends an EOF to the shell, so you won't be able to send more commands after calling `read`. If you want to send more commands, you would need to create a new `InteractiveShell` instance.
use crate::recovery::{self, HusshConnectionError, RetryPolicy};
use pyo3::create_exception;
use pyo3::prelude::*;
use ssh2::{Channel, Session};
use std::io::{BufReader, BufWriter, Read, Seek, Write};
use std::net::TcpStream;
use std::path::Path;

use pyo3::exceptions::{PyIOError, PyRuntimeError, PyTimeoutError, PyTypeError};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, Stdio};

pub(crate) const MAX_BUFF_SIZE: usize = 65536;
create_exception!(
    connection,
    AuthenticationError,
    pyo3::exceptions::PyException
);

struct ProxyAttempt(Option<Child>);
impl Drop for ProxyAttempt {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn transport_error(e: &ssh2::Error) -> bool {
    matches!(
        e.code(),
        ssh2::ErrorCode::Session(-1 | -7 | -13 | -43 | -45) | ssh2::ErrorCode::SFTP(6 | 7)
    )
}
fn ssh_error(e: ssh2::Error) -> PyErr {
    if matches!(e.code(), ssh2::ErrorCode::Session(-9 | -30 | -37)) {
        PyTimeoutError::new_err(e.to_string())
    } else if transport_error(&e) {
        HusshConnectionError::new_err(e.to_string())
    } else {
        PyIOError::new_err(e.to_string())
    }
}
fn retryable(e: &PyErr) -> bool {
    Python::attach(|py| {
        e.is_instance_of::<HusshConnectionError>(py) || e.is_instance_of::<PyTimeoutError>(py)
    })
}
fn auth_error(e: ssh2::Error) -> PyErr {
    if transport_error(&e) || matches!(e.code(), ssh2::ErrorCode::Session(-9 | -30 | -37)) {
        ssh_error(e)
    } else {
        AuthenticationError::new_err(e.to_string())
    }
}
fn command_io_error(e: std::io::Error) -> PyErr {
    match e.kind() {
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => {
            PyTimeoutError::new_err(e.to_string())
        }
        std::io::ErrorKind::InvalidData
        | std::io::ErrorKind::InvalidInput
        | std::io::ErrorKind::Unsupported => PyIOError::new_err(e.to_string()),
        // ssh2 erases transport error codes when converting them to io::ErrorKind::Other.
        _ => recovery::incomplete(e),
    }
}

fn read_from_channel(channel: &mut Channel) -> Result<SSHResult, PyErr> {
    let mut stdout = String::new();
    channel
        .read_to_string(&mut stdout)
        .map_err(command_io_error)?;
    let mut stderr = String::new();
    channel
        .stderr()
        .read_to_string(&mut stderr)
        .map_err(command_io_error)?;
    channel.wait_close().map_err(|e| {
        if transport_error(&e) {
            recovery::incomplete(e)
        } else {
            ssh_error(e)
        }
    })?;
    let mut status = channel.exit_status().map_err(|e| {
        if transport_error(&e) {
            recovery::incomplete(e)
        } else {
            ssh_error(e)
        }
    })?;
    if channel
        .exit_signal()
        .map_err(ssh_error)?
        .exit_signal
        .is_some()
    {
        status = -1;
    }
    Ok(SSHResult {
        stdout,
        stderr,
        status,
    })
}

#[derive(Clone)]
struct ProxyJumpConfig {
    host: String,
    port: i32,
    username: Option<String>,
    private_key: Option<String>,
}

fn parse_proxy_jump(_py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<ProxyJumpConfig> {
    if let Ok(conn) = value.extract::<PyRef<'_, Connection>>() {
        return Ok(ProxyJumpConfig {
            host: conn.get_host().to_string(),
            port: conn.get_port(),
            username: Some(conn.get_username().to_string()),
            private_key: if conn.get_private_key().is_empty() {
                None
            } else {
                Some(conn.get_private_key().to_string())
            },
        });
    }

    if let Ok((host, port)) = value.extract::<(String, i32)>() {
        return Ok(ProxyJumpConfig {
            host,
            port,
            username: None,
            private_key: None,
        });
    }

    if let Ok(spec) = value.extract::<String>() {
        if let Some((username, host_port)) = spec.split_once('@') {
            let (host, port) = parse_host_port(host_port)?;
            return Ok(ProxyJumpConfig {
                host,
                port,
                username: Some(username.to_string()),
                private_key: None,
            });
        }

        let (host, port) = parse_host_port(&spec)?;
        return Ok(ProxyJumpConfig {
            host,
            port,
            username: None,
            private_key: None,
        });
    }

    Err(PyErr::new::<PyTypeError, _>(
        "proxy_jump must be a Connection, (host, port) tuple, or string",
    ))
}

fn parse_host_port(value: &str) -> PyResult<(String, i32)> {
    if let Some((host, port_str)) = value.rsplit_once(':') {
        let port = port_str.parse::<i32>().map_err(|_| {
            PyErr::new::<PyTypeError, _>("proxy_jump string port must be a valid integer")
        })?;
        return Ok((host.to_string(), port));
    }
    Ok((value.to_string(), 22))
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn build_proxy_jump_command(target_host: &str, target_port: i32, jump: &ProxyJumpConfig) -> String {
    let username = jump.username.as_deref().unwrap_or("root");
    // Match the MVP's "blindly accept keys" behavior for the final hop (see
    // ClientHandler::check_server_key) by also skipping host key verification
    // for the jump hop, since there's no mechanism yet for users to manage a
    // known_hosts file for jump hosts.
    let mut command = format!(
        "ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -l {} -p {}",
        shell_quote(username),
        jump.port
    );

    if let Some(private_key) = jump.private_key.as_deref() {
        let expanded_key = shellexpand::tilde(private_key).into_owned();
        command.push_str(&format!(" -i {}", shell_quote(&expanded_key)));
    }

    command.push_str(&format!(
        " -W {}:{} {}",
        shell_quote(target_host),
        target_port,
        shell_quote(&jump.host)
    ));
    command
}

fn render_proxy_command(command: &str, host: &str, port: i32) -> String {
    command.replace("%h", host).replace("%p", &port.to_string())
}

#[cfg(unix)]
fn create_proxy_stream(command: &str) -> PyResult<(UnixStream, Child)> {
    let (stream, proxy_side) = UnixStream::pair().map_err(|e| {
        PyErr::new::<PyIOError, _>(format!("Failed to create proxy transport: {}", e))
    })?;
    let stdin_side = proxy_side
        .try_clone()
        .map_err(|e| PyErr::new::<PyIOError, _>(format!("Failed to clone proxy stream: {}", e)))?;
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdin(Stdio::from(std::os::fd::OwnedFd::from(stdin_side)))
        .stdout(Stdio::from(std::os::fd::OwnedFd::from(proxy_side)))
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            PyErr::new::<PyRuntimeError, _>(format!("Failed to start proxy command: {}", e))
        })?;

    // The shell itself always spawns successfully, even if the proxy command it
    // runs doesn't exist. Give it a brief moment to fail fast (e.g. "command not
    // found") so we can surface a clear error instead of a confusing downstream
    // handshake timeout.
    std::thread::sleep(std::time::Duration::from_millis(100));
    if let Ok(Some(status)) = child.try_wait() {
        let mut stderr_output = String::new();
        if let Some(mut stderr) = child.stderr.take() {
            let _ = stderr.read_to_string(&mut stderr_output);
        }
        return Err(PyErr::new::<PyRuntimeError, _>(format!(
            "Failed to start proxy command (exited with {}): {}",
            status,
            stderr_output.trim()
        )));
    }
    // Process is still running; drop stderr to prevent blocking if the proxy
    // writes diagnostics during operation.
    drop(child.stderr.take());
    Ok((stream, child))
}

#[cfg(not(unix))]
fn create_proxy_stream(_command: &str) -> PyResult<(TcpStream, Child)> {
    Err(PyErr::new::<PyRuntimeError, _>(
        "proxy_command and proxy_jump are currently only supported on Unix platforms",
    ))
}

#[pyclass(from_py_object)]
#[derive(Clone)]
pub struct SSHResult {
    #[pyo3(get)]
    pub stdout: String,
    #[pyo3(get)]
    pub stderr: String,
    #[pyo3(get)]
    pub status: i32,
}

#[pymethods]
impl SSHResult {
    // The __repl__ method for the SSHResult class
    fn __repr__(&self) -> PyResult<String> {
        Ok(format!(
            "SSHResult(stdout={}, stderr={}, status={})",
            self.stdout, self.stderr, self.status
        ))
    }

    // The __str__ method for the SSHResult class
    fn __str__(&self) -> PyResult<String> {
        Ok(format!(
            "stdout:\n{}\nstderr:\n{}\nstatus: {}",
            self.stdout, self.stderr, self.status
        ))
    }
}

/// # Connection
///
/// `Connection` is a class that represents an SSH connection. It provides methods for executing commands, reading and writing files over SCP and SFTP, and creating an interactive shell.
///
/// ## Attributes
///
/// * `session`: The underlying SSH session.
/// * `host`: The host to connect to.
/// * `port`: The port to connect to.
/// * `username`: The username to use for authentication.
/// * `password`: The password to use for authentication.
/// * `private_key`: The path to the private key to use for authentication.
/// * `timeout`: The timeout(ms) for the SSH session.
///
/// ## Methods
///
/// ### `execute`
///
/// Executes a command over the SSH connection and returns the result. It takes the following parameter:
///
/// * `command`: The command to execute.
///
/// ### `scp_read`
///
/// Reads a file over SCP and returns the contents. It takes the following parameters:
///
/// * `remote_path`: The path to the file on the remote system.
/// * `local_path`: The path to save the file on the local system. If not provided, the contents of the file are returned.
///
/// ### `scp_write`
///
/// Writes a file over SCP. It takes the following parameters:
///
/// * `local_path`: The path to the file on the local system.
/// * `remote_path`: The path to save the file on the remote system.
///
/// ### `scp_write_data`
///
/// Writes data over SCP. It takes the following parameters:
///
/// * `data`: The data to write.
/// * `remote_path`: The path to save the data on the remote system.
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
/// * `remote_path`: The path to save the file on the remote system.
///
/// ### `sftp_put_dir`
///
/// Uploads a local directory recursively to a remote path over SFTP. Returns (transferred_files, failed_files). It takes the following parameters:
///
/// * `local_path`: The local directory to upload.
/// * `remote_path`: The remote destination path.
/// * `follow_symlinks`: Whether to follow symlinks (default: true).
/// * `preserve_permissions`: Whether to preserve file permissions (default: true).
/// * `fail_fast`: Whether to raise an exception on the first error instead of collecting failures (default: false).
///
/// ### `sftp_get_dir`
///
/// Downloads a remote directory recursively to a local path over SFTP. Returns (transferred_files, failed_files). It takes the following parameters:
///
/// * `remote_path`: The remote directory to download.
/// * `local_path`: The local destination path.
/// * `follow_symlinks`: Whether to follow symlinks (default: true).
/// * `preserve_permissions`: Whether to preserve file permissions (default: true).
/// * `fail_fast`: Whether to raise an exception on the first error instead of collecting failures (default: false).
///
/// ### `shell`
///
/// Creates an `InteractiveShell` instance. It takes the following parameter:
///
/// ### `remote_copy`
///
/// Copies a file from this connection to another connection. It takes the following parameters:
///
/// * `source_path`: The path to the file on the remote system.
/// * `dest_conn`: The destination connection to copy the file to.
/// * `dest_path`: The path to save the file on the destination system. If not provided, the source path is used.
#[pyclass]
pub struct Connection {
    session: Session,
    closed: bool,
    disconnected: bool,
    #[pyo3(get)]
    pub(crate) auto_reconnect: bool,
    #[pyo3(get)]
    pub(crate) max_retries: i64,
    #[pyo3(get)]
    pub(crate) retry_interval: f64,
    #[pyo3(get)]
    host: String,
    #[pyo3(get)]
    port: i32,
    #[pyo3(get)]
    username: String,
    #[pyo3(get)]
    password: String,
    #[pyo3(get)]
    private_key: String,
    #[pyo3(get)]
    timeout: u32,
    #[pyo3(get)]
    proxy_command: Option<String>,
    proxy_process: Option<Child>,
    sftp_conn: Option<ssh2::Sftp>,
}

// Non-public methods for the Connection class
impl Connection {
    fn establish(&self) -> PyResult<(Session, Option<Child>)> {
        let host = self.host.as_str();
        let port = self.port;
        let mut session = Session::new().map_err(ssh_error)?;
        let mut proxy_process = ProxyAttempt(None);
        let timeout = self.timeout;
        session.set_timeout(timeout);
        if let Some(command) = self.proxy_command.as_deref() {
            let rendered = render_proxy_command(command, host, port);
            let (proxy_stream, child) = create_proxy_stream(&rendered)?;
            proxy_process.0 = Some(child);
            session.set_tcp_stream(proxy_stream);
        } else {
            let conn_str = format!("{}:{}", host, port);
            let tcp_conn = TcpStream::connect(conn_str)
                .map_err(|e| PyErr::new::<PyTimeoutError, _>(format!("{}", e)))?;
            session.set_tcp_stream(tcp_conn);
        }

        session
            .handshake()
            .map_err(|e| PyErr::new::<PyTimeoutError, _>(format!("{}", e)))?;
        let username = self.username.as_str();
        let password = self.password.as_str();
        let private_key = self.private_key.as_str();

        if !private_key.is_empty() {
            let private_key = shellexpand::tilde(private_key).into_owned();
            if !password.is_empty() {
                session
                    .userauth_pubkey_file(username, None, Path::new(&private_key), Some(password))
                    .map_err(auth_error)?;
            } else {
                session
                    .userauth_pubkey_file(username, None, Path::new(&private_key), None)
                    .map_err(auth_error)?;
            }
        } else if !password.is_empty() {
            session
                .userauth_password(username, password)
                .map_err(auth_error)?;
        } else if let Err(error) = session.userauth_agent(username) {
            let error = auth_error(error);
            if retryable(&error) {
                return Err(error);
            }
            let default_keys = [
                "~/.ssh/id_rsa",
                "~/.ssh/id_ed25519",
                "~/.ssh/id_ecdsa",
                "~/.ssh/id_dsa",
            ];

            let mut auth_success = false;
            for key_path in &default_keys {
                let expanded_key_path = shellexpand::tilde(key_path).into_owned();
                if Path::new(&expanded_key_path).exists() {
                    match session.userauth_pubkey_file(
                        username,
                        None,
                        Path::new(&expanded_key_path),
                        None,
                    ) {
                        Ok(()) => {
                            auth_success = true;
                            break;
                        }
                        Err(error) => {
                            let error = auth_error(error);
                            if retryable(&error) {
                                return Err(error);
                            }
                        }
                    }
                }
            }

            if !auth_success {
                return Err(PyErr::new::<AuthenticationError, _>(
                    "Failed to authenticate with ssh-agent and default SSH keys",
                ));
            }
        }

        Ok((session, proxy_process.0.take()))
    }

    fn sftp(&self) -> &ssh2::Sftp {
        self.sftp_conn
            .as_ref()
            .expect("SFTP prepared before file access")
    }

    fn prepare_sftp(&mut self) -> PyResult<()> {
        if let Some(sftp) = self.prepare(|connection| {
            if let Some(sftp) = connection.sftp_conn.as_ref() {
                match sftp.realpath(Path::new(".")) {
                    Ok(_) => {}
                    // Any ordinary SFTP status response confirms the session is alive.
                    Err(error)
                        if matches!(error.code(), ssh2::ErrorCode::SFTP(_))
                            && !transport_error(&error) => {}
                    Err(error) => return Err(error),
                }
                Ok(None)
            } else {
                connection
                    .session
                    .sftp()
                    .map(Some)
                    .map_err(|error| connection.preparation_error(error))
            }
        })? {
            self.sftp_conn = Some(sftp);
        }
        Ok(())
    }

    fn prepare_channel(&mut self, timeout: Option<u32>) -> PyResult<Channel> {
        self.prepare(|connection| {
            let session = &connection.session;
            let original_timeout = session.timeout();
            if let Some(timeout) = timeout {
                session.set_timeout(timeout);
            }
            let result = session
                .channel_session()
                .map_err(|error| connection.preparation_error(error));
            session.set_timeout(original_timeout);
            result
        })
    }

    fn preparation_error(&self, error: ssh2::Error) -> ssh2::Error {
        // libssh2 can mask a lost transport as a subsystem request failure.
        if matches!(error.code(), ssh2::ErrorCode::Session(-21)) {
            if let Err(probe) = self.session.channel_session() {
                if transport_error(&probe) {
                    return probe;
                }
            }
        }
        error
    }

    fn prepare<T>(&mut self, setup: impl Fn(&Self) -> Result<T, ssh2::Error>) -> PyResult<T> {
        if self.closed {
            return Err(HusshConnectionError::new_err(
                "Connection explicitly closed; use reconnect()",
            ));
        }
        let mut cause = if self.disconnected {
            HusshConnectionError::new_err("Disconnected")
        } else {
            match setup(self) {
                Ok(value) => return Ok(value),
                Err(e) if transport_error(&e) => ssh_error(e),
                Err(e) => return Err(ssh_error(e)),
            }
        };
        self.disconnected = true;
        self.sftp_conn = None;
        let retries = if self.auto_reconnect {
            self.max_retries
        } else {
            0
        };
        let policy = RetryPolicy::new(retries, self.retry_interval);
        for attempt in 0..retries {
            Python::attach(|py| py.detach(|| std::thread::sleep(policy.delay(attempt))));
            match self.install().and_then(|()| setup(self).map_err(ssh_error)) {
                Ok(value) => return Ok(value),
                Err(e) if retryable(&e) => cause = e,
                Err(e) => return Err(e),
            }
            self.disconnected = true;
        }
        Err(recovery::exhausted(retries, cause))
    }

    fn install(&mut self) -> PyResult<()> {
        self.disconnected = true;
        self.sftp_conn = None;
        if let Some(mut child) = self.proxy_process.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let (session, process) = Python::attach(|py| py.detach(|| self.establish()))?;
        self.session = session;
        self.proxy_process = process;
        self.disconnected = false;
        self.closed = false;
        Ok(())
    }

    fn replace_session(&mut self, policy: RetryPolicy) -> PyResult<()> {
        self.sftp_conn = None;
        if let Some(mut child) = self.proxy_process.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let attempts = policy.retries + 1;
        for attempt in 0..attempts {
            if attempt > 0 {
                let delay = policy.delay(attempt - 1);
                Python::attach(|py| py.detach(|| std::thread::sleep(delay)));
            }
            match self.install() {
                Ok(()) => return Ok(()),
                Err(e) => {
                    if !retryable(&e) {
                        return Err(e);
                    }
                    if attempt + 1 == attempts {
                        return Err(recovery::exhausted(attempts, e));
                    }
                }
            }
        }
        unreachable!("explicit recovery always attempts once")
    }

    // Public getters for use by multi_conn module
    pub fn get_host(&self) -> &str {
        &self.host
    }

    pub fn get_port(&self) -> i32 {
        self.port
    }

    pub fn get_username(&self) -> &str {
        &self.username
    }

    pub fn get_password(&self) -> &str {
        &self.password
    }

    pub fn get_private_key(&self) -> &str {
        &self.private_key
    }

    pub fn get_timeout(&self) -> u32 {
        self.timeout
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        if let Some(child) = self.proxy_process.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.proxy_process = None;
    }
}

#[pymethods]
impl Connection {
    #[new]
    #[pyo3(signature = (host, port=22, username="root", password=None, private_key=None, proxy_jump=None, proxy_command=None, timeout=0, *, auto_reconnect=true, max_retries=3, retry_interval=1.5))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        py: Python<'_>,
        host: &str,
        port: Option<i32>,
        username: Option<&str>,
        password: Option<&str>,
        private_key: Option<&str>,
        proxy_jump: Option<Bound<'_, PyAny>>,
        proxy_command: Option<&str>,
        timeout: Option<u32>,
        auto_reconnect: bool,
        max_retries: i64,
        retry_interval: f64,
    ) -> PyResult<Connection> {
        let port = port.unwrap_or(22);

        let proxy_jump = match proxy_jump {
            Some(value) => Some(parse_proxy_jump(py, &value)?),
            None => None,
        };
        if proxy_jump.is_some() && proxy_command.is_some() {
            return Err(PyErr::new::<PyTypeError, _>(
                "proxy_jump and proxy_command are mutually exclusive",
            ));
        }
        #[cfg(not(unix))]
        if proxy_jump.is_some() || proxy_command.is_some() {
            return Err(PyErr::new::<PyRuntimeError, _>(
                "proxy_command and proxy_jump are currently only supported on Unix platforms",
            ));
        }
        let effective_proxy_command = if let Some(jump_cfg) = proxy_jump.as_ref() {
            Some(build_proxy_jump_command(host, port, jump_cfg))
        } else {
            proxy_command.map(ToString::to_string)
        };

        recovery::validate(max_retries, retry_interval)?;
        let mut conn = Connection {
            session: Session::new().map_err(ssh_error)?,
            closed: false,
            disconnected: false,
            auto_reconnect,
            max_retries,
            retry_interval,
            port,
            host: host.to_string(),
            username: username.unwrap_or("root").to_string(),
            password: password.unwrap_or("").to_string(),
            private_key: private_key.unwrap_or("").to_string(),
            timeout: timeout.unwrap_or(0),
            proxy_command: effective_proxy_command,
            proxy_process: None,
            sftp_conn: None,
        };
        let (session, process) = py.detach(|| conn.establish())?;
        conn.session = session;
        conn.proxy_process = process;
        Ok(conn)
    }

    #[pyo3(signature = (*, max_retries=None, retry_interval=None, max_interval=30.0))]
    fn reconnect(
        &mut self,
        max_retries: Option<i64>,
        retry_interval: Option<Bound<'_, PyAny>>,
        max_interval: f64,
    ) -> PyResult<()> {
        let policy = RetryPolicy::overrides(
            max_retries.unwrap_or(self.max_retries),
            self.retry_interval,
            retry_interval,
            max_interval,
        )?;
        self.replace_session(policy)
    }
    /// Executes a command over the SSH connection and returns the result.
    /// If `timeout` is provided, it temporarily updates the session timeout for the duration of the command execution.
    #[pyo3(signature = (command, timeout=None))]
    fn execute(
        &mut self,
        py: Python<'_>,
        command: String,
        timeout: Option<u32>,
    ) -> PyResult<SSHResult> {
        py.detach(|| {
            let mut channel = self.prepare_channel(timeout)?;
            let original_timeout = self.session.timeout();
            if let Some(t) = timeout {
                self.session.set_timeout(t);
            }
            let result = (|| {
                // Submission is the no-replay boundary, even if exec itself fails.
                channel.exec(&command).map_err(|e| {
                    let e = self.preparation_error(e);
                    if transport_error(&e) {
                        recovery::incomplete(e)
                    } else {
                        ssh_error(e)
                    }
                })?;
                read_from_channel(&mut channel)
            })();
            self.session.set_timeout(original_timeout);
            if Python::attach(|py| {
                result
                    .as_ref()
                    .err()
                    .is_some_and(|e| e.is_instance_of::<recovery::HusshIncompleteCommandError>(py))
            }) {
                self.disconnected = true;
                self.sftp_conn = None;
            }
            result
        })
    }

    /// Reads a file over SCP and returns the contents.
    /// If `local_path` is provided, the file is saved to the local system.
    /// Otherwise, the contents of the file are returned as a string.
    #[pyo3(signature = (remote_path, local_path=None))]
    fn scp_read(&mut self, remote_path: String, local_path: Option<String>) -> PyResult<String> {
        let _ = self.prepare_channel(None)?;
        let (mut remote_file, stat) = self
            .session
            .scp_recv(Path::new(&remote_path))
            .map_err(|e| PyErr::new::<PyIOError, _>(format!("Failed scp_recv: {}", e)))?;
        match local_path {
            Some(local_path) => {
                let mut local_file = std::fs::File::create(&local_path)
                    .map_err(|e| PyErr::new::<PyIOError, _>(format!("File create error: {}", e)))?;
                let mut buffer = vec![0; std::cmp::min(stat.size() as usize, MAX_BUFF_SIZE)];
                loop {
                    let len = remote_file
                        .read(&mut buffer)
                        .map_err(|e| PyErr::new::<PyIOError, _>(format!("Read error: {}", e)))?;
                    if len == 0 {
                        break;
                    }
                    local_file
                        .write_all(&buffer[..len])
                        .map_err(|e| PyErr::new::<PyIOError, _>(format!("Write error: {}", e)))?;
                }
                Ok("Ok".to_string())
            }
            None => {
                let mut contents = String::new();
                remote_file.read_to_string(&mut contents).map_err(|e| {
                    PyErr::new::<PyIOError, _>(format!("Read to string failed: {}", e))
                })?;
                Ok(contents)
            }
        }
    }

    /// Writes a file over SCP.
    fn scp_write(&mut self, local_path: String, remote_path: String) -> PyResult<()> {
        let _ = self.prepare_channel(None)?;
        // if remote_path is a directory, append the local file name to the remote path
        let remote_path = if remote_path.ends_with('/') {
            format!(
                "{}/{}",
                remote_path,
                Path::new(&local_path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .ok_or_else(|| PyIOError::new_err("local_path must name a file"))?
            )
        } else {
            remote_path
        };
        let mut local_file = std::fs::File::open(&local_path)
            .map_err(|e| PyErr::new::<PyIOError, _>(format!("Local file open error: {}", e)))?;
        let metadata = local_file
            .metadata()
            .map_err(|e| PyIOError::new_err(e.to_string()))?;
        // TODO: better handle permissions. Perhaps from metadata.permissions()?
        let mut remote_file = self
            .session
            .scp_send(Path::new(&remote_path), 0o644, metadata.len(), None)
            .map_err(|e| PyErr::new::<PyIOError, _>(format!("scp_send error: {}", e)))?;
        // create a variable-sized buffer to read the file and loop until EOF
        let mut read_buffer = vec![0; std::cmp::min(metadata.len() as usize, MAX_BUFF_SIZE)];
        loop {
            let bytes_read = local_file
                .read(&mut read_buffer)
                .map_err(|e| PyErr::new::<PyIOError, _>(format!("File read error: {}", e)))?;
            if bytes_read == 0 {
                break;
            }
            remote_file
                .write_all(&read_buffer[..bytes_read])
                .map_err(|e| {
                    PyErr::new::<PyIOError, _>(format!("Remote file write error: {}", e))
                })?;
        }
        remote_file
            .flush()
            .map_err(|e| PyIOError::new_err(e.to_string()))?;
        remote_file
            .send_eof()
            .map_err(|e| PyIOError::new_err(e.to_string()))?;
        remote_file
            .wait_eof()
            .map_err(|e| PyIOError::new_err(e.to_string()))?;
        remote_file
            .close()
            .map_err(|e| PyIOError::new_err(e.to_string()))?;
        remote_file
            .wait_close()
            .map_err(|e| PyIOError::new_err(e.to_string()))?;
        Ok(())
    }

    /// Writes data over SCP.
    fn scp_write_data(&mut self, data: String, remote_path: String) -> PyResult<()> {
        let _ = self.prepare_channel(None)?;
        let mut remote_file = self
            .session
            .scp_send(Path::new(&remote_path), 0o644, data.len() as u64, None)
            .map_err(|e| PyErr::new::<PyIOError, _>(format!("scp_send error: {}", e)))?;
        remote_file
            .write_all(data.as_bytes())
            .map_err(|e| PyErr::new::<PyIOError, _>(format!("Data write error: {}", e)))?;
        remote_file
            .send_eof()
            .map_err(|e| PyIOError::new_err(e.to_string()))?;
        remote_file
            .wait_eof()
            .map_err(|e| PyIOError::new_err(e.to_string()))?;
        remote_file
            .close()
            .map_err(|e| PyIOError::new_err(e.to_string()))?;
        remote_file
            .wait_close()
            .map_err(|e| PyIOError::new_err(e.to_string()))?;
        Ok(())
    }

    /// Reads a file over SFTP and returns the contents.
    /// If `local_path` is provided, the file is saved to the local system.
    /// Otherwise, the contents of the file are returned as a string.
    #[pyo3(signature = (remote_path, local_path=None))]
    fn sftp_read(&mut self, remote_path: String, local_path: Option<String>) -> PyResult<String> {
        self.prepare_sftp()?;
        let mut remote_file = BufReader::new(
            self.sftp()
                .open(Path::new(&remote_path))
                .map_err(|e| PyErr::new::<PyIOError, _>(format!("SFTP open error: {}", e)))?,
        );
        match local_path {
            Some(local_path) => {
                let local_file = std::fs::File::create(&local_path)
                    .map_err(|e| PyErr::new::<PyIOError, _>(format!("File create error: {}", e)))?;
                let mut writer = BufWriter::new(local_file);
                let mut buffer = vec![0; MAX_BUFF_SIZE];
                loop {
                    let len = remote_file.read(&mut buffer).map_err(|e| {
                        PyErr::new::<PyIOError, _>(format!("File read error: {}", e))
                    })?;
                    if len == 0 {
                        break;
                    }
                    writer.write_all(&buffer[..len]).map_err(|e| {
                        PyErr::new::<PyIOError, _>(format!("File write error: {}", e))
                    })?;
                }
                writer
                    .flush()
                    .map_err(|e| PyErr::new::<PyIOError, _>(format!("Flush error: {}", e)))?;
                Ok("Ok".to_string())
            }
            None => {
                let mut contents = String::new();
                remote_file.read_to_string(&mut contents).map_err(|e| {
                    PyErr::new::<PyIOError, _>(format!("Read to string failed: {}", e))
                })?;
                Ok(contents)
            }
        }
    }

    /// Writes a file over SFTP. If `remote_path` is not provided, the local file is written to the same path on the remote system.
    #[pyo3(signature = (local_path, remote_path=None))]
    fn sftp_write(&mut self, local_path: String, remote_path: Option<String>) -> PyResult<()> {
        self.prepare_sftp()?;
        let mut local_file = std::fs::File::open(&local_path)
            .map_err(|e| PyErr::new::<PyIOError, _>(format!("Local file open error: {}", e)))?;
        let remote_path = remote_path.unwrap_or_else(|| local_path.clone());
        let metadata = local_file
            .metadata()
            .map_err(|e| PyIOError::new_err(e.to_string()))?;
        let mut remote_file = self.sftp().create(Path::new(&remote_path)).map_err(|e| {
            PyErr::new::<PyIOError, _>(format!("Remote file creation error: {}", e))
        })?;
        // create a variable-sized buffer to read the file and loop until EOF
        let mut read_buffer = vec![0; std::cmp::min(metadata.len() as usize, MAX_BUFF_SIZE)];
        loop {
            let bytes_read = local_file
                .read(&mut read_buffer)
                .map_err(|e| PyErr::new::<PyIOError, _>(format!("File read error: {}", e)))?;
            if bytes_read == 0 {
                break;
            }
            remote_file
                .write_all(&read_buffer[..bytes_read])
                .map_err(|e| {
                    PyErr::new::<PyIOError, _>(format!("Remote file write error: {}", e))
                })?;
        }
        remote_file
            .close()
            .map_err(|e| PyIOError::new_err(e.to_string()))?;
        Ok(())
    }

    /// Writes data over SFTP.
    fn sftp_write_data(&mut self, data: String, remote_path: String) -> PyResult<()> {
        self.prepare_sftp()?;
        let mut remote_file = self.sftp().create(Path::new(&remote_path)).map_err(|e| {
            PyErr::new::<PyIOError, _>(format!("Remote file creation error: {}", e))
        })?;
        remote_file
            .write_all(data.as_bytes())
            .map_err(|e| PyErr::new::<PyIOError, _>(format!("Data write error: {}", e)))?;
        remote_file
            .close()
            .map_err(|e| PyErr::new::<PyIOError, _>(format!("Close error: {}", e)))?;
        Ok(())
    }

    /// Uploads a local directory recursively to a remote path over SFTP.
    /// Returns a tuple of (transferred_files, failed_files), where each is a list of local file paths.
    /// If `fail_fast` is true, the first transfer error raises an exception immediately.
    #[pyo3(signature = (local_path, remote_path, follow_symlinks=true, preserve_permissions=true, fail_fast=false))]
    fn sftp_put_dir(
        &mut self,
        local_path: String,
        remote_path: String,
        follow_symlinks: bool,
        preserve_permissions: bool,
        fail_fast: bool,
    ) -> PyResult<(Vec<String>, Vec<String>)> {
        self.prepare_sftp()?;
        let mut transferred: Vec<String> = Vec::new();
        let mut failed: Vec<String> = Vec::new();

        // Ensure remote base directory (and all missing parents) exist — like mkdir -p
        {
            let parts: Vec<&str> = remote_path.split('/').filter(|s| !s.is_empty()).collect();
            let is_absolute = remote_path.starts_with('/');
            let mut current = if is_absolute {
                String::from("/")
            } else {
                String::new()
            };
            for part in &parts {
                if !current.is_empty() && !current.ends_with('/') {
                    current.push('/');
                }
                current.push_str(part);
                if self.sftp().stat(Path::new(&current)).is_err() {
                    if let Err(e) = self.sftp().mkdir(Path::new(&current), 0o755) {
                        let msg = format!("Failed to create remote directory '{}': {}", current, e);
                        if fail_fast {
                            return Err(PyErr::new::<PyIOError, _>(msg));
                        }
                        failed.push(local_path.clone());
                        return Ok((transferred, failed));
                    }
                }
            }
        }

        // Stack for depth-first traversal: (local_dir, remote_dir_str).
        // Remote paths are kept as POSIX strings to avoid OS-native path separators on Windows.
        let mut dirs_to_process: Vec<(std::path::PathBuf, String)> =
            vec![(std::path::PathBuf::from(&local_path), remote_path.clone())];

        while let Some((local_dir, remote_dir)) = dirs_to_process.pop() {
            let entries = match std::fs::read_dir(&local_dir) {
                Ok(e) => e,
                Err(e) => {
                    let msg = format!(
                        "Failed to read local directory '{}': {}",
                        local_dir.display(),
                        e
                    );
                    if fail_fast {
                        return Err(PyErr::new::<PyIOError, _>(msg));
                    }
                    failed.push(local_dir.to_string_lossy().to_string());
                    continue;
                }
            };

            for entry in entries {
                let entry = match entry {
                    Ok(e) => e,
                    Err(e) => {
                        if fail_fast {
                            return Err(PyErr::new::<PyIOError, _>(format!(
                                "Directory entry error: {}",
                                e
                            )));
                        }
                        continue;
                    }
                };

                let local_entry = entry.path();
                let local_entry_str = local_entry.to_string_lossy().to_string();
                let file_name_str = entry.file_name().to_string_lossy().to_string();
                let remote_entry_str = format!("{}/{}", remote_dir, file_name_str);

                let metadata = match if follow_symlinks {
                    std::fs::metadata(&local_entry)
                } else {
                    std::fs::symlink_metadata(&local_entry)
                } {
                    Ok(m) => m,
                    Err(e) => {
                        if fail_fast {
                            return Err(PyErr::new::<PyIOError, _>(format!(
                                "Metadata error for '{}': {}",
                                local_entry_str, e
                            )));
                        }
                        failed.push(local_entry_str);
                        continue;
                    }
                };

                if metadata.is_dir() {
                    #[cfg(unix)]
                    let mode = if preserve_permissions {
                        use std::os::unix::fs::PermissionsExt;
                        metadata.permissions().mode() as i32
                    } else {
                        0o755i32
                    };
                    #[cfg(not(unix))]
                    let mode = 0o755i32;
                    if self.sftp().stat(Path::new(&remote_entry_str)).is_err() {
                        match self.sftp().mkdir(Path::new(&remote_entry_str), mode) {
                            Ok(_) => {}
                            Err(e) => {
                                if fail_fast {
                                    return Err(PyErr::new::<PyIOError, _>(format!(
                                        "Failed to create remote directory '{}': {}",
                                        remote_entry_str, e
                                    )));
                                }
                                failed.push(local_entry_str);
                                continue;
                            }
                        }
                    }
                    dirs_to_process.push((local_entry, remote_entry_str));
                } else if metadata.is_file() {
                    let result: Result<(), String> = (|| {
                        let mut local_file = std::fs::File::open(&local_entry)
                            .map_err(|e| format!("File open error: {}", e))?;
                        let file_size = metadata.len();
                        let mut remote_file = self
                            .sftp()
                            .create(Path::new(&remote_entry_str))
                            .map_err(|e| format!("Remote file creation error: {}", e))?;
                        let buf_size = (file_size as usize).min(MAX_BUFF_SIZE);
                        // Fall back to MAX_BUFF_SIZE for empty files (size 0) so the read loop can still run.
                        let buf_size = if buf_size == 0 {
                            MAX_BUFF_SIZE
                        } else {
                            buf_size
                        };
                        let mut buffer = vec![0u8; buf_size];
                        loop {
                            let n = local_file
                                .read(&mut buffer)
                                .map_err(|e| format!("File read error: {}", e))?;
                            if n == 0 {
                                break;
                            }
                            remote_file
                                .write_all(&buffer[..n])
                                .map_err(|e| format!("Remote write error: {}", e))?;
                        }
                        remote_file
                            .close()
                            .map_err(|e| format!("Close error: {}", e))?;
                        #[cfg(unix)]
                        if preserve_permissions {
                            use std::os::unix::fs::PermissionsExt;
                            let mode = metadata.permissions().mode();
                            let _ = self.sftp().setstat(
                                Path::new(&remote_entry_str),
                                ssh2::FileStat {
                                    perm: Some(mode),
                                    size: None,
                                    uid: None,
                                    gid: None,
                                    atime: None,
                                    mtime: None,
                                },
                            );
                        }
                        Ok(())
                    })();
                    match result {
                        Ok(_) => transferred.push(local_entry_str),
                        Err(e) => {
                            if fail_fast {
                                return Err(PyErr::new::<PyIOError, _>(e));
                            }
                            failed.push(local_entry_str);
                        }
                    }
                }
                // Symlinks with follow_symlinks=false are skipped
            }
        }

        Ok((transferred, failed))
    }

    /// Downloads a remote directory recursively to a local path over SFTP.
    /// Returns a tuple of (transferred_files, failed_files), where each is a list of remote file paths.
    /// If `fail_fast` is true, the first transfer error raises an exception immediately.
    #[pyo3(signature = (remote_path, local_path, follow_symlinks=true, preserve_permissions=true, fail_fast=false))]
    fn sftp_get_dir(
        &mut self,
        remote_path: String,
        local_path: String,
        follow_symlinks: bool,
        preserve_permissions: bool,
        fail_fast: bool,
    ) -> PyResult<(Vec<String>, Vec<String>)> {
        self.prepare_sftp()?;
        let mut transferred: Vec<String> = Vec::new();
        let mut failed: Vec<String> = Vec::new();

        // Ensure local base directory exists
        if let Err(e) = std::fs::create_dir_all(&local_path) {
            let msg = format!("Failed to create local directory '{}': {}", local_path, e);
            if fail_fast {
                return Err(PyErr::new::<PyIOError, _>(msg));
            }
            failed.push(remote_path);
            return Ok((transferred, failed));
        }

        // Stack for depth-first traversal: (remote_dir_str, local_dir).
        // Remote paths are kept as POSIX strings to avoid OS-native path separators on Windows.
        let mut dirs_to_process: Vec<(String, std::path::PathBuf)> =
            vec![(remote_path.clone(), std::path::PathBuf::from(&local_path))];

        while let Some((remote_dir, local_dir)) = dirs_to_process.pop() {
            let remote_dir_str = remote_dir.clone();
            let entries = match self.sftp().readdir(Path::new(&remote_dir_str)) {
                Ok(e) => e,
                Err(e) => {
                    let msg = format!(
                        "Failed to read remote directory '{}': {}",
                        remote_dir_str, e
                    );
                    if fail_fast {
                        return Err(PyErr::new::<PyIOError, _>(msg));
                    }
                    failed.push(remote_dir_str);
                    continue;
                }
            };

            for (entry_name, stat) in entries {
                let file_name = match entry_name.file_name() {
                    Some(n) => n.to_os_string(),
                    None => {
                        if fail_fast {
                            return Err(PyErr::new::<PyIOError, _>("Invalid entry path"));
                        }
                        continue;
                    }
                };
                let file_name_str = file_name.to_string_lossy();
                let local_entry = local_dir.join(&file_name);
                // Build remote path with POSIX separator to stay cross-platform.
                let remote_entry_str = format!("{}/{}", remote_dir, file_name_str);

                // When follow_symlinks=true, resolve symlinks on remote via stat()
                let resolved_stat = if follow_symlinks && stat.file_type().is_symlink() {
                    self.sftp()
                        .stat(Path::new(&remote_entry_str))
                        .unwrap_or(stat)
                } else {
                    stat
                };

                if resolved_stat.file_type().is_symlink() {
                    // follow_symlinks=false: skip symlinks
                    continue;
                } else if resolved_stat.is_dir() {
                    match std::fs::create_dir_all(&local_entry) {
                        Ok(_) => {}
                        Err(e) => {
                            if fail_fast {
                                return Err(PyErr::new::<PyIOError, _>(format!(
                                    "Failed to create local directory '{}': {}",
                                    local_entry.display(),
                                    e
                                )));
                            }
                            failed.push(remote_entry_str);
                            continue;
                        }
                    }
                    #[cfg(unix)]
                    if preserve_permissions {
                        if let Some(perm) = resolved_stat.perm {
                            use std::os::unix::fs::PermissionsExt;
                            let _ = std::fs::set_permissions(
                                &local_entry,
                                std::fs::Permissions::from_mode(perm & 0o7777),
                            );
                        }
                    }
                    dirs_to_process.push((remote_entry_str, local_entry));
                } else if resolved_stat.is_file() {
                    let result: Result<(), String> = (|| {
                        let mut remote_file = BufReader::new(
                            self.sftp()
                                .open(Path::new(&remote_entry_str))
                                .map_err(|e| format!("SFTP open error: {}", e))?,
                        );
                        let local_file = std::fs::File::create(&local_entry)
                            .map_err(|e| format!("File create error: {}", e))?;
                        let mut writer = BufWriter::new(local_file);
                        let mut buffer = vec![0u8; MAX_BUFF_SIZE];
                        loop {
                            let n = remote_file
                                .read(&mut buffer)
                                .map_err(|e| format!("File read error: {}", e))?;
                            if n == 0 {
                                break;
                            }
                            writer
                                .write_all(&buffer[..n])
                                .map_err(|e| format!("File write error: {}", e))?;
                        }
                        writer.flush().map_err(|e| format!("Flush error: {}", e))?;
                        #[cfg(unix)]
                        if preserve_permissions {
                            if let Some(perm) = resolved_stat.perm {
                                use std::os::unix::fs::PermissionsExt;
                                let _ = std::fs::set_permissions(
                                    &local_entry,
                                    std::fs::Permissions::from_mode(perm & 0o7777),
                                );
                            }
                        }
                        Ok(())
                    })();
                    match result {
                        Ok(_) => transferred.push(remote_entry_str),
                        Err(e) => {
                            if fail_fast {
                                return Err(PyErr::new::<PyIOError, _>(e));
                            }
                            failed.push(remote_entry_str);
                        }
                    }
                }
            }
        }

        Ok((transferred, failed))
    }

    // Copy a file from this connection to another connection
    #[pyo3(signature = (source_path, dest_conn, dest_path=None))]
    fn remote_copy(
        &mut self,
        source_path: String,
        dest_conn: &mut Connection,
        dest_path: Option<String>,
    ) -> PyResult<()> {
        self.prepare_sftp()?;
        dest_conn.prepare_sftp()?;
        let mut remote_file = BufReader::new(
            self.sftp()
                .open(Path::new(&source_path))
                .map_err(|e| PyErr::new::<PyIOError, _>(format!("Remote open error: {}", e)))?,
        );
        let dest_path = dest_path.unwrap_or_else(|| source_path.clone());
        let mut other_file = dest_conn
            .sftp()
            .create(Path::new(&dest_path))
            .map_err(|e| PyErr::new::<PyIOError, _>(format!("Dest file creation error: {}", e)))?;
        let mut buffer = vec![0; MAX_BUFF_SIZE];
        loop {
            let len = remote_file
                .read(&mut buffer)
                .map_err(|e| PyErr::new::<PyIOError, _>(format!("File read error: {}", e)))?;
            if len == 0 {
                break;
            }
            other_file
                .write_all(&buffer[..len])
                .map_err(|e| PyErr::new::<PyIOError, _>(format!("File write error: {}", e)))?;
        }
        Ok(())
    }

    /// Return a FileTailer instance given a remote file path
    /// This is best used as a context manager, but can be used directly
    /// ```python
    /// with conn.tail("remote_file.log") as tailer:
    ///     time.sleep(5)  # wait or perform other operations
    ///     print(tailer.read())
    ///     time.sleep(5)  # wait or perform other operations
    /// print(tailer.contents)
    /// ```
    fn tail(&mut self, remote_file: String) -> PyResult<FileTailer> {
        FileTailer::new(self, remote_file, None)
    }

    /// Close the connection's session
    fn close(&mut self) -> PyResult<()> {
        self.closed = true;
        self.sftp_conn = None;
        let _ = self.session.disconnect(None, "Bye from Hussh", None);
        if let Some(child) = self.proxy_process.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.proxy_process = None;
        Ok(())
    }

    /// Provide an enter for the context manager
    fn __enter__(slf: PyRef<Self>) -> PyRef<Self> {
        slf
    }

    /// Provide an exit for the context manager
    /// This will close the session
    #[pyo3(signature = (_exc_type=None, _exc_value=None, _traceback=None))]
    fn __exit__(
        &mut self,
        _exc_type: Option<&Bound<'_, PyAny>>,
        _exc_value: Option<&Bound<'_, PyAny>>,
        _traceback: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let _ = self.close();
        Ok(())
    }

    fn __repr__(&self) -> PyResult<String> {
        Ok(format!(
            "Connection(host={}, port={}, username={}, password=*****)",
            self.host, self.port, self.username
        ))
    }

    /// Creates an `InteractiveShell` instance.
    /// If `pty` is `true`, a pseudo-terminal is requested for the shell.
    /// Note: This is best used as a context manager
    /// ```python
    /// with conn.shell() as shell:
    ///     shell.send("ls")
    ///     shell.send("pwd")
    /// print(shell.result.stdout)
    /// ```
    #[pyo3(signature = (pty=None))]
    fn shell(&mut self, pty: Option<bool>) -> PyResult<InteractiveShell> {
        let mut channel = self.prepare_channel(None)?;
        if let Some(pty) = pty {
            if pty {
                channel
                    .request_pty("xterm", None, None)
                    .map_err(ssh_error)?;
            }
        }
        channel.shell().map_err(ssh_error)?;
        Ok(InteractiveShell {
            channel: ChannelWrapper { channel },
            pty: pty.unwrap_or(false),
            result: None,
        })
    }
}

#[pyclass(from_py_object)]
#[derive(Clone)]
pub struct ChannelWrapper {
    channel: Channel,
}

#[pyclass(from_py_object)]
#[derive(Clone)]
pub struct InteractiveShell {
    channel: ChannelWrapper,
    pty: bool,
    #[pyo3(get)]
    result: Option<SSHResult>,
}

#[pymethods]
impl InteractiveShell {
    #[new]
    fn new(channel: ChannelWrapper, pty: bool) -> Self {
        InteractiveShell {
            channel,
            pty,
            result: None,
        }
    }

    /// Reads the output from the shell and returns an `SSHResult`.
    /// Note: This sends an EOF to the shell, so you won't be able to send more commands after calling `read`.
    fn read(&mut self) -> PyResult<SSHResult> {
        self.channel
            .channel
            .flush()
            .map_err(|e| PyErr::new::<PyTimeoutError, _>(format!("Channel flush error: {}", e)))?;
        self.channel
            .channel
            .send_eof()
            .map_err(|e| PyErr::new::<PyTimeoutError, _>(format!("Send EOF error: {}", e)))?;
        match read_from_channel(&mut self.channel.channel) {
            Ok(result) => Ok(result),
            Err(e) => {
                self.channel.channel.close().map_err(|e| {
                    PyErr::new::<PyTimeoutError, _>(format!("Channel close error: {}", e))
                })?;
                self.result = None;
                Err(e)
            }
        }
    }

    /// Sends a command to the shell.
    /// If you don't want to add a newline at the end of the command, set `add_newline` to `false`.
    #[pyo3(signature = (data, add_newline=None))]
    fn send(&mut self, data: String, add_newline: Option<bool>) -> PyResult<()> {
        let add_newline = add_newline.unwrap_or(true);
        let data = if add_newline && !data.ends_with('\n') {
            format!("{}\n", data)
        } else {
            data
        };
        self.channel
            .channel
            .write_all(data.as_bytes())
            .map_err(|e| PyIOError::new_err(e.to_string()))?;
        Ok(())
    }

    /// Closes the shell.
    fn close(&mut self) -> PyResult<()> {
        self.channel.channel.close().map_err(ssh_error)?;
        Ok(())
    }

    fn __enter__(slf: PyRef<Self>) -> PyRef<Self> {
        slf
    }

    #[pyo3(signature = (_exc_type=None, _exc_value=None, _traceback=None))]
    fn __exit__(
        &mut self,
        _exc_type: Option<&Bound<'_, PyAny>>,
        _exc_value: Option<&Bound<'_, PyAny>>,
        _traceback: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        if self.pty {
            self.send("exit\n".to_string(), Some(false))?;
        }
        // Try the normal read flow first (for commands that complete normally)
        match self.read() {
            Ok(result) => {
                self.result = Some(result);
            }
            Err(e) => {
                // Check if this is a timeout error (expected for long-running commands)
                // by examining the error string
                let error_msg = e.to_string();
                if error_msg.contains("Timeout") || error_msg.contains("timeout") {
                    // Timeout error - set an empty result to allow clean exit
                    self.result = Some(SSHResult {
                        stdout: String::new(),
                        stderr: String::new(),
                        status: -1,
                    });
                } else {
                    // Other I/O errors - propagate them as they may indicate unexpected issues
                    return Err(e);
                }
            }
        }
        Ok(())
    }
}

/// `FileTailer` is a structure that represents a remote file tailer.
///
/// It maintains an SFTP connection and the path to a remote file,
/// and allows reading from a specified position in the file.
///
/// # Fields
///
/// * `sftp_conn`: An SFTP connection from the ssh2 crate.
/// * `remote_file`: A string representing the path to the remote file.
/// * `init_pos`: An optional initial position from where to start reading the file.
/// * `last_pos`: The last position read from the file.
/// * `contents`: The contents read from the file.
///
/// # Methods
///
/// * `new`: Constructs a new `FileTailer`.
/// * `seek_end`: Seeks to the end of the remote file.
/// * `read`: Reads the contents of the remote file from a given position.
/// * `__enter__`: Prepares the `FileTailer` for use in a `with` statement.
/// * `__exit__`: Cleans up after the `FileTailer` is used in a `with` statement.
#[pyclass]
pub struct FileTailer {
    sftp_conn: ssh2::Sftp,
    #[pyo3(get)]
    remote_file: String,
    init_pos: Option<u64>,
    #[pyo3(get)]
    last_pos: u64,
    #[pyo3(get)]
    contents: Option<String>,
}

#[pymethods]
impl FileTailer {
    #[new]
    #[pyo3(signature = (conn, remote_file, init_pos=None))]
    fn new(
        conn: &mut Connection,
        remote_file: String,
        init_pos: Option<u64>,
    ) -> PyResult<FileTailer> {
        let sftp_conn = conn.prepare(|connection| {
            connection
                .session
                .sftp()
                .map_err(|error| connection.preparation_error(error))
        })?;
        Ok(FileTailer {
            sftp_conn,
            remote_file,
            init_pos,
            last_pos: 0,
            contents: None,
        })
    }

    // Determine the current end of the remote file
    fn seek_end(&mut self) -> PyResult<Option<u64>> {
        let metadata = self
            .sftp_conn
            .stat(Path::new(&self.remote_file))
            .map_err(|e| PyErr::new::<PyIOError, _>(format!("Stat error: {}", e)))?;
        self.last_pos = metadata.size.unwrap_or(0);
        if self.init_pos.is_none() {
            self.init_pos = metadata.size;
        }
        Ok(metadata.size)
    }

    // Read the contents of the remote file from a given position
    #[pyo3(signature = (from_pos=None))]
    fn read(&mut self, from_pos: Option<u64>) -> PyResult<String> {
        let from_pos = from_pos.unwrap_or(self.last_pos);
        let mut remote_file = BufReader::new(
            self.sftp_conn
                .open(Path::new(&self.remote_file))
                .map_err(ssh_error)?,
        );
        remote_file
            .seek(std::io::SeekFrom::Start(from_pos))
            .map_err(|e| PyIOError::new_err(e.to_string()))?;
        let mut contents = String::new();
        remote_file
            .read_to_string(&mut contents)
            .map_err(|e| PyIOError::new_err(e.to_string()))?;
        self.last_pos = remote_file
            .stream_position()
            .map_err(|e| PyIOError::new_err(e.to_string()))?;
        Ok(contents)
    }

    fn __enter__(mut slf: PyRefMut<Self>) -> PyResult<PyRefMut<Self>> {
        slf.seek_end()?;
        Ok(slf)
    }

    #[pyo3(signature = (_exc_type=None, _exc_value=None, _traceback=None))]
    fn __exit__(
        &mut self,
        _exc_type: Option<&Bound<'_, PyAny>>,
        _exc_value: Option<&Bound<'_, PyAny>>,
        _traceback: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        self.contents = Some(self.read(self.init_pos)?);
        Ok(())
    }
}
