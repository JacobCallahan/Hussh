# Benchmark Testing
Benchmarks against ssh libraries are difficult due to network connection variations.
Libraries are likely to vary from run to run. However, there are some things we can do to reduce this uncertainty.
The first is to remove as much network variability as we reasonably can by running a local test server (see below).
It is also a good idea to run benchmarks a few times and run an average. 

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
To run the test scripts, you just need to execute the run_benchmarks.py script.
```bash
python run_benchmarks.py
```
This will ultimately collect all the benchmark and memray information into a table.

Alternatively, if you'd prefer to run individual benchmarks, you can do that.
```bash
python test_hussh.py
```
This will also create a memray output file for each script ran.
