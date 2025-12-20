"""Common setup for Hussh tests.

Test Server Setup
-----------------
Most tests require SSH test servers running in Docker containers.
The fixtures will automatically start/stop containers as needed.

MultiConnection Test Setup
--------------------------
The MultiConnection tests require unique hostnames that resolve to 127.0.0.1.
Add the following line to your /etc/hosts file:

    127.0.0.1 hussh-server-1.test hussh-server-2.test hussh-server-3.test
    hussh-server-4.test hussh-server-5.test

On Linux/macOS:
    echo "127.0.0.1 hussh-server-1.test hussh-server-2.test hussh-server-3.test \
        hussh-server-4.test hussh-server-5.test" | sudo tee -a /etc/hosts

If these hostnames are not configured, the MultiConnection tests will be skipped.

GitHub Actions automatically configures these hostnames in the CI workflow.
"""

import os
from pathlib import Path, PurePath
import subprocess
import time

import docker
import pexpect
import pytest

TESTDIR = PurePath(__file__).parent
TEST_SERVER_IMAGE = "ghcr.io/jacobcallahan/hussh/hussh-test-server:latest"
BASE_PORT = 8022  # First server port, subsequent servers use BASE_PORT + 1, etc.

# Unique hostnames for multi-connection tests
# These must be mapped to 127.0.0.1 in /etc/hosts (see module docstring)
TEST_HOSTNAMES = [
    "hussh-server-1.test",
    "hussh-server-2.test",
    "hussh-server-3.test",
    "hussh-server-4.test",
    "hussh-server-5.test",
]


def pytest_addoption(parser):
    """Add command-line options for test configuration."""
    parser.addoption(
        "--num-servers",
        action="store",
        default="2",
        help="Number of test SSH servers to spawn (default: 2, max: 5)",
    )


@pytest.fixture(scope="session")
def num_servers(request):
    """Get the number of test servers to spawn."""
    num = int(request.config.getoption("--num-servers"))
    return min(max(num, 1), 5)  # Clamp between 1 and 5


@pytest.fixture(scope="session")
def ensure_test_server_image():
    """Ensure that the test server Docker image is available."""
    client = docker.from_env()
    try:
        client.images.get(TEST_SERVER_IMAGE)
    except docker.errors.ImageNotFound:
        client.images.pull(TEST_SERVER_IMAGE)
    client.close()


@pytest.fixture(scope="session", autouse=True)
def run_test_server(ensure_test_server_image):
    """Run a test server in a Docker container."""
    client = docker.from_env()
    try:  # check to see if the container is already running
        container = client.containers.get("hussh-test-server")
        if container.status != "running":
            container.start()
            time.sleep(5)  # give the server time to start
        managed = False
    except docker.errors.NotFound:  # if not, start it
        container = client.containers.run(
            TEST_SERVER_IMAGE,
            detach=True,
            ports={"22/tcp": 8022},
            name="hussh-test-server",
        )
        managed = True
        time.sleep(5)  # give the server time to start
    yield container
    if managed:
        container.stop()
        container.remove()
    client.close()


@pytest.fixture(scope="session")
def run_second_server(ensure_test_server_image):
    """Run a test server in a Docker container."""
    client = docker.from_env()
    try:  # check to see if the container is already running
        container = client.containers.get("hussh-test-server2")
    except docker.errors.NotFound:  # if not, start it
        container = client.containers.run(
            TEST_SERVER_IMAGE,
            detach=True,
            ports={"22/tcp": 8023},
            name="hussh-test-server2",
        )
        time.sleep(5)  # give the server time to start
    yield container
    container.stop()
    container.remove()
    client.close()


def _check_hostname_resolution(hostname):
    """Check if a hostname resolves (to any address)."""
    import socket

    try:
        socket.gethostbyname(hostname)
        return True
    except socket.gaierror:
        return False


@pytest.fixture(scope="session")
def run_test_servers(ensure_test_server_image, num_servers):  # noqa: PLR0912
    """Run multiple test SSH servers for MultiConnection tests.

    Returns a list of tuples: [(hostname, port), ...]

    Each server gets a unique hostname from TEST_HOSTNAMES.
    These hostnames should resolve to 127.0.0.1 (add to /etc/hosts if needed):
        127.0.0.1 hussh-server-1.test hussh-server-2.test hussh-server-3.test ...

    Note: Uses the same naming convention as run_test_server and run_second_server
    for ports 8022 and 8023 to allow container reuse.
    """
    # Check that hostnames are configured
    missing_hosts = []
    for i in range(num_servers):
        hostname = TEST_HOSTNAMES[i]
        if not _check_hostname_resolution(hostname):
            missing_hosts.append(hostname)

    if missing_hosts:
        hosts_line = "127.0.0.1 " + " ".join(TEST_HOSTNAMES[:num_servers])
        pytest.skip(
            f"Test hostnames not configured. Add to /etc/hosts:\n{hosts_line}\n"
            f"Missing: {', '.join(missing_hosts)}"
        )

    client = docker.from_env()
    containers = []
    server_info = []
    managed_indices = []

    for i in range(num_servers):
        port = BASE_PORT + i
        hostname = TEST_HOSTNAMES[i]
        # Use same naming as existing fixtures for compatibility
        if i == 0:
            container_name = "hussh-test-server"
        elif i == 1:
            container_name = "hussh-test-server2"
        else:
            container_name = f"hussh-test-server-{i}"

        try:
            container = client.containers.get(container_name)
            if container.status != "running":
                container.start()
            containers.append(container)
        except docker.errors.NotFound:
            container = client.containers.run(
                TEST_SERVER_IMAGE,
                detach=True,
                ports={"22/tcp": port},
                name=container_name,
            )
            containers.append(container)
            managed_indices.append(i)

        server_info.append((hostname, port))

    # Give servers time to start
    if managed_indices:
        time.sleep(5)

    yield server_info

    # Clean up only the containers we created
    for i in managed_indices:
        try:
            containers[i].stop()
            containers[i].remove()
        except Exception:
            pass  # Ignore cleanup errors

    client.close()


@pytest.fixture(scope="session")
def setup_agent_auth():
    # Define the key paths
    base_key = Path(TESTDIR / "data/test_key")
    auth_key = Path(TESTDIR / "data/auth_test_key")

    # Ensure proper permissions on the keys
    base_key.chmod(0o600)
    auth_key.chmod(0o600)

    # Start the ssh-agent and get the environment variables
    output = subprocess.check_output(["ssh-agent", "-s"])
    env = {}
    for line in output.decode().splitlines():
        if "=" in line and not line.startswith("echo"):
            key, value = line.split("=", 1)
            # Strip trailing semicolons and 'export' suffix
            value = value.split(";")[0].strip()
            env[key.strip()] = value

    # Set the SSH_AUTH_SOCK and SSH_AGENT_PID environment variables
    os.environ["SSH_AUTH_SOCK"] = env["SSH_AUTH_SOCK"]
    os.environ["SSH_AGENT_PID"] = env["SSH_AGENT_PID"]

    # Add the keys to the ssh-agent
    result = subprocess.run(
        ["ssh-add", str(base_key)], capture_output=True, text=True, check=False
    )
    if result.returncode != 0:
        raise RuntimeError(f"Failed to add key to agent: {result.stderr}")

    # The auth_key is password protected
    child = pexpect.spawn("ssh-add", [str(auth_key)])
    child.expect("Enter passphrase for .*: ")
    child.sendline("husshpuppy")
    child.expect(pexpect.EOF)
    child.close()
    if child.exitstatus != 0:
        raise RuntimeError("Failed to add password-protected key to agent")

    yield

    # Kill the ssh-agent after the tests have run
    agent_pid = env["SSH_AGENT_PID"]
    result = subprocess.run(["kill", agent_pid], check=False, capture_output=True)
    if result.returncode != 0:
        print(f"Warning: Failed to kill ssh-agent (PID {agent_pid}): {result.stderr.decode()}")
