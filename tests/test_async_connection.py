import asyncio

import pytest

import hussh


@pytest.mark.asyncio
async def test_async_connection_basic(run_test_server):
    # run_test_server returns a container object.
    # We need to know the port.
    # conftest.py maps 22 to 8022.

    host = "localhost"
    port = 8022
    username = "root"
    password = "toor"

    async with hussh.aio.AsyncConnection(
        host, username=username, password=password, port=port
    ) as conn:
        result = await conn.execute("echo hello")
        # result is (stdout, stderr, exit_code)
        assert result[0].strip() == "hello"
        assert result[2] == 0


@pytest.mark.asyncio
async def test_async_connection_manual(run_test_server):
    host = "localhost"
    port = 8022
    username = "root"
    password = "toor"

    conn = hussh.aio.AsyncConnection(host, username=username, password=password, port=port)
    await conn.connect()

    result = await conn.execute("echo manual")
    assert result[0].strip() == "manual"
    assert result[2] == 0

    await conn.close()


@pytest.mark.asyncio
async def test_async_sftp(run_test_server, tmp_path):
    host = "localhost"
    port = 8022
    username = "root"
    password = "toor"

    async with hussh.aio.AsyncConnection(
        host, username=username, password=password, port=port
    ) as conn:
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
    host = "localhost"
    port = 8022
    username = "root"
    password = "toor"

    async with hussh.aio.AsyncConnection(
        host, username=username, password=password, port=port
    ) as conn:
        shell = await conn.shell(pty=True)

        # We need to wait a bit for the shell to be ready and print the prompt
        await asyncio.sleep(0.5)
        _ = await shell.read()  # Clear initial banner/prompt

        await shell.write(b"echo hello shell\n")

        # Give it a moment to process
        await asyncio.sleep(0.5)

        output = await shell.read()
        # Output will contain the echo command itself and the result
        assert b"hello shell" in output

        await shell.close()


@pytest.mark.asyncio
async def test_async_file_tailer(run_test_server, tmp_path):
    host = "localhost"
    port = 8022
    username = "root"
    password = "toor"

    async with hussh.aio.AsyncConnection(
        host, username=username, password=password, port=port
    ) as conn:
        # Create a test file on the remote server
        await conn.execute("echo 'line 1' > /root/test_tail.log")
        await conn.execute("echo 'line 2' >> /root/test_tail.log")

        async with conn.tail("/root/test_tail.log") as tailer:
            # Initially, should have the full content
            content = await tailer.read(0)
            assert "line 1" in content
            assert "line 2" in content

            # Add more lines
            await conn.execute("echo 'line 3' >> /root/test_tail.log")

            # Read from last position
            new_content = await tailer.read(None)
            assert "line 3" in new_content
