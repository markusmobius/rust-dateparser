# RustDateParser

RustDateParser parses localized date and time strings, including absolute dates,
relative expressions and Unix timestamps. It uses native Rust code and embedded
locale data, with no Go, Python, native RE2 library, or network service required
at runtime.

This development port follows **go-dateparser v1.4.4**. Localized `Parse` behavior
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

Measured on 2026-09-12 with Rust 1.98.1 and Go 1.27.1 running published
go-dateparser v1.4.3 and v1.4.4, on an AMD Ryzen AI 7 PRO 350 under Linux
x86_64/WSL2. All three optimized binaries were pinned to CPU 2 with one parsing
caller; Go used `GOMAXPROCS=1`, default
garbage collection and its default pure-Go backend. Rust used the portable
release profile, without native-CPU flags or internal parallelism.

Known-word lookup uses a cached **Aho-Corasick multi-pattern matcher per locale**.
Each matcher is built once on first use and shared through `OnceLock`; it is not
build-time generated source or a re2go conversion. Matching preserves Go's
dictionary-order ties, first-occurrence behavior and Unicode boundary rules.
The runtime also caches default locale orderings, shares locale-independent
normalization and digit conversion, and checks applicability lazily in locale
priority order until parsing succeeds. Configuration validation stays eager,
and candidate coverage, callbacks and parsing results are unchanged.

Go v1.4.4 also includes lazy applicability, shared input preparation and cached
default locale orderings. It retains its existing dictionary scan: the tested
Go multi-pattern matchers did not justify their speed/memory tradeoffs. The
comparison below uses this improved, published Go baseline.

Both repositories use **one fixture, one runner and the same measured samples**
for their current performance tables. The benchmark reuses
[testdata/go-core.json](testdata/go-core.json). Its 2,951
stateless public parsing cases include expected failures; the 16 detector/history
cases are excluded. All three implementations validate every selected result against
the Go fixture before timing. Dates, errors, periods, locales, offsets and
nanoseconds match; successful-parse counts below are not accuracy scores. Each
case retains its original settings, formats and frozen reference time. The
cohorts have different inputs, but every engine gets the same cases within a row.

Each cohort has six fresh launches per engine, cycling through all six engine
orders so each engine runs first, second and third twice. Separate preflight
launches are discarded. Each measured process validates
one full first pass, then times eight warmed passes in the same input order.
Settings and independent per-case parsers are prepared outside the timers and
reused. Warm timings include public parsing, result consumption and cleanup,
but exclude fixture loading, initial regex/matcher construction and output
validation.

| Cohort | Inputs (Parsed) | Go v1.4.3 | Go v1.4.4 | Rust | Go Old/New | Go v1.4.4/Rust |
| --- | --- | --- | --- | --- | --- | --- |
| Automatic locale detection | 226 (222) | 717.43 ms | 186.44 ms | 26.14 ms | 3.85x | 7.13x |
| Explicit locales/languages | 2,530 (2,388) | 846.20 ms | 915.56 ms | 66.66 ms | 0.92x | 13.74x |
| HtmlDate strict/past configuration | 195 (167) | 732.02 ms | 263.22 ms | 48.73 ms | 2.78x | 5.40x |

Values are medians of the six per-process pass medians. Their min/max ranges
were Go v1.4.3/Go v1.4.4/Rust **693.30-806.51 / 159.18-219.91 / 24.38-33.94 ms**
for automatic detection, **723.10-1,147.53 / 723.33-1,133.65 / 65.25-98.13 ms**
for explicit locales, and **699.72-853.03 / 255.47-316.95 / 44.60-77.83 ms**
for HtmlDate. Go's explicit-locale median is higher in v1.4.4 in this run, but
the ranges overlap widely; these samples establish neither a reliable gain nor
a regression there. Rust's warmed ranges separate from both Go versions in all
three cohorts. This is a regression corpus, not a production sample or a general
claim that one language is faster.

Fresh-process latency is reported separately:

| Cohort | Go v1.4.3 First Pass | Go v1.4.4 First Pass | Rust First Pass |
| --- | --- | --- | --- |
| Automatic locale detection | 1,144.24 ms | 557.09 ms | 1,076.83 ms |
| Explicit locales/languages | 1,312.68 ms | 1,343.08 ms | 1,501.85 ms |
| HtmlDate strict/past configuration | 1,123.16 ms | 667.42 ms | 1,410.90 ms |

These medians include process/test-harness startup, fixture decoding, library
initialization, lazy regex/matcher construction and the entire first validated
pass. They are not isolated library startup or cold-disk measurements. Go v1.4.4 reaches
this endpoint sooner for automatic detection and HtmlDate; the explicit-locale
ranges overlap. Rust's warmed advantage does not imply a first-use advantage.

Reproduce from the repository root under Linux/WSL with Python 3.9+, Rust and Go:

```sh
python3 tools/benchmark.py --runs 6 --passes 8 --cpu 2
```

[tools/benchmark.py](tools/benchmark.py) builds all selected runners, enforces single-core
execution and saves all samples, toolchain versions, fixture/binary hashes and
summary statistics under the ignored `target/benchmark/` directory. Use
`--cohort auto`, `--cohort explicit` or `--cohort htmldate` for one cohort and
`--output` to choose a report path. For Go-only comparison, add
`--engine go-v1.4.3 --engine go-v1.4.4`; that mode needs no Rust toolchain.
Three-engine runs must be a multiple of six; two-engine runs must be even.
Temporary module files select the checksum-verified Go tags without changing
the oracle's dependency locks or the module cache. Historical Go versions are
benchmark-only; fixture generation stays pinned to v1.4.4.

The table used separate reports for each cohort with identical binaries and fixture SHA-256
`c07b2bb77d75a959b86364a5c2f1fbd6a24b9c6b3619b70099141ab75cb2f24f`.
The local reports are `shared-auto.json`, `shared-explicit.json` and
`shared-htmldate.json`. Their combined
[raw report](https://github.com/markusmobius/go-dateparser/releases/download/v1.4.4/shared-dateparser-2026-09-12.json)
retains all **54 processes and 432 warm passes**, engine orders, binary hashes
and Go module provenance. Its SHA-256 is
`3aa6e8f3439418229d9825570377127df191565a1c406a419713b9cf4be85592`.
Both Go versions match the fixture exactly, with no local module replacements.
The Go repository's earlier 762-input comparison is historical and is no longer
the source for its current table. Timed parsing loops are unchanged.

### Historical Rust Optimization

Before upgrading the Go oracle, a separate balanced run compared the original
published Rust implementation at commit `0ab168c` with the optimized runtime
against the original v1.4.3-derived fixture. It used six launches per version,
eight warm passes each and discarded preflights on the same core, validating
exact results before timing:

| Cohort | Original Rust Warm Pass | Optimized Rust Warm Pass | Original/Optimized Time |
| --- | --- | --- | --- |
| Automatic locale detection | 702.39 ms | 24.42 ms | 28.76x |
| Explicit locales/languages | 127.41 ms | 71.19 ms | 1.79x |
| HtmlDate strict/past configuration | 691.92 ms | 46.88 ms | 14.76x |

Per-process median ranges, original/optimized, were
**669.25-787.09 / 23.71-28.21 ms**, **111.66-147.21 / 65.50-82.82 ms**, and
**640.72-738.56 / 41.90-59.69 ms**, respectively. These samples isolate the Rust
changes from differences in Go timing between runs; do not combine their
timings with the current Go/Rust table. The preserved release executables and
all 36 processes/288 passes are recorded in `optimized-vs-0ab168c.json` under
the ignored benchmark directory. Earlier Go v1.4.3 comparisons remain in
`optimized-auto.json`, `optimized-explicit.json` and `optimized-htmldate.json`;
they are not the current Go baseline. The separate two-engine v1.4.4 runs are
retained as `go-v1.4.4-{auto,explicit,htmldate}.json`; current Go/Rust ratios come
only from the shared three-engine experiment above.

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