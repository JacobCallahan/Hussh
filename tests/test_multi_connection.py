"""Tests for MultiConnection functionality."""

import time

import pytest

from hussh import Connection
from hussh.aio import AsyncConnection
from hussh.multi_conn import (
    MultiConnection,
    MultiFileTailer,
    MultiResult,
    PartialFailureException,
)


class TestMultiConnectionConstructors:
    """Tests for MultiConnection constructor variants."""

    def test_from_shared_auth(self, run_test_servers):
        """Test creating MultiConnection with shared authentication."""
        # Use unique hostnames for all servers with a common port
        host_list = [host for host, _ in run_test_servers]
        port = run_test_servers[0][1]

        BATCH_SIZE = 10
        mc = MultiConnection.from_shared_auth(
            host_list,
            username="root",
            password="toor",
            port=port,
            batch_size=BATCH_SIZE,
        )

        # hosts are stored as hostname only
        assert mc.hosts == host_list
        assert mc.batch_size == BATCH_SIZE

    def test_from_async_connections(self, run_test_servers):
        """Test creating MultiConnection from AsyncConnection instances."""
        connections = []
        for host, port in run_test_servers:
            conn = AsyncConnection(host, username="root", password="toor", port=port)
            connections.append(conn)

        BATCH_SIZE = 5
        mc = MultiConnection(connections, batch_size=BATCH_SIZE)
        assert len(mc.hosts) == len(run_test_servers)
        assert mc.batch_size == BATCH_SIZE

    def test_from_sync_connections(self, run_test_servers):
        """Test creating MultiConnection from sync Connection instances."""
        connections = []
        for host, port in run_test_servers:
            conn = Connection(host, port=port, username="root", password="toor")
            connections.append(conn)

        BATCH_SIZE = 20
        mc = MultiConnection.from_connections(connections, batch_size=BATCH_SIZE)
        assert len(mc.hosts) == len(run_test_servers)
        assert mc.batch_size == BATCH_SIZE

        # Clean up sync connections
        for conn in connections:
            conn.close()


class TestMultiConnectionOperations:
    """Tests for MultiConnection operations."""

    def test_execute_on_multiple_hosts(self, run_test_servers):
        """Test executing the same command on multiple hosts."""
        connections = []
        for host, port in run_test_servers:
            conn = AsyncConnection(host, username="root", password="toor", port=port)
            connections.append(conn)

        mc = MultiConnection(connections)

        # Connect first (manual mode)
        connect_results = mc.connect()
        assert len(connect_results) == len(run_test_servers)

        # Execute command
        results = mc.execute("whoami")

        assert isinstance(results, MultiResult)
        assert len(results) == len(run_test_servers)

        for host in mc.hosts:
            assert host in results
            assert results[host].stdout.strip() == "root"
            assert results[host].status == 0

        mc.close()

    def test_execute_with_context_manager(self, run_test_servers):
        """Test execute using context manager (eager connect)."""
        connections = []
        for host, port in run_test_servers:
            conn = AsyncConnection(host, username="root", password="toor", port=port)
            connections.append(conn)

        with MultiConnection(connections) as mc:
            results = mc.execute("echo hello")

            for host in mc.hosts:
                assert results[host].stdout.strip() == "hello"
                assert results[host].status == 0

    def test_execute_map_different_commands(self, run_test_servers):
        """Test executing different commands on different hosts."""
        connections = []
        for host, port in run_test_servers:
            conn = AsyncConnection(host, username="root", password="toor", port=port)
            connections.append(conn)

        with MultiConnection(connections) as mc:
            # Create command map with different commands for each host
            command_map = {}
            for i, host in enumerate(mc.hosts):
                command_map[host] = f"echo host_{i}"

            results = mc.execute_map(command_map)

            for i, host in enumerate(mc.hosts):
                assert results[host].stdout.strip() == f"host_{i}"

    def test_connect_with_prune_failures(self, run_test_servers):
        """Test connect with prune_failures option."""
        # Create connections with one bad host
        connections = []
        for host, port in run_test_servers:
            conn = AsyncConnection(host, username="root", password="toor", port=port)
            connections.append(conn)

        # Add a bad connection
        bad_conn = AsyncConnection(
            "localhost",
            username="root",
            password="toor",
            port=9999,  # Invalid port
        )
        connections.append(bad_conn)

        mc = MultiConnection(connections, timeout=5)
        initial_count = len(mc.hosts)

        # Connect with prune_failures=True
        _results = mc.connect(prune_failures=True, timeout=5)

        # The bad host should have been removed
        assert len(mc.hosts) < initial_count
        assert len(mc.hosts) == len(run_test_servers)  # Only good hosts remain

        # Verify we can still execute on remaining hosts
        exec_results = mc.execute("whoami")
        assert len(exec_results) == len(mc.hosts)

        mc.close()


class TestMultiResult:
    """Tests for MultiResult functionality."""

    def test_multiresult_iteration(self, run_test_servers):
        """Test iterating over MultiResult."""
        connections = []
        for host, port in run_test_servers:
            conn = AsyncConnection(host, username="root", password="toor", port=port)
            connections.append(conn)

        with MultiConnection(connections) as mc:
            results = mc.execute("whoami")

            # Test keys()
            keys = results.keys()
            assert len(keys) == len(mc.hosts)

            # Test values()
            values = results.values()
            assert len(values) == len(mc.hosts)

            # Test items()
            items = results.items()
            assert len(items) == len(mc.hosts)
            for host, result in items:
                assert host in mc.hosts
                assert result.status == 0

            # Test __contains__
            assert mc.hosts[0] in results
            assert "nonexistent_host" not in results

            # Test __len__
            assert len(results) == len(mc.hosts)

    def test_multiresult_failed_succeeded(self, run_test_servers):
        """Test failed and succeeded properties."""
        connections = []
        for host, port in run_test_servers:
            conn = AsyncConnection(host, username="root", password="toor", port=port)
            connections.append(conn)

        with MultiConnection(connections) as mc:
            # Execute a command that will succeed on all hosts
            results = mc.execute("whoami")

            succeeded = results.succeeded
            failed = results.failed

            assert len(succeeded) == len(mc.hosts)
            assert failed is None  # No failures, so None

            # Now run a command that will fail
            results = mc.execute("exit 1")

            succeeded = results.succeeded
            failed = results.failed

            assert succeeded is None  # No successes, so None
            assert len(failed) == len(mc.hosts)

    def test_raise_if_any_failed(self, run_test_servers):
        """Test raise_if_any_failed method."""
        connections = []
        for host, port in run_test_servers:
            conn = AsyncConnection(host, username="root", password="toor", port=port)
            connections.append(conn)

        with MultiConnection(connections) as mc:
            # This should not raise
            results = mc.execute("whoami")
            results.raise_if_any_failed()  # Should not raise

            # This should raise
            results = mc.execute("exit 1")
            with pytest.raises(PartialFailureException) as exc_info:
                results.raise_if_any_failed()

            exc = exc_info.value
            assert hasattr(exc, "succeeded")
            assert hasattr(exc, "failed")
            assert exc.succeeded is None  # No successes, so None
            assert len(exc.failed) == len(mc.hosts)


class TestMultiConnectionSFTP:
    """Tests for MultiConnection SFTP operations."""

    def test_sftp_write_read(self, run_test_servers):
        """Test SFTP write and read operations."""
        connections = []
        for host, port in run_test_servers:
            conn = AsyncConnection(host, username="root", password="toor", port=port)
            connections.append(conn)

        with MultiConnection(connections) as mc:
            # Write data to all hosts
            test_data = "Hello from MultiConnection test!"
            remote_path = "/tmp/mc_test_file.txt"

            write_results = mc.sftp_write_data(test_data, remote_path)

            for host in mc.hosts:
                assert write_results[host].status == 0

            # Read the file back
            read_results = mc.sftp_read(remote_path)

            for host in mc.hosts:
                assert read_results[host].status == 0
                assert read_results[host].stdout == test_data


class TestMultiFileTailer:
    """Tests for MultiFileTailer functionality."""

    def test_tail_same_file(self, run_test_servers):
        """Test tailing the same file on all hosts."""
        connections = []
        for host, port in run_test_servers:
            conn = AsyncConnection(host, username="root", password="toor", port=port)
            connections.append(conn)

        with MultiConnection(connections) as mc:
            # Create a test file on all hosts
            test_file = "/tmp/tail_test.log"
            mc.sftp_write_data("initial content\n", test_file)

            # Start tailing
            with mc.tail(test_file) as tailer:
                assert isinstance(tailer, MultiFileTailer)
                assert len(tailer.hosts) == len(mc.hosts)

                # Append some content
                mc.execute(f"echo 'new line' >> {test_file}")
                time.sleep(0.5)

                # Read new content
                new_content = tailer.read()
                for host in mc.hosts:
                    assert "new line" in new_content[host]

            # After exit, contents should be populated
            assert len(tailer.contents) == len(mc.hosts)

    def test_tail_map_different_files(self, run_test_servers):
        """Test tailing different files on different hosts."""
        connections = []
        for host, port in run_test_servers:
            conn = AsyncConnection(host, username="root", password="toor", port=port)
            connections.append(conn)

        with MultiConnection(connections) as mc:
            # Create different files on each host
            file_map = {}
            command_map = {}
            for i, host in enumerate(mc.hosts):
                file_path = f"/tmp/tail_test_{i}.log"
                file_map[host] = file_path
                command_map[host] = f"echo 'content for host {i}' > {file_path}"
            # Execute all commands concurrently
            mc.execute_map(command_map)

            # Tail different files
            with mc.tail_map(file_map) as tailer:
                contents = tailer.read(from_pos=0)

                for i, host in enumerate(mc.hosts):
                    assert f"content for host {i}" in contents[host]


class TestMultiConnectionEdgeCases:
    """Tests for edge cases and error handling."""

    def test_empty_hosts_list(self):
        """Test behavior with empty hosts list."""
        mc = MultiConnection.from_shared_auth(
            [],
            username="root",
            password="pass",
        )
        assert len(mc.hosts) == 0

    def test_single_host(self, run_test_servers):
        """Test MultiConnection with single host."""
        host, port = run_test_servers[0]
        conn = AsyncConnection(host, username="root", password="toor", port=port)

        with MultiConnection([conn]) as mc:
            results = mc.execute("whoami")
            assert len(results) == 1
            # Key is the hostname
            assert results[host].stdout.strip() == "root"

    def test_repr_str(self, run_test_servers):
        """Test __repr__ and __str__ methods."""
        connections = []
        for host, port in run_test_servers:
            conn = AsyncConnection(host, username="root", password="toor", port=port)
            connections.append(conn)

        mc = MultiConnection(connections, batch_size=50)

        repr_str = repr(mc)
        assert "MultiConnection" in repr_str
        assert str(len(connections)) in repr_str
        assert "50" in repr_str

        # Test MultiResult repr
        with mc:
            results = mc.execute("whoami")
            result_repr = repr(results)
            assert "MultiResult" in result_repr
            assert "succeeded" in result_repr

    def test_timeout_handling(self, run_test_servers):
        """Test command timeout handling."""
        connections = []
        for host, port in run_test_servers:
            conn = AsyncConnection(host, username="root", password="toor", port=port)
            connections.append(conn)

        with MultiConnection(connections) as mc:
            # Execute a command that takes longer than timeout
            results = mc.execute("sleep 10", timeout=1)

            for host in mc.hosts:
                # Should have timed out
                assert results[host].status == -1
                assert "timed out" in results[host].stderr.lower()
