"""Tests for hussh.async_connection module."""

from pathlib import Path

import pytest
import pytest_asyncio

from hussh import AsyncConnection, SSHResult

TEXT_FILE = Path("tests/data/hp.txt").resolve()
IMG_FILE = Path("tests/data/puppy.jpeg").resolve()


@pytest_asyncio.fixture
async def conn():
    """Return a basic AsyncConnection object."""
    connection = await AsyncConnection(host="localhost", port=8022, password="toor")
    yield connection
    await connection.close()


async def test_password_auth():
    """Test that we can establish a connection with password-based authentication."""
    conn = await AsyncConnection(host="localhost", port=8022, password="toor")
    assert conn
    await conn.close()


async def test_key_auth():
    """Test that we can establish a connection with key-based authentication."""
    conn = await AsyncConnection(host="localhost", port=8022, private_key="tests/data/test_key")
    assert conn
    await conn.close()


async def test_key_in_user_home():
    """Test that we can establish a connection with a key in the user's home directory."""
    # temporarily copy the key to the user's home directory
    key_path = Path("tests/data/test_key")
    new_path = Path.home() / ".ssh" / "test_key"
    new_path.parent.mkdir(exist_ok=True)
    key_path.rename(new_path)
    try:
        conn = await AsyncConnection(host="localhost", port=8022, private_key="~/.ssh/test_key")
        assert conn
        await conn.close()
    finally:
        new_path.rename(key_path)


async def test_key_with_password_auth():
    """Test that we can establish a connection with key-based authentication and a password."""
    conn = await AsyncConnection(
        host="localhost",
        port=8022,
        private_key="tests/data/auth_test_key",
        password="husshpuppy",
    )
    assert conn
    await conn.close()


@pytest.mark.skip("fixture-based setup for agent-based auth currently not working")
async def test_agent_auth(setup_agent_auth):
    """Test that we can establish a connection with agent-based authentication."""
    conn = await AsyncConnection(host="localhost", port=8022)
    assert conn
    await conn.close()


async def test_basic_command(conn: AsyncConnection):
    """Test that we can run a basic command."""
    result = await conn.execute("echo hello")
    assert isinstance(result, SSHResult)
    assert result.status == 0
    assert result.stdout == "hello\n"


async def test_bad_command(conn: AsyncConnection):
    """Test that we can run a bad command."""
    result = await conn.execute("kira")
    assert result.status != 0
    assert "command not found" in result.stderr


async def test_conn_context():
    """Test that the AsyncConnection class' context manager works."""
    async with await AsyncConnection(host="localhost", port=8022, password="toor") as conn:
        result = await conn.execute("echo hello")
    assert result.status == 0
    assert result.stdout == "hello\n"


async def test_text_scp(conn: AsyncConnection):
    """Test that we can copy a file to the server and read it back."""
    # copy a local file to the server
    await conn.scp_write(str(TEXT_FILE), "/root/hp.txt")
    assert "hp.txt" in (await conn.execute("ls /root")).stdout
    # read the file back from the server
    read_text = await conn.scp_read("/root/hp.txt")
    hp_text = Path(str(TEXT_FILE)).read_text()
    assert read_text == hp_text
    # copy the file from the server to a local file
    await conn.scp_read("/root/hp.txt", "scp_hp.txt")
    scp_hp_text = Path("scp_hp.txt").read_text()
    Path("scp_hp.txt").unlink()
    assert scp_hp_text == hp_text


async def test_scp_write_data(conn: AsyncConnection):
    """Test that we can write a string to a file on the server."""
    await conn.scp_write_data("hello", "/root/hello.txt")
    assert "hello.txt" in (await conn.execute("ls /root")).stdout
    read_text = await conn.scp_read("/root/hello.txt")
    assert read_text == "hello"


@pytest.mark.skip("non-text files are not supported by scp")
async def test_non_utf8_scp(conn: AsyncConnection):
    """Test that we can copy a non-text file to the server and read it back."""
    # copy an image file to the server
    await conn.scp_write(str(IMG_FILE), "/root/puppy.jpeg")
    assert "puppy.jpeg" in (await conn.execute("ls /root")).stdout
    # read the file back from the server
    read_img = await conn.scp_read("/root/puppy.jpeg")
    img_data = Path(str(IMG_FILE)).read_bytes()
    assert read_img == img_data
    # copy the file from the server to a local file
    await conn.scp_read("/root/puppy.jpeg", "scp_puppy.jpeg")
    scp_img_data = Path("scp_puppy.jpeg").read_bytes()
    Path("scp_puppy.jpeg").unlink()
    assert scp_img_data == img_data


async def test_text_sftp(conn: AsyncConnection):
    """Test that we can copy a file to the server and read it back."""
    # copy a local file to the server
    await conn.sftp_write(str(TEXT_FILE), "/root/hp.txt")
    assert "hp.txt" in (await conn.execute("ls /root")).stdout
    # read the file back from the server
    read_text = await conn.sftp_read("/root/hp.txt")
    hp_text = Path(str(TEXT_FILE)).read_text()
    assert read_text == hp_text
    # copy the file from the server to a local file
    await conn.sftp_read("/root/hp.txt", "sftp_hp.txt")
    sftp_hp_text = Path("sftp_hp.txt").read_text()
    Path("sftp_hp.txt").unlink()
    assert sftp_hp_text == hp_text


async def test_sftp_write_data(conn: AsyncConnection):
    """Test that we can write a string to a file on the server."""
    await conn.sftp_write_data("hello", "/root/hello.txt")
    assert "hello.txt" in (await conn.execute("ls /root")).stdout
    read_text = await conn.sftp_read("/root/hello.txt")
    assert read_text == "hello"


@pytest.mark.skip("non-text files are not supported by sftp")
async def test_non_utf8_sftp(conn: AsyncConnection):
    """Test that we can copy a non-text file to the server and read it back."""
    # copy an image file to the server
    await conn.sftp_write(str(IMG_FILE), "/root/puppy.jpeg")
    assert "puppy.jpeg" in (await conn.execute("ls /root")).stdout
    # read the file back from the server
    read_img = await conn.sftp_read("/root/puppy.jpeg")
    img_data = Path(str(IMG_FILE)).read_bytes()
    assert read_img == img_data
    # copy the file from the server to a local file
    await conn.sftp_read("/root/puppy.jpeg", "sftp_puppy.jpeg")
    sftp_img_data = Path("sftp_puppy.jpeg").read_bytes()
    Path("sftp_puppy.jpeg").unlink()
    assert sftp_img_data == img_data


async def test_shell_context(conn: AsyncConnection):
    """Test that we can run multiple commands in a shell context."""
    async with await conn.shell() as sh:
        await sh.send("echo test shell")
        await sh.send("bad command")
    assert "test shell" in sh.result.stdout
    assert "command not found" in sh.result.stderr
    assert sh.result.status != 0


async def test_pty_shell_context(conn: AsyncConnection):
    """Test that we can run multiple commands in a pty shell context."""
    async with await conn.shell(pty=True) as sh:
        await sh.send("echo test shell")
        await sh.send("bad command")
    assert "test shell" in sh.result.stdout
    assert "command not found" in sh.result.stdout
    assert sh.result.status != 0


@pytest.mark.skip("not yet implemented")
async def test_hangup_shell_context(conn: AsyncConnection):
    """Test that we can hang up a running shell while a previous command is still running."""
    async with await conn.shell() as sh:
        await sh.send("tail -f /dev/random")
    assert sh.result.stdout


async def test_session_timeout():
    """Test that we can trigger a timeout on session handshake."""
    with pytest.raises(TimeoutError):
        await AsyncConnection(host="localhost", port=8022, password="toor", timeout=10)


async def test_command_timeout(conn: AsyncConnection):
    """Test that we can trigger a timeout on command execution."""
    with pytest.raises(TimeoutError):
        await conn.execute("sleep 5", timeout=3000)


@pytest.mark.usefixtures("run_second_server")
async def test_remote_copy(conn: AsyncConnection):
    """Test that we can copy a file from one server to another."""
    # First copy the test file to the first server
    await conn.scp_write(str(TEXT_FILE), "/root/hp.txt")
    assert "hp.txt" in (await conn.execute("ls /root")).stdout
    # Now copy the file from the first server to the second server
    dest_conn = await AsyncConnection(host="localhost", port=8023, password="toor")
    await conn.remote_copy("/root/hp.txt", dest_conn)
    assert "hp.txt" in (await dest_conn.execute("ls /root")).stdout
    await dest_conn.close()


async def test_tail(conn: AsyncConnection):
    """Test that we can tail a file."""
    TEST_STR = "hello\nworld\n"
    await conn.scp_write_data(TEST_STR, "/root/hello.txt")
    async with await conn.tail("/root/hello.txt") as tf:
        assert await tf.read(0) == TEST_STR
        assert tf.last_pos == len(TEST_STR)
        await conn.execute("echo goodbye >> /root/hello.txt")
    assert tf.contents == "goodbye\n"
