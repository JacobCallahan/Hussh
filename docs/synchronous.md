# Synchronous Usage

This guide covers all features of Hussh's synchronous `Connection` class.

## Table of Contents

- [Authentication](#authentication)
- [Executing Commands](#executing-commands)
- [SFTP](#sftp)
- [SCP](#scp)
- [Tailing Files](#tailing-files)
- [Interactive Shell](#interactive-shell)

---

## Authentication

Hussh supports multiple authentication methods: password, key-based, and SSH agent.

### Password Authentication
```python
from hussh import Connection

# With username and password
conn = Connection(host="my.test.server", username="user", password="pass")

# Leave out username to connect as root
conn = Connection(host="my.test.server", password="pass")
```

### Key-Based Authentication
```python
# Using a private key file
conn = Connection(host="my.test.server", private_key="~/.ssh/id_rsa")

# If your key is password protected, use the password argument
conn = Connection(host="my.test.server", private_key="~/.ssh/id_rsa", password="pass")
```

### SSH Agent Authentication
If you have an SSH agent running with your keys loaded:
```python
conn = Connection("my.test.server")
```

### Connection Lifecycle

Hussh automatically cleans up connections when the `Connection` object is garbage collected. However, you can also manage the lifecycle explicitly.

#### Explicit Close
```python
conn = Connection(host="my.test.server", password="pass")
# ... do work ...
conn.close()
```

#### Context Manager (Recommended)
```python
with Connection(host="my.test.server", password="pass") as conn:
    result = conn.execute("ls")
# Connection is automatically closed when exiting the context
assert result.status == 0
```

---

## Executing Commands

The `execute` method is the foundation for running commands on the remote host.

```python
result = conn.execute("whoami")
print(result.stdout, result.stderr, result.status)
```

Each `execute` call returns an `SSHResult` object with three attributes:
- `stdout`: The standard output from the command
- `stderr`: The standard error from the command
- `status`: The exit code (0 typically means success)

### Examples

```python
# Simple command
result = conn.execute("ls -la")
print(result.stdout)

# Check command success
result = conn.execute("systemctl status nginx")
if result.status == 0:
    print("nginx is running")
else:
    print(f"nginx check failed: {result.stderr}")

# Chain commands
result = conn.execute("cd /var/log && tail -n 100 syslog")
```

---

## SFTP

SFTP (SSH File Transfer Protocol) is the recommended way to transfer files.

### Writing Files

```python
# Write a local file to the remote server
conn.sftp_write(local_path="/path/to/my/file", remote_path="/dest/path/file")

# Write string data directly to a remote file
conn.sftp_write_data(data="Hello there!", remote_path="/dest/path/file")
```

### Reading Files

```python
# Copy a remote file to a local destination
conn.sftp_read(remote_path="/dest/path/file", local_path="/path/to/my/file")

# Read remote file contents directly into a string
contents = conn.sftp_read(remote_path="/dest/path/file")
print(contents)
```

### Copy Files Between Connections

Hussh provides a convenient way to copy files between two remote servers:

```python
source_conn = Connection("my.first.server")
dest_conn = Connection("my.second.server", password="secret")

# Copy from source to destination (same path)
source_conn.remote_copy(source_path="/root/myfile.txt", dest_conn=dest_conn)

# Copy to a different path on destination
source_conn.remote_copy(
    source_path="/root/myfile.txt",
    dest_conn=dest_conn,
    dest_path="/tmp/myfile.txt"
)
```

---

## SCP

For servers that support SCP (Secure Copy Protocol), Hussh provides SCP operations that mirror the SFTP API.

### Writing Files

```python
# Write a local file to the remote server
conn.scp_write(local_path="/path/to/my/file", remote_path="/dest/path/file")

# Write string data directly to a remote file
conn.scp_write_data(data="Hello there!", remote_path="/dest/path/file")
```

### Reading Files

```python
# Copy a remote file to a local destination
conn.scp_read(remote_path="/dest/path/file", local_path="/path/to/my/file")

# Read remote file contents directly into a string
contents = conn.scp_read(remote_path="/dest/path/file")
print(contents)
```

---

## Tailing Files

Hussh provides a built-in method for tailing files, useful for monitoring logs or watching file changes.

```python
with conn.tail("/path/to/file.txt") as tf:
    # Perform some actions that might write to the file
    conn.execute("echo 'new line' >> /path/to/file.txt")
    
    # Read any new content at any time
    new_content = tf.read()
    print(new_content)
    
    # Continue doing other work...
    conn.execute("echo 'another line' >> /path/to/file.txt")

# After exiting the context, you can access all accumulated contents
print(tf.contents)
```

### Use Cases

```python
# Monitor a log file during an operation
with conn.tail("/var/log/app.log") as tf:
    conn.execute("systemctl restart myapp")
    import time
    time.sleep(2)  # Wait for logs
    print("Startup logs:", tf.read())

# Watch for specific content
with conn.tail("/var/log/syslog") as tf:
    conn.execute("logger 'Test message from hussh'")
    if "Test message" in tf.read():
        print("Message logged successfully!")
```

---

## Interactive Shell

For complex interactions that require maintaining state between commands, use an interactive shell.

### Basic Usage

```python
with conn.shell() as shell:
    shell.send("cd /var/log")
    shell.send("ls -la")
    shell.send("pwd")

# Access all output after exiting the context
print(shell.result.stdout)
```

### Important Notes

- The `shell()` context manager is the recommended way to use interactive shells
- Commands sent via `send()` are executed sequentially in the same shell session
- Environment variables and working directory persist between commands
- The `read()` method sends an EOF to the shell, ending the session

### Examples

```python
# Set up environment and run commands
with conn.shell() as shell:
    shell.send("export MY_VAR='hello'")
    shell.send("echo $MY_VAR")
    shell.send("cd /tmp && mkdir -p test_dir && cd test_dir")
    shell.send("pwd")

print(shell.result.stdout)
# Output will show 'hello' and '/tmp/test_dir'

# Interactive script execution
with conn.shell() as shell:
    shell.send("python3 << 'EOF'")
    shell.send("import sys")
    shell.send("print(f'Python {sys.version}')")
    shell.send("EOF")

print(shell.result.stdout)
```
