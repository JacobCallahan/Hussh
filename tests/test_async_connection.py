import asyncio
from pathlib import Path

import pytest

from hussh.aio import AsyncConnection


@pytest.mark.asyncio
async def test_async_connection_basic(run_test_server):
    async with AsyncConnection("localhost", username="root", password="toor", port=8022) as conn:
        result = await conn.execute("echo hello")
        assert result.stdout.strip() == "hello"
        assert result.status == 0


@pytest.mark.asyncio
async def test_async_connection_manual(run_test_server):
    conn = AsyncConnection("localhost", username="root", password="toor", port=8022)
    await conn.connect()

    result = await conn.execute("echo manual")
    assert result.stdout.strip() == "manual"
    assert result.status == 0

    await conn.close()


@pytest.mark.asyncio
async def test_async_sftp(run_test_server, tmp_path):
    async with AsyncConnection("localhost", username="root", password="toor", port=8022) as conn:
        sftp = await conn.sftp()

        # Test put
        local_file = tmp_path / "test_put.txt"
        local_file.write_text("hello sftp")
        remote_path = "/root/test_put.txt"

        await sftp.put(str(local_file), remote_path)

        # Test list
        files = await sftp.list("/root")
        assert "test_put.txt" in files

        # Test get
        download_path = tmp_path / "test_get.txt"
        await sftp.get(remote_path, str(download_path))

        assert download_path.read_text() == "hello sftp"


@pytest.mark.asyncio
async def test_async_shell(run_test_server):
    async with (
        AsyncConnection("localhost", username="root", password="toor", port=8022) as conn,
        await conn.shell(pty=True) as shell,
    ):
        # We need to wait a bit for the shell to be ready and print the prompt
        await asyncio.sleep(0.5)
        _ = await shell.read()  # Clear initial banner/prompt

        await shell.send("echo hello shell")

        # Give it a moment to process
        await asyncio.sleep(0.5)

        result = await shell.read()
        # Output will contain the echo command itself and the result
        assert "hello shell" in result.stdout


@pytest.mark.asyncio
async def test_async_file_tailer(run_test_server, tmp_path):
    async with AsyncConnection("localhost", username="root", password="toor", port=8022) as conn:
        # Create a test file on the remote server
        await conn.execute("echo 'line 1' > /root/test_tail.log")
        await conn.execute("echo 'line 2' >> /root/test_tail.log")

        async with conn.tail("/root/test_tail.log") as tailer:
            # Initially, we should be at the end of the file
            content = await tailer.read()
            assert content == ""

            # Add more lines
            await conn.execute("echo 'line 3' >> /root/test_tail.log")

            # Read from last position
            new_content = await tailer.read()
            assert "line 3" in new_content

        # Check contents after exit
        assert "line 3" in tailer.contents


@pytest.mark.asyncio
async def test_async_password_auth(run_test_server):
    """Test that we can establish a connection with password-based authentication."""
    async with AsyncConnection("localhost", port=8022, password="toor", username="root") as conn:
        result = await conn.execute("echo hello")
        assert result.status == 0


@pytest.mark.asyncio
async def test_async_key_auth(run_test_server):
    """Test that we can establish a connection with key-based authentication."""
    async with AsyncConnection(
        "localhost", port=8022, username="root", key_path="tests/data/test_key"
    ) as conn:
        result = await conn.execute("echo hello")
        assert result.status == 0


@pytest.mark.asyncio
async def test_async_key_with_password_auth(run_test_server):
    """Test that we can establish a connection with key-based authentication and a password."""
    async with AsyncConnection(
        "localhost",
        port=8022,
        username="root",
        key_path="tests/data/auth_test_key",
        password="husshpuppy",
    ) as conn:
        result = await conn.execute("echo hello")
        assert result.status == 0


@pytest.mark.asyncio
async def test_async_key_in_user_home(run_test_server):
    """Test that we can establish a connection with a key in the user's home directory."""
    # temporarily copy the key to the user's home directory
    key_path = Path("tests/data/test_key")
    new_path = Path.home() / ".ssh" / "test_key"
    new_path.parent.mkdir(exist_ok=True)
    if new_path.exists():
        new_path.unlink()
    new_path.write_bytes(key_path.read_bytes())
    try:
        async with AsyncConnection(
            "localhost", port=8022, username="root", key_path="~/.ssh/test_key"
        ) as conn:
            result = await conn.execute("echo hello")
            assert result.status == 0
    finally:
        if new_path.exists():
            new_path.unlink()


@pytest.mark.asyncio
async def test_async_bad_command(run_test_server):
    """Test that we can run a bad command."""
    async with AsyncConnection("localhost", username="root", password="toor", port=8022) as conn:
        result = await conn.execute("kira")
        assert result.status != 0
        assert "command not found" in result.stderr


@pytest.mark.asyncio
async def test_async_session_timeout():
    """Test that we can trigger a timeout on session handshake."""
    # Use a non-routable IP to force timeout
    with pytest.raises(TimeoutError):
        async with AsyncConnection(
            "10.255.255.1", username="root", password="toor", port=8022, timeout=1
        ):
            pass


@pytest.mark.asyncio
async def test_async_connect_timeout():
    """Test that we can trigger a timeout on manual connect."""
    conn = AsyncConnection("10.255.255.1", username="root", password="toor", port=8022)
    with pytest.raises(TimeoutError):
        await conn.connect(timeout=1)


@pytest.mark.asyncio
async def test_async_command_timeout(run_test_server):
    """Test that we can trigger a timeout on command execution."""
    async with AsyncConnection("localhost", username="root", password="toor", port=8022) as conn:
        with pytest.raises(TimeoutError):
            await conn.execute("sleep 5", timeout=1)
