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
//! Multiple forms of authentication are supported. You can use a password, a private key, or the default ssh-agent.
//!
//! ```python
//! conn = Connection("my.test.server", username="user", private_key="~/.ssh/id_rsa")
//! conn = Connection("my.test.server", username="user", password="pass")
//! conn = Connection("my.test.server", username="user")
//! ````
//!
//! If you don't pass a port, the default SSH port (22) is used.
//! If you don't pass a username, "root" is used.
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
use pyo3::create_exception;
use pyo3::prelude::*;
use ssh2::{Channel, Session};
use std::io::{BufReader, BufWriter, Read, Seek, Write};
use std::net::TcpStream;
use std::path::Path;

use pyo3::exceptions::{PyIOError, PyTimeoutError};

const MAX_BUFF_SIZE: usize = 65536;
create_exception!(
    connection,
    AuthenticationError,
    pyo3::exceptions::PyException
);

fn read_from_channel(channel: &mut Channel) -> Result<SSHResult, PyErr> {
    let mut stdout = String::new();
    channel
        .read_to_string(&mut stdout)
        .map_err(|e| PyErr::new::<PyTimeoutError, _>(format!("Timeout reading stdout: {}", e)))?;
    let mut stderr = String::new();
    channel
        .stderr()
        .read_to_string(&mut stderr)
        .map_err(|e| PyErr::new::<PyTimeoutError, _>(format!("Timeout reading stderr: {}", e)))?;
    channel.wait_close().map_err(|e| {
        PyErr::new::<PyTimeoutError, _>(format!("Timeout waiting for channel to close: {}", e))
    })?;
    let status = channel.exit_status().map_err(|e| {
        PyErr::new::<PyTimeoutError, _>(format!("Timeout getting exit status: {}", e))
    })?;
    Ok(SSHResult {
        stdout,
        stderr,
        status,
    })
}

#[pyclass]
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
    sftp_conn: Option<ssh2::Sftp>,
}

// Non-public methods for the Connection class
impl Connection {
    // Emulate a python-like sftp property
    fn sftp(&mut self) -> &ssh2::Sftp {
        if self.sftp_conn.is_none() {
            self.sftp_conn = Some(self.session.sftp().unwrap());
        }
        self.sftp_conn.as_ref().unwrap()
    }
}

#[pymethods]
impl Connection {
    #[new]
    #[pyo3(signature = (host, port=22, username="root", password=None, private_key=None, timeout=0))]
    fn new(
        host: &str,
        port: Option<i32>,
        username: Option<&str>,
        password: Option<&str>,
        private_key: Option<&str>,
        timeout: Option<u32>,
    ) -> PyResult<Connection> {
        // if port isn't set, use the default ssh port 22
        let port = port.unwrap_or(22);
        // combine the host and port into a single string
        let conn_str = format!("{}:{}", host, port);
        let tcp_conn = TcpStream::connect(conn_str)
            .map_err(|e| PyErr::new::<PyTimeoutError, _>(format!("{}", e)))?;
        let mut session = Session::new().unwrap();
        // if a timeout is set, use it
        let timeout = timeout.unwrap_or(0);
        session.set_timeout(timeout);
        session.set_tcp_stream(tcp_conn);
        session
            .handshake()
            .map_err(|e| PyErr::new::<PyTimeoutError, _>(format!("{}", e)))?;
        // if username isn't set, try using root
        let username = username.unwrap_or("root");
        let password = password.unwrap_or("");
        let private_key = private_key.unwrap_or("");
        // if private_key is set, use it to authenticate
        if !private_key.is_empty() {
            // If a user uses a tilde to represent the home directory,
            // replace it with the actual home directory
            let private_key = shellexpand::tilde(private_key).into_owned();
            // if a password is set, use it to decrypt the private key
            if !password.is_empty() {
                session
                    .userauth_pubkey_file(username, None, Path::new(&private_key), Some(password))
                    .map_err(|e| PyErr::new::<AuthenticationError, _>(format!("{}", e)))?;
            } else {
                // otherwise, try using the private key without a passphrase
                session
                    .userauth_pubkey_file(username, None, Path::new(&private_key), None)
                    .map_err(|e| PyErr::new::<AuthenticationError, _>(format!("{}", e)))?;
            }
        } else if !password.is_empty() {
            session
                .userauth_password(username, password)
                .map_err(|e| PyErr::new::<AuthenticationError, _>(format!("{}", e)))?;
        } else {
            // if password isn't set, try using the default ssh-agent
            if session.userauth_agent(username).is_err() {
                return Err(PyErr::new::<AuthenticationError, _>(
                    "Failed to authenticate with ssh-agent",
                ));
            }
        }
        Ok(Connection {
            session,
            port,
            host: host.to_string(),
            username: username.to_string(),
            password: password.to_string(),
            private_key: private_key.to_string(),
            timeout,
            sftp_conn: None,
        })
    }

    /// Executes a command over the SSH connection and returns the result.
    /// If `timeout` is provided, it temporarily updates the session timeout for the duration of the command execution.
    #[pyo3(signature = (command, timeout=None))]
    fn execute(&self, command: String, timeout: Option<u32>) -> PyResult<SSHResult> {
        let original_timeout = self.session.timeout();
        if let Some(t) = timeout {
            self.session.set_timeout(t);
        }

        let mut channel = self.session.channel_session().map_err(|e| {
            PyErr::new::<PyTimeoutError, _>(format!(
                "Timed out establishing channel session.\n{}",
                e
            ))
        })?;
        // exec is non-blocking, so we don't check for a timeout here, but in read_from_channel
        channel.exec(&command).unwrap();
        let result = match read_from_channel(&mut channel) {
            Ok(res) => res,
            Err(e) => {
                self.session.set_timeout(original_timeout);
                return Err(e);
            }
        };
        self.session.set_timeout(original_timeout);
        Ok(result)
    }

    /// Reads a file over SCP and returns the contents.
    /// If `local_path` is provided, the file is saved to the local system.
    /// Otherwise, the contents of the file are returned as a string.
    #[pyo3(signature = (remote_path, local_path=None))]
    fn scp_read(&self, remote_path: String, local_path: Option<String>) -> PyResult<String> {
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
    fn scp_write(&self, local_path: String, remote_path: String) -> PyResult<()> {
        // if remote_path is a directory, append the local file name to the remote path
        let remote_path = if remote_path.ends_with('/') {
            format!(
                "{}/{}",
                remote_path,
                Path::new(&local_path)
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
            )
        } else {
            remote_path
        };
        let mut local_file = std::fs::File::open(&local_path)
            .map_err(|e| PyErr::new::<PyIOError, _>(format!("Local file open error: {}", e)))?;
        let metadata = local_file.metadata().unwrap();
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
        remote_file.flush().unwrap();
        remote_file.send_eof().unwrap();
        remote_file.wait_eof().unwrap();
        remote_file.close().unwrap();
        remote_file.wait_close().unwrap();
        Ok(())
    }

    /// Writes data over SCP.
    fn scp_write_data(&self, data: String, remote_path: String) -> PyResult<()> {
        let mut remote_file = self
            .session
            .scp_send(Path::new(&remote_path), 0o644, data.len() as u64, None)
            .map_err(|e| PyErr::new::<PyIOError, _>(format!("scp_send error: {}", e)))?;
        remote_file
            .write_all(data.as_bytes())
            .map_err(|e| PyErr::new::<PyIOError, _>(format!("Data write error: {}", e)))?;
        remote_file.send_eof().unwrap();
        remote_file.wait_eof().unwrap();
        remote_file.close().unwrap();
        remote_file.wait_close().unwrap();
        Ok(())
    }

    /// Reads a file over SFTP and returns the contents.
    /// If `local_path` is provided, the file is saved to the local system.
    /// Otherwise, the contents of the file are returned as a string.
    #[pyo3(signature = (remote_path, local_path=None))]
    fn sftp_read(&mut self, remote_path: String, local_path: Option<String>) -> PyResult<String> {
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
        let mut local_file = std::fs::File::open(&local_path)
            .map_err(|e| PyErr::new::<PyIOError, _>(format!("Local file open error: {}", e)))?;
        let remote_path = remote_path.unwrap_or_else(|| local_path.clone());
        let metadata = local_file.metadata().unwrap();
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
        remote_file.close().unwrap();
        Ok(())
    }

    /// Writes data over SFTP.
    fn sftp_write_data(&mut self, data: String, remote_path: String) -> PyResult<()> {
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

    // Copy a file from this connection to another connection
    #[pyo3(signature = (source_path, dest_conn, dest_path=None))]
    fn remote_copy(
        &self,
        source_path: String,
        dest_conn: &mut Connection,
        dest_path: Option<String>,
    ) -> PyResult<()> {
        let mut remote_file = BufReader::new(
            self.session
                .sftp()
                .map_err(|e| PyErr::new::<PyIOError, _>(format!("SFTP error: {}", e)))?
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
    fn tail(&self, remote_file: String) -> FileTailer {
        FileTailer::new(self, remote_file, None)
    }

    /// Close the connection's session
    fn close(&self) -> PyResult<()> {
        self.session
            .disconnect(None, "Bye from Hussh", None)
            .unwrap();
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
    fn shell(&self, pty: Option<bool>) -> PyResult<InteractiveShell> {
        let mut channel = self.session.channel_session().unwrap();
        if let Some(pty) = pty {
            if pty {
                channel.request_pty("xterm", None, None).unwrap();
            }
        }
        channel.shell().unwrap();
        Ok(InteractiveShell {
            channel: ChannelWrapper { channel },
            pty: pty.unwrap_or(false),
            result: None,
        })
    }
}

#[pyclass]
#[derive(Clone)]
pub struct ChannelWrapper {
    channel: Channel,
}

#[pyclass]
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
        self.channel.channel.write_all(data.as_bytes()).unwrap();
        Ok(())
    }

    /// Closes the shell.
    fn close(&mut self) -> PyResult<()> {
        self.channel.channel.close().unwrap();
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
            self.send("exit\n".to_string(), Some(false)).unwrap();
        }
        self.result = Some(self.read()?);
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
    fn new(conn: &Connection, remote_file: String, init_pos: Option<u64>) -> FileTailer {
        FileTailer {
            sftp_conn: conn.session.sftp().unwrap(),
            remote_file,
            init_pos,
            last_pos: 0,
            contents: None,
        }
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
    fn read(&mut self, from_pos: Option<u64>) -> String {
        let from_pos = from_pos.unwrap_or(self.last_pos);
        let mut remote_file = BufReader::new(
            self.sftp_conn
                .open(Path::new(&self.remote_file))
                .expect("Opening remote file failed"),
        );
        remote_file
            .seek(std::io::SeekFrom::Start(from_pos))
            .unwrap();
        let mut contents = String::new();
        remote_file.read_to_string(&mut contents).unwrap();
        self.last_pos = remote_file.stream_position().unwrap();
        contents
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
        self.contents = Some(self.read(self.init_pos));
        Ok(())
    }
}
