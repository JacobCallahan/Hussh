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
