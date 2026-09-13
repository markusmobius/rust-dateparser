"""Qualify Dateutil integration corrections without changing historical fixtures."""

import argparse
from datetime import datetime, timedelta, timezone
import hashlib
import importlib.metadata
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import tempfile
from zoneinfo import ZoneInfo

from dateparser import DateDataParser


ROOT = Path(__file__).resolve().parents[2]


def source_reference(root, git):
    files = {}
    for directory, children, names in os.walk(root):
        children[:] = [child for child in children if not child.startswith(".")]
        for name in names:
            path = Path(directory) / name
            relative = path.relative_to(root).as_posix()
            if path.suffix == ".go" or relative in ("go.mod", "go.sum", "internal/parser/calendars/data.json"):
                files[relative] = hashlib.sha256(path.read_bytes().replace(b"\r\n", b"\n")).hexdigest()
    encoded = json.dumps(files, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()
    return {"commit": subprocess.check_output([git, "-C", str(root), "rev-parse", "HEAD"], text=True).strip(),
            "source_sha256": hashlib.sha256(encoded).hexdigest(), "files": files}


def python_result(case, text=None):
    config = case["configuration"]
    base = datetime.fromisoformat(config["current_time"])
    if config.get("current_timezone"):
        base = base.astimezone(ZoneInfo(config["current_timezone"]))
    settings = {"RELATIVE_BASE": base, "PREFER_DATES_FROM": ["current_period", "past", "future"][config.get("preferred_date_source", 0)],
                "STRICT_PARSING": config.get("strict_parsing", False),
                "PREFER_DAY_OF_MONTH": ["current", "first", "last"][config.get("preferred_day_of_month", 0)],
                "PREFER_MONTH_OF_YEAR": ["current", "first", "last"][config.get("preferred_month_of_year", 0)]}
    if config.get("date_order"):
        settings.update(DATE_ORDER=config["date_order"], PREFER_LOCALE_DATE_ORDER=False)
    if config.get("default_timezone"):
        settings["TIMEZONE"] = config["default_timezone"]
    if config.get("required_parts"):
        settings["REQUIRE_PARTS"] = config["required_parts"]
    kinds = case.get("parser_types", [])
    if kinds:
        settings["PARSERS"] = [["timestamp", "negative-timestamp", "relative-time", "custom-formats", "absolute-time", "no-spaces-time"][kind] for kind in kinds]
    if case["stage"] in ("absolute", "relative"):
        settings["PARSERS"] = ["absolute-time" if case["stage"] == "absolute" else "relative-time"]
    languages = config.get("languages") or (["en"] if case["stage"] in ("absolute", "relative") else None)
    parser = DateDataParser(languages=languages, locales=config.get("locales") or None, settings=settings)
    try:
        value = parser.get_date_data(case["input"] if text is None else text)
        return value.date_obj
    except (ValueError, OverflowError):
        return None


def verify_date(case, actual, text=None):
    expected = python_result(case, text)
    if expected is None:
        if actual["parsed"]:
            raise RuntimeError(f"{case['id']} {text or case['input']!r}: Python rejects, Go accepts {actual}")
        return {"accepted": False}
    if not actual["parsed"]:
        raise RuntimeError(f"{case['id']} {text or case['input']!r}: Python accepts {expected}, Go rejects")
    offset = actual.get("offset", 0)
    wall = datetime(1970, 1, 1, tzinfo=timezone.utc) + timedelta(seconds=actual["unix_seconds"] + offset, microseconds=actual.get("nanosecond", 0) // 1000)
    if wall.replace(tzinfo=None) != expected.replace(tzinfo=None):
        raise RuntimeError(f"{case['id']} {text or case['input']!r}: Go wall={wall}, Python={expected}")
    if expected.utcoffset() is not None and offset != int(expected.utcoffset().total_seconds()):
        raise RuntimeError(f"{case['id']}: timezone offset differs from Python")
    return {"accepted": True, "datetime": expected.isoformat()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--go-source", type=Path, required=True)
    parser.add_argument("--go", default="go")
    parser.add_argument("--git", default="git")
    checks = parser.add_mutually_exclusive_group()
    checks.add_argument("--check", action="store_true")
    checks.add_argument("--check-current", action="store_true", help="recheck behavior with published Dateutil v2.9.1 without rewriting historical provenance")
    arguments = parser.parse_args()
    if platform.python_version() != "3.14.6":
        raise RuntimeError("expected CPython3.14.6")
    for package, version in [("dateparser", "1.4.3"), ("python-dateutil", "2.9.0.post0")]:
        if importlib.metadata.version(package) != version:
            raise RuntimeError(f"expected {package}=={version}")
    source = arguments.go_source.resolve()
    provenance = source_reference(source, arguments.git)
    environment = os.environ | {"GOTOOLCHAIN": "go1.27.1", "GOWORK": "off", "GOMAXPROCS": "1", "CGO_ENABLED": "0", "TZ": "UTC"}
    reference = ROOT / "tools/go-reference"
    with tempfile.TemporaryDirectory(prefix="dateutil-corrections-") as temporary:
        module = Path(temporary) / "reference.mod"
        shutil.copyfile(reference / "go.mod", module)
        shutil.copyfile(reference / "go.sum", module.with_suffix(".sum"))
        output = Path(temporary) / "corrections.json"
        subprocess.run([arguments.go, "mod", "edit", f"-modfile={module}", f"-replace=github.com/markusmobius/go-dateparser={source.as_posix()}"], cwd=reference, env=environment, check=True)
        subprocess.run([arguments.go, "mod", "tidy", f"-modfile={module}"], cwd=reference, env=environment, check=True)
        flags = f"-X=main.benchmarkSourceCommit={provenance['commit']} -X=main.benchmarkSourceSHA256={provenance['source_sha256']}"
        if arguments.check_current:
            flags += " -X=main.correctionsDateutilVersion=v2.9.1"
        subprocess.run([arguments.go, "run", f"-modfile={module}", "-mod=readonly", f"-ldflags={flags}", ".", "-corrections-source", str(source), "-output", str(output)], cwd=reference, env=environment, check=True)
        corrections = json.loads(output.read_text(encoding="utf-8"))
    if provenance != source_reference(source, arguments.git):
        raise RuntimeError("Go source changed during correction generation")
    core = {case["id"]: case for case in json.loads((ROOT / "testdata/go-core.json").read_text(encoding="utf-8"))["cases"]}
    features = {case["id"]: case for case in json.loads((ROOT / "testdata/go-features.json").read_text(encoding="utf-8"))["cases"]}
    evidence = {}
    for identity, result in corrections["core"].items():
        evidence[identity] = verify_date(core[identity], result)
    for identity, matches in corrections["features"].items():
        original = features[identity]
        if len(matches) != len(original["matches"]):
            raise RuntimeError(f"{identity}: unexpected search membership change")
        checked = []
        for actual, previous in zip(matches, original["matches"]):
            if actual["text"] != previous["text"]:
                raise RuntimeError(f"{identity}: search match text changed")
            if actual["date"] != previous["date"]:
                checked.append(verify_date(original, actual["date"], actual["text"]))
        evidence[identity] = checked
    corrections["python"] = {"version": platform.python_version(), "dateparser": "1.4.3", "dateutil": "2.9.0.post0", "evidence": evidence}
    corrections["source_files"] = provenance["files"]
    corrections["source_line_endings"] = "LF"
    encoded = (json.dumps(corrections, ensure_ascii=True, indent=2) + "\n").encode()
    destination = ROOT / "testdata/dateutil-corrections.json"
    if arguments.check_current:
        historical = json.loads(destination.read_text(encoding="utf-8"))
        for field in ("core_sha256", "features_sha256", "core", "features", "python"):
            if corrections[field] != historical[field]:
                raise RuntimeError(f"Current Dateutil behavior changed: {field}")
        print(f"Current Go source {provenance['commit']}, SHA-256 {provenance['source_sha256']}; published Dateutil v2.9.1 verified")
        print(f"Historical correction provenance remains {historical['reference']['commit']}")
    elif arguments.check:
        if destination.read_bytes() != encoded:
            raise RuntimeError("Dateutil corrections changed")
    else:
        destination.write_bytes(encoded)
    print(f"Python verified {len(corrections['core'])} core and {len(corrections['features'])} feature corrections")


if __name__ == "__main__":
    main()