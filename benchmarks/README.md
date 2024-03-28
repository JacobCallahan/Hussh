
## Install benchmarking requirements
First, you will either need to have your own test target, putting that information in the `target.json` file in this directory.
Alternatively, you can install Docker or Podman, then build the hussh-test-server image in the `tests/setup/` directory of this repo.
Then you can start the test server with this command
```bash
docker run --rm -d -p 8022:22 hussh-test-server
```
Once either of those are satisfied, install the libraries to be benchmarked by running this command in this directory.
```bash
pip install -r requirements.txt
```

## Running all benchmark scripts
To run the test scripts, you can either manually run each one, or use this bash loop.
```bash
for file in bench_*.py; do echo "$file"; python "$file"; done
```
This will also create a memray output file for each script ran.
We'll use these in the next step.

## Getting the total memory consumption for all benchmarks
This loop will get summaries for each benchmark's memory consumption and pull out the total memory and allocations.
```bash
for file in memray-bench_*; do echo "$file"; memray summary -r 1 "$file" | grep " at " | tr -d '[:space:]' | awk -F '│' '{print "Memory: " $3 ", Allocations: " $7}'; done
```

## Cleanup
You likely don't want the memray files to hang around, so you can easily delete them by running this.
```bash
rm -f memray-bench_*
```

# ToDo
- Improve reporting by putting it all in a nice table.
- Remove the need to execute commands manually by scripting it all.
