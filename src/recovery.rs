use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use std::time::Duration;

pyo3::create_exception!(hussh, HusshConnectionError, PyRuntimeError);
pyo3::create_exception!(hussh, HusshIncompleteCommandError, HusshConnectionError);

pub(crate) fn validate(max_retries: i64, retry_interval: f64) -> PyResult<()> {
    if max_retries < 0 || max_retries == i64::MAX {
        return Err(PyValueError::new_err(
            "max_retries must be a nonnegative integer",
        ));
    }
    interval(retry_interval)
}

fn interval(value: f64) -> PyResult<()> {
    if !value.is_finite() || value < 0.0 || Duration::try_from_secs_f64(value).is_err() {
        return Err(PyValueError::new_err(
            "retry_interval must be nonnegative finite seconds representable as a duration",
        ));
    }
    Ok(())
}

pub(crate) struct RetryPolicy {
    pub retries: i64,
    delay: f64,
    exponential: bool,
    cap: f64,
}

impl RetryPolicy {
    pub fn new(retries: i64, delay: f64) -> Self {
        Self {
            retries,
            delay,
            exponential: false,
            cap: 30.0,
        }
    }

    pub fn overrides(
        retries: i64,
        default_delay: f64,
        value: Option<Bound<'_, PyAny>>,
        cap: f64,
    ) -> PyResult<Self> {
        if !cap.is_finite() || cap <= 0.0 || Duration::try_from_secs_f64(cap).is_err() {
            return Err(PyValueError::new_err(
                "max_interval must be positive finite seconds representable as a duration",
            ));
        }
        let exponential = value
            .as_ref()
            .and_then(|v| v.extract::<String>().ok())
            .as_deref()
            == Some("exponential");
        let delay = if exponential {
            1.5
        } else if let Some(v) = value {
            v.extract::<f64>()?
        } else {
            default_delay
        };
        validate(retries, delay)?;
        Ok(Self {
            retries,
            delay,
            exponential,
            cap,
        })
    }

    pub fn delay(&self, retry: i64) -> Duration {
        let seconds = if self.exponential {
            (self.delay * 2_f64.powf(retry as f64)).min(self.cap)
        } else {
            self.delay
        };
        Duration::from_secs_f64(seconds)
    }
}

pub(crate) fn exhausted(attempts: i64, cause: PyErr) -> PyErr {
    let error = HusshConnectionError::new_err(format!(
        "Connection recovery failed after {attempts} fresh attempt(s): {cause}"
    ));
    Python::attach(|py| error.set_cause(py, Some(cause)));
    error
}

pub(crate) fn incomplete(cause: impl std::fmt::Display) -> PyErr {
    HusshIncompleteCommandError::new_err(format!("SSH transport failed after command submission started; execution may have occurred. The command was not repeated: {cause}"))
}
