use connection::AuthenticationError;
use pyo3::prelude::*;

mod connection;

/// A Python module implemented in Rust.
#[pymodule]
fn hussh(_py: Python, m: &PyModule) -> PyResult<()> {
    m.add_class::<connection::Connection>()?; // Add the Connection class
    m.add_class::<connection::SSHResult>()?;
    // m.add_class::<connection::InteractiveShell>()?;
    m.add("AuthenticationError", _py.get_type::<AuthenticationError>())?;
    Ok(())
}
