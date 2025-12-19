# AI Code Assistant Context

## Project Overview

This project, `hussh`, is a Python library that provides a high-level, user-friendly interface for SSH operations. It is built on top of the Rust `ssh2` (synchronous) and `russh` (asynchronous) libraries, using `pyo3` to create Python bindings. This approach aims to combine the performance of a low-level language like Rust with the ease of use of Python.

The library supports various authentication methods, including password-based, key-based, and agent-based authentication. It provides functionalities for executing commands, file transfers using both SCP and SFTP, and managing interactive shell sessions.

**For detailed user-facing functionality and usage examples, see the README.md.**

## Design Priorities

When making changes or additions to this project, prioritize in this order:

1. **User Experience** - The API should be intuitive, Pythonic, and easy to use. Minimize boilerplate and provide clear error messages.
2. **Performance** - Leverage Rust's speed while maintaining Python's convenience.
3. **Reliability** - Ensure robust error handling and comprehensive test coverage.

## Building and Running

The project uses `maturin` for building and packaging the Rust-based Python extension.

*   **To build the project:**
    ```bash
    maturin build
    ```

*   **To install for development:**
    ```bash
    maturin develop
    ```
*   **To install the project:**
    ```bash
    uv pip install -e .[dev]
    ```

## Testing

**IMPORTANT: Always use `tox` for running tests.** This ensures tests are run in isolated environments across multiple Python versions.

*   **To run all tests across all Python versions:**
    ```bash
    tox
    ```

*   **To run tests for a specific Python version:**
    ```bash
    tox -e py312  # or py39, py310, py311, py313, py314
    ```

*   **To run only the test environments:**
    ```bash
    tox -m test
    ```

*   **To run linting:**
    ```bash
    tox -e lint
    ```

*   **To run benchmarks:**
    ```bash
    tox -e benchmarks-sync
    tox -e benchmarks-async
    ```

*   **To pass additional arguments to pytest:**
    ```bash
    tox -e py312 -- -k test_specific_test
    ```

**Do not use `pytest` directly** unless you have a specific reason to do so. The `tox` configuration ensures proper isolation and dependency management.

## Development Conventions

*   **Linting and Formatting:** The project uses `ruff` for code linting and formatting. The configuration can be found in the `pyproject.toml` file. Run linting with `tox -e lint`.
*   **Pre-commit Hooks:** The `.pre-commit-config.yaml` file indicates the use of pre-commit hooks to enforce code quality standards before commits.
*   **Testing:** Tests are written using the `pytest` framework and are located in the `tests/` directory. Always run tests through `tox`.
*   **Dependencies:** Python dependencies are managed in the `pyproject.toml` file, while Rust dependencies are managed in `Cargo.toml`.
*   **Continuous Integration:** The CI pipeline, defined in `.github/workflows/build_and_test.yml`, automates the process of building wheels for various architectures and operating systems, including Linux, macOS, and Windows.
