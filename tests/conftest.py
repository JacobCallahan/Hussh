"""Common setup for Hussh tests."""

import os
from pathlib import PurePath
import subprocess
import time

import docker
import pexpect
import pytest

TESTDIR = PurePath(__file__).parent


@pytest.fixture(scope="session")
def ensure_test_server_image():
    """Ensure that the test server Docker image is available."""
    client = docker.from_env()
    try:
        client.images.get("hussh-test-server")
    except docker.errors.ImageNotFound:
        client.images.build(
            path=str(TESTDIR / "setup"),
            tag="hussh-test-server",
        )
    client.close()


@pytest.fixture(scope="session", autouse=True)
def run_test_server(ensure_test_server_image):
    """Run a test server in a Docker container."""
    client = docker.from_env()
    try:  # check to see if the container is already running
        container = client.containers.get("hussh-test-server")
    except docker.errors.NotFound:  # if not, start it
        container = client.containers.run(
            "hussh-test-server",
            detach=True,
            ports={"22/tcp": 8022},
            name="hussh-test-server",
        )
        time.sleep(5)  # give the server time to start
    yield container
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
            "hussh-test-server",
            detach=True,
            ports={"22/tcp": 8023},
            name="hussh-test-server2",
        )
        time.sleep(5)  # give the server time to start
    yield container
    container.stop()
    container.remove()
    client.close()


@pytest.fixture(scope="session")
def setup_agent_auth():
    # Define the key paths
    base_key = TESTDIR / "data/test_key"
    auth_key = TESTDIR / "data/auth_test_key"

    # Start the ssh-agent and get the environment variables
    output = subprocess.check_output(["ssh-agent", "-s"])
    env = dict(line.split("=", 1) for line in output.decode().splitlines() if "=" in line)

    # Set the SSH_AUTH_SOCK and SSH_AGENT_PID environment variables
    os.environ["SSH_AUTH_SOCK"] = env["SSH_AUTH_SOCK"]
    os.environ["SSH_AGENT_PID"] = env["SSH_AGENT_PID"]

    # Add the keys to the ssh-agent
    # subprocess.run(["ssh-add", str(base_key)], check=True)
    result = subprocess.run(
        ["ssh-add", str(base_key)], capture_output=True, text=True, check=False
    )
    print(result.stdout)
    print(result.stderr)
    # The auth_key is password protected
    child = pexpect.spawn("ssh-add", [str(auth_key)])
    child.expect("Enter passphrase for .*: ")
    child.sendline("husshpuppy")
    yield
    # Kill the ssh-agent after the tests have run
    subprocess.run(["ssh-agent", "-k"], check=True)
