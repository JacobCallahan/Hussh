"""Regression cases raised in PR #114's recovery review."""

import asyncio
from concurrent.futures import ThreadPoolExecutor
import time

from conftest import TEST_SERVER_IMAGE
import docker
import pytest
from test_recovery import async_connection, sync_connection

from hussh import Connection, HusshConnectionError
from hussh.aio import AsyncConnection
from hussh.multi_conn import MultiConnection


@pytest.fixture
def single_session_server(run_test_server):
    client = docker.from_env()
    container = client.containers.run(
        TEST_SERVER_IMAGE,
        command=["/usr/sbin/sshd", "-D", "-e", "-o", "MaxSessions=1"],
        detach=True,
        ports={"22/tcp": None},
    )
    try:
        container.reload()
        port = int(container.attrs["NetworkSettings"]["Ports"]["22/tcp"][0]["HostPort"])
        deadline = time.monotonic() + 10
        while True:
            try:
                with Connection("127.0.0.1", port=port, password="toor", timeout=500):
                    break
            except (OSError, TimeoutError, HusshConnectionError):
                if time.monotonic() >= deadline:
                    pytest.fail(
                        f"Single-session SSH server did not start: {container.logs().decode()}"
                    )
                time.sleep(0.05)
        yield port
    finally:
        container.remove(force=True)
        client.close()


def test_sync_cached_sftp_single_channel(single_session_server):
    with Connection("127.0.0.1", port=single_session_server, password="toor") as conn:
        first = conn.sftp_read("/etc/hosts")
        assert conn.sftp_read("/etc/hosts") == first
        assert conn.sftp_read("/etc/hosts") == first


@pytest.mark.asyncio
async def test_async_cached_sftp_single_channel(single_session_server):
    async with AsyncConnection(
        "127.0.0.1", port=single_session_server, username="root", password="toor"
    ) as conn:
        first = await conn.sftp_read("/etc/hosts")
        assert await conn.sftp_read("/etc/hosts") == first
        async with conn.tail("/etc/hosts") as tailer:
            assert await tailer.read(0) == first


def test_sync_channel_setup_timeout(fault_proxy):
    with sync_connection(fault_proxy, retry_interval=0) as conn, ThreadPoolExecutor() as pool:
        fault_proxy.command("pause", True)
        future = pool.submit(conn.execute, "printf should-not-run", timeout=100)
        try:
            with pytest.raises(TimeoutError, match=r"(?i)timed out|timeout"):
                future.result(timeout=2)
        finally:
            fault_proxy.command("pause", False)
        assert conn.execute("sleep 0.2; printf restored").stdout == "restored"
        assert fault_proxy.attempts == 1


@pytest.mark.asyncio
async def test_async_channel_setup_timeout(fault_proxy):
    async with async_connection(fault_proxy, retry_interval=0) as conn:
        fault_proxy.command("pause", True)
        try:
            with pytest.raises(TimeoutError, match="Command timed out"):
                await asyncio.wait_for(conn.execute("true", timeout=1), timeout=3)
        finally:
            fault_proxy.command("pause", False)
        assert (await conn.execute("printf restored")).stdout == "restored"
        assert fault_proxy.attempts == 1


@pytest.mark.parametrize("mapped", [False, True])
def test_multi_channel_setup_timeout(fault_proxy, mapped):
    with (
        MultiConnection([async_connection(fault_proxy)]) as group,
        ThreadPoolExecutor() as pool,
    ):
        fault_proxy.command("pause", True)
        if mapped:
            future = pool.submit(group.execute_map, {"127.0.0.1": "true"}, timeout=1)
        else:
            future = pool.submit(group.execute, "true", timeout=1)
        try:
            result = future.result(timeout=3)["127.0.0.1"]
        finally:
            fault_proxy.command("pause", False)
        assert result.status == -1
        assert "Command timed out" in result.stderr
        assert fault_proxy.attempts == 1


async def wait_for_proxy(proxy, command, minimum):
    async def ready():
        while proxy.command(command) < minimum:
            await asyncio.sleep(0)

    await asyncio.wait_for(ready(), timeout=8)


@pytest.mark.asyncio
async def test_async_setup_and_execution_share_deadline(fault_proxy):
    async with async_connection(fault_proxy) as conn:
        fault_proxy.command("pause", True)
        command = asyncio.ensure_future(conn.execute("sleep 0.7", timeout=1))
        try:
            await wait_for_proxy(fault_proxy, "pending", 1)
            await asyncio.sleep(0.6)
        finally:
            fault_proxy.command("pause", False)
        with pytest.raises(TimeoutError, match="Command timed out"):
            await asyncio.wait_for(command, timeout=2)


@pytest.mark.asyncio
@pytest.mark.parametrize("phase", ["connect", "reconnect", "recovery", "setup", "sleep"])
async def test_async_close_interrupts_pending_work(fault_proxy, phase):
    conn = async_connection(fault_proxy, retry_interval=3 if phase == "sleep" else 0)
    task = None
    try:
        if phase == "connect":
            fault_proxy.command("allow", "hold")
            task = asyncio.ensure_future(conn.connect())
            await wait_for_proxy(fault_proxy, "count", 1)
        else:
            await conn.connect()
            if phase == "setup":
                fault_proxy.command("pause", True)
            else:
                fault_proxy.drop(allow=False if phase == "sleep" else "hold")
            task = asyncio.ensure_future(
                conn.reconnect(max_retries=0) if phase == "reconnect" else conn.execute("true")
            )
            await wait_for_proxy(
                fault_proxy,
                "pending" if phase == "setup" else "count",
                1 if phase == "setup" else 2,
            )
        await asyncio.wait_for(conn.close(), timeout=1)
        with pytest.raises(HusshConnectionError, match="closed"):
            await asyncio.wait_for(task, timeout=1)
        with pytest.raises(HusshConnectionError, match="explicitly closed"):
            await conn.execute("true")
        fault_proxy.command("pause", False)
        fault_proxy.command("allow", True)
        await conn.reconnect(max_retries=0)
        assert (await conn.execute("true")).status == 0
    finally:
        if task is not None and not task.done():
            task.cancel()
            await asyncio.gather(task, return_exceptions=True)
        fault_proxy.command("pause", False)
        fault_proxy.command("allow", True)
        await conn.close()


@pytest.mark.parametrize("signal", ["TERM", "KILL"])
def test_sync_exit_signal_is_failed_result(signal):
    with Connection("127.0.0.1", port=8022, password="toor") as conn:
        result = conn.execute(f"printf before; printf warning >&2; kill -{signal} $$")
        assert result.status == -1
        assert result.stdout == "before"
        assert result.stderr == "warning"


@pytest.mark.asyncio
@pytest.mark.parametrize("signal", ["TERM", "KILL"])
async def test_async_exit_signal_is_failed_result(signal):
    async with AsyncConnection("127.0.0.1", port=8022, username="root", password="toor") as conn:
        result = await conn.execute(f"printf before; printf warning >&2; kill -{signal} $$")
        assert result.status == -1
        assert result.stdout == "before"
        assert result.stderr == "warning"
