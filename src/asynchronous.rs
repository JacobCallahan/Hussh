use async_trait::async_trait;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use russh::client::{Config, Handle, Handler};
use russh_sftp::client::SftpSession;
use std::path::Path;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;

#[derive(Clone)]
struct ClientHandler;

impl From<(russh::ChannelId, russh::ChannelMsg)> for ClientHandler {
    fn from(_: (russh::ChannelId, russh::ChannelMsg)) -> Self {
        ClientHandler
    }
}

#[async_trait]
impl Handler for ClientHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _server_public_key: &russh_keys::key::PublicKey,
    ) -> Result<bool, Self::Error> {
        // For now, we blindly accept keys (MVP).
        Ok(true)
    }
}

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
    config: Arc<Config>,
}

#[pymethods]
impl AsyncConnection {
    #[new]
    #[pyo3(signature = (host, username=None, password=None, key_path=None, port=22, keepalive_interval=0))]
    fn new(
        host: String,
        username: Option<String>,
        password: Option<String>,
        key_path: Option<String>,
        port: u16,
        keepalive_interval: u64,
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
            config: Arc::new(config),
        }
    }

    fn connect<'p>(&self, py: Python<'p>) -> PyResult<Bound<'p, PyAny>> {
        let config = self.config.clone();
        let host = self.host.clone();
        let port = self.port;
        let username = self.username.clone().unwrap_or("".to_string());
        let password = self.password.clone();
        let key_path = self.key_path.clone();
        let session_arc = self.session.clone();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let handler = ClientHandler {};
            let mut session = russh::client::connect(config, (host.as_str(), port), handler)
                .await
                .map_err(|e| PyRuntimeError::new_err(format!("Connection failed: {}", e)))?;

            // Authentication
            let auth_res = if let Some(key_p) = key_path {
                let key_path = Path::new(&key_p);
                let key_pair = russh_keys::load_secret_key(key_path, None)
                    .map_err(|e| PyRuntimeError::new_err(format!("Failed to load key: {}", e)))?;
                session
                    .authenticate_publickey(&username, Arc::new(key_pair))
                    .await
            } else if let Some(pwd) = password {
                session.authenticate_password(&username, pwd).await
            } else {
                // Try no-auth or agent? For now, fail if no auth provided
                return Err(PyValueError::new_err("No authentication method provided"));
            };

            if let Err(e) = auth_res {
                return Err(PyRuntimeError::new_err(format!(
                    "Authentication failed: {}",
                    e
                )));
            }

            if !auth_res.unwrap() {
                return Err(PyRuntimeError::new_err("Authentication failed"));
            }

            let mut guard = session_arc.lock().await;
            *guard = Some(Arc::new(session));

            Ok(())
        })
    }

    fn execute<'p>(&self, py: Python<'p>, command: String) -> PyResult<Bound<'p, PyAny>> {
        let session_arc = self.session.clone();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
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

            Ok((stdout_str, stderr_str, exit_code))
        })
    }

    fn close<'p>(&self, py: Python<'p>) -> PyResult<Bound<'p, PyAny>> {
        let session_arc = self.session.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
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

        let py_self = Bound::new(py, (*slf).clone())?.into_any().unbind();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let handler = ClientHandler {};
            let mut session = russh::client::connect(config, (host.as_str(), port), handler)
                .await
                .map_err(|e| PyRuntimeError::new_err(format!("Connection failed: {}", e)))?;

            // Auth ...
            let auth_res = if let Some(key_p) = key_path {
                let key_path = Path::new(&key_p);
                let key_pair = russh_keys::load_secret_key(key_path, None)
                    .map_err(|e| PyRuntimeError::new_err(format!("Failed to load key: {}", e)))?;
                session
                    .authenticate_publickey(&username, Arc::new(key_pair))
                    .await
            } else if let Some(pwd) = password {
                session.authenticate_password(&username, pwd).await
            } else {
                return Err(PyValueError::new_err("No authentication method provided"));
            };

            if let Err(e) = auth_res {
                return Err(PyRuntimeError::new_err(format!(
                    "Authentication failed: {}",
                    e
                )));
            }
            if !auth_res.unwrap() {
                return Err(PyRuntimeError::new_err("Authentication failed"));
            }

            let mut guard = session_arc.lock().await;
            *guard = Some(Arc::new(session));

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
        self.close(py)
    }

    fn sftp<'p>(&self, py: Python<'p>) -> PyResult<Bound<'p, PyAny>> {
        let session_arc = self.session.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let guard = session_arc.lock().await;
            let session = guard
                .as_ref()
                .ok_or_else(|| PyRuntimeError::new_err("Not connected"))?;
            let session = session.clone();
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

            Ok(AsyncSftpClient {
                client: Arc::new(sftp),
            })
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

#[pyclass]
pub struct AsyncSftpClient {
    client: Arc<SftpSession>,
}

#[pymethods]
impl AsyncSftpClient {
    fn list<'p>(&self, py: Python<'p>, path: String) -> PyResult<Bound<'p, PyAny>> {
        let client = self.client.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let entries = client
                .read_dir(path)
                .await
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;

            let mut results = Vec::new();
            for entry in entries {
                let filename = entry.file_name();
                results.push(filename);
            }
            Ok(results)
        })
    }

    fn get<'p>(
        &self,
        py: Python<'p>,
        remote_path: String,
        local_path: String,
    ) -> PyResult<Bound<'p, PyAny>> {
        let client = self.client.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut remote_file = client
                .open(remote_path)
                .await
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
            let mut local_file = tokio::fs::File::create(local_path)
                .await
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;

            // Simple copy implementation
            // In a real implementation, we might want to stream this more efficiently or handle large files better
            // russh-sftp File implements AsyncRead
            // tokio File implements AsyncWrite

            // However, russh-sftp File might not implement tokio::io::AsyncRead directly in a way compatible with tokio::io::copy
            // Let's check if we can just read into a buffer and write.

            // russh_sftp::client::File implements tokio::io::AsyncRead

            // We need to make sure we are using the right traits.
            // Since we imported tokio::io::AsyncReadExt, we should be good if it implements it.

            // Actually, let's just use a buffer loop to be safe and explicit.
            let mut buffer = vec![0u8; 32768]; // 32KB buffer
            loop {
                let n = remote_file
                    .read(&mut buffer)
                    .await
                    .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
                if n == 0 {
                    break;
                }
                local_file
                    .write_all(&buffer[..n])
                    .await
                    .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
            }

            Ok(())
        })
    }

    fn put<'p>(
        &self,
        py: Python<'p>,
        local_path: String,
        remote_path: String,
    ) -> PyResult<Bound<'p, PyAny>> {
        let client = self.client.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut local_file = tokio::fs::File::open(local_path)
                .await
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
            let mut remote_file = client
                .create(remote_path)
                .await
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;

            let mut buffer = vec![0u8; 32768];
            loop {
                let n = local_file
                    .read(&mut buffer)
                    .await
                    .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
                if n == 0 {
                    break;
                }
                remote_file
                    .write_all(&buffer[..n])
                    .await
                    .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
            }

            Ok(())
        })
    }
}

#[pyclass]
pub struct AsyncInteractiveShell {
    channel: Arc<Mutex<russh::Channel<russh::client::Msg>>>,
}

#[pymethods]
impl AsyncInteractiveShell {
    fn write<'p>(&self, py: Python<'p>, data: Vec<u8>) -> PyResult<Bound<'p, PyAny>> {
        let channel = self.channel.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let guard = channel.lock().await;
            guard
                .data(&data[..])
                .await
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
            Ok(())
        })
    }

    fn read<'p>(&self, py: Python<'p>) -> PyResult<Bound<'p, PyAny>> {
        let channel = self.channel.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut guard = channel.lock().await;
            let mut buffer = Vec::new();

            // First, wait for at least one data packet (ignoring non-data packets)
            loop {
                match tokio::time::timeout(std::time::Duration::from_secs(2), guard.wait()).await {
                    Ok(Some(russh::ChannelMsg::Data { data })) => {
                        buffer.extend_from_slice(&data);
                        break;
                    }
                    Ok(Some(russh::ChannelMsg::ExtendedData { data, .. })) => {
                        buffer.extend_from_slice(&data);
                        break;
                    }
                    Ok(Some(_)) => continue,       // Ignore other events
                    Ok(None) => return Ok(buffer), // Channel closed
                    Err(_) => return Ok(buffer),   // Timeout waiting for first packet
                }
            }

            // Then, consume any subsequent packets that arrive quickly
            loop {
                match tokio::time::timeout(std::time::Duration::from_millis(50), guard.wait()).await
                {
                    Ok(Some(russh::ChannelMsg::Data { data })) => {
                        buffer.extend_from_slice(&data);
                    }
                    Ok(Some(russh::ChannelMsg::ExtendedData { data, .. })) => {
                        buffer.extend_from_slice(&data);
                    }
                    Ok(Some(_)) => continue,
                    Ok(None) => break,
                    Err(_) => break,
                }
            }

            Ok(buffer)
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
}

struct TailerState {
    sftp: Option<SftpSession>,
    init_pos: Option<u64>,
    last_pos: u64,
    contents: Option<String>,
}

#[pyclass]
#[derive(Clone)]
pub struct AsyncFileTailer {
    conn_session: Arc<Mutex<Option<Arc<Handle<ClientHandler>>>>>,
    remote_file: String,
    state: Arc<Mutex<TailerState>>,
}

#[pymethods]
impl AsyncFileTailer {
    fn seek_end<'p>(&self, py: Python<'p>) -> PyResult<Bound<'p, PyAny>> {
        let state = self.state.clone();
        let remote_file = self.remote_file.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
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
        })
    }

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

            let mut state_guard = state.lock().await;
            state_guard.sftp = Some(sftp);

            if let Some(sftp) = &state_guard.sftp {
                let metadata = sftp
                    .metadata(&remote_file)
                    .await
                    .map_err(|e| PyRuntimeError::new_err(format!("Stat error: {}", e)))?;
                let size = metadata.size.unwrap_or(0);
                state_guard.last_pos = size;
                if state_guard.init_pos.is_none() {
                    state_guard.init_pos = Some(size);
                }
            }

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
        match self.state.try_lock() {
            Ok(guard) => Ok(guard.contents.clone()),
            Err(_) => Err(PyRuntimeError::new_err("Could not acquire lock")),
        }
    }
}
