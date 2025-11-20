# /// script
# requires-python = ">=3.14"
# dependencies = [
#     "asyncssh",
#     "memray",
# ]
# ///
import asyncio
import json
from pathlib import Path
from pprint import pprint
import time

import asyncssh
import memray

if (mem_path := Path("memray-bench_asyncssh.bin")).exists():
    mem_path.unlink()
with memray.Tracker("memray-bench_asyncssh.bin"):
    results_dict = {}

    start_time = time.time()

    host_info = json.loads(Path("target.json").read_text())

    async def benchmark_task(task_id=0, semaphore=None):
        if semaphore:
            async with semaphore:
                return await _do_benchmark(task_id)
        else:
            return await _do_benchmark(task_id)

    async def _do_benchmark(task_id):
        start = time.time()
        async with asyncssh.connect(
            host=host_info["host"],
            port=host_info["port"],
            username="root",
            password=host_info["password"],
            known_hosts=None,
        ) as conn:
            connect_time = (time.time() - start) * 1000

            start = time.time()
            await conn.run("echo test", check=True)
            cmd_time = (time.time() - start) * 1000

            async with conn.start_sftp_client() as sftp:
                # small file (1kb)
                start = time.time()
                await sftp.put("1kb.txt", "/root/1kb.txt")
                s_put_time = (time.time() - start) * 1000

                start = time.time()
                small_file = f"small_{task_id}.txt"
                await sftp.get("/root/1kb.txt", small_file)
                s_get_time = (time.time() - start) * 1000
                Path(small_file).unlink()

                # medium file (14kb)
                start = time.time()
                await sftp.put("14kb.txt", "/root/14kb.txt")
                m_put_time = (time.time() - start) * 1000

                start = time.time()
                medium_file = f"medium_{task_id}.txt"
                await sftp.get("/root/14kb.txt", medium_file)
                m_get_time = (time.time() - start) * 1000
                Path(medium_file).unlink()

                # large file (64kb)
                start = time.time()
                await sftp.put("64kb.txt", "/root/64kb.txt")
                l_put_time = (time.time() - start) * 1000

                start = time.time()
                large_file = f"large_{task_id}.txt"
                await sftp.get("/root/64kb.txt", large_file)
                l_get_time = (time.time() - start) * 1000
                Path(large_file).unlink()

        return {
            "connect_time": f"{connect_time:.2f} ms",
            "cmd_time": f"{cmd_time:.2f} ms",
            "s_put_time": f"{s_put_time:.2f} ms",
            "s_get_time": f"{s_get_time:.2f} ms",
            "m_put_time": f"{m_put_time:.2f} ms",
            "m_get_time": f"{m_get_time:.2f} ms",
            "l_put_time": f"{l_put_time:.2f} ms",
            "l_get_time": f"{l_get_time:.2f} ms",
        }

    async def main():
        semaphore = asyncio.Semaphore(20)  # Limit to 20 concurrent connections

        # Single task
        print("Running single task benchmark...")
        result = await benchmark_task(0, None)
        results_dict["single"] = result

        # 10 tasks
        print("Running 10 concurrent tasks benchmark...")
        start = time.time()
        results = await asyncio.gather(*[benchmark_task(i, semaphore) for i in range(10)])
        total_time = (time.time() - start) * 1000
        avg_result = {}
        for key in results[0]:
            values = [float(r[key].split()[0]) for r in results]
            avg = sum(values) / len(values)
            avg_result[key] = f"{avg:.2f} ms"
        avg_result["total_time"] = f"{total_time:.2f} ms"
        results_dict["10_tasks"] = avg_result

        # 100 tasks
        print("Running 100 concurrent tasks benchmark...")
        start = time.time()
        results = await asyncio.gather(*[benchmark_task(i, semaphore) for i in range(100)])
        total_time = (time.time() - start) * 1000
        avg_result = {}
        for key in results[0]:
            values = [float(r[key].split()[0]) for r in results]
            avg = sum(values) / len(values)
            avg_result[key] = f"{avg:.2f} ms"
        avg_result["total_time"] = f"{total_time:.2f} ms"
        results_dict["100_tasks"] = avg_result

        results_dict["total_time"] = f"{(time.time() - start_time) * 1000:.2f} ms"

        pprint(results_dict, sort_dicts=False)

        # Save to results
        if Path("bench_results.json").exists():
            all_results = json.loads(Path("bench_results.json").read_text())
        else:
            all_results = {}
        all_results.update({"asyncssh": results_dict})
        Path("bench_results.json").write_text(json.dumps(all_results, indent=2))

    if __name__ == "__main__":
        asyncio.run(main())
