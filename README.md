# Hussh: SSH for humans.
[![image](https://img.shields.io/pypi/v/hussh.svg)](https://pypi.python.org/pypi/hussh)
[![image](https://img.shields.io/pypi/pyversions/hussh.svg)](https://pypi.python.org/pypi/hussh)
![PyPI - Wheel](https://img.shields.io/pypi/wheel/hussh)
[![Actions status](https://github.com/jacobcallahan/hussh/actions/workflows/build_and_test.yml/badge.svg)](https://github.com/jacobcallahan/hussh/actions)

Hussh (pronounced "hush") is a client-side ssh library that offers low level performance through a high level interface.

Hussh uses [pyo3](https://docs.rs/pyo3/latest/pyo3/) to create Python bindings around the [ssh2](https://docs.rs/ssh2/latest/ssh2/) library for Rust.

# Installation
```
pip install hussh
```

# QuickStart
Hussh currently just offers a `Connection` class as your primary interface.
```python
from hussh import Connection

conn = Connection(host="my.test.server", username="user", password="pass")
result = conn.execute("ls")
print(result.stdout)
```

That's it! One import and class instantion is all you need to:
- Execute commands
- Perform SCP actions
- Perform SFTP actions
- Get an interactive shell

# Why Hussh?
- 🔥 Blazingly fast!
- 🪶 Incredibly lightweight!
- 🧠 Super easy to use!

## Benchmarks
Hussh demonstrates the performance you'd expect from a low level ssh library.
Hussh is also much lighter weight in both total memory and memory allocations.

Local Server
![Local Server Benchmarks](benchmarks/local_server_bench.png)

Remote Server
![Remote Server Benchmarks](benchmarks/remote_server_bench.png)

### Try it for yourself!
Hussh's benchmark script are also open sourced in the `benchmarks` directory in this repository.
Clone the repo, follow the setup instructions, then let us know how it did!

# Authentication
You've already seen password-based authentication, but here it is again.
```python
conn = Connection(host="my.test.server", username="user", password="pass")

#  or leave out username and connect as root
conn = Connection(host="my.test.server", password="pass")
```

If you prefer key-based authentication, Hussh can do that as well.
```python
conn = Connection(host="my.test.server", private_key="~/.ssh/id_rsa")

# If your key is password protected, just use the password argument
conn = Connection(host="my.test.server", private_key="~/.ssh/id_rsa", password="pass")
```

Hussh can also do agent-based authentication, if you've already established it.
```python
conn = Connection("my.test.server")
```

# Executing commands
The most basic foundation of ssh libraries is the ability to execute commands against the remote host.
For Hussh, just use the `Connection` object's `execute` method.
```python
result = conn.execute("whoami")
print(result.stdout, result.stderr, result.status)
```
Each execute returns an `SSHResult` object with command's stdout, stderr, and status.

# SFTP
If you need to transfer files to/from the remote host, SFTP may be your best bet.

## Writing Files and Data
```python
# write a local file to the remote destination
conn.sftp_write(local_path="/path/to/my/file", remote_path="/dest/path/file")

# Write UTF-8 data to a remote file
conn.sftp_write_data(data="Hello there!", remote_path="/dest/path/file")
```

## Reading Files
```python
# You can copy a remote file to a local destination
conn.sftp_read(remote_path="/dest/path/file", local_path="/path/to/my/file")
# Or copy the remote file contents to a string
contents = conn.sftp_read(remote_path="/dest/path/file")
```

## Copy files from one connection to another
Hussh offers a shortcut that allows you to copy a file between two established connections.
```python
source_conn = Connection("my.first.server")
dest_conn = Connection("my.second.server", password="secret")
# Copy from source to destination
source_conn.remote_copy(source_path="/root/myfile.txt", dest_conn=dest_conn)
```
By default, if you don't pass in an alternate `dest_path`, Hussh will copy it to the same path as it came from on source.


# SCP
For remote servers that support SCP, Hussh can do that to.

## Writing Files and Data
```python
# write a local file to the remote destination
conn.scp_write(local_path="/path/to/my/file", remote_path="/dest/path/file")

# Write UTF-8 data to a remote file
conn.scp_write_data(data="Hello there!", remote_path="/dest/path/file")
```

## Reading Files
```python
# You can copy a remote file to a local destination
conn.scp_read(remote_path="/dest/path/file", local_path="/path/to/my/file")
# Or copy the remote file contents to a string
contents = conn.scp_read(remote_path="/dest/path/file")
```

# Tailing Files
Hussh offers a built-in method for tailing files on a `Connection` with the `tail` method.
```python
with conn.tail("/path/to/file.txt") as tf:
   # perform some actions or wait
   print(tf.read())  # at any time, you can read any unread contents
   # when you're done tailing, exit the context manager
print(tf.contents)
```

# Interactive Shell
If you need to keep a shell open to perform more complex interactions, you can get an `InteractiveShell` instance from the `Connection` class instance.
To use the interactive shell, it is recommended to use the `shell()` context manager from the `Connection` class.
You can send commands to the shell using the `send` method, then get the results from `result` when you exit the context manager.

```python
with conn.shell() as shell:
   shell.send("ls")
   shell.send("pwd")
   shell.send("whoami")

print(shell.result.stdout)
```
**Note:** The `read` method sends an EOF to the shell, so you won't be able to send more commands after calling `read`. If you want to send more commands, you would need to create a new `InteractiveShell` instance.

# Disclaimer
This is a VERY early project that should not be used in production code!
There isn't even proper exception handling, so try/except won't work.
With that said, try it out and let me know your thoughts!

# Future Features
- Proper exception handling
- Concurrent actions class
- Async Connection class
- Low level bindings
- Misc codebase improvements
- TBD...
