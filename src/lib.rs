use connection::AuthenticationError;
use pyo3::prelude::*;

pub mod async_connection;
mod connection;

/// A Python module implemented in Rust.
#[pymodule]
fn hussh(_py: Python, m: &PyModule) -> PyResult<()> {
    m.add_class::<connection::Connection>()?; // Add the Connection class
    m.add_class::<connection::SSHResult>()?;
    m.add_class::<connection::InteractiveShell>()?;
    m.add_class::<connection::FileTailer>()?;
    m.add_class::<async_connection::PyAsyncConnection>()?;
    m.add_class::<async_connection::PyAsyncInteractiveShell>()?;
    m.add_class::<async_connection::PyAsyncFileTailer>()?;
    m.add("AuthenticationError", _py.get_type::<AuthenticationError>())?;
    Ok(())
}
