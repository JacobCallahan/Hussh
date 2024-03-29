import json
from pathlib import Path
import subprocess

from rich.console import Console
from rich.table import Table


def run_all():
    """Find all the python files in this directory starting with bench_ and run them."""
    for file in Path(__file__).parent.glob("bench_*.py"):
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
    """Print out the report in a rich table"""
    report_table = Table(title="Benchmark Report")
    # Add the columns
    report_table.add_column("Library")
    for key in report_dict["hussh"]:
        report_table.add_column(key)
    for lib in report_dict:
        report_table.add_row(lib, *[report_dict[lib][key] for key in report_dict[lib]])
    Console().print(report_table)


if __name__ == "__main__":
    run_all()
    report_dict = json.loads(Path("bench_results.json").read_text())
    run_memray_reports(report_dict)
    print_report(report_dict)
    Path("bench_results.json").unlink()
