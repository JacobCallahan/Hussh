# Asynchronous Usage

This guide covers all features of Hussh's asynchronous `AsyncConnection` class for use with Python's `asyncio`.

## Table of Contents

- [Authentication](#authentication)
- [QuickStart](#quickstart)
- [Timeouts](#timeouts)
- [Executing Commands](#executing-commands)
- [SFTP](#sftp)
- [Interactive Shell](#interactive-shell)
- [Tailing Files](#tailing-files)

---

## Authentication

The `AsyncConnection` class supports the same authentication methods as the synchronous `Connection` class.

### Password Authentication
```python
import asyncio
from hussh.aio import AsyncConnection

async def main():
    # With username and password
    async with AsyncConnection(host="my.test.server", username="user", password="pass") as conn:
        result = await conn.execute("whoami")
        print(result.stdout)

asyncio.run(main())
```

### Key-Based Authentication
```python
async with AsyncConnection(host="my.test.server", key_path="~/.ssh/id_rsa") as conn:
    result = await conn.execute("whoami")

# With password-protected key
async with AsyncConnection(
    host="my.test.server",
    key_path="~/.ssh/id_rsa",
    password="keypass"
) as conn:
    result = await conn.execute("whoami")

# `private_key=` is also accepted as an alias for `key_path=`.
```

### SSH Agent Authentication
Async connections currently authenticate with explicit credentials or discovered default key files.
When no `password`, `key_path`, or `private_key` is provided, Hussh attempts default keys from `~/.ssh`.

### Jump Hosts and Proxy Commands
```python
jump = AsyncConnection("bastion.example.com", username="user", key_path="~/.ssh/id_rsa")
async with AsyncConnection("target.internal", username="user", proxy_jump=jump) as conn:
    await conn.execute("hostname")

async with AsyncConnection(
    "target.internal",
    username="user",
    proxy_command="ssh -W %h:%p bastion.example.com",
) as conn:
    await conn.execute("hostname")
```

```python
async with AsyncConnection("my.test.server") as conn:
    result = await conn.execute("whoami")
```

### Connection Lifecycle

The async context manager (`async with`) is the recommended pattern:

```python
async with AsyncConnection(host="my.test.server", password="pass") as conn:
    result = await conn.execute("ls")
# Connection is automatically closed
```

For manual management:
```python
conn = AsyncConnection(host="my.test.server", password="pass")
await conn.connect()
try:
    result = await conn.execute("ls")
finally:
    await conn.close()
```

---

## QuickStart

```python
import asyncio
from hussh.aio import AsyncConnection

async def main():
    async with AsyncConnection(host="my.test.server", username="user", password="pass") as conn:
        result = await conn.execute("ls")
    print(result.stdout)

asyncio.run(main())
```

---

## Timeouts

Async connections support timeouts at both the connection level and per-command.

### Connection-Level Timeout

Set a default timeout (in seconds) for all operations:

```python
async with AsyncConnection(host="my.test.server", timeout=10) as conn:
    # All commands will timeout after 10 seconds by default
    result = await conn.execute("sleep 5")  # OK - completes in 5s
```

### Per-Command Timeout

Override the default timeout for specific commands:

```python
async with AsyncConnection(host="my.test.server", timeout=30) as conn:
    # Use the default 30s timeout
    await conn.execute("quick_command")
    
    # Override with a shorter timeout
    try:
        await conn.execute("sleep 60", timeout=5)
    except TimeoutError:
        print("Command timed out!")
    
    # Override with a longer timeout for slow operations
    await conn.execute("long_running_backup", timeout=300)
```

### Handling Timeouts

```python
async with AsyncConnection(host="my.test.server", timeout=10) as conn:
    try:
        result = await conn.execute("sleep 20", timeout=5)
    except TimeoutError:
        print("The command took too long!")
        # Handle the timeout - maybe retry or alert
```

---

## Executing Commands

Commands are executed asynchronously using `await`:

```python
async with AsyncConnection(host="my.test.server", password="pass") as conn:
    result = await conn.execute("whoami")
    print(result.stdout, result.stderr, result.status)
```

### Concurrent Command Execution

One of the main benefits of async is running multiple commands concurrently:

```python
import asyncio

async def check_server(host, password):
    async with AsyncConnection(host=host, password=password) as conn:
        result = await conn.execute("uptime")
        return host, result.stdout

async def main():
    hosts = ["server1", "server2", "server3"]
    tasks = [check_server(host, "pass") for host in hosts]
    results = await asyncio.gather(*tasks)
    
    for host, uptime in results:
        print(f"{host}: {uptime}")

asyncio.run(main())
```

---

## SFTP

Async SFTP operations mirror the synchronous API with `await`.

### Writing Files

```python
async with AsyncConnection(host="my.test.server", password="pass") as conn:
    # Write a local file to the remote server
    await conn.sftp_write(local_path="/path/to/my/file", remote_path="/dest/path/file")

    # Write string data directly to a remote file
    await conn.sftp_write_data(data="Hello there!", remote_path="/dest/path/file")
```

### Reading Files

```python
async with AsyncConnection(host="my.test.server", password="pass") as conn:
    # Copy a remote file to a local destination
    await conn.sftp_read(remote_path="/dest/path/file", local_path="/path/to/my/file")

    # Read remote file contents directly into a string
    contents = await conn.sftp_read(remote_path="/dest/path/file")
    print(contents)
```

### Listing Directories

```python
async with AsyncConnection(host="my.test.server", password="pass") as conn:
    files = await conn.sftp_list("/remote/path")
    for file in files:
        print(file)
```

### Directory Transfers

Upload or download entire directory trees with a single awaitable call. Both methods return a tuple of `(transferred_files, failed_files)`, where each is a list of file paths.

```python
async with AsyncConnection(host="my.test.server", password="pass") as conn:
    # Upload an entire local directory to the remote server
    transferred, failed = await conn.sftp_put_dir(
        local_path="/local/build/",
        remote_path="/remote/app/",
    )
    if failed:
        print(f"Failed to upload: {failed}")

    # Download an entire remote directory to a local destination
    transferred, failed = await conn.sftp_get_dir(
        remote_path="/remote/logs/",
        local_path="/local/logs/",
    )
```

Optional keyword arguments control symlink, permission, and error-handling behaviour:

| Parameter | Default | Description |
|-----------|---------|-------------|
| `follow_symlinks` | `True` | Follow symlinks; set `False` to skip them |
| `preserve_permissions` | `True` | Mirror source permissions on the destination |
| `fail_fast` | `False` | Raise on the first error instead of collecting failures |

### Concurrent File Operations

```python
async with AsyncConnection(host="my.test.server", password="pass") as conn:
    # Upload multiple files concurrently
    await asyncio.gather(
        conn.sftp_write("/local/file1.txt", "/remote/file1.txt"),
        conn.sftp_write("/local/file2.txt", "/remote/file2.txt"),
        conn.sftp_write("/local/file3.txt", "/remote/file3.txt"),
    )
```

---

## Interactive Shell

The async interactive shell supports multiple read/write cycles, unlike the synchronous version.

### Basic Usage

```python
async with AsyncConnection(host="my.test.server", password="pass") as conn:
    async with await conn.shell() as shell:
        await shell.send("ls")
        result = await shell.read()
        print(result.stdout)
```

### Multiple Read/Write Cycles

One advantage of the async shell is the ability to read and write multiple times:

```python
async with AsyncConnection(host="my.test.server", password="pass") as conn:
    async with await conn.shell() as shell:
        # First command
        await shell.send("whoami")
        result = await shell.read()
        print(f"User: {result.stdout.strip()}")
        
        # Second command
        await shell.send("pwd")
        result = await shell.read()
        print(f"Directory: {result.stdout.strip()}")
        
        # Change directory and verify
        await shell.send("cd /tmp")
        await shell.send("pwd")
        result = await shell.read()
        print(f"New directory: {result.stdout.strip()}")
```

### Interactive Sessions

```python
async with AsyncConnection(host="my.test.server", password="pass") as conn:
    async with await conn.shell() as shell:
        # Set up environment
        await shell.send("export PATH=$PATH:/opt/myapp/bin")
        await shell.read()
        
        # Run application commands
        await shell.send("myapp --version")
        result = await shell.read()
        print(f"App version: {result.stdout}")
        
        await shell.send("myapp status")
        result = await shell.read()
        print(f"App status: {result.stdout}")
```

---

## Tailing Files

Async file tailing allows you to monitor files while performing other async operations.

### Basic Usage

```python
async with AsyncConnection(host="my.test.server", password="pass") as conn:
    async with conn.tail("/path/to/file.txt") as tf:
        # Do something that writes to the file
        await conn.execute("echo 'test' >> /path/to/file.txt")
        
        # Read new content
        content = await tf.read()
        print(content)
    
    # Access all contents after exiting
    print(tf.contents)
```

### Monitoring Logs During Operations

```python
async with AsyncConnection(host="my.test.server", password="pass") as conn:
    async with conn.tail("/var/log/app.log") as tf:
        # Restart a service
        await conn.execute("systemctl restart myapp")
        
        # Wait a bit for logs
        await asyncio.sleep(2)
        
        # Check startup logs
        logs = await tf.read()
        if "Started successfully" in logs:
            print("Service started OK")
        else:
            print(f"Startup logs: {logs}")
```

### Concurrent Tailing

```python
async with AsyncConnection(host="my.test.server", password="pass") as conn:
    async with conn.tail("/var/log/syslog") as syslog:
        async with conn.tail("/var/log/auth.log") as authlog:
            # Perform some operation
            await conn.execute("sudo -u testuser whoami")
            
            await asyncio.sleep(1)
            
            print("Syslog:", await syslog.read())
            print("Auth log:", await authlog.read())
```

## Connection recovery

`Connection` and `AsyncConnection` automatically recover a lost transport before a new
command, file operation, shell, or tailer is initialized. Recovery is enabled by default:
`auto_reconnect=True, max_retries=3, retry_interval=1.5` (intervals are seconds).
Choose `auto_reconnect=False` for immediate failure or when managing your own retries.
Async connections still require `await connect()` or context entry before their first
operation. Ordinary operations cannot reopen an explicitly closed connection.

Automatic recovery waits before each of up to three fresh connection attempts. By
default this adds up to 4.5 seconds of waiting, plus connection/authentication time.
Retry counts bound attempts, not total elapsed time; configure a connection timeout
when bounded failure latency matters. Authentication rejection, permission errors,
invalid configuration, and application errors are not retried.

Use `conn.reconnect()` (or `await conn.reconnect()`) to replace the session explicitly,
even with automatic recovery disabled or after `close()`. It returns `None`, makes
one immediate attempt, then up to `max_retries` additional attempts. Omitted values
inherit constructor settings. Overrides affect only that call:

```python
conn.reconnect(max_retries=5, retry_interval="exponential", max_interval=30.0)
# Async: await conn.reconnect(...)
```

Numeric intervals give fixed delays. Explicit `"exponential"` uses 1.5, 3, 6, ...
seconds, capped at `max_interval`. Retry counts must be nonnegative integers;
intervals must be nonnegative finite numbers. Exponential caps must be positive
and finite. `max_retries=0` disables automatic attempts but still allows one
explicit reconnect attempt.

Command timeouts cover channel setup and command execution; recovery delays and
fresh connection establishment are excluded. Async channel setup and execution
share the same timeout budget. Existing timeout units remain:
sync command/session timeouts use milliseconds; async timeouts use seconds.
Timeouts still raise `TimeoutError`. Exhausted recovery raises
`hussh.HusshConnectionError`, a `RuntimeError`, with the attempt count and cause.

Commands are **never replayed after submission starts**, including failures during
submission. An interrupted command raises `hussh.HusshIncompleteCommandError`
(a subclass of `HusshConnectionError`) because execution may already have occurred.
For a disruptive command such as a reboot, handle this uncertainty before reconnecting:

```python
from hussh import HusshIncompleteCommandError

try:
    conn.execute("reboot")
except HusshIncompleteCommandError:
    pass  # Verify the desired remote state before deciding to issue another command.
conn.reconnect()
```

Commands terminated by an SSH exit signal return an `SSHResult` with `status=-1`
and preserve stdout and stderr. A confirmed signal is a completed, unsuccessful
command, rather than an incomplete-command error.

Async `close()` interrupts pending connection establishment, channel/subsystem
preparation, and recovery sleeps before closing the connection.

A successful reconnect establishes a fresh authenticated session; it does not prove
the host rebooted. File transfers recover only during transport/subsystem setup;
once file access starts, failures propagate without replay, resumption, or rollback.
Partially written files may remain. Existing shells and initialized tailers stay
attached to their original sessions. `MultiConnection` inherits each connection's
retry settings and retains its per-host error aggregation; it has no group reconnect API.
