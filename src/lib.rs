use connection::AuthenticationError;
use pyo3::prelude::*;

#[cfg(feature = "async")]
mod asynchronous;
mod connection;
#[cfg(feature = "async")]
mod multi_conn;

/// A Python module implemented in Rust.
#[pymodule]
fn hussh(_py: Python, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<connection::Connection>()?; // Add the Connection class
    m.add_class::<connection::SSHResult>()?;
    m.add_class::<connection::InteractiveShell>()?;
    m.add_class::<connection::FileTailer>()?;
    m.add("AuthenticationError", _py.get_type::<AuthenticationError>())?;

    #[cfg(feature = "async")]
    {
        // aio submodule for async single-connection operations
        let async_submodule = PyModule::new(_py, "aio")?;
        async_submodule.add_class::<asynchronous::AsyncConnection>()?;
        async_submodule.add_class::<asynchronous::AsyncInteractiveShell>()?;
        async_submodule.add_class::<asynchronous::AsyncFileTailer>()?;
        m.add_submodule(&async_submodule)?;

        // multi_conn submodule for multi-host concurrent operations
        let multi_conn_submodule = PyModule::new(_py, "multi_conn")?;
        multi_conn_submodule.add_class::<multi_conn::MultiConnection>()?;
        multi_conn_submodule.add_class::<multi_conn::MultiResult>()?;
        multi_conn_submodule.add_class::<multi_conn::MultiFileTailer>()?;
        multi_conn_submodule.add(
            "PartialFailureException",
            _py.get_type::<multi_conn::PartialFailureException>(),
        )?;
        // Also expose AsyncConnection in multi_conn for convenience
        multi_conn_submodule.add_class::<asynchronous::AsyncConnection>()?;
        m.add_submodule(&multi_conn_submodule)?;

        let sys = PyModule::import(_py, "sys")?;
        let modules = sys.getattr("modules")?;
        modules.set_item("hussh.aio", &async_submodule)?;
        modules.set_item("hussh.multi_conn", &multi_conn_submodule)?;
    }

    Ok(())
}
