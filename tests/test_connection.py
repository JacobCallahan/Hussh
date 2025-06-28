"""Tests for hussh.connection module."""

import shutil
from pathlib import Path

import pytest

from hussh import Connection, SSHResult

TEXT_FILE = Path("tests/data/hp.txt").resolve()
IMG_FILE = Path("tests/data/puppy.jpeg").resolve()


@pytest.fixture
def conn():
    """Return a basic Connection object."""
    return Connection(host="localhost", port=8022, password="toor")


def test_password_auth():
    """Test that we can establish a connection with password-based authentication."""
    assert Connection(host="localhost", port=8022, password="toor")


def test_key_auth():
    """Test that we can establish a connection with key-based authentication."""
    assert Connection(host="localhost", port=8022, private_key="tests/data/test_key")


def test_key_in_user_home():
    """Test that we can establish a connection with a key in the user's home directory."""
    # temporarily copy the key to the user's home directory
    key_path = Path("tests/data/test_key")
    new_path = Path.home() / ".ssh" / "test_key"
    new_path.parent.mkdir(exist_ok=True)
    key_path.rename(new_path)
    try:
        assert Connection(host="localhost", port=8022, private_key="~/.ssh/test_key")
    finally:
        new_path.rename(key_path)


def test_key_with_password_auth():
    """Test that we can establish a connection with key-based authentication and a password."""
    assert Connection(
        host="localhost",
        port=8022,
        private_key="tests/data/auth_test_key",
        password="husshpuppy",
    )


@pytest.mark.skip("fixture-based setup for agent-based auth currently not working")
def test_agent_auth(setup_agent_auth):
    """Test that we can establish a connection with agent-based authentication."""
    assert Connection(host="localhost", port=8022)


def test_default_key_auth():
    """Test that we can establish a connection with default SSH key discovery."""
    # temporarily copy the test key to ~/.ssh/id_rsa (most common default key)
    key_path = Path("tests/data/test_key")
    ssh_dir = Path.home() / ".ssh"
    ssh_dir.mkdir(exist_ok=True, mode=0o700)
    default_key_path = ssh_dir / "id_rsa"
    
    # Only run test if default key doesn't already exist to avoid conflicts
    if default_key_path.exists():
        pytest.skip("Default SSH key already exists, skipping test to avoid conflicts")
    
    # Copy the test key to the default location
    import shutil
    shutil.copy2(key_path, default_key_path)
    default_key_path.chmod(0o600)  # Set correct permissions for SSH key
    
    try:
        # Test connection without specifying private_key - should use default key
        conn = Connection(host="localhost", port=8022)
        assert conn is not None
        # Test that we can actually execute a command to confirm auth worked
        result = conn.execute("echo 'default key auth test'")
        assert result.status == 0
        assert "default key auth test" in result.stdout
    finally:
        # Clean up - remove the copied key
        if default_key_path.exists():
            default_key_path.unlink()


def test_no_default_keys_auth_failure():
    """Test that authentication fails gracefully when no default keys are found and no ssh-agent."""
    # Ensure no default SSH keys exist by temporarily renaming ~/.ssh directory if it exists  
    ssh_dir = Path.home() / ".ssh"
    backup_dir = Path.home() / ".ssh_backup_for_test"
    
    # Backup existing .ssh directory if it exists
    ssh_exists = ssh_dir.exists()
    if ssh_exists:
        ssh_dir.rename(backup_dir)
    
    try:
        # This should fail since there are no default keys and likely no ssh-agent
        with pytest.raises(Exception) as exc_info:
            Connection(host="localhost", port=8022)
        
        # Verify the error message mentions both ssh-agent and default keys
        assert "Failed to authenticate with ssh-agent and default SSH keys" in str(exc_info.value)
    finally:
        # Restore the .ssh directory if it existed
        if ssh_exists and backup_dir.exists():
            backup_dir.rename(ssh_dir)


def test_basic_command(conn):
    """Test that we can run a basic command."""
    result = conn.execute("echo hello")
    assert isinstance(result, SSHResult)
    assert result.status == 0
    assert result.stdout == "hello\n"


def test_bad_command(conn):
    """Test that we can run a bad command."""
    result = conn.execute("kira")
    assert result.status != 0
    assert "command not found" in result.stderr


def test_conn_context():
    """Test that the Connection class' context manager works."""
    with Connection(host="localhost", port=8022, password="toor") as conn:
        result = conn.execute("echo hello")
    assert result.status == 0
    assert result.stdout == "hello\n"


def test_text_scp(conn):
    """Test that we can copy a file to the server and read it back."""
    # copy a local file to the server
    conn.scp_write(str(TEXT_FILE), "/root/hp.txt")
    assert "hp.txt" in conn.execute("ls /root").stdout
    # read the file back from the server
    read_text = conn.scp_read("/root/hp.txt")
    hp_text = Path(str(TEXT_FILE)).read_text()
    assert read_text == hp_text
    # copy the file from the server to a local file
    conn.scp_read("/root/hp.txt", "scp_hp.txt")
    scp_hp_text = Path("scp_hp.txt").read_text()
    Path("scp_hp.txt").unlink()
    assert scp_hp_text == hp_text


def test_scp_write_data(conn):
    """Test that we can write a string to a file on the server."""
    conn.scp_write_data("hello", "/root/hello.txt")
    assert "hello.txt" in conn.execute("ls /root").stdout
    read_text = conn.scp_read("/root/hello.txt")
    assert read_text == "hello"


@pytest.mark.skip("non-text files are not supported by scp")
def test_non_utf8_scp(conn):
    """Test that we can copy a non-text file to the server and read it back."""
    # copy an image file to the server
    conn.scp_write(str(IMG_FILE), "/root/puppy.jpeg")
    assert "puppy.jpeg" in conn.execute("ls /root").stdout
    # read the file back from the server
    read_img = conn.scp_read("/root/puppy.jpeg")
    img_data = Path(str(IMG_FILE)).read_bytes()
    assert read_img == img_data
    # copy the file from the server to a local file
    conn.scp_read("/root/puppy.jpeg", "scp_puppy.jpeg")
    scp_img_data = Path("scp_puppy.jpeg").read_bytes()
    Path("scp_puppy.jpeg").unlink()
    assert scp_img_data == img_data


def test_text_sftp(conn):
    """Test that we can copy a file to the server and read it back."""
    # copy a local file to the server
    conn.sftp_write(str(TEXT_FILE), "/root/hp.txt")
    assert "hp.txt" in conn.execute("ls /root").stdout
    # read the file back from the server
    read_text = conn.sftp_read("/root/hp.txt")
    hp_text = Path(str(TEXT_FILE)).read_text()
    assert read_text == hp_text
    # copy the file from the server to a local file
    conn.sftp_read("/root/hp.txt", "sftp_hp.txt")
    sftp_hp_text = Path("sftp_hp.txt").read_text()
    Path("sftp_hp.txt").unlink()
    assert sftp_hp_text == hp_text


def test_sftp_write_data(conn):
    """Test that we can write a string to a file on the server."""
    conn.sftp_write_data("hello", "/root/hello.txt")
    assert "hello.txt" in conn.execute("ls /root").stdout
    read_text = conn.sftp_read("/root/hello.txt")
    assert read_text == "hello"


@pytest.mark.skip("non-text files are not supported by sftp")
def test_non_utf8_sftp(conn):
    """Test that we can copy a non-text file to the server and read it back."""
    # copy an image file to the server
    conn.sftp_write(str(IMG_FILE), "/root/puppy.jpeg")
    assert "puppy.jpeg" in conn.execute("ls /root").stdout
    # read the file back from the server
    read_img = conn.sftp_read("/root/puppy.jpeg")
    img_data = Path(str(IMG_FILE)).read_bytes()
    assert read_img == img_data
    # copy the file from the server to a local file
    conn.sftp_read("/root/puppy.jpeg", "sftp_puppy.jpeg")
    sftp_img_data = Path("sftp_puppy.jpeg").read_bytes()
    Path("sftp_puppy.jpeg").unlink()
    assert sftp_img_data == img_data


def test_shell_context(conn):
    """Test that we can run multiple commands in a shell context."""
    with conn.shell() as sh:
        sh.send("echo test shell")
        sh.send("bad command")
    assert "test shell" in sh.result.stdout
    assert "command not found" in sh.result.stderr
    assert sh.result.status != 0


def test_pty_shell_context(conn):
    """Test that we can run multiple commands in a pty shell context."""
    with conn.shell(pty=True) as sh:
        sh.send("echo test shell")
        sh.send("bad command")
    assert "test shell" in sh.result.stdout
    assert "command not found" in sh.result.stdout
    assert sh.result.status != 0


@pytest.mark.skip("not yet implemented")
def test_hangup_shell_context(conn):
    """Test that we can hang up a running shell while a previous command is still running."""
    with conn.shell() as sh:
        sh.send("tail -f /dev/random")
    assert sh.result.stdout


def test_remote_copy(conn, run_second_server):
    """Test that we can copy a file from one server to another."""
    # First copy the test file to the first server
    conn.scp_write(str(TEXT_FILE), "/root/hp.txt")
    assert "hp.txt" in conn.execute("ls /root").stdout
    # Now copy the file from the first server to the second server
    dest_conn = Connection(host="localhost", port=8023, password="toor")
    conn.remote_copy("/root/hp.txt", dest_conn)
    assert "hp.txt" in dest_conn.execute("ls /root").stdout


def test_tail(conn):
    """Test that we can tail a file."""
    TEST_STR = "hello\nworld\n"
    conn.scp_write_data(TEST_STR, "/root/hello.txt")
    with conn.tail("/root/hello.txt") as tf:
        assert tf.read(0) == TEST_STR
        assert tf.last_pos == len(TEST_STR)
        conn.execute("echo goodbye >> /root/hello.txt")
    assert tf.contents == "goodbye\n"


# ------------- Negative Tests -------------


def test_session_timeout():
    """Test that we can trigger a timeout on session handshake."""
    with pytest.raises(TimeoutError):
        Connection(host="localhost", port=8022, password="toor", timeout=10)


def test_command_timeout(conn):
    """Test that we can trigger a timeout on command execution."""
    with pytest.raises(TimeoutError):
        conn.execute("sleep 5", timeout=3000)


def test_scp_write_missing_directory(conn):
    """Test that IOError is raised if scp_write attempts to write to a missing directory."""
    with pytest.raises(IOError):  # noqa: PT011
        conn.scp_write_data("data", "/no_such_dir/test.txt")


def test_sftp_read_invalid_path(conn):
    """Test that IOError is raised if sftp_read is given an invalid remote path."""
    with pytest.raises(IOError):  # noqa: PT011
        conn.sftp_read("/invalid/path/file.txt")


def test_scp_read_directory_as_file(conn):
    """Test that IOError is raised if scp_read tries to read a directory as a file."""
    with pytest.raises(IOError):  # noqa: PT011
        conn.scp_read("/root")
