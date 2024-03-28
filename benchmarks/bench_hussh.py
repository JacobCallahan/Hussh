import json
import memray
import timeit
from pathlib import Path


with memray.Tracker("memray-bench_hussh.bin"):
    start_time = timeit.default_timer()
    from hussh import Connection
    import_time = timeit.default_timer() - start_time

    host_info = json.loads(Path("target.json").read_text())

    temp_time = timeit.default_timer()
    conn = Connection(
        host=host_info["host"],
        port=host_info["port"],
        password=host_info["password"],
    )
    connect_time = timeit.default_timer() - temp_time

    temp_time = timeit.default_timer()
    result = conn.execute("echo test")
    run_time = timeit.default_timer() - temp_time

    # small file (1kb)
    temp_time = timeit.default_timer()
    conn.sftp_write("1kb.txt", "/root/1kb.txt")
    s_put_time = timeit.default_timer() - temp_time

    temp_time = timeit.default_timer()
    conn.sftp_read("/root/1kb.txt", "small.txt")
    s_get_time = timeit.default_timer() - temp_time
    Path("small.txt").unlink()

    # medium file (14kb)
    temp_time = timeit.default_timer()
    conn.sftp_write("14kb.txt", "/root/14kb.txt")
    m_put_time = timeit.default_timer() - temp_time

    temp_time = timeit.default_timer()
    conn.sftp_read("/root/14kb.txt", "medium.txt")
    m_get_time = timeit.default_timer() - temp_time
    Path("medium.txt").unlink()

    # large file (64kb)
    temp_time = timeit.default_timer()
    conn.sftp_write("64kb.txt", "/root/64kb.txt")
    l_put_time = timeit.default_timer() - temp_time

    temp_time = timeit.default_timer()
    conn.sftp_read("/root/64kb.txt", "large.txt")
    l_get_time = timeit.default_timer() - temp_time
    Path("large.txt").unlink()

    total_time = timeit.default_timer() - start_time

print(f"import_time: {import_time * 1000:.2f} ms")
print(f"connect_time: {connect_time * 1000:.2f} ms")
print(f"run_time: {run_time * 1000:.2f} ms")
print(f"s_put_time: {s_put_time * 1000:.2f} ms")
print(f"s_get_time: {s_get_time * 1000:.2f} ms")
print(f"m_put_time: {m_put_time * 1000:.2f} ms")
print(f"m_get_time: {m_get_time * 1000:.2f} ms")
print(f"l_put_time: {l_put_time * 1000:.2f} ms")
print(f"l_get_time: {l_get_time * 1000:.2f} ms")
print(f"total_time: {total_time * 1000:.2f} ms")
