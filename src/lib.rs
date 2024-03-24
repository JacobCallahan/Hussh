use pyo3::prelude::*;

mod connection;

/// A Python module implemented in Rust.
#[pymodule]
fn hussh(_py: Python, m: &PyModule) -> PyResult<()> {
    m.add_class::<connection::Connection>()?; // Add the Connection class
    Ok(())
}
