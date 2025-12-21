# MultiConnection Usage

This guide covers `MultiConnection`, which enables concurrent operations across multiple hosts with a simple synchronous API.

## Table of Contents

- [Overview](#overview)
- [Creating a MultiConnection](#creating-a-multiconnection)
- [Executing Commands](#executing-commands)
- [Working with MultiResult](#working-with-multiresult)
- [SFTP Operations](#sftp-operations)
- [Tailing Files](#tailing-files)
- [Connection Management](#connection-management)
- [Controlling Concurrency](#controlling-concurrency)

---

## Overview

`MultiConnection` allows you to execute commands or transfer files across multiple hosts simultaneously. It uses async operations internally but exposes a simple synchronous API, making it easy to use without writing async code.

```python
from hussh.multi_conn import MultiConnection

mc = MultiConnection.from_shared_auth(
    hosts=["server1", "server2", "server3"],
    username="user",
    password="pass",
)

with mc:
    results = mc.execute("whoami")
    for host, result in results.items():
        print(f"{host}: {result.stdout.strip()}")
```

---

## Creating a MultiConnection

### From AsyncConnection Instances

Create individual `AsyncConnection` objects with different configurations:

```python
from hussh.aio import AsyncConnection
from hussh.multi_conn import MultiConnection

connections = [
    AsyncConnection("server1", username="user", password="pass", port=22),
    AsyncConnection("server2", username="admin", private_key="~/.ssh/id_rsa"),
    AsyncConnection("server3", username="user", password="pass", port=2222),
]

mc = MultiConnection(connections, batch_size=50)
```

### From Sync Connection Instances

Convert existing synchronous `Connection` objects:

```python
from hussh import Connection
from hussh.multi_conn import MultiConnection

connections = [
    Connection("server1", username="user", password="pass"),
    Connection("server2", username="user", password="pass"),
]

mc = MultiConnection.from_connections(connections)
```

### With Shared Authentication

The simplest approach when all hosts use the same credentials:

```python
mc = MultiConnection.from_shared_auth(
    hosts=["server1", "server2", "server3"],
    username="user",
    password="pass",
    port=22,
    batch_size=100,  # max concurrent operations
)
```

With key-based authentication:

```python
mc = MultiConnection.from_shared_auth(
    hosts=["server1", "server2", "server3"],
    private_key="~/.ssh/id_rsa",
)
```

---

## Executing Commands

### Same Command on All Hosts

```python
with MultiConnection.from_shared_auth(hosts, password="pass") as mc:
    results = mc.execute("uptime")
    for host, result in results.items():
        print(f"{host}: {result.stdout.strip()}")
```

### Different Commands per Host

Use `execute_map` to run different commands on different hosts:

```python
with MultiConnection.from_shared_auth(hosts, password="pass") as mc:
    command_map = {
        "web1": "systemctl status nginx",
        "web2": "systemctl status nginx",
        "db1": "systemctl status postgresql",
        "cache1": "systemctl status redis",
    }
    results = mc.execute_map(command_map)
    
    for host, result in results.items():
        status = "running" if result.status == 0 else "stopped"
        print(f"{host}: {status}")
```

---

## Working with MultiResult

The `execute` and other methods return a `MultiResult` object, which behaves like a dictionary mapping hostnames to `SSHResult` objects.

### Basic Usage

```python
with MultiConnection.from_shared_auth(hosts, password="pass") as mc:
    results = mc.execute("whoami")

    # Iterate like a dictionary
    for host, result in results.items():
        print(f"{host}: status={result.status}")

    # Access specific hosts
    print(results["server1"].stdout)

    # Get all hostnames
    print(list(results.keys()))

    # Get all results
    print(list(results.values()))
```

### Filtering by Success/Failure

```python
with MultiConnection.from_shared_auth(hosts, password="pass") as mc:
    results = mc.execute("some_command")

    # Get only successful results (status == 0)
    succeeded = results.succeeded
    print(f"Succeeded on {len(succeeded)} hosts")

    # Get only failed results (status != 0)
    failed = results.failed
    print(f"Failed on {len(failed)} hosts")

    # Process failures
    for host, result in failed.items():
        print(f"{host} failed: {result.stderr}")
```

### Handling Partial Failures

When some hosts fail, you have two options:

#### Option 1: Check and Handle Manually

```python
with MultiConnection.from_shared_auth(hosts, password="pass") as mc:
    results = mc.execute("risky_command")

    if results.failed:
        print("Some hosts failed:")
        for host, result in results.failed.items():
            print(f"  {host}: {result.stderr}")
    
    if results.succeeded:
        print("Succeeded on:")
        for host in results.succeeded.keys():
            print(f"  {host}")
```

#### Option 2: Raise an Exception

```python
from hussh.multi_conn import MultiConnection, PartialFailureException

with MultiConnection.from_shared_auth(hosts, password="pass") as mc:
    results = mc.execute("some_command")

    try:
        results.raise_if_any_failed()
        print("All hosts succeeded!")
    except PartialFailureException as e:
        print(f"Partial failure: {e}")
        print(f"Succeeded: {list(e.succeeded.keys())}")
        print(f"Failed: {list(e.failed.keys())}")
        
        # Access individual failures
        for host, result in e.failed.items():
            print(f"  {host}: exit code {result.status}")
```

---

## SFTP Operations

### Writing Data to All Hosts

```python
with MultiConnection.from_shared_auth(hosts, password="pass") as mc:
    # Write string data to all hosts
    mc.sftp_write_data("config_value=123", "/etc/myapp/config.txt")
```

### Writing Files to All Hosts

```python
with MultiConnection.from_shared_auth(hosts, password="pass") as mc:
    # Copy a local file to all hosts
    mc.sftp_write("/local/path/app.tar.gz", "/opt/app.tar.gz")
```

### Reading Files from All Hosts

```python
with MultiConnection.from_shared_auth(hosts, password="pass") as mc:
    results = mc.sftp_read("/var/log/app.log")
    
    for host, result in results.items():
        print(f"=== {host} ===")
        print(result.stdout[:500])  # First 500 chars
        print()
```

### Deployment Example

```python
with MultiConnection.from_shared_auth(hosts, password="pass") as mc:
    # Upload new configuration
    mc.sftp_write("/local/nginx.conf", "/etc/nginx/nginx.conf")
    
    # Restart nginx on all hosts
    results = mc.execute("systemctl restart nginx")
    
    # Check for failures
    if results.failed:
        print("Failed to restart nginx on:")
        for host in results.failed.keys():
            print(f"  {host}")
```

---

## Tailing Files

### Same File on All Hosts

```python
with MultiConnection.from_shared_auth(hosts, password="pass") as mc:
    with mc.tail("/var/log/syslog") as tailer:
        # Perform an action that generates logs
        mc.execute("logger 'Test message from hussh'")
        
        import time
        time.sleep(1)
        
        # Read new content from all hosts
        contents = tailer.read()
        for host, content in contents.items():
            print(f"{host}: {content}")
```

### Different Files per Host

Use `tail_map` when hosts have different log locations:

```python
with MultiConnection.from_shared_auth(hosts, password="pass") as mc:
    file_map = {
        "web1": "/var/log/nginx/access.log",
        "web2": "/var/log/nginx/access.log",
        "app1": "/var/log/myapp/app.log",
        "db1": "/var/log/postgresql/postgresql.log",
    }
    
    with mc.tail_map(file_map) as tailer:
        # Perform operations...
        mc.execute("curl -s http://localhost/health")
        
        import time
        time.sleep(1)
        
        contents = tailer.read()
        for host, content in contents.items():
            if content:
                print(f"{host}: {content[:200]}...")
```

---

## Connection Management

### Using the Context Manager (Recommended)

```python
with MultiConnection.from_shared_auth(hosts, password="pass") as mc:
    results = mc.execute("whoami")
# All connections are automatically closed
```

### Manual Connection Management

```python
mc = MultiConnection.from_shared_auth(hosts, password="pass")

# Explicitly connect
connect_results = mc.connect()

# Check for connection failures
if connect_results.failed:
    print("Failed to connect to:")
    for host in connect_results.failed.keys():
        print(f"  {host}")

# Do work
results = mc.execute("whoami")

# Don't forget to close!
mc.close()
```

### Pruning Failed Connections

Remove hosts that fail to connect from the pool:

```python
mc = MultiConnection.from_shared_auth(hosts, password="pass")

# prune_failures=True removes failed hosts from the pool
mc.connect(prune_failures=True)

# Now mc only contains successfully connected hosts
print(f"Connected to {len(mc.hosts)} out of {len(hosts)} hosts")

# Operations only run on connected hosts
results = mc.execute("whoami")

mc.close()
```

This is useful when you want to proceed with available hosts rather than failing completely.

---

## Controlling Concurrency

The `batch_size` parameter limits how many operations run concurrently, preventing resource exhaustion when working with many hosts.

### Setting Batch Size

```python
# Default batch size
mc = MultiConnection.from_shared_auth(hosts, password="pass")

# Custom batch size
mc = MultiConnection.from_shared_auth(
    hosts=large_host_list,  # e.g., 1000 hosts
    password="pass",
    batch_size=50,  # Only 50 concurrent operations
)
```

### When to Adjust Batch Size

**Increase batch size** (e.g., 100-200) when:
- You have plenty of local resources (CPU, memory, network)
- Operations are quick and lightweight
- Minimizing total time is critical

**Decrease batch size** (e.g., 10-25) when:
- Local resources are limited
- Operations are resource-intensive
- Remote hosts have connection limits
- You're experiencing connection timeouts

### Example with Large Host Lists

```python
# Processing 1000 servers in batches of 50
hosts = [f"server{i}.example.com" for i in range(1000)]

mc = MultiConnection.from_shared_auth(
    hosts=hosts,
    username="deploy",
    private_key="~/.ssh/deploy_key",
    batch_size=50,
)

with mc:
    # This will process 50 hosts at a time
    results = mc.execute("apt-get update && apt-get upgrade -y")
    
    print(f"Succeeded: {len(results.succeeded)}")
    print(f"Failed: {len(results.failed)}")
```
