import argparse
import hashlib
import itertools
import json
import math
import os
from pathlib import Path
import platform
import shutil
import statistics
import subprocess
import tempfile
import time


ENGINES = ("rust", "go-v1.4.3", "go-v1.4.4")
GO_MODULE = "github.com/markusmobius/go-dateparser"


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
        for engine in sorted({record["engine"] for record in records}):
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
        if "go-v1.4.3" in summary[cohort] and "go-v1.4.4" in summary[cohort]:
            summary[cohort]["go_old_over_new_time"] = (
                summary[cohort]["go-v1.4.3"]["warm_pass_median_ms"]
                / summary[cohort]["go-v1.4.4"]["warm_pass_median_ms"]
            )
        if "rust" in summary[cohort] and "go-v1.4.4" in summary[cohort]:
            summary[cohort]["go_over_rust_time"] = (
                summary[cohort]["go-v1.4.4"]["warm_pass_median_ms"]
                / summary[cohort]["rust"]["warm_pass_median_ms"]
            )
    return summary


def main():
    parser = argparse.ArgumentParser(description="Single-core Go/Rust parsing comparison")
    parser.add_argument("--runs", type=int, default=6)
    parser.add_argument("--passes", type=int, default=8)
    parser.add_argument("--cpu", type=int, default=2)
    parser.add_argument("--cohort", action="append", choices=("auto", "explicit", "htmldate"))
    parser.add_argument("--engine", action="append", choices=ENGINES,
                        help="repeat to select engines; defaults to all three")
    parser.add_argument("--output", type=Path, default=Path("target/benchmark/latest.json"))
    arguments = parser.parse_args()
    engines = arguments.engine or list(ENGINES)
    if len(engines) < 2 or len(set(engines)) != len(engines):
        parser.error("select at least two distinct engines")
    orders = list(itertools.permutations(engines))
    if arguments.runs < len(orders) or arguments.runs % len(orders) or arguments.passes < 1:
        parser.error(f"use a positive multiple of {len(orders)} runs and at least one pass")
    if arguments.cohort and len(set(arguments.cohort)) != len(arguments.cohort):
        parser.error("select each cohort at most once")
    if not hasattr(os, "sched_getaffinity") or arguments.cpu not in os.sched_getaffinity(0):
        parser.error("run under Linux/WSL and select an available CPU with --cpu")
    root = Path(__file__).resolve().parents[1]
    suite = root / "testdata/go-core.json"
    fixture_bytes = suite.read_bytes()
    fixture_sha256 = hashlib.sha256(fixture_bytes).hexdigest()
    environment = os.environ.copy()
    environment.update({
        "GOTOOLCHAIN": "go1.27.1", "GOMAXPROCS": "1", "CGO_ENABLED": "0",
        "GOAMD64": "v1", "GOFLAGS": "", "TZ": "UTC", "LC_ALL": "C.UTF-8",
        "GOGC": "100", "GOMEMLIMIT": "off", "GOEXPERIMENT": "", "GODEBUG": "",
        "RUSTFLAGS": "", "CARGO_ENCODED_RUSTFLAGS": "",
        "DATEPARSER_BENCHMARK_SUITE": str(suite),
    })
    binaries = {}
    versions = {}
    if "rust" in engines:
        cargo = shutil.which("cargo") or str(Path.home() / ".cargo/bin/cargo")
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
        binaries["rust"] = Path(executables[0])
        versions["rust"] = subprocess.check_output(
            [str(Path(cargo).with_name("rustc")), "--version"], cwd=root,
            env=environment, text=True,
        ).strip()
    versions["go"] = subprocess.check_output(
        ["go", "version"], cwd=root, env=environment, text=True,
    ).strip()
    reference_root = root / "tools/go-reference"
    build_root = root / "target/benchmark"
    build_root.mkdir(parents=True, exist_ok=True)
    go_references = {}
    with tempfile.TemporaryDirectory(prefix="go-versions-", dir=build_root) as temporary:
        for engine in engines:
            if engine == "rust":
                continue
            version = engine.removeprefix("go-")
            module_file = Path(temporary) / f"{engine}.mod"
            shutil.copyfile(reference_root / "go.mod", module_file)
            shutil.copyfile(reference_root / "go.sum", module_file.with_suffix(".sum"))
            subprocess.run(
                ["go", "mod", "edit", f"-modfile={module_file}", f"-require={GO_MODULE}@{version}"],
                cwd=reference_root, env=environment, check=True,
            )
            module = json.loads(subprocess.check_output(
                ["go", "mod", "download", "-json", f"-modfile={module_file}",
                 f"{GO_MODULE}@{version}"],
                cwd=reference_root, env=environment, text=True,
            ))
            if (module.get("Error") or module.get("Path") != GO_MODULE
                    or module.get("Version") != version or not module.get("Sum")
                    or not module.get("Origin", {}).get("Hash")):
                raise RuntimeError(f"Could not verify the published {version} module")
            go_references[engine] = {
                "module": GO_MODULE, "version": version,
                "commit": module["Origin"]["Hash"], "module_sum": module["Sum"],
            }
            binaries[engine] = build_root / engine
            subprocess.run(
                ["go", "build", f"-modfile={module_file}", "-mod=readonly", "-trimpath",
                 "-buildvcs=false", "-o", str(binaries[engine]), "."],
                cwd=reference_root, env=environment, check=True,
            )
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
            [str(binaries[engine]), "--exact", "upstream_tests::benchmark_public_parse",
             "--ignored", "--nocapture", "--test-threads=1"]
            if engine == "rust" else
            [str(binaries[engine]), "-benchmark", str(suite), "-cohort", cohort,
             "-passes", str(passes), "-benchmark-version", engine.removeprefix("go-")]
        )
        result = measure(command, child_environment, root, passes)
        metadata = result["metadata"]
        if engine != "rust" and metadata.get("go_reference") != go_references[engine]:
            raise RuntimeError("Benchmark binary does not match its published Go module")
        identity = {key: metadata[key] for key in ("cohort", "cases", "parsed", "fixture_sha256")}
        if metadata["fixture_sha256"] != fixture_sha256 or metadata["cohort"] != cohort:
            raise RuntimeError("Benchmark used a different fixture or cohort")
        if identity != identities.setdefault(cohort, identity):
            raise RuntimeError("Go/Rust cohort membership or parsed counts differ")
        return result | {"engine": engine, "cohort": cohort}

    for cohort in arguments.cohort or ("auto", "explicit", "htmldate"):
        for engine in engines:
            launch(engine, cohort, 1)
        print(f"{cohort}: preflight discarded", flush=True)
        for index in range(arguments.runs):
            for engine in orders[index % len(orders)]:
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
        "engine_orders": orders, "go_references": go_references,
        "fixture_sha256": fixture_sha256, "fixture_reference": json.loads(fixture_bytes)["reference"],
        "runner_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "binary_sha256": {
            engine: hashlib.sha256(binary.read_bytes()).hexdigest()
            for engine, binary in binaries.items()
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