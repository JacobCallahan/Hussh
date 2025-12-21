# Hussh Documentation

Welcome to the full documentation for Hussh. This directory contains detailed guides and examples for all of Hussh's features.

## Documentation Index

| Guide | Description |
|-------|-------------|
| [Synchronous Usage](synchronous.md) | Complete guide to the `Connection` class including authentication, executing commands, SFTP, SCP, file tailing, and interactive shells |
| [Asynchronous Usage](asynchronous.md) | Complete guide to the `AsyncConnection` class with async/await patterns, timeouts, and all async operations |
| [MultiConnection Usage](multi-connection.md) | Concurrent operations across multiple hosts using `MultiConnection`, including batch operations and error handling |

## Quick Links

- **Getting Started**: See the main [README](../README.md) for installation and quickstart examples
- **Benchmarks**: Performance comparisons are in the main README and the `benchmarks/` directory
- **Source Code**: The Rust implementation is in `src/` and Python bindings are exposed via PyO3

## Need Help?

- Check the [GitHub Issues](https://github.com/jacobcallahan/hussh/issues) for known issues
- Open a new issue if you find a bug or have a feature request
