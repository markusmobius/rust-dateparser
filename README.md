# RustDateParser

RustDateParser parses localized date and time strings, including absolute dates,
relative expressions and Unix timestamps. It uses native Rust code and embedded
locale data, with no Go, Python, native RE2 library, or network service required
at runtime.

This development port follows **go-dateparser v1.4.3**. Localized `Parse` behavior
is implemented; the separate search and non-Gregorian calendar APIs are not yet
ported. It is not a complete replacement for every Go-DateParser public API.

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
regexes are shared. Independent parsers maintain independent history.

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

Measured on 2026-09-12 with Rust 1.98.1 and Go 1.27.1/go-dateparser v1.4.3,
on an AMD Ryzen AI 7 PRO 350 under Linux x86_64/WSL2. Both optimized binaries
were pinned to CPU 2 with one parsing caller; Go used `GOMAXPROCS=1`, default
garbage collection and its default pure-Go backend. Rust used the portable
release profile, without native-CPU flags or internal parallelism.

The benchmark reuses [testdata/go-core.json](testdata/go-core.json). Its 2,951
stateless public parsing cases include expected failures; the 16 detector/history
cases are excluded. Both implementations validate every selected result against
the Go fixture before timing. Dates, errors, periods, locales, offsets and
nanoseconds match; successful-parse counts below are not accuracy scores.

Each cohort has six fresh launches per engine, alternating which engine runs
first. Separate preflight launches are discarded. Each measured process validates
one full first pass, then times eight warmed passes in the same input order.
Settings and independent per-case parsers are prepared outside the timers and
reused. Warm timings include public parsing, result consumption and cleanup,
but exclude fixture loading, initial regex compilation and output validation.

| Cohort | Inputs (Parsed) | Rust Warm Pass | Go Warm Pass | Go/Rust Time |
| --- | --- | --- | --- | --- |
| Automatic locale detection | 226 (222) | 692.75 ms | 659.22 ms | 0.95x |
| Explicit locales/languages | 2,530 (2,388) | 108.18 ms | 844.99 ms | 7.81x |
| HtmlDate strict/past configuration | 195 (167) | 614.49 ms | 648.76 ms | 1.06x |

Values are medians of the six per-process pass medians. Their min/max ranges
were Rust/Go **666.57-710.80 / 621.86-713.35 ms** for automatic detection,
**104.86-136.98 / 610.87-991.46 ms** for explicit locales, and
**600.31-628.92 / 611.62-685.75 ms** for HtmlDate. Explicit locales show a clear
Rust advantage on this suite, with a wide Go spread. The roughly 5% differences
in the other rows have overlapping ranges and do not establish a robust winner.
The cohorts contain different inputs/settings, so compare engines within a row,
not rows against each other. This is a regression corpus, not a production sample
or a general claim that one language is faster.

Fresh-process latency is reported separately:

| Cohort | Rust Launch To Validated First Pass | Go Launch To Validated First Pass |
| --- | --- | --- |
| Automatic locale detection | 1,801.72 ms | 1,031.19 ms |
| Explicit locales/languages | 1,356.08 ms | 1,233.55 ms |
| HtmlDate strict/past configuration | 1,911.65 ms | 1,036.69 ms |

These medians include process/test-harness startup, fixture decoding, library
initialization, lazy regex compilation and the entire first validated pass.
They are not isolated library startup or cold-disk measurements. Go was quicker
to this first-pass endpoint in all three cohorts.

Reproduce from the repository root under Linux/WSL with Python 3.9+, Rust and Go:

```sh
python3 tools/benchmark.py --runs 6 --passes 8 --cpu 2
```

[tools/benchmark.py](tools/benchmark.py) builds both runners, enforces single-core
execution and saves all samples, toolchain versions, fixture/binary hashes and
summary statistics under the ignored `target/benchmark/` directory. Use
`--cohort auto`, `--cohort explicit` or `--cohort htmldate` for one cohort and
`--output` to choose a report path. The table used separate reports for each
cohort with identical binaries and fixture SHA-256
`3901d69226f0386b4672c947a1d925b38804f1504b0ed853b0ae24f0d841fd0a`.
No parsing implementation was changed for this comparison.

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
in order. The default suite passes 26 library tests and both README doctests;
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
Linux; hosted CI has not yet run for this component. Local validation on
2026-09-12 passed under Linux/WSL and native Windows x86_64/GNU. The Windows
release run passed all 27 library tests, including the opt-in live Go comparison,
and both doctests before the separate benchmark was added: 4,302 exact parsing
cases and 8,952 language cases across all
512 locales. Linux also passed formatting, Clippy and the release build.

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
| Go-DateParser | v1.4.3, source commit `ce55302a57663c33e2d7bb668a29686b4c036fde` |
| Annotated Go tag object | `ba44b53f4321ef9d2895a5d160104a2b82aeab2b` (not the source commit) |
| Go module checksum | `h1:FWb52fQDRTdHcRfU8R2hkTW+U4HA4m39ldonanH5x0E=` |
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