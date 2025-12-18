import json
from pathlib import Path
import subprocess
import time

import docker
from rich.console import Console
from rich.table import Table

TEST_SERVER_IMAGE = "ghcr.io/jacobcallahan/hussh/hussh-test-server:latest"


def start_server():
    """Start the test server container."""
    client = docker.from_env()
    try:
        client.images.get(TEST_SERVER_IMAGE)
    except docker.errors.ImageNotFound:
        print("Pulling test server image...")
        client.images.pull(TEST_SERVER_IMAGE)

    # Ensure the container is removed before starting a new one
    try:
        container = client.containers.get("hussh-test-server")
        container.stop()
        container.remove()
        print("Removed existing hussh-test-server container.")
    except docker.errors.NotFound:
        pass  # Container doesn't exist, no need to remove

    print("Starting test server container...")
    container = client.containers.run(
        TEST_SERVER_IMAGE,
        command=[
            "/usr/sbin/sshd",
            "-D",
            "-o",
            "MaxStartups=110:30:300",
            "-o",
            "MaxSessions=200",
        ],
        detach=True,
        ports={"22/tcp": 8022},
        name="hussh-test-server",
    )
    time.sleep(5)
    return container, True  # Always managed by us now


def run_all(run_sync=False, run_async=False):
    """Find all the python files in this directory starting with bench_ and run them in groups."""
    container, managed = start_server()
    try:
        if not run_sync and not run_async:
            run_sync = run_async = True  # run both

        sync_benchmarks = []
        async_benchmarks = []

        for file in Path(__file__).parent.glob("bench_*.py"):
            if "async" in file.stem:
                async_benchmarks.append(file)
            else:
                sync_benchmarks.append(file)

        if run_sync:
            print("Running synchronous benchmarks...")
            for file in sync_benchmarks:
                print(f"Running {file}")
                subprocess.run(["python", file], check=True, cwd=Path(__file__).parent)

        if run_async:
            print("\nRunning asynchronous benchmarks...")
            for file in async_benchmarks:
                print(f"Running {file}")
                subprocess.run(["python", file], check=True, cwd=Path(__file__).parent)
    finally:
        if managed:
            print("Stopping test server container...")
            container.stop()
            container.remove()


def run_memray_reports(report_dict):
    """Find all memray reports, run them, then delete them."""
    for file in Path(__file__).parent.glob("memray-*.bin"):
        # Figure out what library we're looking at
        lib = file.stem.replace("memray-bench_", "")
        json_file = Path(__file__).parent / f"{lib}.json"
        if json_file.exists():
            json_file.unlink()
        subprocess.run(["memray", "stats", "--json", str(file), "-o", str(json_file)], check=True)
        file.unlink()
        # load the new json file
        results = json.loads(json_file.read_text())
        if "_" in lib and lib.startswith("async"):
            base_lib, suffix = lib.rsplit("_", 1)
            if suffix == "10":
                suffix = "10_tasks"
            elif suffix == "100":
                suffix = "100_tasks"
            # else suffix is "single"

            if base_lib in report_dict and suffix in report_dict[base_lib]:
                report_dict[base_lib][suffix]["peak_memory"] = (
                    f'{results["metadata"]["peak_memory"] / 1024 / 1024:.2f} MB'
                )
                report_dict[base_lib][suffix]["allocations"] = str(
                    results["metadata"]["total_allocations"]
                )
        else:
            # sync
            report_dict[lib]["peak_memory"] = (
                f'{results["metadata"]["peak_memory"] / 1024 / 1024:.2f} MB'
            )
            report_dict[lib]["allocations"] = str(results["metadata"]["total_allocations"])
        json_file.unlink()


def print_report(report_dict):
    """Print out the report in rich tables"""
    sync_libs = [lib for lib in report_dict if not lib.startswith("async")]
    async_libs = [lib for lib in report_dict if lib.startswith("async")]

    if sync_libs:
        sync_table = Table(title="Synchronous Benchmark Report")
        sync_table.add_column("Library")
        if sync_libs:
            for key in report_dict[sync_libs[0]]:
                sync_table.add_column(key.replace("_", " ").title())
        for lib in sync_libs:
            row = [lib.replace("_", " ").title()] + [
                report_dict[lib][key] for key in report_dict[lib]
            ]
            sync_table.add_row(*row)
        Console().print(sync_table)
        Console().print()

    if async_libs:
        async_table = Table(title="Asynchronous Benchmark Report")
        async_table.add_column("Library")
        async_table.add_column("Concurrency")
        # Get metric keys from single task, excluding memory stats
        sample_lib = async_libs[0]
        sample_concurrency = report_dict[sample_lib]["single"]
        metric_keys = [
            key for key in sample_concurrency if key not in ("peak_memory", "allocations")
        ]
        for key in metric_keys:
            async_table.add_column(key.replace("_", " ").title())
        # Add peak memory and allocations
        async_table.add_column("Peak Memory")
        async_table.add_column("Allocations")

        for lib in async_libs:
            lib_data = report_dict[lib]
            for concurrency in ["single", "10_tasks", "100_tasks"]:
                if concurrency in lib_data:
                    row = [
                        lib.replace("async_", "").replace("_", " ").title(),
                        concurrency.replace("_", " ").title(),
                    ]
                    row.extend(lib_data[concurrency][key] for key in metric_keys)
                    row.append(lib_data[concurrency].get("peak_memory", ""))
                    row.append(lib_data[concurrency].get("allocations", ""))
                    async_table.add_row(*row)
        Console().print(async_table)


if __name__ == "__main__":
    import argparse

    parser = argparse.ArgumentParser()
    group = parser.add_mutually_exclusive_group()
    group.add_argument("--sync", action="store_true", help="Run only synchronous benchmarks")
    group.add_argument("--run-async", action="store_true", help="Run only asynchronous benchmarks")
    args = parser.parse_args()

    run_all(run_sync=args.sync, run_async=args.run_async)
    results_path = Path(__file__).parent / "bench_results.json"
    if results_path.exists():
        report_dict = json.loads(results_path.read_text())
        run_memray_reports(report_dict)
        print_report(report_dict)
        results_path.unlink()
