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


ENGINES = ("rust", "rust-baseline", "go-v1.4.3", "go-v1.4.4", "go-v1.4.5", "go-worktree")
GO_MODULE = "github.com/markusmobius/go-dateparser"
PARSE_COHORTS = ("auto", "explicit", "htmldate")
FEATURE_COHORTS = ("search-auto", "search-split", "search-ngram", "time-span", "jalali", "hijri")


def source_reference(root):
    sources = {}
    for directory, children, files in os.walk(root):
        children[:] = [child for child in children if not child.startswith(".")]
        for name in files:
            path = Path(directory) / name
            relative = path.relative_to(root).as_posix()
            if path.suffix == ".go" or relative in ("go.mod", "go.sum", "internal/parser/calendars/data.json"):
                sources[relative] = hashlib.sha256(path.read_bytes()).hexdigest()
    encoded = json.dumps(sources, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode("utf-8")
    return {
        "module": GO_MODULE, "version": "worktree", "module_sum": "",
        "commit": subprocess.check_output(["git", "-C", str(root), "rev-parse", "HEAD"], text=True).strip(),
        "source_sha256": hashlib.sha256(encoded).hexdigest(),
    }


def published_reference(version, module_file, root, environment):
    tag_commit = None
    if version == "v1.4.5":
        tag = f"refs/tags/{version}"
        resolved = subprocess.run(
            ["git", "ls-remote", "--exit-code", "--tags", f"https://{GO_MODULE}.git", tag, f"{tag}^{{}}"],
            cwd=root, env=environment, text=True, capture_output=True,
        )
        if resolved.returncode == 2:
            raise RuntimeError(f"Go {version} is not published yet; use --go-source for an explicitly labelled worktree comparison")
        resolved.check_returncode()
        refs = {name: commit for commit, name in (line.split() for line in resolved.stdout.splitlines())}
        tag_commit = refs.get(f"{tag}^{{}}", refs.get(tag))
        if not tag_commit:
            raise RuntimeError(f"Could not resolve Go {version}'s source commit")
    module = json.loads(subprocess.check_output(
        ["go", "mod", "download", "-json", f"-modfile={module_file}", f"{GO_MODULE}@{version}"],
        cwd=root, env=environment, text=True,
    ))
    if (module.get("Error") or module.get("Path") != GO_MODULE
            or module.get("Version") != version or not module.get("Sum")
            or not module.get("Origin", {}).get("Hash")):
        raise RuntimeError(f"Could not verify the published {version} module")
    if tag_commit and module["Origin"]["Hash"] != tag_commit:
        raise RuntimeError(f"Downloaded Go {version} module does not match its Git tag")
    return {
        "module": GO_MODULE, "version": version,
        "commit": module["Origin"]["Hash"], "module_sum": module["Sum"],
    }


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
            iterations = metadata.get("iterations", 1)
            summary[cohort][engine] = {
                "cases": metadata["cases"],
                "parsed": metadata["parsed"],
                "iterations": iterations,
                "warm_pass_median_ms": median,
                "warm_pass_median_range_ms": [min(medians), max(medians)],
                "microseconds_per_input": median * 1000 / (metadata["cases"] * iterations),
                "inputs_per_second": metadata["cases"] * iterations * 1000 / median,
                "launch_to_ready_median_ms": statistics.median(
                    record["launch_to_ready_ms"] for record in selected
                ),
            }
            if "python_matches" in metadata:
                summary[cohort][engine].update({
                    key: metadata[key] for key in ("python_matches", "python_mismatches", "panics")
                })
                for key in ("parsed", "matched_dates", "python_matches", "python_mismatches", "panics"):
                    values = [record["metadata"][key] for record in selected]
                    summary[cohort][engine][f"{key}_range"] = [min(values), max(values)]
                summary[cohort][engine]["outcome_variants"] = len({
                    record["metadata"]["outcome_sha256"] for record in selected
                })
        current_go = next((engine for engine in ("go-worktree", "go-v1.4.5", "go-v1.4.4") if engine in summary[cohort]), None)
        if "go-v1.4.3" in summary[cohort] and current_go in summary[cohort]:
            summary[cohort]["go_old_over_new_time"] = (
                summary[cohort]["go-v1.4.3"]["warm_pass_median_ms"]
                / summary[cohort][current_go]["warm_pass_median_ms"]
            )
        if "rust" in summary[cohort] and current_go in summary[cohort]:
            summary[cohort]["go_over_rust_time"] = (
                summary[cohort][current_go]["warm_pass_median_ms"]
                / summary[cohort]["rust"]["warm_pass_median_ms"]
            )
        if "rust-baseline" in summary[cohort] and "rust" in summary[cohort]:
            summary[cohort]["rust_before_over_after_time"] = (
                summary[cohort]["rust-baseline"]["warm_pass_median_ms"]
                / summary[cohort]["rust"]["warm_pass_median_ms"]
            )
    return summary


def main():
    parser = argparse.ArgumentParser(description="Single-core Go/Rust parsing comparison")
    parser.add_argument("--runs", type=int, default=6)
    parser.add_argument("--passes", type=int, default=8)
    parser.add_argument("--cpu", type=int, default=2)
    parser.add_argument("--cohort", action="append", choices=PARSE_COHORTS + FEATURE_COHORTS)
    parser.add_argument("--features", action="store_true", help="benchmark search, time spans, Jalali and Hijri against pinned Python outputs")
    parser.add_argument("--go-source", type=Path, help="explicit corrected Go checkout for feature benchmarks; never modifies the published reference")
    parser.add_argument("--rust-baseline", type=Path, help="preserved Rust benchmark executable for an interleaved before/after comparison")
    parser.add_argument("--iterations", type=int, default=16, help="feature corpus repetitions per warm pass")
    parser.add_argument("--engine", action="append", choices=ENGINES,
                        help="repeat to select engines; defaults to Rust and published Go v1.4.5")
    parser.add_argument("--output", type=Path, default=Path("target/benchmark/latest.json"))
    arguments = parser.parse_args()
    engines = arguments.engine or (["rust-baseline", "rust"] if arguments.rust_baseline else ["rust", "go-worktree" if arguments.go_source else "go-v1.4.5"])
    cohorts = FEATURE_COHORTS if arguments.features else PARSE_COHORTS
    if arguments.rust_baseline:
        arguments.rust_baseline = arguments.rust_baseline.resolve()
        if "rust-baseline" not in engines or not arguments.rust_baseline.is_file():
            parser.error("--rust-baseline must name a preserved executable and select rust-baseline")
    elif "rust-baseline" in engines:
        parser.error("rust-baseline requires --rust-baseline")
    if arguments.features and any(engine.startswith("go-") and engine not in ("go-v1.4.3", "go-v1.4.5", "go-worktree") for engine in engines):
        parser.error("feature benchmarks support Go v1.4.3, Go v1.4.5, or an explicit --go-source checkout")
    if arguments.go_source:
        arguments.go_source = arguments.go_source.resolve()
        if not arguments.features or "go-worktree" not in engines or not (arguments.go_source / "go.mod").is_file():
            parser.error("--go-source requires feature mode, the go-worktree engine, and a Go module checkout")
    elif "go-worktree" in engines:
        parser.error("go-worktree requires --go-source")
    if arguments.iterations < 1 or (arguments.cohort and any(cohort not in cohorts for cohort in arguments.cohort)):
        parser.error("select cohorts belonging to the chosen suite and positive iterations")
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
    suite = root / ("testdata/python-features.json" if arguments.features else "testdata/go-core.json")
    fixture_bytes = suite.read_bytes()
    fixture_sha256 = hashlib.sha256(fixture_bytes).hexdigest()
    environment = os.environ.copy()
    environment.update({
        "GOTOOLCHAIN": "go1.27.1", "GOMAXPROCS": "1", "CGO_ENABLED": "0",
        "GOAMD64": "v1", "GOFLAGS": "", "TZ": "UTC", "LC_ALL": "C.UTF-8",
        "GOGC": "100", "GOMEMLIMIT": "off", "GOEXPERIMENT": "", "GODEBUG": "",
        "RUSTFLAGS": "", "CARGO_ENCODED_RUSTFLAGS": "",
        "DATEPARSER_BENCHMARK_SUITE": str(suite),
        "DATEPARSER_BENCHMARK_ITERATIONS": str(arguments.iterations),
    })
    binaries = {}
    versions = {}
    if "rust-baseline" in engines:
        binaries["rust-baseline"] = arguments.rust_baseline
        versions["rust-baseline"] = "preserved executable; identified by binary_sha256"
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
            if engine.startswith("rust"):
                continue
            version = "v1.4.5" if engine == "go-worktree" else engine.removeprefix("go-")
            module_file = Path(temporary) / f"{engine}.mod"
            shutil.copyfile(reference_root / "go.mod", module_file)
            shutil.copyfile(reference_root / "go.sum", module_file.with_suffix(".sum"))
            subprocess.run(
                ["go", "mod", "edit", f"-modfile={module_file}", f"-require={GO_MODULE}@{version}"],
                cwd=reference_root, env=environment, check=True,
            )
            build_flags = []
            if engine == "go-worktree":
                go_references[engine] = source_reference(arguments.go_source)
                subprocess.run(
                    ["go", "mod", "edit", f"-modfile={module_file}", f"-replace={GO_MODULE}={arguments.go_source}"],
                    cwd=reference_root, env=environment, check=True,
                )
                identity = go_references[engine]
                build_flags = [f"-ldflags=-X=main.benchmarkSourceCommit={identity['commit']} -X=main.benchmarkSourceSHA256={identity['source_sha256']}"]
            else:
                go_references[engine] = published_reference(version, module_file, reference_root, environment)
                if version == "v1.4.5":
                    identity = go_references[engine]
                    build_flags = [f"-ldflags=-X=main.benchmarkReleaseCommit={identity['commit']} -X=main.benchmarkReleaseModuleSum={identity['module_sum']}"]
            subprocess.run(
                ["go", "mod", "tidy", f"-modfile={module_file}"],
                cwd=reference_root, env=environment, check=True,
            )
            binaries[engine] = build_root / engine
            subprocess.run(
                ["go", "build", f"-modfile={module_file}", "-mod=readonly", "-trimpath",
                 "-buildvcs=false", *build_flags, "-o", str(binaries[engine]), "."],
                cwd=reference_root, env=environment, check=True,
            )
            if engine == "go-worktree" and source_reference(arguments.go_source) != go_references[engine]:
                raise RuntimeError("Go source changed during the benchmark build")
    cpu_model = next(
        line.partition(":")[2].strip()
        for line in Path("/proc/cpuinfo").read_text().splitlines()
        if line.startswith("model name")
    )
    os.sched_setaffinity(0, {arguments.cpu})
    records = []
    identities = {}
    engine_identities = {}

    def launch(engine, cohort, passes):
        child_environment = environment | {
            "DATEPARSER_BENCHMARK_COHORT": cohort,
            "DATEPARSER_BENCHMARK_PASSES": str(passes),
        }
        test = "benchmark_features" if arguments.features else "benchmark_public_parse"
        command = (
            [str(binaries[engine]), "--exact", f"upstream_tests::{test}",
             "--ignored", "--nocapture", "--test-threads=1"]
            if engine.startswith("rust") else
            [str(binaries[engine]), "-benchmark", str(suite), "-cohort", cohort,
             "-passes", str(passes)]
        )
        if engine.startswith("go-"):
            if arguments.features:
                command += ["-benchmark-features", "-iterations", str(arguments.iterations)]
            if engine == "go-worktree":
                command += ["-benchmark-source", str(arguments.go_source)]
            else:
                command += ["-benchmark-version", engine.removeprefix("go-")]
        result = measure(command, child_environment, root, passes)
        metadata = result["metadata"]
        if engine.startswith("go-") and metadata.get("go_reference") != go_references[engine]:
            raise RuntimeError("Benchmark binary does not match its published Go module")
        identity = {key: metadata[key] for key in ("cohort", "cases", "fixture_sha256")}
        if arguments.features:
            identity["iterations"] = metadata["iterations"]
        else:
            identity["parsed"] = metadata["parsed"]
        if metadata["fixture_sha256"] != fixture_sha256 or metadata["cohort"] != cohort:
            raise RuntimeError("Benchmark used a different fixture or cohort")
        if identity != identities.setdefault(cohort, identity):
            raise RuntimeError("Benchmark cohort membership differs between engines")
        engine_identity = {
            key: metadata[key] for key in ("parsed", "matched_dates", "outcome_sha256", "python_matches", "panics")
            if key in metadata
        }
        if not (arguments.features and engine == "go-v1.4.3") and engine_identity != engine_identities.setdefault((engine, cohort), engine_identity):
            raise RuntimeError(f"{engine} results changed between launches for {cohort}")
        return result | {"engine": engine, "cohort": cohort}

    for cohort in arguments.cohort or cohorts:
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
        "suite": "features" if arguments.features else "parse",
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