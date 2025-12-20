//! # multi_conn.rs
//!
//! This module provides multi-connection functionality for executing SSH operations
//! concurrently across multiple hosts.
//!
//! ## Classes
//!
//! ### MultiConnection
//! A synchronous Python class that manages multiple SSH connections and executes
//! operations concurrently using async under the hood.
//!
//! ### MultiResult
//! A dict-like result object that maps hostnames to their individual SSHResult.
//!
//! ### MultiFileTailer
//! A context manager for tailing files on multiple hosts concurrently.
//!
//! ## Usage
//!
//! ```python
//! from hussh.multi_conn import MultiConnection
//!
//! # Create from shared auth
//! with MultiConnection.from_shared_auth(
//!     ["host1", "host2", "host3"],
//!     username="user",
//!     password="pass",
//!     batch_size=50
//! ) as mc:
//!     results = mc.execute("whoami")
//!     for host, result in results.items():
//!         print(f"{host}: {result.stdout}")
//! ```

use crate::asynchronous::{AsyncConnection, AsyncFileTailer};
use crate::connection::{Connection, SSHResult};
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyType};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

// Create the PartialFailureException
pyo3::create_exception!(
    multi_conn,
    PartialFailureException,
    pyo3::exceptions::PyException,
    "Raised when some hosts succeed and some fail in a MultiConnection operation."
);

/// # MultiResult
///
/// A dict-like result object that maps hostnames to their individual SSHResult.
/// Provides helper methods for filtering successful/failed results and raising
/// exceptions on partial failures.
///
/// ## Methods
///
/// ### `failed`
/// Returns a new MultiResult containing only the failed results (status != 0).
///
/// ### `succeeded`
/// Returns a new MultiResult containing only the successful results (status == 0).
///
/// ### `raise_if_any_failed`
/// Raises a PartialFailureException if any results have non-zero status.
///
#[pyclass(mapping)]
#[derive(Clone)]
pub struct MultiResult {
    results: HashMap<String, SSHResult>,
}

impl MultiResult {
    pub fn new(results: HashMap<String, SSHResult>) -> Self {
        MultiResult { results }
    }
}

#[pymethods]
impl MultiResult {
    fn __len__(&self) -> usize {
        self.results.len()
    }

    fn __getitem__(&self, key: &str) -> PyResult<SSHResult> {
        self.results
            .get(key)
            .cloned()
            .ok_or_else(|| pyo3::exceptions::PyKeyError::new_err(key.to_string()))
    }

    fn __contains__(&self, key: &str) -> bool {
        self.results.contains_key(key)
    }

    fn __iter__(&self) -> PyResult<Py<PyList>> {
        Python::attach(|py| {
            let keys: Vec<&str> = self.results.keys().map(|s| s.as_str()).collect();
            Ok(PyList::new(py, keys)?.into())
        })
    }

    fn keys(&self) -> Vec<String> {
        self.results.keys().cloned().collect()
    }

    fn values(&self) -> Vec<SSHResult> {
        self.results.values().cloned().collect()
    }

    fn items(&self) -> Vec<(String, SSHResult)> {
        self.results
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    fn get(&self, key: &str, default: Option<SSHResult>) -> Option<SSHResult> {
        self.results.get(key).cloned().or(default)
    }

    /// Returns a new MultiResult containing only the failed results (status != 0)
    /// Returns None if there are no failed results
    #[getter]
    fn failed(&self) -> Option<MultiResult> {
        let failed_results: HashMap<String, SSHResult> = self
            .results
            .iter()
            .filter(|(_, result)| result.status != 0)
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        if failed_results.is_empty() {
            None
        } else {
            Some(MultiResult {
                results: failed_results,
            })
        }
    }

    /// Returns a new MultiResult containing only the successful results (status == 0)
    /// Returns None if there are no successful results
    #[getter]
    fn succeeded(&self) -> Option<MultiResult> {
        let succeeded_results: HashMap<String, SSHResult> = self
            .results
            .iter()
            .filter(|(_, result)| result.status == 0)
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        if succeeded_results.is_empty() {
            None
        } else {
            Some(MultiResult {
                results: succeeded_results,
            })
        }
    }

    /// Raises a PartialFailureException if any results have non-zero status
    fn raise_if_any_failed(&self, py: Python<'_>) -> PyResult<()> {
        let failed: Vec<(&String, &SSHResult)> = self
            .results
            .iter()
            .filter(|(_, result)| result.status != 0)
            .collect();

        if !failed.is_empty() {
            let failed_count = failed.len();
            let total_count = self.results.len();

            // These will always be Some since we checked !failed.is_empty()
            let succeeded = self.succeeded();
            let failed_result = self.failed();

            let msg = format!(
                "Operation failed on {} of {} host(s)",
                failed_count, total_count
            );

            // Create kwargs dict for the exception
            let kwargs = PyDict::new(py);
            kwargs.set_item("succeeded", succeeded.map(|s| Py::new(py, s)).transpose()?)?;
            kwargs.set_item("failed", failed_result.map(|f| Py::new(py, f)).transpose()?)?;

            // Create the exception with attributes
            let exc = PartialFailureException::new_err(msg);
            let exc_obj = exc.value(py);
            exc_obj.setattr("succeeded", kwargs.get_item("succeeded")?)?;
            exc_obj.setattr("failed", kwargs.get_item("failed")?)?;

            return Err(exc);
        }
        Ok(())
    }

    fn __repr__(&self) -> String {
        let succeeded = self.results.values().filter(|r| r.status == 0).count();
        let failed = self.results.len() - succeeded;
        format!(
            "MultiResult({} hosts: {} succeeded, {} failed)",
            self.results.len(),
            succeeded,
            failed
        )
    }

    fn __str__(&self) -> String {
        self.__repr__()
    }
}

/// # MultiConnection
///
/// A class for managing multiple SSH connections and executing operations concurrently.
///
/// ## Constructor
///
/// Create from a list of AsyncConnection instances:
/// ```python
/// mc = MultiConnection([async_conn1, async_conn2], batch_size=50)
/// ```
///
/// ## Class Methods
///
/// ### `from_connections`
/// Create from sync Connection instances (copies connection details):
/// ```python
/// mc = MultiConnection.from_connections([conn1, conn2], batch_size=50)
/// ```
///
/// ### `from_shared_auth`
/// Create with shared authentication for multiple hosts:
/// ```python
/// mc = MultiConnection.from_shared_auth(
///     ["host1", "host2"],
///     username="user",
///     password="pass",
///     batch_size=50
/// )
/// ```
///
/// ## Methods
///
/// ### `connect`
/// Explicitly connect to all hosts. If `prune_failures=True`, failed hosts are
/// removed from the connection pool.
///
/// ### `execute`
/// Execute the same command on all hosts concurrently.
///
/// ### `execute_map`
/// Execute different commands on different hosts using a hostname -> command mapping.
///
/// ### `sftp_write`
/// Write a local file to all hosts.
///
/// ### `sftp_read`
/// Read a file from all hosts.
///
/// ### `tail`
/// Create a MultiFileTailer for tailing a file on all hosts.
///
/// ### `close`
/// Close all connections.
///
#[pyclass]
pub struct MultiConnection {
    connections: Vec<AsyncConnection>,
    #[pyo3(get)]
    hosts: Vec<String>,
    #[pyo3(get)]
    batch_size: usize,
    timeout: u64,
}

#[pymethods]
impl MultiConnection {
    /// Create from AsyncConnection instances directly
    #[new]
    #[pyo3(signature = (connections, batch_size=None, timeout=None))]
    fn new(
        connections: Vec<AsyncConnection>,
        batch_size: Option<usize>,
        timeout: Option<u64>,
    ) -> PyResult<Self> {
        let hosts: Vec<String> = connections.iter().map(|c| c.get_host()).collect();
        Ok(MultiConnection {
            connections,
            hosts,
            batch_size: batch_size.unwrap_or(100),
            timeout: timeout.unwrap_or(0),
        })
    }

    /// Create from sync Connection instances by copying their connection details
    #[classmethod]
    #[pyo3(signature = (connections, batch_size=None, timeout=None))]
    fn from_connections(
        _cls: &Bound<'_, PyType>,
        connections: Vec<PyRef<Connection>>,
        batch_size: Option<usize>,
        timeout: Option<u64>,
    ) -> PyResult<Self> {
        let mut async_conns = Vec::new();
        let mut hosts = Vec::new();

        for conn in connections {
            let port = conn.get_port() as u16;
            let async_conn = AsyncConnection::create(
                conn.get_host().to_string(),
                Some(conn.get_username().to_string()),
                if conn.get_password().is_empty() {
                    None
                } else {
                    Some(conn.get_password().to_string())
                },
                if conn.get_private_key().is_empty() {
                    None
                } else {
                    Some(conn.get_private_key().to_string())
                },
                port,
                0,                    // keepalive_interval
                timeout.unwrap_or(0), // Use explicit timeout or 0 for no timeout
            );
            hosts.push(conn.get_host().to_string());
            async_conns.push(async_conn);
        }

        Ok(MultiConnection {
            connections: async_conns,
            hosts,
            batch_size: batch_size.unwrap_or(100),
            timeout: timeout.unwrap_or(0),
        })
    }

    /// Create with shared authentication for multiple hosts
    #[classmethod]
    #[allow(clippy::too_many_arguments)]
    #[pyo3(signature = (hosts, username=None, password=None, key_path=None, port=22, timeout=0, batch_size=None))]
    fn from_shared_auth(
        _cls: &Bound<'_, PyType>,
        hosts: Vec<String>,
        username: Option<String>,
        password: Option<String>,
        key_path: Option<String>,
        port: u16,
        timeout: u64,
        batch_size: Option<usize>,
    ) -> PyResult<Self> {
        let connections: Vec<AsyncConnection> = hosts
            .iter()
            .map(|host| {
                AsyncConnection::create(
                    host.clone(),
                    username.clone(),
                    password.clone(),
                    key_path.clone(),
                    port,
                    0, // keepalive_interval
                    timeout,
                )
            })
            .collect();

        Ok(MultiConnection {
            connections,
            hosts,
            batch_size: batch_size.unwrap_or(100),
            timeout,
        })
    }

    /// Connect to all hosts concurrently.
    /// If `prune_failures` is True, failed hosts are removed from the pool.
    /// Returns a MultiResult with connection status (empty stdout/stderr, status 0 for success).
    #[pyo3(signature = (prune_failures=false, timeout=None))]
    fn connect(&mut self, prune_failures: bool, timeout: Option<u64>) -> PyResult<MultiResult> {
        let timeout = timeout.unwrap_or(self.timeout);
        let batch_size = self.batch_size;

        // Clone data for the async block
        let connections: Vec<(String, AsyncConnection)> = self
            .hosts
            .iter()
            .zip(self.connections.iter())
            .map(|(h, c)| (h.clone(), c.clone()))
            .collect();

        let results = pyo3_async_runtimes::tokio::get_runtime().block_on(async {
            let mut set = JoinSet::new();
            let semaphore = Arc::new(Semaphore::new(batch_size));

            for (host, conn) in connections {
                let permit = semaphore
                    .clone()
                    .acquire_owned()
                    .await
                    .expect("semaphore closed unexpectedly");

                set.spawn(async move {
                    let _permit = permit;

                    let result = if timeout > 0 {
                        tokio::time::timeout(Duration::from_secs(timeout), conn.connect_async(None))
                            .await
                            .unwrap_or_else(|_| {
                                Err(PyRuntimeError::new_err(format!(
                                    "Connection timed out after {} seconds",
                                    timeout
                                )))
                            })
                    } else {
                        conn.connect_async(None).await
                    };

                    (host, result)
                });
            }

            let mut results = HashMap::new();
            while let Some(res) = set.join_next().await {
                match res {
                    Ok((host, Ok(()))) => {
                        results.insert(
                            host,
                            SSHResult {
                                stdout: String::new(),
                                stderr: String::new(),
                                status: 0,
                            },
                        );
                    }
                    Ok((host, Err(e))) => {
                        let error_msg = format!("{}", e);
                        // Check for file descriptor limit errors
                        if error_msg.contains("Too many open files") || error_msg.contains("EMFILE")
                        {
                            return Err(PyRuntimeError::new_err(format!(
                                "Too many open files. Try reducing batch_size (current: {}) or increasing ulimit -n",
                                batch_size
                            )));
                        }
                        results.insert(
                            host,
                            SSHResult {
                                stdout: String::new(),
                                stderr: error_msg,
                                status: -1,
                            },
                        );
                    }
                    Err(join_err) => {
                        return Err(PyRuntimeError::new_err(format!(
                            "Task panic: {:?}",
                            join_err
                        )));
                    }
                }
            }
            Ok(results)
        })?;

        // If prune_failures is true, remove failed hosts
        if prune_failures {
            let successful_hosts: HashSet<String> = results
                .iter()
                .filter(|(_, r)| r.status == 0)
                .map(|(h, _)| h.clone())
                .collect();

            // Rebuild connections list keeping only successful ones
            let mut new_connections = Vec::new();
            let mut new_hosts = Vec::new();

            for (i, host) in self.hosts.iter().enumerate() {
                if successful_hosts.contains(host) {
                    new_connections.push(self.connections[i].clone());
                    new_hosts.push(host.clone());
                }
            }

            self.connections = new_connections;
            self.hosts = new_hosts;
        }

        Ok(MultiResult::new(results))
    }

    /// Execute the same command on all hosts concurrently
    #[pyo3(signature = (command, timeout=None))]
    fn execute(&self, command: String, timeout: Option<u64>) -> PyResult<MultiResult> {
        let batch_size = self.batch_size;

        let connections: Vec<(String, AsyncConnection)> = self
            .hosts
            .iter()
            .zip(self.connections.iter())
            .map(|(h, c)| (h.clone(), c.clone()))
            .collect();

        pyo3_async_runtimes::tokio::get_runtime().block_on(async {
            let mut set = JoinSet::new();
            let semaphore = Arc::new(Semaphore::new(batch_size));

            for (host, conn) in connections {
                let cmd = command.clone();
                let permit = semaphore
                    .clone()
                    .acquire_owned()
                    .await
                    .expect("semaphore closed unexpectedly");

                set.spawn(async move {
                    let _permit = permit;

                    let result = if let Some(timeout_secs) = timeout {
                        tokio::time::timeout(
                            Duration::from_secs(timeout_secs),
                            conn.execute_async(cmd, None),
                        )
                        .await
                        .unwrap_or_else(|_| {
                            Ok(SSHResult {
                                stdout: String::new(),
                                stderr: format!(
                                    "Operation timed out after {} seconds",
                                    timeout_secs
                                ),
                                status: -1,
                            })
                        })
                    } else {
                        conn.execute_async(cmd, None).await
                    };

                    (host, result)
                });
            }

            let mut results = HashMap::new();
            while let Some(res) = set.join_next().await {
                match res {
                    Ok((host, Ok(ssh_result))) => {
                        results.insert(host, ssh_result);
                    }
                    Ok((host, Err(e))) => {
                        let error_msg = format!("{}", e);
                        if error_msg.contains("Too many open files") || error_msg.contains("EMFILE")
                        {
                            return Err(PyRuntimeError::new_err(format!(
                                "Too many open files. Try reducing batch_size (current: {}) or increasing ulimit -n",
                                batch_size
                            )));
                        }
                        results.insert(
                            host,
                            SSHResult {
                                stdout: String::new(),
                                stderr: error_msg,
                                status: -1,
                            },
                        );
                    }
                    Err(join_err) => {
                        return Err(PyRuntimeError::new_err(format!(
                            "Task panic: {:?}",
                            join_err
                        )));
                    }
                }
            }

            Ok(MultiResult::new(results))
        })
    }

    /// Execute different commands on different hosts using a hostname -> command mapping
    #[pyo3(signature = (host_command_map, timeout=None))]
    fn execute_map(
        &self,
        host_command_map: HashMap<String, String>,
        timeout: Option<u64>,
    ) -> PyResult<MultiResult> {
        let batch_size = self.batch_size;

        let connections: Vec<(String, AsyncConnection, String)> = self
            .hosts
            .iter()
            .zip(self.connections.iter())
            .filter_map(|(h, c)| {
                host_command_map
                    .get(h)
                    .map(|cmd| (h.clone(), c.clone(), cmd.clone()))
            })
            .collect();

        pyo3_async_runtimes::tokio::get_runtime().block_on(async {
            let mut set = JoinSet::new();
            let semaphore = Arc::new(Semaphore::new(batch_size));

            for (host, conn, cmd) in connections {
                let permit = semaphore
                    .clone()
                    .acquire_owned()
                    .await
                    .expect("semaphore closed unexpectedly");

                set.spawn(async move {
                    let _permit = permit;

                    let result = if let Some(timeout_secs) = timeout {
                        tokio::time::timeout(
                            Duration::from_secs(timeout_secs),
                            conn.execute_async(cmd, None),
                        )
                        .await
                        .unwrap_or_else(|_| {
                            Ok(SSHResult {
                                stdout: String::new(),
                                stderr: format!(
                                    "Operation timed out after {} seconds",
                                    timeout_secs
                                ),
                                status: -1,
                            })
                        })
                    } else {
                        conn.execute_async(cmd, None).await
                    };

                    (host, result)
                });
            }

            let mut results = HashMap::new();
            while let Some(res) = set.join_next().await {
                match res {
                    Ok((host, Ok(ssh_result))) => {
                        results.insert(host, ssh_result);
                    }
                    Ok((host, Err(e))) => {
                        results.insert(
                            host,
                            SSHResult {
                                stdout: String::new(),
                                stderr: format!("{}", e),
                                status: -1,
                            },
                        );
                    }
                    Err(join_err) => {
                        return Err(PyRuntimeError::new_err(format!(
                            "Task panic: {:?}",
                            join_err
                        )));
                    }
                }
            }

            Ok(MultiResult::new(results))
        })
    }

    /// Write a local file to all hosts via SFTP
    #[pyo3(signature = (local_path, remote_path=None))]
    fn sftp_write(&self, local_path: String, remote_path: Option<String>) -> PyResult<MultiResult> {
        let batch_size = self.batch_size;
        let remote_p = remote_path.unwrap_or_else(|| local_path.clone());

        let connections: Vec<(String, AsyncConnection)> = self
            .hosts
            .iter()
            .zip(self.connections.iter())
            .map(|(h, c)| (h.clone(), c.clone()))
            .collect();

        pyo3_async_runtimes::tokio::get_runtime().block_on(async {
            let mut set = JoinSet::new();
            let semaphore = Arc::new(Semaphore::new(batch_size));

            for (host, conn) in connections {
                let local = local_path.clone();
                let remote = remote_p.clone();
                let permit = semaphore
                    .clone()
                    .acquire_owned()
                    .await
                    .expect("semaphore closed unexpectedly");

                set.spawn(async move {
                    let _permit = permit;
                    let result = conn.sftp_write_async(local, Some(remote)).await;
                    (host, result)
                });
            }

            let mut results = HashMap::new();
            while let Some(res) = set.join_next().await {
                match res {
                    Ok((host, Ok(()))) => {
                        results.insert(
                            host,
                            SSHResult {
                                stdout: "Ok".to_string(),
                                stderr: String::new(),
                                status: 0,
                            },
                        );
                    }
                    Ok((host, Err(e))) => {
                        results.insert(
                            host,
                            SSHResult {
                                stdout: String::new(),
                                stderr: format!("{}", e),
                                status: -1,
                            },
                        );
                    }
                    Err(join_err) => {
                        return Err(PyRuntimeError::new_err(format!(
                            "Task panic: {:?}",
                            join_err
                        )));
                    }
                }
            }

            Ok(MultiResult::new(results))
        })
    }

    /// Read a file from all hosts via SFTP
    /// Returns a MultiResult where stdout contains the file contents
    #[pyo3(signature = (remote_path, local_path=None))]
    fn sftp_read(&self, remote_path: String, local_path: Option<String>) -> PyResult<MultiResult> {
        let batch_size = self.batch_size;

        let connections: Vec<(String, AsyncConnection)> = self
            .hosts
            .iter()
            .zip(self.connections.iter())
            .map(|(h, c)| (h.clone(), c.clone()))
            .collect();

        pyo3_async_runtimes::tokio::get_runtime().block_on(async {
            let mut set = JoinSet::new();
            let semaphore = Arc::new(Semaphore::new(batch_size));

            for (host, conn) in connections {
                let remote = remote_path.clone();
                let local = local_path.clone();
                let permit = semaphore
                    .clone()
                    .acquire_owned()
                    .await
                    .expect("semaphore closed unexpectedly");

                set.spawn(async move {
                    let _permit = permit;
                    let result = conn.sftp_read_async(remote, local).await;
                    (host, result)
                });
            }

            let mut results = HashMap::new();
            while let Some(res) = set.join_next().await {
                match res {
                    Ok((host, Ok(content))) => {
                        results.insert(
                            host,
                            SSHResult {
                                stdout: content,
                                stderr: String::new(),
                                status: 0,
                            },
                        );
                    }
                    Ok((host, Err(e))) => {
                        results.insert(
                            host,
                            SSHResult {
                                stdout: String::new(),
                                stderr: format!("{}", e),
                                status: -1,
                            },
                        );
                    }
                    Err(join_err) => {
                        return Err(PyRuntimeError::new_err(format!(
                            "Task panic: {:?}",
                            join_err
                        )));
                    }
                }
            }

            Ok(MultiResult::new(results))
        })
    }

    /// Write string data to a file on all hosts via SFTP
    fn sftp_write_data(&self, data: String, remote_path: String) -> PyResult<MultiResult> {
        let batch_size = self.batch_size;

        let connections: Vec<(String, AsyncConnection)> = self
            .hosts
            .iter()
            .zip(self.connections.iter())
            .map(|(h, c)| (h.clone(), c.clone()))
            .collect();

        pyo3_async_runtimes::tokio::get_runtime().block_on(async {
            let mut set = JoinSet::new();
            let semaphore = Arc::new(Semaphore::new(batch_size));

            for (host, conn) in connections {
                let data_clone = data.clone();
                let remote = remote_path.clone();
                let permit = semaphore
                    .clone()
                    .acquire_owned()
                    .await
                    .expect("semaphore closed unexpectedly");

                set.spawn(async move {
                    let _permit = permit;
                    let result = conn.sftp_write_data_async(data_clone, remote).await;
                    (host, result)
                });
            }

            let mut results = HashMap::new();
            while let Some(res) = set.join_next().await {
                match res {
                    Ok((host, Ok(()))) => {
                        results.insert(
                            host,
                            SSHResult {
                                stdout: "Ok".to_string(),
                                stderr: String::new(),
                                status: 0,
                            },
                        );
                    }
                    Ok((host, Err(e))) => {
                        results.insert(
                            host,
                            SSHResult {
                                stdout: String::new(),
                                stderr: format!("{}", e),
                                status: -1,
                            },
                        );
                    }
                    Err(join_err) => {
                        return Err(PyRuntimeError::new_err(format!(
                            "Task panic: {:?}",
                            join_err
                        )));
                    }
                }
            }

            Ok(MultiResult::new(results))
        })
    }

    /// Create a MultiFileTailer for tailing a file on all hosts
    fn tail(&self, remote_file: String) -> MultiFileTailer {
        let tailers: Vec<(String, AsyncFileTailer)> = self
            .hosts
            .iter()
            .zip(self.connections.iter())
            .map(|(h, c)| (h.clone(), c.create_tailer(remote_file.clone())))
            .collect();

        MultiFileTailer {
            hosts: self.hosts.clone(),
            tailers,
            contents: HashMap::new(),
            batch_size: self.batch_size,
        }
    }

    /// Create a MultiFileTailer for tailing different files on different hosts
    fn tail_map(&self, host_file_map: HashMap<String, String>) -> MultiFileTailer {
        let tailers: Vec<(String, AsyncFileTailer)> = self
            .hosts
            .iter()
            .zip(self.connections.iter())
            .filter_map(|(h, c)| {
                host_file_map
                    .get(h)
                    .map(|file| (h.clone(), c.create_tailer(file.clone())))
            })
            .collect();

        let hosts: Vec<String> = tailers.iter().map(|(h, _)| h.clone()).collect();

        MultiFileTailer {
            hosts,
            tailers,
            contents: HashMap::new(),
            batch_size: self.batch_size,
        }
    }

    /// Close all connections
    fn close(&self) -> PyResult<()> {
        pyo3_async_runtimes::tokio::get_runtime().block_on(async {
            for conn in &self.connections {
                conn.close_async().await;
            }
        });
        Ok(())
    }

    /// Context manager entry - connects to all hosts
    fn __enter__(mut slf: PyRefMut<'_, Self>) -> PyResult<PyRefMut<'_, Self>> {
        // Connect to all hosts (eager connect in context manager)
        slf.connect(false, None)?;
        Ok(slf)
    }

    /// Context manager exit - closes all connections
    #[pyo3(signature = (_exc_type=None, _exc_value=None, _traceback=None))]
    fn __exit__(
        &self,
        _exc_type: Option<Bound<'_, PyAny>>,
        _exc_value: Option<Bound<'_, PyAny>>,
        _traceback: Option<Bound<'_, PyAny>>,
    ) -> PyResult<bool> {
        self.close()?;
        Ok(false)
    }

    fn __repr__(&self) -> String {
        format!(
            "MultiConnection({} hosts, batch_size={})",
            self.hosts.len(),
            self.batch_size
        )
    }
}

/// # MultiFileTailer
///
/// A context manager for tailing files on multiple hosts concurrently.
///
/// ## Attributes
///
/// * `contents`: Dict mapping hostnames to their file contents (available after exit).
///
/// ## Methods
///
/// ### `read`
/// Read new content from all tailed files.
///
#[pyclass]
pub struct MultiFileTailer {
    #[pyo3(get)]
    hosts: Vec<String>,
    tailers: Vec<(String, AsyncFileTailer)>,
    #[pyo3(get)]
    contents: HashMap<String, String>,
    batch_size: usize,
}

#[pymethods]
impl MultiFileTailer {
    /// Read new content from all tailed files concurrently
    #[pyo3(signature = (from_pos=None))]
    fn read(&self, from_pos: Option<u64>) -> PyResult<HashMap<String, String>> {
        let batch_size = self.batch_size;

        let tailers: Vec<(String, AsyncFileTailer)> = self.tailers.clone();

        pyo3_async_runtimes::tokio::get_runtime().block_on(async {
            let mut set = JoinSet::new();
            let semaphore = Arc::new(Semaphore::new(batch_size));

            for (host, tailer) in tailers {
                let permit = semaphore
                    .clone()
                    .acquire_owned()
                    .await
                    .expect("semaphore closed unexpectedly");

                set.spawn(async move {
                    let _permit = permit;
                    let result = tailer.read_async(from_pos).await;
                    (host, result)
                });
            }

            let mut results = HashMap::new();
            while let Some(res) = set.join_next().await {
                match res {
                    Ok((host, Ok(content))) => {
                        results.insert(host, content);
                    }
                    Ok((host, Err(e))) => {
                        results.insert(host, format!("Error: {}", e));
                    }
                    Err(join_err) => {
                        return Err(PyRuntimeError::new_err(format!(
                            "Task panic during file tailing: {:?}",
                            join_err
                        )));
                    }
                }
            }

            Ok(results)
        })
    }

    /// Context manager entry - initializes all tailers
    fn __enter__(slf: PyRefMut<'_, Self>) -> PyResult<PyRefMut<'_, Self>> {
        let batch_size = slf.batch_size;
        let tailers: Vec<(String, AsyncFileTailer)> = slf.tailers.clone();

        pyo3_async_runtimes::tokio::get_runtime().block_on(async {
            let mut set = JoinSet::new();
            let semaphore = Arc::new(Semaphore::new(batch_size));

            for (host, tailer) in tailers {
                let permit = semaphore
                    .clone()
                    .acquire_owned()
                    .await
                    .expect("semaphore closed unexpectedly");

                set.spawn(async move {
                    let _permit = permit;
                    let result = tailer.enter_async().await;
                    (host, result)
                });
            }

            while let Some(res) = set.join_next().await {
                match res {
                    Ok((_, Ok(()))) => {}
                    Ok((host, Err(e))) => {
                        return Err(PyRuntimeError::new_err(format!(
                            "Failed to initialize tailer for {}: {}",
                            host, e
                        )));
                    }
                    Err(join_err) => {
                        return Err(PyRuntimeError::new_err(format!(
                            "Task panic during tailer initialization: {:?}",
                            join_err
                        )));
                    }
                }
            }

            Ok(())
        })?;

        Ok(slf)
    }

    /// Context manager exit - collects final contents from all tailers
    #[pyo3(signature = (_exc_type=None, _exc_value=None, _traceback=None))]
    fn __exit__(
        &mut self,
        _exc_type: Option<Bound<'_, PyAny>>,
        _exc_value: Option<Bound<'_, PyAny>>,
        _traceback: Option<Bound<'_, PyAny>>,
    ) -> PyResult<bool> {
        let batch_size = self.batch_size;
        let tailers: Vec<(String, AsyncFileTailer)> = self.tailers.clone();

        let contents = pyo3_async_runtimes::tokio::get_runtime().block_on(async {
            let mut set = JoinSet::new();
            let semaphore = Arc::new(Semaphore::new(batch_size));

            for (host, tailer) in tailers {
                let permit = semaphore
                    .clone()
                    .acquire_owned()
                    .await
                    .expect("semaphore closed unexpectedly");

                set.spawn(async move {
                    let _permit = permit;
                    let result = tailer.exit_async().await;
                    (host, result)
                });
            }

            let mut results = HashMap::new();
            while let Some(res) = set.join_next().await {
                match res {
                    Ok((host, Ok(content))) => {
                        results.insert(host, content);
                    }
                    Ok((host, Err(e))) => {
                        results.insert(host, format!("Error: {}", e));
                    }
                    Err(_) => {
                        // Ignore panics during exit
                    }
                }
            }

            results
        });

        self.contents = contents;
        Ok(false)
    }
}
