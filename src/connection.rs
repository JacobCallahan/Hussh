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
//! You can send commands to the shell using the `send` method, then get the results from exit_result when you exit the context manager.
//! Due to the nature of reading from the shell, do not use the `read` method if you want to send more commands.
//!
//! ```python
//! with conn.shell() as shell:
//!    shell.send("ls")
//!    shell.send("pwd")
//!    shell.send("whoami")
//!
//! print(shell.exit_result.stdout)
//! ```
//!
//! Note: The `read` method sends an EOF to the shell, so you won't be able to send more commands after calling `read`. If you want to send more commands, you would need to create a new `InteractiveShell` instance.
use pyo3::create_exception;
use pyo3::exceptions::PyTimeoutError;
use pyo3::prelude::*;
use ssh2::{Channel, Session};
use std::io::{BufReader, BufWriter, Read, Write};
use std::net::TcpStream;
use std::path::Path;
// use ssh2::FileStat;

const MAX_BUFF_SIZE: usize = 65536;
create_exception!(
    connection,
    AuthenticationError,
    pyo3::exceptions::PyException
);

fn read_from_channel(channel: &mut Channel) -> SSHResult {
    let mut stdout = String::new();
    channel.read_to_string(&mut stdout).unwrap();
    let mut stderr = String::new();
    channel.stderr().read_to_string(&mut stderr).unwrap();
    channel.wait_close().unwrap();
    let status = channel.exit_status().unwrap();
    SSHResult {
        stdout,
        stderr,
        status,
    }
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
            "stdout:\n{}stderr:\n{}status: {}",
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
///
/// ## Methods
///
/// ### `new`
///
/// Creates a new `Connection` instance. It takes the following parameters:
///
/// * `host`: The host to connect to.
/// * `port`: The port to connect to. If not provided, the default SSH port (22) is used.
/// * `username`: The username to use for authentication. If not provided, "root" is used.
/// * `password`: The password to use for authentication. If not provided, an empty string is used.
/// * `private_key`: The path to the private key to use for authentication. If not provided, an empty string is used.
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
/// ### `__repr__`
///
/// Returns a string representation of the `Connection` instance.
///
/// ### `shell`
///
/// Creates an `InteractiveShell` instance. It takes the following parameter:
///
/// * `pty`: Whether to request a pseudo-terminal for the shell. If not provided, a pseudo-terminal is not requested.
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

#[pymethods]
impl Connection {
    #[new]
    fn new(
        host: String,
        port: Option<i32>,
        username: Option<String>,
        password: Option<String>,
        private_key: Option<String>,
        timeout: Option<u32>,
    ) -> PyResult<Connection> {
        // if port isn't set, use the default ssh port 22
        let port = port.unwrap_or(22);
        // combine the host and port into a single string
        let conn_str = format!("{}:{}", host, port);
        let tcp_conn = TcpStream::connect(&conn_str)
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
        let username = username.unwrap_or("root".to_string());
        let password = password.unwrap_or("".to_string());
        let private_key = private_key.unwrap_or("".to_string());
        // if private_key is set, use it to authenticate
        if private_key != "" {
            // if a password is set, use it to decrypt the private key
            if password != "" {
                session
                    .userauth_pubkey_file(&username, None, Path::new(&private_key), Some(&password))
                    .map_err(|e| PyErr::new::<AuthenticationError, _>(format!("{}", e)))?;
            } else {
                // otherwise, try using the private key without a passphrase
                session
                    .userauth_pubkey_file(&username, None, Path::new(&private_key), None)
                    .map_err(|e| PyErr::new::<AuthenticationError, _>(format!("{}", e)))?;
            }
        } else if password != "" {
            session
                .userauth_password(&username, &password)
                .map_err(|e| PyErr::new::<AuthenticationError, _>(format!("{}", e)))?;
        } else {
            // if password isn't set, try using the default ssh-agent
            if session.userauth_agent(&username).is_err() {
                return Err(PyErr::new::<AuthenticationError, _>(
                    "Failed to authenticate with ssh-agent",
                ));
            }
        }
        Ok(Connection {
            session,
            port,
            host,
            username,
            password,
            private_key,
            timeout,
            sftp_conn: None,
        })
    }

    /// Executes a command over the SSH connection and returns the result.
    fn execute(&self, command: String) -> PyResult<SSHResult> {
        let mut channel = self.session.channel_session().unwrap();
        if let Err(e) = channel.exec(&command) {
            return Err(PyErr::new::<PyTimeoutError, _>(format!("{}", e)));
        }
        Ok(read_from_channel(&mut channel))
    }

    /// Reads a file over SCP and returns the contents.
    /// If `local_path` is provided, the file is saved to the local system.
    /// Otherwise, the contents of the file are returned as a string.
    fn scp_read(&self, remote_path: String, local_path: Option<String>) -> PyResult<String> {
        let (mut remote_file, stat) = self.session.scp_recv(Path::new(&remote_path)).unwrap();
        match local_path {
            Some(local_path) => {
                let mut local_file = std::fs::File::create(local_path).unwrap();
                let mut buffer = vec![0; std::cmp::min(stat.size() as usize, MAX_BUFF_SIZE)];
                loop {
                    let len = remote_file.read(&mut buffer).unwrap();
                    if len == 0 {
                        break;
                    }
                    local_file.write_all(&buffer[..len]).unwrap();
                }
                Ok("Ok".to_string())
            }
            None => {
                let mut contents = String::new();
                remote_file.read_to_string(&mut contents).unwrap();
                Ok(contents)
            }
        }
    }

    /// Writes a file over SCP.
    fn scp_write(&self, local_path: String, remote_path: String) -> PyResult<()> {
        // if remote_path is a directory, append the local file name to the remote path
        let remote_path = if remote_path.ends_with("/") {
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
        let mut local_file = std::fs::File::open(&local_path).unwrap();
        let metadata = local_file.metadata().unwrap();
        // TODO: better handle permissions. Perhaps from metadata.permissions()?
        let mut remote_file = self
            .session
            .scp_send(Path::new(&remote_path), 0o644, metadata.len(), None)
            .unwrap();
        // create a variable-sized buffer to read the file and loop until EOF
        let mut read_buffer = vec![0; std::cmp::min(metadata.len() as usize, MAX_BUFF_SIZE)];
        loop {
            let bytes_read = local_file.read(&mut read_buffer).unwrap();
            if bytes_read == 0 {
                break;
            }
            remote_file.write(&read_buffer[..bytes_read]).unwrap();
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
            .unwrap();
        remote_file.write_all(data.as_bytes()).unwrap();
        remote_file.send_eof().unwrap();
        remote_file.wait_eof().unwrap();
        remote_file.close().unwrap();
        remote_file.wait_close().unwrap();
        Ok(())
    }

    /// Reads a file over SFTP and returns the contents.
    /// If `local_path` is provided, the file is saved to the local system.
    /// Otherwise, the contents of the file are returned as a string.
    fn sftp_read(&mut self, remote_path: String, local_path: Option<String>) -> PyResult<String> {
        if self.sftp_conn.is_none() {
            self.sftp_conn = Some(self.session.sftp().unwrap());
        }
        let mut remote_file = BufReader::new(
            self.sftp_conn
                .as_ref()
                .unwrap()
                .open(Path::new(&remote_path))
                .unwrap(),
        );
        match local_path {
            Some(local_path) => {
                let local_file = std::fs::File::create(local_path)?;
                let mut writer = BufWriter::new(local_file);
                let mut buffer = vec![0; MAX_BUFF_SIZE];
                loop {
                    let len = remote_file.read(&mut buffer)?;
                    if len == 0 {
                        break;
                    }
                    writer.write_all(&buffer[..len])?;
                }
                writer.flush()?;
                Ok("Ok".to_string())
            }
            None => {
                let mut contents = String::new();
                remote_file.read_to_string(&mut contents)?;
                Ok(contents)
            }
        }
    }

    /// Writes a file over SFTP.
    fn sftp_write(&mut self, local_path: String, remote_path: String) -> PyResult<()> {
        let mut local_file = std::fs::File::open(&local_path).unwrap();
        let metadata = local_file.metadata().unwrap();
        // If we don't already have an SFTP connection, create one
        if self.sftp_conn.is_none() {
            self.sftp_conn = Some(self.session.sftp().unwrap());
        }
        let mut remote_file = self
            .sftp_conn
            .as_ref()
            .unwrap()
            .create(Path::new(&remote_path))
            .unwrap();
        // create a variable-sized buffer to read the file and loop until EOF
        let mut read_buffer = vec![0; std::cmp::min(metadata.len() as usize, MAX_BUFF_SIZE)];
        loop {
            let bytes_read = local_file.read(&mut read_buffer)?;
            if bytes_read == 0 {
                break;
            }
            remote_file.write(&read_buffer[..bytes_read])?;
        }
        // let stat = FileStat {
        //     size: None,
        //     uid: None,
        //     gid: None,
        //     perm: Some(0o644),
        //     atime: None,
        //     mtime: None,
        // };
        // remote_file.setstat(stat).unwrap();
        remote_file.close().unwrap();
        Ok(())
    }

    /// Writes data over SFTP.
    fn sftp_write_data(&self, data: String, remote_path: String) -> PyResult<()> {
        let mut remote_file = self
            .session
            .sftp()
            .unwrap()
            .create(Path::new(&remote_path))
            .unwrap();
        remote_file.write_all(data.as_bytes()).unwrap();
        remote_file.close().unwrap();
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
    ///   shell.send("ls")
    ///  shell.send("pwd")
    /// print(shell.exit_result.stdout)
    /// ```
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
            exit_result: None,
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
struct InteractiveShell {
    channel: ChannelWrapper,
    #[pyo3(get)]
    exit_result: Option<SSHResult>,
}

#[pymethods]
impl InteractiveShell {
    #[new]
    fn new(channel: ChannelWrapper) -> Self {
        InteractiveShell {
            channel,
            exit_result: None,
        }
    }

    /// Reads the output from the shell and returns an `SSHResult`.
    /// Note: This sends an EOF to the shell, so you won't be able to send more commands after calling `read`.
    fn read(&mut self) -> SSHResult {
        self.channel.channel.flush().unwrap();
        self.channel.channel.send_eof().unwrap();
        read_from_channel(&mut self.channel.channel)
    }

    /// Sends a command to the shell.
    /// If you don't want to add a newline at the end of the command, set `add_newline` to `false`.
    fn send(&mut self, data: String, add_newline: Option<bool>) -> PyResult<()> {
        let add_newline = add_newline.unwrap_or(true);
        let data = if add_newline && !data.ends_with("\n") {
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

    fn __exit__(
        &mut self,
        _exc_type: Option<&PyAny>,
        _exc_value: Option<&PyAny>,
        _traceback: Option<&PyAny>,
    ) -> PyResult<()> {
        self.exit_result = Some(self.read());
        Ok(())
    }
}
