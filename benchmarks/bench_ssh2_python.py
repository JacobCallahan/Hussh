import json
import memray
import timeit
from pathlib import Path


with memray.Tracker("memray-bench_ssh2_python.bin"):
    start_time = timeit.default_timer()
    import socket
    from ssh2.session import Session
    from ssh2 import sftp
    import_time = timeit.default_timer() - start_time

    host_info = json.loads(Path("target.json").read_text())

    # connect to the server
    temp_time = timeit.default_timer()
    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    sock.connect((host_info["host"], host_info["port"]))
    session = Session()
    session.handshake(sock)
    session.userauth_password(host_info["username"], host_info["password"])
    connect_time = timeit.default_timer() - temp_time

    # execute a command
    temp_time = timeit.default_timer()
    channel = session.open_session()
    channel.execute("echo test")
    channel.wait_eof()
    channel.close()
    channel.wait_closed()
    size, data = channel.read()
    stdout = ""
    while size > 0:
        stdout += data.decode("utf-8")
        size, data = channel.read()
    stderr = channel.read_stderr()
    status = channel.get_exit_status()
    run_time = timeit.default_timer() - temp_time

    # small file (1kb)
    temp_time = timeit.default_timer()
    SFTP_MODE = (
        sftp.LIBSSH2_SFTP_S_IRUSR
        | sftp.LIBSSH2_SFTP_S_IWUSR
        | sftp.LIBSSH2_SFTP_S_IRGRP
        | sftp.LIBSSH2_SFTP_S_IROTH
    )
    FILE_FLAGS = (
        sftp.LIBSSH2_FXF_CREAT | sftp.LIBSSH2_FXF_WRITE | sftp.LIBSSH2_FXF_TRUNC
    )
    data = Path("1kb.txt").read_bytes()
    sftp_conn = session.sftp_init()
    with sftp_conn.open("/root/1kb.txt", FILE_FLAGS, SFTP_MODE) as f:
        f.write(data)
    s_put_time = timeit.default_timer() - temp_time

    temp_time = timeit.default_timer()
    with sftp_conn.open("/root/1kb.txt", sftp.LIBSSH2_FXF_READ, sftp.LIBSSH2_SFTP_S_IRUSR) as f:
        read_data= b""
        for _rc, data in f:
            read_data += data
    Path("small.txt").write_bytes(read_data)
    s_get_time = timeit.default_timer() - temp_time
    Path("small.txt").unlink()

    # medium file (14kb)
    temp_time = timeit.default_timer()
    data = Path("14kb.txt").read_bytes()
    with sftp_conn.open("/root/14kb.txt", FILE_FLAGS, SFTP_MODE) as f:
        f.write(data)
    m_put_time = timeit.default_timer() - temp_time

    temp_time = timeit.default_timer()
    with sftp_conn.open("/root/14kb.txt", sftp.LIBSSH2_FXF_READ, sftp.LIBSSH2_SFTP_S_IRUSR) as f:
        read_data= b""
        for _rc, data in f:
            read_data += data
    Path("medium.txt").write_bytes(read_data)
    m_get_time = timeit.default_timer() - temp_time
    Path("medium.txt").unlink()

    # large file (64kb)
    temp_time = timeit.default_timer()
    data = Path("64kb.txt").read_bytes()
    with sftp_conn.open("/root/14kb.txt", FILE_FLAGS, SFTP_MODE) as f:
        f.write(data)
    l_put_time = timeit.default_timer() - temp_time

    temp_time = timeit.default_timer()
    with sftp_conn.open("/root/64kb.txt", sftp.LIBSSH2_FXF_READ, sftp.LIBSSH2_SFTP_S_IRUSR) as f:
        read_data= b""
        for _rc, data in f:
            read_data += data
    Path("large.txt").write_bytes(read_data)
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
