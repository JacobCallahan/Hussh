use async_trait::async_trait;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use russh::client::{Config, Handle, Handler};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone)]
struct ClientHandler;

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
}
