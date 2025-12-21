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
async with AsyncConnection(host="my.test.server", private_key="~/.ssh/id_rsa") as conn:
    result = await conn.execute("whoami")

# With password-protected key
async with AsyncConnection(
    host="my.test.server",
    private_key="~/.ssh/id_rsa",
    password="keypass"
) as conn:
    result = await conn.execute("whoami")
```

### SSH Agent Authentication
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
