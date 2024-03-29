import json
from pathlib import Path
from pprint import pprint
import timeit

import memray

results_dict = {}

if (mem_path := Path("memray-bench_ssh2-python.bin")).exists():
    mem_path.unlink()
with memray.Tracker("memray-bench_ssh2-python.bin"):
    start_time = timeit.default_timer()
    import socket

    from ssh2 import sftp
    from ssh2.session import Session

    results_dict["import_time"] = f"{(timeit.default_timer() - start_time) * 1000:.2f} ms"

    host_info = json.loads(Path("target.json").read_text())

    # connect to the server
    temp_time = timeit.default_timer()
    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    sock.connect((host_info["host"], host_info["port"]))
    session = Session()
    session.handshake(sock)
    session.userauth_password(host_info["username"], host_info["password"])
    results_dict["connect_time"] = f"{(timeit.default_timer() - temp_time) * 1000:.2f} ms"

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
    results_dict["cmd_time"] = f"{(timeit.default_timer() - temp_time) * 1000:.2f} ms"

    # small file (1kb)
    temp_time = timeit.default_timer()
    SFTP_MODE = (
        sftp.LIBSSH2_SFTP_S_IRUSR
        | sftp.LIBSSH2_SFTP_S_IWUSR
        | sftp.LIBSSH2_SFTP_S_IRGRP
        | sftp.LIBSSH2_SFTP_S_IROTH
    )
    FILE_FLAGS = sftp.LIBSSH2_FXF_CREAT | sftp.LIBSSH2_FXF_WRITE | sftp.LIBSSH2_FXF_TRUNC
    data = Path("1kb.txt").read_bytes()
    sftp_conn = session.sftp_init()
    with sftp_conn.open("/root/1kb.txt", FILE_FLAGS, SFTP_MODE) as f:
        f.write(data)
    results_dict["s_put_time"] = f"{(timeit.default_timer() - temp_time) * 1000:.2f} ms"

    temp_time = timeit.default_timer()
    with sftp_conn.open("/root/1kb.txt", sftp.LIBSSH2_FXF_READ, sftp.LIBSSH2_SFTP_S_IRUSR) as f:
        read_data = b""
        for _rc, data in f:
            read_data += data
    Path("small.txt").write_bytes(read_data)
    results_dict["s_get_time"] = f"{(timeit.default_timer() - temp_time) * 1000:.2f} ms"
    Path("small.txt").unlink()

    # medium file (14kb)
    temp_time = timeit.default_timer()
    data = Path("14kb.txt").read_bytes()
    with sftp_conn.open("/root/14kb.txt", FILE_FLAGS, SFTP_MODE) as f:
        f.write(data)
    results_dict["m_put_time"] = f"{(timeit.default_timer() - temp_time) * 1000:.2f} ms"

    temp_time = timeit.default_timer()
    with sftp_conn.open("/root/14kb.txt", sftp.LIBSSH2_FXF_READ, sftp.LIBSSH2_SFTP_S_IRUSR) as f:
        read_data = b""
        for _rc, data in f:
            read_data += data
    Path("medium.txt").write_bytes(read_data)
    results_dict["m_get_time"] = f"{(timeit.default_timer() - temp_time) * 1000:.2f} ms"
    Path("medium.txt").unlink()

    # large file (64kb)
    temp_time = timeit.default_timer()
    data = Path("64kb.txt").read_bytes()
    with sftp_conn.open("/root/14kb.txt", FILE_FLAGS, SFTP_MODE) as f:
        f.write(data)
    results_dict["l_put_time"] = f"{(timeit.default_timer() - temp_time) * 1000:.2f} ms"

    temp_time = timeit.default_timer()
    with sftp_conn.open("/root/64kb.txt", sftp.LIBSSH2_FXF_READ, sftp.LIBSSH2_SFTP_S_IRUSR) as f:
        read_data = b""
        for _rc, data in f:
            read_data += data
    Path("large.txt").write_bytes(read_data)
    results_dict["l_get_time"] = f"{(timeit.default_timer() - temp_time) * 1000:.2f} ms"
    Path("large.txt").unlink()

    results_dict["total_time"] = f"{(timeit.default_timer() - start_time) * 1000:.2f} ms"

pprint(results_dict, sort_dicts=False)

if Path("bench_results.json").exists():
    results = json.loads(Path("bench_results.json").read_text())
else:
    results = {}
results.update({"ssh2-python": results_dict})
Path("bench_results.json").write_text(json.dumps(results, indent=2))
