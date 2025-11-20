import json
from pathlib import Path
import subprocess

from rich.console import Console
from rich.table import Table


def run_all():
    """Find all the python files in this directory starting with bench_ and run them in groups."""
    sync_benchmarks = []
    async_benchmarks = []

    for file in Path(__file__).parent.glob("bench_*.py"):
        if "async" in file.stem:
            async_benchmarks.append(file)
        else:
            sync_benchmarks.append(file)

    print("Running synchronous benchmarks...")
    for file in sync_benchmarks:
        print(f"Running {file}")
        subprocess.run(["python", file], check=True)

    print("\nRunning asynchronous benchmarks...")
    for file in async_benchmarks:
        print(f"Running {file}")
        subprocess.run(["python", file], check=True)


def run_memray_reports(report_dict):
    """Find all memray reports, run them, then delete them."""
    for file in Path(__file__).parent.glob("memray-*.bin"):
        # Figure out what library we're looking at
        lib = file.stem.replace("memray-bench_", "")
        json_file = Path(f"{lib}.json")
        subprocess.run(["memray", "stats", "--json", file, "-o", str(json_file)], check=True)
        file.unlink()
        # load the new json file
        results = json.loads(json_file.read_text())
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
        # Get metric keys from single task
        sample_lib = async_libs[0]
        sample_concurrency = report_dict[sample_lib]["single"]
        for key in sample_concurrency:
            async_table.add_column(key.replace("_", " ").title())
        # Add peak memory and allocations if present
        if "peak_memory" in report_dict[sample_lib]:
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
                    row.extend(lib_data[concurrency][key] for key in sample_concurrency)
                    if "peak_memory" in lib_data:
                        row.append(lib_data["peak_memory"])
                        row.append(lib_data["allocations"])
                    async_table.add_row(*row)
        Console().print(async_table)


if __name__ == "__main__":
    run_all()
    report_dict = json.loads(Path("bench_results.json").read_text())
    run_memray_reports(report_dict)
    print_report(report_dict)
    Path("bench_results.json").unlink()
