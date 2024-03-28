import json
import memray
import timeit
from pathlib import Path


with memray.Tracker("memray-bench_paramiko.bin"):
    start_time = timeit.default_timer()
    import paramiko
    import_time = timeit.default_timer() - start_time

    host_info = json.loads(Path("target.json").read_text())

    temp_time = timeit.default_timer()
    ssh = paramiko.SSHClient()
    ssh.set_missing_host_key_policy(paramiko.AutoAddPolicy())
    ssh.connect(
        hostname=host_info["host"],
        port=host_info["port"],
        username=host_info["username"],
        password=host_info["password"],
        look_for_keys=False,
        allow_agent=False,
    )
    connect_time = timeit.default_timer() - temp_time

    temp_time = timeit.default_timer()
    stdin, stdout, stderr = ssh.exec_command("echo test")
    result = stdout.read()
    run_time = timeit.default_timer() - temp_time


    # small file (1kb)
    temp_time = timeit.default_timer()
    sftp = ssh.open_sftp()
    sftp.put("1kb.txt", "/root/1kb.txt")
    s_put_time = timeit.default_timer() - temp_time

    temp_time = timeit.default_timer()
    sftp.get("/root/1kb.txt", "small.txt")
    s_get_time = timeit.default_timer() - temp_time
    Path("small.txt").unlink()

    # medium file (14kb)
    temp_time = timeit.default_timer()
    sftp.put("14kb.txt", "/root/14kb.txt")
    m_put_time = timeit.default_timer() - temp_time

    temp_time = timeit.default_timer()
    sftp.get("/root/14kb.txt", "medium.txt")
    m_get_time = timeit.default_timer() - temp_time
    Path("medium.txt").unlink()

    # large file (64kb)
    temp_time = timeit.default_timer()
    sftp.put("64kb.txt", "/root/64kb.txt")
    l_put_time = timeit.default_timer() - temp_time

    temp_time = timeit.default_timer()
    sftp.get("/root/64kb.txt", "large.txt")
    l_get_time = timeit.default_timer() - temp_time
    Path("large.txt").unlink()

    sftp.close()
    ssh.close()

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