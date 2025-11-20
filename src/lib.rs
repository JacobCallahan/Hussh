use connection::AuthenticationError;
use pyo3::prelude::*;

#[cfg(feature = "async")]
mod asynchronous;
mod connection;

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
        let async_submodule = PyModule::new(_py, "aio")?;
        async_submodule.add_class::<asynchronous::AsyncConnection>()?;
        async_submodule.add_class::<asynchronous::AsyncSftpClient>()?;
        async_submodule.add_class::<asynchronous::AsyncInteractiveShell>()?;
        async_submodule.add_class::<asynchronous::AsyncFileTailer>()?;
        m.add_submodule(&async_submodule)?;
    }

    Ok(())
}
