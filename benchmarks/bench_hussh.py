import json
import memray
import timeit
from pprint import pprint
from pathlib import Path

results_dict = {}

if (mem_path := Path("memray-bench_hussh.bin")).exists():
    mem_path.unlink()
with memray.Tracker("memray-bench_hussh.bin"):
    start_time = timeit.default_timer()
    from hussh import Connection
    results_dict["import_time"] = f"{(timeit.default_timer() - start_time) * 1000:.2f} ms"

    host_info = json.loads(Path("target.json").read_text())

    temp_time = timeit.default_timer()
    conn = Connection(
        host=host_info["host"],
        port=host_info["port"],
        password=host_info["password"],
    )
    results_dict["connect_time"] = f"{(timeit.default_timer() - temp_time) * 1000:.2f} ms"

    temp_time = timeit.default_timer()
    result = conn.execute("echo test")
    results_dict["cmd_time"] = f"{(timeit.default_timer() - temp_time) * 1000:.2f} ms"

    # small file (1kb)
    temp_time = timeit.default_timer()
    conn.sftp_write("1kb.txt", "/root/1kb.txt")
    results_dict["s_put_time"] = f"{(timeit.default_timer() - temp_time) * 1000:.2f} ms"

    temp_time = timeit.default_timer()
    conn.sftp_read("/root/1kb.txt", "small.txt")
    results_dict["s_get_time"] = f"{(timeit.default_timer() - temp_time) * 1000:.2f} ms"
    Path("small.txt").unlink()

    # medium file (14kb)
    temp_time = timeit.default_timer()
    conn.sftp_write("14kb.txt", "/root/14kb.txt")
    results_dict["m_put_time"] = f"{(timeit.default_timer() - temp_time) * 1000:.2f} ms"

    temp_time = timeit.default_timer()
    conn.sftp_read("/root/14kb.txt", "medium.txt")
    results_dict["m_get_time"] = f"{(timeit.default_timer() - temp_time) * 1000:.2f} ms"
    Path("medium.txt").unlink()

    # large file (64kb)
    temp_time = timeit.default_timer()
    conn.sftp_write("64kb.txt", "/root/64kb.txt")
    results_dict["l_put_time"] = f"{(timeit.default_timer() - temp_time) * 1000:.2f} ms"

    temp_time = timeit.default_timer()
    conn.sftp_read("/root/64kb.txt", "large.txt")
    results_dict["l_get_time"] = f"{(timeit.default_timer() - temp_time) * 1000:.2f} ms"
    Path("large.txt").unlink()

    results_dict["total_time"] = f"{(timeit.default_timer() - start_time) * 1000:.2f} ms"

pprint(results_dict, sort_dicts=False)

if Path("bench_results.json").exists():
   results = json.loads(Path("bench_results.json").read_text())
else:
    results = {}
results.update({"hussh": results_dict})
Path("bench_results.json").write_text(json.dumps(results, indent=2))
