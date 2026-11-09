"""Recovery must happen before requests, and never replay submitted commands."""

import asyncio
from concurrent.futures import ThreadPoolExecutor
import os
from pathlib import Path
import shlex
import shutil
import time
from uuid import uuid4

import pytest

from hussh import (
    AuthenticationError,
    Connection,
    HusshConnectionError,
    HusshIncompleteCommandError,
)
from hussh.aio import AsyncConnection
from hussh.multi_conn import MultiConnection


def sync_connection(proxy, **settings):
    return Connection("127.0.0.1", port=proxy.port, password="toor", **settings)


def async_connection(proxy, **settings):
    return AsyncConnection(
        "127.0.0.1", port=proxy.port, username="root", password="toor", **settings
    )


def test_recovery_exception_hierarchy():
    assert issubclass(HusshConnectionError, RuntimeError)
    assert issubclass(HusshIncompleteCommandError, HusshConnectionError)


@pytest.mark.parametrize("factory", [Connection, AsyncConnection])
@pytest.mark.parametrize(
    "settings",
    [
        {"max_retries": -1},
        {"max_retries": 1.5},
        {"retry_interval": -1},
        {"retry_interval": float("nan")},
        {"retry_interval": float("inf")},
        {"retry_interval": "exponential"},
    ],
)
def test_recovery_constructor_validation(factory, settings):
    with pytest.raises((ValueError, TypeError, OverflowError)):
        factory("127.0.0.1", port=1, **settings)


def test_sync_recovery_defaults(fault_proxy):
    with sync_connection(fault_proxy) as conn:
        assert conn.auto_reconnect is True
        assert conn.max_retries == 3  # noqa: PLR2004
        assert conn.retry_interval == 1.5  # noqa: PLR2004
        fault_proxy.drop()
        result = conn.execute("printf recovered", timeout=200)
        assert result.stdout == "recovered"
        assert 2 <= fault_proxy.attempts <= conn.max_retries + 1  # noqa: PLR2004


@pytest.mark.parametrize("settings", [{"auto_reconnect": False}, {"max_retries": 0}])
def test_sync_recovery_opt_out(fault_proxy, settings):
    with sync_connection(fault_proxy, retry_interval=0, **settings) as conn:
        fault_proxy.drop()
        with pytest.raises(HusshConnectionError, match="0 fresh attempt"):
            conn.execute("true")
        assert fault_proxy.attempts == 1
        assert conn.reconnect(max_retries=0) is None
        assert conn.execute("exit 7").status == 7  # noqa: PLR2004


def test_sync_recovery_exhaustion(fault_proxy):
    with sync_connection(fault_proxy, retry_interval=0, max_retries=2) as conn:
        fault_proxy.drop(allow=False)
        with pytest.raises(HusshConnectionError, match="2 fresh attempt") as error:
            conn.execute("true")
        assert error.value.__cause__ is not None
        assert fault_proxy.attempts == 3  # noqa: PLR2004
        fault_proxy.command("allow", True)
        conn.reconnect(max_retries=0)
        assert conn.execute("true").status == 0


def test_sync_recovery_closed(fault_proxy):
    conn = sync_connection(fault_proxy, retry_interval=0)
    conn.close()
    for operation in (lambda: conn.execute("true"), lambda: conn.sftp_read("/etc/hosts")):
        with pytest.raises(HusshConnectionError, match="explicitly closed"):
            operation()
    assert fault_proxy.attempts == 1
    conn.reconnect(max_retries=0)
    assert conn.execute("true").status == 0
    conn.close()


@pytest.mark.parametrize("method", ["sftp", "scp", "shell", "tail", "directory", "copy"])
def test_sync_recovery_operations(fault_proxy, tmp_path, method):
    with sync_connection(fault_proxy, retry_interval=0) as conn:
        # Initialize the cache so this exercises replacement as well as initial setup.
        assert conn.sftp_read("/etc/hosts")
        fault_proxy.drop()
        remote = f"/tmp/recovery-{uuid4().hex}"
        if method == "sftp":
            conn.sftp_write_data("recovered", remote)
            assert conn.sftp_read(remote) == "recovered"
        elif method == "scp":
            conn.scp_write_data("recovered", remote)
            assert conn.scp_read(remote) == "recovered"
        elif method == "shell":
            with conn.shell() as shell:
                shell.send("echo recovered")
            assert "recovered" in shell.result.stdout
        elif method == "tail":
            with conn.tail("/etc/hosts") as tailer:
                assert tailer.read(0)
        elif method == "directory":
            (tmp_path / "file").write_text("recovered")
            transferred, failed = conn.sftp_put_dir(str(tmp_path), remote)
            assert transferred
            assert not failed
            assert conn.sftp_read(f"{remote}/file") == "recovered"
        else:
            with sync_connection(fault_proxy, retry_interval=0) as destination:
                fault_proxy.drop()
                conn.remote_copy("/etc/hosts", destination, remote)
                assert destination.sftp_read(remote)


def wait_for_side_effect(observer, path):
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        if observer.execute(f"test -s {path}").status == 0:
            return
    pytest.fail("command did not record its side effect")


def test_sync_recovery_no_replay(fault_proxy):
    remote = f"/tmp/recovery-{uuid4().hex}"
    with (
        Connection("127.0.0.1", port=8022, password="toor") as observer,
        sync_connection(fault_proxy, retry_interval=0) as conn,
        ThreadPoolExecutor(max_workers=1) as pool,
    ):
        result = pool.submit(conn.execute, f"echo once >> {remote}; sleep 30")
        wait_for_side_effect(observer, remote)
        fault_proxy.drop()
        with pytest.raises(HusshIncompleteCommandError, match="may have occurred"):
            result.result(timeout=10)
        assert fault_proxy.attempts == 1
        assert conn.execute(f"cat {remote}").stdout == "once\n"
        assert fault_proxy.attempts == 2  # noqa: PLR2004


def test_sync_recovery_interrupted_transfer(fault_proxy):
    with sync_connection(fault_proxy, retry_interval=0) as conn:
        conn.sftp_read("/etc/hosts")
        fault_proxy.command("arm", 65536)
        with pytest.raises(OSError, match=r".+"):
            conn.sftp_write_data("x" * 4_000_000, f"/tmp/recovery-{uuid4().hex}")
        assert fault_proxy.attempts == 1


@pytest.mark.asyncio
async def test_async_recovery_lifecycle(fault_proxy):
    conn = async_connection(fault_proxy, retry_interval=0)
    with pytest.raises(HusshConnectionError, match="call connect"):
        await conn.execute("true")
    assert fault_proxy.attempts == 0
    await conn.connect()
    assert conn.auto_reconnect is True
    await conn.close()
    with pytest.raises(HusshConnectionError, match="explicitly closed"):
        await conn.execute("true")
    assert await conn.reconnect(max_retries=0) is None
    assert (await conn.execute("exit 7")).status == 7  # noqa: PLR2004
    await conn.close()


@pytest.mark.asyncio
async def test_async_recovery_concurrent(fault_proxy):
    async with async_connection(fault_proxy, retry_interval=0) as conn:
        await conn.sftp_read("/etc/hosts")
        fault_proxy.drop()
        results = await asyncio.gather(*(conn.execute("printf recovered") for _ in range(8)))
        assert all(result.stdout == "recovered" for result in results)
        assert fault_proxy.attempts == 2  # noqa: PLR2004
        assert await conn.sftp_read("/etc/hosts")
        assert fault_proxy.attempts == 2  # noqa: PLR2004


@pytest.mark.asyncio
@pytest.mark.parametrize("settings", [{"auto_reconnect": False}, {"max_retries": 0}])
async def test_async_recovery_opt_out(fault_proxy, settings):
    async with async_connection(fault_proxy, retry_interval=0, **settings) as conn:
        fault_proxy.drop()
        with pytest.raises(HusshConnectionError, match="0 fresh attempt"):
            await conn.execute("true")
        assert fault_proxy.attempts == 1
        await conn.reconnect(max_retries=0)
        assert (await conn.execute("true")).status == 0


@pytest.mark.asyncio
async def test_async_recovery_exhaustion(fault_proxy):
    async with async_connection(fault_proxy, retry_interval=0, max_retries=2) as conn:
        fault_proxy.drop(allow=False)
        with pytest.raises(HusshConnectionError, match="2 fresh attempt") as error:
            await conn.execute("true")
        assert error.value.__cause__ is not None
        assert fault_proxy.attempts == 3  # noqa: PLR2004


@pytest.mark.asyncio
@pytest.mark.parametrize("method", ["sftp", "shell", "tail", "directory"])
async def test_async_recovery_operations(fault_proxy, tmp_path, method):
    async with async_connection(fault_proxy, retry_interval=0) as conn:
        await conn.sftp_read("/etc/hosts")
        tailer = conn.tail("/etc/hosts")
        fault_proxy.drop()
        remote = f"/tmp/recovery-{uuid4().hex}"
        if method == "sftp":
            await conn.sftp_write_data("recovered", remote)
            assert await conn.sftp_read(remote) == "recovered"
        elif method == "shell":
            shell = await conn.shell()
            await shell.send("echo recovered\nexit\n")
            assert "recovered" in (await shell.read()).stdout
        elif method == "directory":
            (tmp_path / "file").write_text("recovered")
            transferred, failed = await conn.sftp_put_dir(str(tmp_path), remote)
            assert transferred
            assert not failed
            assert await conn.sftp_read(f"{remote}/file") == "recovered"
        else:
            async with tailer:
                assert await tailer.read(0)


@pytest.mark.asyncio
async def test_async_recovery_no_replay(fault_proxy):
    remote = f"/tmp/recovery-{uuid4().hex}"
    async with (
        AsyncConnection("127.0.0.1", port=8022, username="root", password="toor") as observer,
        async_connection(fault_proxy, retry_interval=0) as conn,
    ):
        task = asyncio.ensure_future(conn.execute(f"echo once >> {remote}; sleep 30"))

        async def wait_for_effect():
            while (await observer.execute(f"test -s {remote}")).status:
                await asyncio.sleep(0)

        await asyncio.wait_for(wait_for_effect(), timeout=10)
        fault_proxy.drop()
        with pytest.raises(HusshIncompleteCommandError, match="may have occurred"):
            await asyncio.wait_for(task, timeout=10)
        assert fault_proxy.attempts == 1
        assert (await conn.execute(f"cat {remote}")).stdout == "once\n"


@pytest.mark.asyncio
async def test_async_recovery_cancel_and_close(fault_proxy):
    conn = async_connection(fault_proxy, retry_interval=0)
    await conn.connect()
    fault_proxy.drop(allow="hold")
    task = asyncio.ensure_future(conn.execute("true"))

    async def wait_for_attempt():
        while fault_proxy.attempts < 2:  # noqa: PLR2004
            await asyncio.sleep(0)

    # Cancel only after the replacement TCP connection is accepted, before SSH authentication.
    await asyncio.wait_for(wait_for_attempt(), timeout=5)
    close = asyncio.ensure_future(conn.close())
    task.cancel()
    with pytest.raises(asyncio.CancelledError):
        await task
    await asyncio.wait_for(close, timeout=2)
    assert fault_proxy.attempts == 2  # noqa: PLR2004
    with pytest.raises(HusshConnectionError, match="explicitly closed"):
        await conn.execute("true")
    fault_proxy.command("allow", True)
    await conn.reconnect(max_retries=0)
    assert (await conn.execute("true")).status == 0
    await conn.close()


@pytest.mark.asyncio
async def test_async_recovery_interrupted_transfer(fault_proxy):
    async with async_connection(fault_proxy, retry_interval=0) as conn:
        await conn.sftp_read("/etc/hosts")
        fault_proxy.command("arm", 65536)
        with pytest.raises(RuntimeError):
            await conn.sftp_write_data("x" * 4_000_000, f"/tmp/recovery-{uuid4().hex}")
        assert fault_proxy.attempts == 1


def test_multi_recovery_settings(fault_proxy):
    with sync_connection(fault_proxy, auto_reconnect=False, max_retries=0) as conn:
        group = MultiConnection.from_connections([conn])
        group.connect()
        assert group.execute("exit 7")["127.0.0.1"].status == 7  # noqa: PLR2004
        fault_proxy.drop()
        result = group.execute("true")["127.0.0.1"]
        assert result.status == -1
        assert "0 fresh attempt" in result.stderr
        assert fault_proxy.attempts == 2  # noqa: PLR2004
        group.close()


def test_multi_recovery_aggregation(fault_proxy):
    conn = async_connection(fault_proxy, auto_reconnect=False)
    group = MultiConnection([conn])
    group.connect()
    fault_proxy.drop()
    result = group.execute("true")["127.0.0.1"]
    assert result.status == -1
    assert "0 fresh attempt" in result.stderr
    assert fault_proxy.attempts == 1
    group.close()


def test_multi_recovery_default(fault_proxy):
    conn = async_connection(fault_proxy, retry_interval=0)
    with MultiConnection([conn]) as group:
        fault_proxy.drop()
        assert group.execute("printf recovered", timeout=1)["127.0.0.1"].stdout == "recovered"
        assert fault_proxy.attempts == 2  # noqa: PLR2004


@pytest.mark.parametrize(
    "options",
    [
        {"max_retries": -1},
        {"retry_interval": -1},
        {"retry_interval": "bad"},
        {"max_interval": 0},
        {"max_interval": float("inf")},
    ],
)
def test_sync_recovery_override_validation(fault_proxy, options):
    with sync_connection(fault_proxy) as conn:
        with pytest.raises((ValueError, TypeError)):
            conn.reconnect(**options)
        assert fault_proxy.attempts == 1


@pytest.mark.parametrize(
    ("delay", "cap", "minimum"), [(0.04, 30, 0.08), ("exponential", 0.02, 0.04)]
)
def test_sync_recovery_delays(fault_proxy, delay, cap, minimum):
    with sync_connection(fault_proxy, retry_interval=0, max_retries=0) as conn:
        fault_proxy.drop(allow=False)
        start = time.monotonic()
        with pytest.raises(HusshConnectionError, match="3 fresh attempt"):
            conn.reconnect(max_retries=2, retry_interval=delay, max_interval=cap)
        assert time.monotonic() - start >= minimum
        assert fault_proxy.attempts == 4  # noqa: PLR2004
        assert conn.max_retries == 0
        assert conn.retry_interval == 0


@pytest.mark.asyncio
async def test_async_recovery_deadline_and_responsiveness(fault_proxy):
    async with async_connection(fault_proxy, retry_interval=1.1) as conn:
        fault_proxy.drop()
        command = asyncio.ensure_future(conn.execute("printf recovered", timeout=1))
        # This timer must run while the recovery future is sleeping.
        await asyncio.wait_for(asyncio.sleep(0.05), timeout=0.5)
        assert not command.done()
        assert (await command).stdout == "recovered"


def test_sync_recovery_timeout_restored(fault_proxy):
    with sync_connection(fault_proxy, retry_interval=0) as conn:
        with pytest.raises(TimeoutError):
            conn.execute("sleep 2", timeout=100)
        assert conn.execute("sleep 0.2; printf restored").stdout == "restored"


@pytest.mark.asyncio
async def test_async_recovery_override_validation(fault_proxy):
    async with async_connection(fault_proxy, retry_interval=0, max_retries=0) as conn:
        for options in (
            {"max_retries": -1},
            {"retry_interval": float("nan")},
            {"max_interval": 0},
        ):
            with pytest.raises(ValueError, match="must be"):
                await conn.reconnect(**options)
        fault_proxy.drop(allow=False)
        with pytest.raises(HusshConnectionError, match="3 fresh attempt"):
            await conn.reconnect(max_retries=2, retry_interval="exponential", max_interval=0.01)
        assert fault_proxy.attempts == 4  # noqa: PLR2004
        assert conn.max_retries == 0
        assert conn.retry_interval == 0


def test_sync_recovery_application_error(fault_proxy):
    with sync_connection(fault_proxy, retry_interval=0) as conn:
        with pytest.raises(OSError, match="SFTP open error"):
            conn.sftp_read(f"/missing-{uuid4().hex}")
        assert fault_proxy.attempts == 1


@pytest.mark.asyncio
async def test_async_recovery_application_error(fault_proxy):
    async with async_connection(fault_proxy, retry_interval=0) as conn:
        with pytest.raises(RuntimeError, match="Failed to open remote file"):
            await conn.sftp_read(f"/missing-{uuid4().hex}")
        assert fault_proxy.attempts == 1


@pytest.mark.asyncio
async def test_async_recovery_concurrent_sftp(fault_proxy):
    async with async_connection(fault_proxy, retry_interval=0) as conn:
        fault_proxy.drop()
        contents = await asyncio.gather(*(conn.sftp_read("/etc/hosts") for _ in range(32)))
        assert all(contents)
        assert fault_proxy.attempts == 2  # noqa: PLR2004


@pytest.mark.asyncio
async def test_async_recovery_old_tailer(fault_proxy):
    async with async_connection(fault_proxy, retry_interval=0) as conn:
        tailer = conn.tail("/etc/hosts")
        await tailer.__aenter__()
        fault_proxy.drop()
        assert (await conn.execute("true")).status == 0
        with pytest.raises(RuntimeError, match="Open error"):
            await tailer.read(0)
        assert fault_proxy.attempts == 2  # noqa: PLR2004


def test_sync_recovery_exponential_schedule(fault_proxy):
    with sync_connection(fault_proxy, max_retries=0) as conn:
        fault_proxy.drop(allow=False)
        with pytest.raises(HusshConnectionError, match="3 fresh attempt"):
            conn.reconnect(max_retries=2, retry_interval="exponential")
        immediate, first_retry, second_retry = fault_proxy.command("times")[-3:]
        assert first_retry - immediate >= 1.5  # noqa: PLR2004
        assert second_retry - first_retry >= 3  # noqa: PLR2004


def test_sync_recovery_old_tailer(fault_proxy):
    with sync_connection(fault_proxy, retry_interval=0) as conn:
        tailer = conn.tail("/etc/hosts")
        tailer.__enter__()
        fault_proxy.drop()
        assert conn.execute("true").status == 0
        with pytest.raises((OSError, HusshConnectionError), match=r".+"):
            tailer.read(0)
        assert fault_proxy.attempts == 2  # noqa: PLR2004


def test_sync_recovery_output_encoding_error(fault_proxy):
    with sync_connection(fault_proxy, retry_interval=0) as conn:
        with pytest.raises(OSError, match="UTF-8"):
            conn.execute(r"printf '\377'")
        assert fault_proxy.attempts == 1


def test_sync_recovery_proxy_cleanup(tmp_path):
    if shutil.which("ssh") is None or os.name != "posix":
        pytest.skip("ProxyCommand requires OpenSSH on Unix")
    pid_file = tmp_path / "proxy.pid"
    key = Path("tests/data/test_key").resolve()
    key.chmod(0o600)
    command = (
        f"echo $$ > {shlex.quote(str(pid_file))}; exec ssh "
        "-o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null "
        f"-i {shlex.quote(str(key))} -p 8022 root@localhost -W %h:%p"
    )
    with Connection("127.0.0.1", password="toor", proxy_command=command) as conn:
        first_pid = int(pid_file.read_text())
        conn.reconnect(max_retries=0)
        second_pid = int(pid_file.read_text())
        assert first_pid != second_pid
        with pytest.raises(ProcessLookupError):
            os.kill(first_pid, 0)
        assert conn.execute("true").status == 0
    with pytest.raises(ProcessLookupError):
        os.kill(second_pid, 0)
    with pytest.raises(AuthenticationError):
        Connection("127.0.0.1", password="wrong", proxy_command=command)
    with pytest.raises(ProcessLookupError):
        os.kill(int(pid_file.read_text()), 0)


def test_sync_recovery_submission_failure(fault_proxy):
    with sync_connection(fault_proxy, retry_interval=0) as conn:
        # Channel setup fits below this threshold; the larger exec request is cut mid-packet.
        fault_proxy.command("arm", 1024)
        with pytest.raises(HusshIncompleteCommandError, match="may have occurred"):
            conn.execute("true #" + "x" * 8192)
        assert fault_proxy.attempts == 1


@pytest.mark.asyncio
async def test_async_recovery_submission_failure(fault_proxy):
    async with async_connection(fault_proxy, retry_interval=0) as conn:
        fault_proxy.command("arm", 1024)
        with pytest.raises(HusshIncompleteCommandError, match="may have occurred"):
            await conn.execute("true #" + "x" * 8192)
        assert fault_proxy.attempts == 1
