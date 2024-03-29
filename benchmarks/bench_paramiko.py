import json
from pathlib import Path
from pprint import pprint
import timeit

import memray

results_dict = {}

if (mem_path := Path("memray-bench_paramiko.bin")).exists():
    mem_path.unlink()
with memray.Tracker("memray-bench_paramiko.bin"):
    start_time = timeit.default_timer()
    import paramiko

    results_dict["import_time"] = f"{(timeit.default_timer() - start_time) * 1000:.2f} ms"

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
    results_dict["connect_time"] = f"{(timeit.default_timer() - temp_time) * 1000:.2f} ms"

    temp_time = timeit.default_timer()
    stdin, stdout, stderr = ssh.exec_command("echo test")
    result = stdout.read()
    results_dict["cmd_time"] = f"{(timeit.default_timer() - temp_time) * 1000:.2f} ms"

    # small file (1kb)
    temp_time = timeit.default_timer()
    sftp = ssh.open_sftp()
    sftp.put("1kb.txt", "/root/1kb.txt")
    results_dict["s_put_time"] = f"{(timeit.default_timer() - temp_time) * 1000:.2f} ms"

    temp_time = timeit.default_timer()
    sftp.get("/root/1kb.txt", "small.txt")
    results_dict["s_get_time"] = f"{(timeit.default_timer() - temp_time) * 1000:.2f} ms"
    Path("small.txt").unlink()

    # medium file (14kb)
    temp_time = timeit.default_timer()
    sftp.put("14kb.txt", "/root/14kb.txt")
    results_dict["m_put_time"] = f"{(timeit.default_timer() - temp_time) * 1000:.2f} ms"

    temp_time = timeit.default_timer()
    sftp.get("/root/14kb.txt", "medium.txt")
    results_dict["m_get_time"] = f"{(timeit.default_timer() - temp_time) * 1000:.2f} ms"
    Path("medium.txt").unlink()

    # large file (64kb)
    temp_time = timeit.default_timer()
    sftp.put("64kb.txt", "/root/64kb.txt")
    results_dict["l_put_time"] = f"{(timeit.default_timer() - temp_time) * 1000:.2f} ms"

    temp_time = timeit.default_timer()
    sftp.get("/root/64kb.txt", "large.txt")
    results_dict["l_get_time"] = f"{(timeit.default_timer() - temp_time) * 1000:.2f} ms"
    Path("large.txt").unlink()

    sftp.close()
    ssh.close()

    results_dict["total_time"] = f"{(timeit.default_timer() - start_time) * 1000:.2f} ms"

pprint(results_dict, sort_dicts=False)

if Path("bench_results.json").exists():
    results = json.loads(Path("bench_results.json").read_text())
else:
    results = {}
results.update({"paramiko": results_dict})
Path("bench_results.json").write_text(json.dumps(results, indent=2))
