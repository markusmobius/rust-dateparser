# RustDateParser

RustDateParser is a Rust implementation of the Python library
[scrapinghub/dateparser](https://github.com/scrapinghub/dateparser), built by
faithfully replicating its Go port,
[markusmobius/go-dateparser](https://github.com/markusmobius/go-dateparser).

It parses localized date and time strings, including absolute dates, relative
expressions and Unix timestamps. The implementation uses native Rust code and
embedded locale data, with no Go, Python, native RE2 library, or network service
required at runtime.

The published source implements localized `Parse` behavior. Search and
non-Gregorian calendar APIs are implemented in the unreleased development
checkout measured below, but are not yet included in the published source.
This is not yet a complete replacement for either upstream library.

## Usage

```rust
use chrono::TimeZone;
use rust_dateparser::{parse, parse_with_formats, Configuration, Timezone};

let configuration = Configuration {
    current_time: Some(Timezone::Utc.with_ymd_and_hms(2026, 9, 12, 12, 0, 0).unwrap()),
    ..Configuration::default()
};

let date = parse(&configuration, "20 fevrier 2012")?;
assert_eq!(date.time.format("%F").to_string(), "2012-02-20");
assert_eq!(date.locale, "fr");

let relative = parse(&configuration, "2 days ago")?;
assert_eq!(relative.time.format("%F").to_string(), "2026-09-10");

let formatted = parse_with_formats(&configuration, "2024-060", &["2006-002"])?;
assert_eq!(formatted.time.format("%F").to_string(), "2024-02-29");
assert!(formatted.locale.is_empty());
# Ok::<(), rust_dateparser::Error>(())
```

Explicit formats use **Go time layouts**, not strptime directives. They are tried
before locale detection. A successful untranslated format therefore has an
empty `locale`, as in Go.

`Date` contains `time: chrono::DateTime<Timezone>`, `period: Period`, and
`locale: String`. `Timezone` retains UTC, local, named fixed-offset, or IANA zone
identity. By default, absent date components come from `current_time`, which
defaults to the current UTC time. Set it explicitly for repeatable results.

## Configuration

`Configuration` supports explicit locales or languages/region, given order,
fallback languages, previous-locale reuse, strict parsing, required date parts,
skip tokens, surrounding-text handling, date-source/month/day preferences,
default timezones, time precision, and end-of-month preservation.

Use `date_order: Some(DateOrder::Dmy)` for a fixed component order. For a
locale-dependent order, set `date_order_for_locale` to a `DateOrderResolver`;
it takes precedence over the fixed order. Returning `None` retains that locale's
order while preserving Go's explicit-override semantics.

An independent `Parser` owns its previous-locale history. Configure its
`parser_types` and optional `detect_languages_function` before sharing it.
The detector receives normalized text and is bypassed when locales or languages
are explicitly supplied. Its type is `Arc<dyn Fn(&str) -> Vec<String> + Send + Sync>`.

```rust
use rust_dateparser::{Configuration, Parser, ParserType, PreferredDateSource};

let mut parser = Parser::new();
parser.parser_types = vec![ParserType::CustomFormat, ParserType::AbsoluteTime];
let configuration = Configuration {
    strict_parsing: true,
    preferred_date_source: PreferredDateSource::Past,
    ..Configuration::default()
};
let date = parser.parse(&configuration, "12 August 2021", &[])?;
assert_eq!(date.time.format("%F").to_string(), "2021-08-12");
assert!(parser.parse(&configuration, "August 2021", &[]).is_err());
# Ok::<(), rust_dateparser::Error>(())
```

This parser selection and strict/past configuration match Go-HtmlDate's default
dateparser integration.

The library starts no worker threads. Callers own concurrency. `Parser` and
`Configuration` are `Send + Sync`; parsing snapshots configuration and serializes
calls sharing one parser's locale history. Embedded locale data and compiled
regexes and dictionary matchers are shared. Independent parsers maintain
independent history.

## Coverage

| Surface | Status |
| --- | --- |
| `parse`, `parse_with_formats`, `Parser::parse` | Implemented; 2,967 public Go comparison cases |
| Timestamp, negative timestamp, relative, formatted, absolute, compact parsers | Implemented; 1,335 additional core cases |
| Locale data, normalization, tokenization, translation, applicability | 512 locales / 205 language codes; 8,952 Go comparison cases |
| `is_known_locale`, `pop_tz_offset`, `Timezone::load` | Implemented |
| `parse_absolute`, `parse_relative`, `parse_formatted`, `parse_no_spaces`, `parse_timestamp` | Core-stage helpers; these do not perform the complete locale dispatch |
| `Search`, `SearchWithLanguage`, n-grams, time spans, full-text language detection | Not yet ported |
| `ParseJalali`, `ParseHijri` / Umm al-Qura | Not yet ported |

Search-related configuration fields are retained for the later search port;
they do not enable search through `parse`.
The original regex definitions are executed with Rust's `regex` crate. The
re2go-generated exact-match functions and optional native/WASM backends are not
ported or required.

Compatibility follows Go, including its quirks: raw locale date-order metadata,
two-digit-year preferences, ignored weekday labels in explicit layouts, negative
timestamp fractions, compact-date strictness shortcuts, and relative-unit
conversion order. It does not replace those decisions with another date parser.

Remaining qualification boundaries include arbitrary Go layouts beyond the
fixture coverage, extreme year/arithmetic ranges, fixed offsets of 24 hours or
more (which Chrono rejects), timezone-database differences, and ambiguous inputs
containing multiple timezone aliases. Rust inputs are valid UTF-8 `&str`; Go's
arbitrary byte-string behavior is not provided. Exact matches on the recorded
corpus are not an exhaustive compatibility proof.

## Speed Comparison

The optimized RustDateParser development checkout versus published
**Go-DateParser v1.4.5**, measured on 2026-09-13 with Rust
1.98.1 and Go 1.27.1 on an AMD Ryzen AI 7 PRO 350, Linux x86_64/WSL2. Both use
portable optimized builds, CPU 2 and one parsing caller, without internal
parallelism. Go uses `GOMAXPROCS=1`, default garbage collection and its pure-Go
backend.

Both repositories use the same Go v1.4.5 samples. The three core cohorts contain
2,951 stateless public cases from the [core fixture](testdata/go-core.json), with
their original settings, formats and frozen reference times; 16 detector/history
cases are excluded. The six feature cohorts contain 7,676 Python-reference cases,
excluding six callbacks and ten Python exception inputs. Both implementations
validate the expected results, including rejections, before timing. Parsed counts
count inputs yielding at least one date, not individual matches or accuracy.

| Cohort | Inputs (Parsed) | Rust Warm Pass | Go v1.4.5 Warm Pass | Go/Rust Time |
| --- | --- | --- | --- | --- |
| Automatic locale detection | 226 (222) | 14.90 ms | 150.67 ms | 10.11x |
| Explicit locales/languages | 2,530 (2,388) | 44.27 ms | 1,017.53 ms | 22.98x |
| HtmlDate strict/past | 195 (167) | 27.58 ms | 264.97 ms | 9.61x |
| Automatic search | 3 (3) | 1.11 ms | 9.82 ms | 8.85x |
| Split search | 34 (29) | 0.87 ms | 23.05 ms | 26.47x |
| N-gram search | 39 (36) | 3.26 ms | 65.79 ms | 20.18x |
| Time-span search | 96 (96) | 2.19 ms | 21.88 ms | 9.97x |
| Jalali parsing | 1,311 (1,087) | 6.99 ms | 12.47 ms | 1.78x |
| Hijri parsing | 6,193 (5,994) | 8.40 ms | 44.65 ms | 5.32x |

Times are per complete corpus traversal, summarized as the median of six
per-process pass medians. Each process performs eight measured passes after a
validated first pass; feature passes repeat the corpus 16 times and are divided
by 16 here. Execution order is balanced and separate preflights are discarded.
Setup, fixture decoding, initial matcher/regex construction and output checks are
outside the warm timers. Automatic search has only three texts. Compare engines
within a row, not different cohorts. These regression-corpus results are not a
production throughput guarantee.

Go reaches the first validated full pass sooner for automatic detection and
HtmlDate; warmed gains do not imply startup gains. The
[raw report](https://github.com/markusmobius/go-dateparser/releases/download/v1.4.5/dateparser-nine-cohorts-2026-09-13.json)
retains all 108 Rust/Go v1.4.5 processes and 864 warm passes across nine cohorts,
plus the separate Go v1.4.3 comparison. It includes ranges, first-pass latency,
execution orders, per-suite binary hashes and module provenance. This is a
development-checkout measurement, not a newly published Rust runtime release.

The Rust runtime caches per-locale Aho-Corasick word matchers and default locale
orders, shares input preparation and checks locale applicability lazily.
Matchers are built once on first use, not generated at build time. Exact
dictionary priority, first-occurrence behavior and Unicode boundaries are retained.

Reproduce from the development checkout under Linux/WSL with Python 3.9+, Rust
and Go; the published core-only checkout does not yet contain the feature runner:

```sh
python3 tools/benchmark.py --runs 6 --passes 8 --cpu 2
python3 tools/benchmark.py --features --runs 6 --passes 8 --iterations 16 --cpu 2
```

[tools/benchmark.py](tools/benchmark.py) builds all selected runners, enforces single-core
execution and saves all samples, toolchain versions, fixture/binary hashes and
summary statistics under the ignored `target/benchmark/` directory. Use
`--cohort auto`, `--cohort explicit` or `--cohort htmldate` for one cohort and
`--output` to choose a report path. The default compares only Rust and the
published Go v1.4.5 module, without local module replacements or lockfile changes.
The recorded core fixture remains a separate baseline; upgrading the fixture
exporter is independent of selecting the published module for benchmarking.

## Verification

Rust **1.98.1** is pinned in [rust-toolchain.toml](rust-toolchain.toml).
Normal tests require no Go, Python or network after Cargo dependencies are fetched.

```sh
cargo fmt --all -- --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked --release
```

The default suite embeds [testdata/go-core.json](testdata/go-core.json), compares
exact integer seconds, nanoseconds, offsets, timezone identity, periods, locales,
errors and callback inputs, and tests shared-parser isolation. Language tests
also compare normalized text, split tokens, applicability and every translation
in order. Focused tests cover overlapping dictionary matches, first-occurrence
and priority rules, Unicode boundaries, cached locale ordering, detector inputs
and configuration errors with previous-locale reuse.
The default suite passes 30 library tests and both README doctests;
the live Go comparison and timing benchmark are opt-in tests.

To generate a fresh, development-only Go reference from this repository root:

```sh
GOTOOLCHAIN=go1.27.1 CGO_ENABLED=0 TZ=UTC \
  go -C tools/go-reference run -mod=readonly . -output ../../target/go-reference.json
cargo test --locked --lib live_go_parity -- --ignored --nocapture
go -C tools/go-reference vet -mod=readonly ./...
```

Omit `-output` to refresh the checked-in snapshot. The generator also reproduces
the locale/timezone data, but never writes or splits the combined project license.
It uses Go's structured data and regex syntax APIs, verifies the reference versions
and module checksum, and never derives expected dates from Rust.

All date references are frozen in the fixture. Go's compact-date validator also
consults the wall-clock year; the oracle records that year, and Rust fixture tests
use the recorded value. Timestamp results are compared in UTC to avoid host-local
zone differences; the timestamp runtime itself retains local-zone identity.

CI is configured for Linux and Windows/MSVC, with a separate live Go check on
Linux. The optimized runtime and v1.4.4 reference passed local validation on
2026-09-12 under Linux/WSL and native Windows x86_64/GNU. The Windows release run passed all 31
library tests, including the opt-in live Go comparison, and both doctests; only
the timing benchmark was excluded. Both platforms matched 4,302 exact parsing
cases and 8,952 language cases across all 512 locales. Linux also passed
formatting, Clippy, Go vet and the release build. Fresh Go generation reproduced
the new saved fixture byte-for-byte. Rebaselining changed version/source
provenance, not expected results, actual locale data, timezone rules or the
combined license. Go module verification also passed.

For a local Windows GNU toolchain, select the GNU **host toolchain**, not only a
cross-compilation target:

```powershell
$env:CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER = 'C:/msys64/ucrt64/bin/gcc.exe'
$env:PATH = 'C:/msys64/ucrt64/bin;' + $env:PATH
& "$env:USERPROFILE/.cargo/bin/cargo.exe" +1.98.1-x86_64-pc-windows-gnu test --locked --target-dir target/windows-gnu
```

## Provenance

| Reference | Pin |
| --- | --- |
| Go-DateParser | v1.4.4, source commit `577619dabf1814609ac9e3010b34e4dc6b213694` |
| Annotated Go tag object | `a2c7c90d123ea85fbb1cedc43f5122b096cee9ca` (not the source commit) |
| Go module checksum | `h1:79+zZ9o3OAo4x7BHlSLhq7u8BD7qBr7kbwb6ilHZVgg=` |
| Go oracle | Go 1.27.1, `golang.org/x/text v0.42.0` |
| Python source behind the Go port | dateparser 1.4.3, `9ce60b1958f1b285886bcfbb743f6419feacfc92` |

The versioned Go implementation is the immediate behavior reference. An
independent Python comparison has not yet been run for this Rust milestone.
Go intentionally differs from Python in several documented settings and results.

[data/locales.json](data/locales.json) records hashes of its Go source files and
retains merged locale dictionaries, ordered substitutions, shared regexes and
digit mappings. [src/timezone_data.rs](src/timezone_data.rs) contains 432 timezone
names and their matching rules. The fixture verifies the locale-data and combined
project-license hashes. The original upstream notice hashes are recorded
separately as provenance. Cargo and Go lockfiles retain their dependency checksums.

## License

BSD-3-Clause. The combined [LICENSE](LICENSE) credits Scrapinghub (2014) and
Markus Mobius (2026) for the Python-derived code/data and the Rust port.
Dependency notices remain under their respective licenses. Binary-distribution
notice packaging remains a later release task.