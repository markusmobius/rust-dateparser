import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import shutil
import statistics
import subprocess
import time


def measure(command, environment, root, passes):
    started = time.perf_counter()
    ready = None
    result = None
    launch_to_ready_ms = None
    output = []
    with subprocess.Popen(
        command, cwd=root, env=environment, stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT, text=True, encoding="utf-8",
    ) as process:
        for line in process.stdout:
            output.append(line)
            if "BENCHMARK_READY " in line:
                launch_to_ready_ms = (time.perf_counter() - started) * 1000
                ready = json.loads(line.partition("BENCHMARK_READY ")[2])
            if "BENCHMARK_RESULT " in line:
                result = json.loads(line.partition("BENCHMARK_RESULT ")[2])
        status = process.wait()
    if status != 0 or ready is None or result is None:
        raise RuntimeError(f"Benchmark failed ({status}):\n{''.join(output[-25:])}")
    if ready != result["metadata"]:
        raise RuntimeError("Benchmark metadata changed between ready and result")
    if len(result["pass_ms"]) != passes or any(
        not math.isfinite(value) or value <= 0 for value in result["pass_ms"]
    ):
        raise RuntimeError("Invalid benchmark timings")
    result["launch_to_ready_ms"] = launch_to_ready_ms
    return result


def summarize(records):
    summary = {}
    for cohort in sorted({record["cohort"] for record in records}):
        summary[cohort] = {}
        for engine in ("rust", "go"):
            selected = [
                record for record in records
                if record["cohort"] == cohort and record["engine"] == engine
            ]
            medians = [statistics.median(record["pass_ms"]) for record in selected]
            metadata = selected[0]["metadata"]
            median = statistics.median(medians)
            summary[cohort][engine] = {
                "cases": metadata["cases"],
                "parsed": metadata["parsed"],
                "warm_pass_median_ms": median,
                "warm_pass_median_range_ms": [min(medians), max(medians)],
                "microseconds_per_input": median * 1000 / metadata["cases"],
                "inputs_per_second": metadata["cases"] * 1000 / median,
                "launch_to_ready_median_ms": statistics.median(
                    record["launch_to_ready_ms"] for record in selected
                ),
            }
        summary[cohort]["go_over_rust_time"] = (
            summary[cohort]["go"]["warm_pass_median_ms"]
            / summary[cohort]["rust"]["warm_pass_median_ms"]
        )
    return summary


def main():
    parser = argparse.ArgumentParser(description="Single-core Go/Rust parsing comparison")
    parser.add_argument("--runs", type=int, default=6)
    parser.add_argument("--passes", type=int, default=8)
    parser.add_argument("--cpu", type=int, default=2)
    parser.add_argument("--cohort", action="append", choices=("auto", "explicit", "htmldate"))
    parser.add_argument("--output", type=Path, default=Path("target/benchmark/latest.json"))
    arguments = parser.parse_args()
    if arguments.runs < 2 or arguments.runs % 2 or arguments.passes < 1:
        parser.error("use an even number of runs >= 2 and at least one measured pass")
    if not hasattr(os, "sched_getaffinity") or arguments.cpu not in os.sched_getaffinity(0):
        parser.error("run under Linux/WSL and select an available CPU with --cpu")
    root = Path(__file__).resolve().parents[1]
    suite = root / "testdata/go-core.json"
    fixture_sha256 = hashlib.sha256(suite.read_bytes()).hexdigest()
    cargo = shutil.which("cargo") or str(Path.home() / ".cargo/bin/cargo")
    environment = os.environ.copy()
    environment.update({
        "GOTOOLCHAIN": "go1.27.1", "GOMAXPROCS": "1", "CGO_ENABLED": "0",
        "GOAMD64": "v1", "GOFLAGS": "", "TZ": "UTC", "LC_ALL": "C.UTF-8",
        "GOGC": "100", "GOMEMLIMIT": "off", "GOEXPERIMENT": "", "GODEBUG": "",
        "RUSTFLAGS": "", "CARGO_ENCODED_RUSTFLAGS": "",
        "DATEPARSER_BENCHMARK_SUITE": str(suite),
    })
    built = subprocess.run(
        [cargo, "test", "--locked", "--release", "--lib", "--no-run", "--message-format=json"],
        cwd=root, env=environment, check=True, stdout=subprocess.PIPE, text=True,
    )
    executables = [
        message["executable"]
        for line in built.stdout.splitlines()
        if (message := json.loads(line)).get("reason") == "compiler-artifact"
        and message.get("executable") and message["target"]["name"] == "rust_dateparser"
    ]
    if len(executables) != 1:
        raise RuntimeError(f"Expected one Rust test executable, found {executables}")
    rust_binary = Path(executables[0])
    go_binary = root / "target/dateparser-go-benchmark"
    subprocess.run(
        ["go", "build", "-mod=readonly", "-trimpath", "-o", str(go_binary), "."],
        cwd=root / "tools/go-reference", env=environment, check=True,
    )
    versions = {
        "rust": subprocess.check_output(
            [str(Path(cargo).with_name("rustc")), "--version"], cwd=root,
            env=environment, text=True,
        ).strip(),
        "go": subprocess.check_output(
            ["go", "version"], cwd=root, env=environment, text=True,
        ).strip(),
    }
    cpu_model = next(
        line.partition(":")[2].strip()
        for line in Path("/proc/cpuinfo").read_text().splitlines()
        if line.startswith("model name")
    )
    os.sched_setaffinity(0, {arguments.cpu})
    records = []
    identities = {}

    def launch(engine, cohort, passes):
        child_environment = environment | {
            "DATEPARSER_BENCHMARK_COHORT": cohort,
            "DATEPARSER_BENCHMARK_PASSES": str(passes),
        }
        command = (
            [str(rust_binary), "--exact", "upstream_tests::benchmark_public_parse",
             "--ignored", "--nocapture", "--test-threads=1"]
            if engine == "rust" else
            [str(go_binary), "-benchmark", str(suite), "-cohort", cohort, "-passes", str(passes)]
        )
        result = measure(command, child_environment, root, passes)
        metadata = result["metadata"]
        identity = {key: metadata[key] for key in ("cohort", "cases", "parsed", "fixture_sha256")}
        if metadata["fixture_sha256"] != fixture_sha256 or metadata["cohort"] != cohort:
            raise RuntimeError("Benchmark used a different fixture or cohort")
        if identity != identities.setdefault(cohort, identity):
            raise RuntimeError("Go/Rust cohort membership or parsed counts differ")
        return result | {"engine": engine, "cohort": cohort}

    for cohort in arguments.cohort or ("auto", "explicit", "htmldate"):
        for engine in ("rust", "go"):
            launch(engine, cohort, 1)
        print(f"{cohort}: preflight discarded", flush=True)
        for index in range(arguments.runs):
            engines = ("rust", "go") if index % 2 == 0 else ("go", "rust")
            for engine in engines:
                record = launch(engine, cohort, arguments.passes)
                record["round"] = index + 1
                records.append(record)
                print(
                    f"{cohort} {engine} {index + 1}/{arguments.runs}: "
                    f"ready {record['launch_to_ready_ms']:.3f} ms, "
                    f"warm {statistics.median(record['pass_ms']):.3f} ms", flush=True,
                )
    report = {
        "date": time.strftime("%Y-%m-%d"), "platform": platform.platform(),
        "cpu_model": cpu_model, "cpu": arguments.cpu, "versions": versions,
        "runs_per_engine": arguments.runs, "passes_per_run": arguments.passes,
        "fixture_sha256": fixture_sha256,
        "binary_sha256": {
            "rust": hashlib.sha256(rust_binary.read_bytes()).hexdigest(),
            "go": hashlib.sha256(go_binary.read_bytes()).hexdigest(),
        },
        "summary": summarize(records), "runs": records,
    }
    destination = arguments.output
    if not destination.is_absolute():
        destination = root / destination
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report["summary"], indent=2))
    print(f"Report: {destination}")


if __name__ == "__main__":
    main()