# RustDateParser

RustDateParser is a Rust implementation of the Python library
[scrapinghub/dateparser](https://github.com/scrapinghub/dateparser), built by
faithfully replicating its Go port,
[markusmobius/go-dateparser](https://github.com/markusmobius/go-dateparser).

It parses localized date and time strings, including absolute dates, relative
expressions and Unix timestamps. The implementation uses native Rust code and
embedded locale data, with no Go, Python, native RE2 library, or network service
required at runtime.

The public API includes localized parsing, split and n-gram search, time spans,
Jalali parsing and Hijri/Umm al-Qura parsing. RustDateParser **v1.4.5** mirrors
**Go-DateParser v1.4.5** and **Python dateparser v1.4.3**. Its release version
follows the Go reference, not the Python version. Calendar and search behavior
is independently checked against Python; compatibility limits and verification
coverage are documented below.

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

## Search and Calendars

`search` returns a detected language and `SearchResult` values containing the
matched `text` and parsed `date`. `search_with_language` accepts a known language
and returns just the matches. Both also have methods on `Parser`.

```rust
use chrono::TimeZone;
use rust_dateparser::{parse_hijri, parse_jalali, search, Configuration, Timezone};

let configuration = Configuration {
  current_time: Some(Timezone::Utc.with_ymd_and_hms(2025, 2, 15, 12, 0, 0).unwrap()),
  languages: vec!["en".into()],
  search_strategy: "ngram".into(),
  ..Configuration::default()
};

let (language, matches) = search(
  &configuration,
  "The first satellite was launched on 4 October 1957.",
)?;
assert_eq!(language, "en");
assert_eq!(matches[0].text, "4 October 1957");
assert_eq!(matches[0].date.time.format("%F").to_string(), "1957-10-04");

let jalali = parse_jalali(&configuration, "1/1/1403")?;
assert_eq!(jalali.time.format("%F").to_string(), "2024-03-20");
let hijri = parse_hijri(&configuration, "1/1/1444")?;
assert_eq!(hijri.time.format("%F").to_string(), "2022-07-30");
# Ok::<(), rust_dateparser::Error>(())
```

`search_strategy` defaults to `"split"`; `"ngram"` tries longer token sequences
first. Set `return_time_span` to include start/end matches for expressions such
as `"past week"`. `default_start_of_week` and `default_days_in_month` control
span boundaries. Search is heuristic; restrict languages when they are known.

Both calendar parsers default to MDY. Missing components come from the reference
date converted to that calendar; only an omitted day is clamped to the month's
last day. Explicit overflowing days retain upstream rollover behavior.
Umm al-Qura supports years 1343-1500 AH. Unsupported reference or result dates
return errors. Conversion tables and their exact bounds are embedded in
[data/calendar-conversions.json](data/calendar-conversions.json).

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
| `search`, `search_with_language`, n-grams, time spans, full-text language detection | Implemented |
| `parse_jalali`, `parse_hijri` / Umm al-Qura | Implemented with native conversion tables |

Search and calendars have 9,886 exact supplementary Go v1.4.5 comparison cases,
with no excluded cases or substituted inputs. Independent Python fixtures cover
7,504 calendar cases, 178 exact search cases and ten exception-safety inputs.
Search-related configuration applies to the search APIs, not `parse`.
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

The optimized RustDateParser implementation versus published
**Go-DateParser v1.4.5**, measured before publication on 2026-09-13 with Rust
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
dated measurement of the development build, not a claim about every later build.

The Rust runtime caches per-locale Aho-Corasick word matchers and default locale
orders, shares input preparation and checks locale applicability lazily.
Matchers are built once on first use, not generated at build time. Exact
dictionary priority, first-occurrence behavior and Unicode boundaries are retained.

Reproduce from the repository root under Linux/WSL with Python 3.9+, Rust and Go:

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
The current fixture exporter is pinned to Go v1.4.5. The archived core report
predates that provenance update; all 4,302 core case records and 8,952 language
case records remained unchanged during the rebaseline. The archived report and
its original hashes are retained without relabelling measurements.

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
and configuration errors with previous-locale reuse. The default suite also
includes the complete supplementary Go fixture and independent Python calendar,
search and overflow checks. Two live Go comparisons and two timing benchmarks
are opt-in; the checked-in Go feature fixture is not ignored or filtered.

To generate a fresh, development-only Go reference from this repository root:

```sh
GOTOOLCHAIN=go1.27.1 CGO_ENABLED=0 TZ=UTC \
  go -C tools/go-reference run -mod=readonly . -output ../../target/go-reference.json
GOTOOLCHAIN=go1.27.1 CGO_ENABLED=0 GOMAXPROCS=1 TZ=UTC \
  go -C tools/go-reference run -mod=readonly . -features-output ../../target/go-features-reference.json
cargo test --locked --lib live_go_ -- --ignored --nocapture
go -C tools/go-reference vet -mod=readonly ./...
```

Omit `-output` to refresh the checked-in core snapshot; use
`-features-output ../../testdata/go-features.json` for the feature snapshot.
The generator also reproduces locale/timezone data, calendar vocabulary and the
native conversion tables, but never writes or splits the combined project license.
It uses Go's structured data and regex syntax APIs, verifies the reference versions
and module checksum, and never derives expected dates from Rust.

All date references are frozen in the fixture. Go's compact-date validator also
consults the wall-clock year; the oracle records that year, and Rust fixture tests
use the recorded value. Timestamp results are compared in UTC to avoid host-local
zone differences; the timestamp runtime itself retains local-zone identity.

CI runs the full suite, formatting, Clippy and release builds on Linux and
Windows/MSVC. Linux also regenerates both Go fixtures, checks live parity and
rejects changes to the generated data, timezone rules and license notices.
Fixture generation preserves the combined license byte-for-byte.

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
| Go-DateParser | v1.4.5, source commit `e02a0cfd80decdd47412d773b4799a89af078409` |
| Annotated Go tag object | `78257ee87860f23232eced745827eff829cd1a12` (not the source commit) |
| Go module checksum | `h1:Y34+feJSV/d7QGMbLFlQSfdPDVdhQS5qVL8/sHJ9eKk=` |
| Go oracle | Go 1.27.1, `golang.org/x/text v0.42.0` |
| Python source behind the Go port | dateparser 1.4.3, `9ce60b1958f1b285886bcfbb743f6419feacfc92` |

Python dateparser 1.4.3 is the behavior authority; Go v1.4.5 is the immediate
porting reference. The independent [Python fixture](testdata/python-features.json)
records package versions and imported source hashes. Python exception inputs
are checked for safe handling, not fabricated successful dates. Go intentionally
differs from Python in several documented settings and result representations.

[data/locales.json](data/locales.json) records hashes of its Go source files and
retains merged locale dictionaries, ordered substitutions, shared regexes and
digit mappings. [src/timezone_data.rs](src/timezone_data.rs) contains 432 timezone
names and their matching rules. The fixture verifies the locale-data and combined
project-license hashes. The original upstream notice hashes are recorded
separately as provenance. Cargo and Go lockfiles retain their dependency checksums.

[data/calendars.json](data/calendars.json) contains Go-derived calendar vocabulary.
[data/calendar-conversions.json](data/calendar-conversions.json) is byte-identical
to the published Go tables generated with `convertdate` 2.4.1 and `hijridate`
2.6.0. Their source hashes and bounds are recorded with the data; neither package
is a runtime dependency. See [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

## License

BSD-3-Clause. The combined [LICENSE](LICENSE) credits Scrapinghub (2014) and
Markus Mobius (2026) for the Python-derived code/data and the Rust port.
Calendar data-source notices are retained in
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). Dependency notices remain under
their respective licenses. Binary-distribution notice packaging remains a later
release task.