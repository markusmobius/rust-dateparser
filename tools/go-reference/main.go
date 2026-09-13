package main

import (
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"flag"
	"fmt"
	"os"
	"path/filepath"
	"runtime/debug"
	"strings"
	"time"

	"github.com/markusmobius/go-dateparser/date"
	"github.com/markusmobius/go-dateparser/internal/parser/absolute"
	"github.com/markusmobius/go-dateparser/internal/parser/formatted"
	"github.com/markusmobius/go-dateparser/internal/parser/nospace"
	"github.com/markusmobius/go-dateparser/internal/parser/relative"
	"github.com/markusmobius/go-dateparser/internal/parser/timestamp"
	"github.com/markusmobius/go-dateparser/internal/setting"
	"github.com/markusmobius/go-dateparser/internal/timezone"
)

type configuration struct {
	Locales               []string          `json:"locales,omitempty"`
	Languages             []string          `json:"languages,omitempty"`
	Region                string            `json:"region,omitempty"`
	UseGivenOrder         bool              `json:"use_given_order,omitempty"`
	TryPreviousLocales    bool              `json:"try_previous_locales,omitempty"`
	DefaultLanguages      []string          `json:"default_languages,omitempty"`
	SkipTokens            []string          `json:"skip_tokens,omitempty"`
	IgnoreSurroundingText bool              `json:"ignore_surrounding_text,omitempty"`
	DateOrderForLocales   map[string]string `json:"date_order_for_locales,omitempty"`
	CurrentTime           string            `json:"current_time"`
	CurrentTimezone       string            `json:"current_timezone,omitempty"`
	DefaultTimezone       string            `json:"default_timezone,omitempty"`
	DateOrder             string            `json:"date_order,omitempty"`
	DateOrderIsExplicit   bool              `json:"date_order_is_explicit,omitempty"`
	PreferredDayOfMonth   uint8             `json:"preferred_day_of_month,omitempty"`
	PreferredMonthOfYear  uint8             `json:"preferred_month_of_year,omitempty"`
	PreferredDateSource   uint8             `json:"preferred_date_source,omitempty"`
	StrictParsing         bool              `json:"strict_parsing,omitempty"`
	RequiredParts         []string          `json:"required_parts,omitempty"`
	ReturnTimeAsPeriod    bool              `json:"return_time_as_period,omitempty"`
	PreserveEndOfMonth    bool              `json:"preserve_end_of_month,omitempty"`
	SearchStrategy        string            `json:"search_strategy,omitempty"`
	ReturnTimeSpan        bool              `json:"return_time_span,omitempty"`
	DefaultStartOfWeek    string            `json:"default_start_of_week,omitempty"`
	DefaultDaysInMonth    int               `json:"default_days_in_month,omitempty"`
}

func (cfg configuration) internal() *setting.Configuration {
	current, err := time.Parse(time.RFC3339Nano, cfg.CurrentTime)
	if err != nil {
		panic(err)
	}
	if cfg.CurrentTimezone != "" {
		location, err := time.LoadLocation(cfg.CurrentTimezone)
		if err != nil {
			panic(err)
		}
		current = current.In(location)
	}
	var defaultTimezone *time.Location
	if cfg.DefaultTimezone != "" {
		defaultTimezone, err = time.LoadLocation(cfg.DefaultTimezone)
		if err != nil {
			panic(err)
		}
	}
	order := cfg.DateOrder
	if order == "" {
		order = "MDY"
	}
	return &setting.Configuration{
		CurrentTime:          current,
		DefaultTimezone:      defaultTimezone,
		DateOrder:            order,
		DateOrderIsExplicit:  cfg.DateOrderIsExplicit,
		PreferredDayOfMonth:  setting.PreferredDayOfMonth(cfg.PreferredDayOfMonth),
		PreferredMonthOfYear: setting.PreferredMonthOfYear(cfg.PreferredMonthOfYear),
		PreferredDateSource:  setting.PreferredDateSource(cfg.PreferredDateSource),
		StrictParsing:        cfg.StrictParsing,
		RequiredParts:        cfg.RequiredParts,
		ReturnTimeAsPeriod:   cfg.ReturnTimeAsPeriod,
		PreserveEndOfMonth:   cfg.PreserveEndOfMonth,
	}
}

type result struct {
	Locale      string `json:"locale,omitempty"`
	Error       string `json:"error,omitempty"`
	Parsed      bool   `json:"parsed"`
	UnixSeconds int64  `json:"unix_seconds,omitempty"`
	Nanosecond  int    `json:"nanosecond,omitempty"`
	Offset      int    `json:"offset,omitempty"`
	Period      string `json:"period,omitempty"`
	Timezone    string `json:"timezone,omitempty"`
}

type testCase struct {
	ID                string        `json:"id"`
	Stage             string        `json:"stage"`
	Input             string        `json:"input"`
	Configuration     configuration `json:"configuration"`
	Negative          bool          `json:"negative,omitempty"`
	Formats           []string      `json:"formats,omitempty"`
	ParserTypes       []int         `json:"parser_types,omitempty"`
	DetectorLanguages *[]string     `json:"detector_languages,omitempty"`
	DetectionInputs   []string      `json:"detection_inputs,omitempty"`
	Session           string        `json:"session,omitempty"`
	Expected          result        `json:"expected"`
}

type reference struct {
	Module               string `json:"module"`
	Version              string `json:"version"`
	Commit               string `json:"commit"`
	ModuleSum            string `json:"module_sum"`
	SourceSHA256         string `json:"source_sha256,omitempty"`
	GoVersion            string `json:"go_version"`
	TextVersion          string `json:"text_version"`
	TimezoneSourceSHA256 string `json:"timezone_source_sha256"`
	LicenseSHA256        string `json:"license_sha256"`
	PythonLicenseSHA256  string `json:"python_license_sha256"`
	ProjectLicenseSHA256 string `json:"project_license_sha256"`
	LocaleDataSHA256     string `json:"locale_data_sha256"`
	LanguageTestsSHA256  string `json:"language_tests_sha256"`
	ParserTestsSHA256    string `json:"parser_tests_sha256"`
	WallYear             int    `json:"wall_year"`
}

type fixture struct {
	Reference     reference      `json:"reference"`
	Cases         []testCase     `json:"cases"`
	LanguageCases []languageCase `json:"language_cases"`
}

func pinnedReference(version string) reference {
	var commit, moduleSum string
	switch version {
	case "v1.4.3":
		commit = "ce55302a57663c33e2d7bb668a29686b4c036fde"
		moduleSum = "h1:FWb52fQDRTdHcRfU8R2hkTW+U4HA4m39ldonanH5x0E="
	case "v1.4.4":
		commit = "577619dabf1814609ac9e3010b34e4dc6b213694"
		moduleSum = "h1:79+zZ9o3OAo4x7BHlSLhq7u8BD7qBr7kbwb6ilHZVgg="
	case "v1.4.5":
		commit = "e02a0cfd80decdd47412d773b4799a89af078409"
		moduleSum = "h1:Y34+feJSV/d7QGMbLFlQSfdPDVdhQS5qVL8/sHJ9eKk="
	default:
		panic("unsupported Go-DateParser reference version")
	}
	return reference{Module: "github.com/markusmobius/go-dateparser", Version: version, Commit: commit, ModuleSum: moduleSum}
}

var benchmarkReleaseCommit, benchmarkReleaseModuleSum string

func benchmarkReleaseReference() reference {
	commit, commitErr := hex.DecodeString(benchmarkReleaseCommit)
	checksum, checksumErr := base64.StdEncoding.DecodeString(strings.TrimPrefix(benchmarkReleaseModuleSum, "h1:"))
	if commitErr != nil || len(commit) != 20 || checksumErr != nil || len(checksum) != 32 || !strings.HasPrefix(benchmarkReleaseModuleSum, "h1:") {
		panic("release benchmark provenance must be verified and embedded by tools/benchmark.py")
	}
	return reference{
		Module: "github.com/markusmobius/go-dateparser", Version: "v1.4.5",
		Commit: benchmarkReleaseCommit, ModuleSum: benchmarkReleaseModuleSum,
	}
}

func main() {
	output := flag.String("output", "../../testdata/go-core.json", "Go-derived fixture destination")
	featuresOutput := flag.String("features-output", "", "export supplementary search and calendar fixtures without changing parsing data")
	benchmark := flag.String("benchmark", "", "benchmark an existing Go fixture without regenerating it")
	benchmarkVersion := flag.String("benchmark-version", "", "benchmark-only Go version: v1.4.3, v1.4.4, or verified v1.4.5")
	benchmarkFeatures := flag.Bool("benchmark-features", false, "benchmark the Python-qualified search and calendar fixture")
	benchmarkSource := flag.String("benchmark-source", "", "benchmark-only local Go source matching a temporary module replacement")
	benchmarkCorrections := flag.String("benchmark-corrections", "", "Python-qualified expectations for a core worktree benchmark")
	iterations := flag.Int("iterations", 16, "feature corpus repetitions per measured pass")
	cohort := flag.String("cohort", "auto", "benchmark cohort: auto, explicit, or htmldate")
	passes := flag.Int("passes", 8, "measured benchmark passes after the first validated pass")
	correctionsSource := flag.String("corrections-source", "", "export reviewed worktree corrections without rewriting historical fixtures")
	flag.Parse()
	if *correctionsSource != "" {
		if *benchmark != "" || *featuresOutput != "" || *benchmarkVersion != "" || *benchmarkSource != "" || *benchmarkCorrections != "" {
			panic("corrections-source cannot be combined with other export modes")
		}
		exportCorrections(*correctionsSource, *output)
		return
	}
	if *benchmarkFeatures && *benchmark == "" {
		panic("benchmark-features requires benchmark mode")
	}
	if *benchmarkCorrections != "" && (*benchmark == "" || *benchmarkSource == "" || *benchmarkFeatures) {
		panic("benchmark-corrections requires a core worktree benchmark")
	}
	if *benchmarkSource != "" {
		if *benchmark == "" || *benchmarkVersion != "" || *featuresOutput != "" {
			panic("benchmark-source is only allowed for benchmarks")
		}
		if !*benchmarkFeatures && *benchmarkCorrections == "" {
			panic("core worktree benchmarks require Python-qualified corrections")
		}
		absolute, err := filepath.Abs(*benchmarkSource)
		if err != nil {
			panic(err)
		}
		*benchmarkSource = absolute
	}
	info, ok := debug.ReadBuildInfo()
	if !ok {
		panic("Go build provenance is unavailable")
	}
	provenance := pinnedReference("v1.4.5")
	if *benchmarkVersion != "" {
		if *benchmark == "" {
			panic("benchmark-version requires benchmark mode")
		}
		if *benchmarkVersion == "v1.4.5" {
			provenance = benchmarkReleaseReference()
		} else {
			provenance = pinnedReference(*benchmarkVersion)
		}
	}
	expectedModuleSum := provenance.ModuleSum
	provenance.ModuleSum = ""
	provenance.GoVersion = info.GoVersion
	for _, dependency := range info.Deps {
		if dependency.Path == provenance.Module {
			if *benchmarkSource != "" {
				if dependency.Version != provenance.Version || dependency.Replace == nil || filepath.Clean(dependency.Replace.Path) != *benchmarkSource {
					panic("benchmark source does not match its temporary module replacement")
				}
				provenance.ModuleSum = expectedModuleSum
			} else if dependency.Version != provenance.Version || dependency.Sum != expectedModuleSum || dependency.Replace != nil {
				panic("unexpected Go-DateParser reference dependency")
			} else {
				provenance.ModuleSum = dependency.Sum
			}
		}
		if dependency.Path == "golang.org/x/text" {
			if dependency.Version != "v0.42.0" || dependency.Replace != nil {
				panic("unexpected x/text reference dependency")
			}
			provenance.TextVersion = dependency.Version
		}
	}
	if provenance.ModuleSum == "" || provenance.TextVersion == "" || info.GoVersion != "go1.27.1" {
		panic("reference versions could not be verified")
	}
	provenance.WallYear = time.Now().Year()
	if *benchmark != "" {
		if *benchmarkSource != "" {
			provenance = benchmarkSourceReference(provenance)
		}
		if *benchmarkFeatures {
			runFeatureBenchmark(*benchmark, *cohort, *passes, *iterations, provenance)
		} else {
			runBenchmark(*benchmark, *cohort, *passes, provenance, *benchmarkCorrections)
		}
		return
	}
	if *featuresOutput != "" {
		exportFeatures(*featuresOutput, provenance)
		return
	}
	translations, public := exportData(&provenance)
	data := fixture{Reference: provenance, LanguageCases: translations}
	base := configuration{CurrentTime: "2026-09-12T12:30:45.987654321Z"}
	add := func(stage, input string, cfg configuration, negative bool, formats ...string) {
		var parsed date.Date
		switch stage {
		case "timestamp":
			parsed = timestamp.Parse(cfg.internal(), input, negative)
			parsed.Time = parsed.Time.UTC()
		case "absolute":
			parsed, _ = absolute.Parse(cfg.internal(), input, timezone.OffsetData{})
		case "formatted":
			parsed = formatted.Parse(cfg.internal(), input, formats...)
		case "nospace":
			internal := cfg.internal()
			internal.DateOrder = cfg.DateOrder
			parsed, _ = nospace.Parse(internal, input)
		case "relative":
			parsed = relative.Parse(cfg.internal(), input)
		default:
			panic("unknown reference stage")
		}
		expected := result{Parsed: !parsed.IsZero()}
		if expected.Parsed {
			_, expected.Offset = parsed.Time.Zone()
			expected.UnixSeconds = parsed.Time.Unix()
			expected.Nanosecond = parsed.Time.Nanosecond()
			expected.Period = parsed.Period.String()
			expected.Timezone = parsed.Time.Location().String()
		}
		data.Cases = append(data.Cases, testCase{
			ID:    fmt.Sprintf("%s-%04d", stage, len(data.Cases)+1),
			Stage: stage, Input: input, Configuration: cfg, Negative: negative, Formats: formats, Expected: expected,
		})
	}
	for _, input := range []string{
		"1570308760", "1570308760263", "1570308760263111", "0000000000", "9999999999999999",
		"-1570308760", "-1570308760263", "-1570308760263111", "+1570308760", " 1570308760",
		"15703087602631", "157030876026xx", "1570308760263x", "157030876026311",
		"15703087602631x", "15703087602631xx", "15703087602631111", "1570308760263111x",
		"1570308760263111xx", "1570308760263111222", "1570308760.123456789", "1570308760 text",
		"1570308760\ttext", "1570308760\ntext", "1570308760\rtext", "1570308760\ftext", "1570308760\vtext",
		"1570308760\u00a0", "\u0661570308760", "", "9", "999999999999999999999999999999",
	} {
		for _, negative := range []bool{false, true} {
			for _, precision := range []bool{false, true} {
				cfg := base
				cfg.ReturnTimeAsPeriod = precision
				cfg.DefaultTimezone = "Asia/Kathmandu"
				add("timestamp", input, cfg, negative)
			}
		}
	}
	inputs := []string{
		"", " ", "invalid date string", "!", ":", "12 August 2021", "2021-10-11", "2021-10", "2021",
		"February 2000", "23 January, 15:10:01", "13.20", "11:30", "0 AM", "12 AM", "12 PM", "16:50 pm",
		"13:30:00 PM", "24:00", "10:60", "10:30:60", "2018-04-12 17:20:03.12345678999",
		"2018-04-12t17:20:03", "2018-04-12T17:20:03", "2018-04-12 17:20:03,1234", "2021.10.11",
		"12/04/2018", "04/12/2018", "31/12/2024", "01/02/03", "12/11/10", "03 2019 25", "2014-02-01",
		"29 February 2015", "32 January 2015", "31 April 2015", "31 June 2015", "31 September 2015",
		"29 February", "February 29", "February 30", "February 31", "February 32", "January 0", "00",
		"0000", "0001", "0002", "9999", "10000", "12/09/18567", "Aug 7, 2014Aug 7, 2014",
		"april 2010", "11 March", "March", "31 2010", "31/2010", "1", "12", "13", "31", "32", "69", "99",
		"sunday", "monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "Sun Mon",
		"Tuesday 11 april 2010", "January 15, 64", "January 15, 24", "January 15, 35", "January 15, 2050",
		"2/29/00", "1/1/26", "12:30:45.123456", "12:30:45.1", "12:30:45.000000001", "12:30:45. pm",
		"2018-04-12 12:30:00 13:00:00", "10.20.30", "May 31 2024", "2024-02-29", "2023-02-29",
	}
	for _, input := range inputs {
		add("absolute", input, base, false)
	}
	for _, order := range []string{"YMD", "YDM", "MDY", "MYD", "DMY", "DYM"} {
		for _, input := range []string{"01/02/03", "12/11/10", "2018-04-12", "2021-10", "03 2019 25", "31/12/2024", "69-01-02", "12 August 2021", "31 2010", "04-99", "02 2023", "2014-02-01"} {
			cfg := base
			cfg.DateOrder, cfg.DateOrderIsExplicit = order, true
			add("absolute", input, cfg, false)
		}
	}
	for _, current := range []string{"2026-01-01T00:00:00Z", "1980-01-31T23:59:59Z", "2023-02-28T12:00:00Z", "2024-02-29T12:00:00Z"} {
		for _, source := range []uint8{0, 1, 2} {
			for _, input := range []string{"March", "January 15, 64", "January 15, 24", "2/29/00", "February 29", "February 2000", "sunday", "monday", "10:00", "15:00"} {
				cfg := base
				cfg.CurrentTime, cfg.PreferredDateSource = current, source
				add("absolute", input, cfg, false)
			}
		}
	}
	for _, day := range []uint8{0, 1, 2} {
		for _, month := range []uint8{0, 1, 2} {
			for _, input := range []string{"2021", "2024-02", "February 2023", "February 2024", "March", "31 2024"} {
				cfg := base
				cfg.CurrentTime = "2024-01-31T12:30:00Z"
				cfg.PreferredDayOfMonth, cfg.PreferredMonthOfYear = day, month
				add("absolute", input, cfg, false)
			}
		}
	}
	for _, required := range [][]string{nil, {"day"}, {"month"}, {"year"}, {"day", "month", "year"}, {"YEAR"}} {
		for _, strict := range []bool{false, true} {
			for _, input := range []string{"March", "March 2024", "11 March", "2024", "11 March 2024", "11:30"} {
				cfg := base
				cfg.RequiredParts, cfg.StrictParsing = required, strict
				add("absolute", input, cfg, false)
			}
		}
	}
	for _, zone := range []string{"America/New_York", "Europe/Berlin", "Asia/Kathmandu"} {
		for _, input := range []string{"2024-03-10 02:30", "2024-11-03 01:30", "2024-03-31 02:30", "2024-10-27 02:30", "11:30", "2024-02-29"} {
			cfg := base
			cfg.CurrentTime, cfg.CurrentTimezone = "2024-03-09T17:30:45Z", zone
			cfg.ReturnTimeAsPeriod = true
			add("absolute", input, cfg, false)
		}
	}
	for _, input := range []string{"11:30", "3 PM", "12:30:45.123456789", "2018-04-12 17:20:03.12345678999"} {
		cfg := base
		cfg.ReturnTimeAsPeriod = true
		add("absolute", input, cfg, false)
	}
	formattedInputs := [][2]string{
		{"2023-100", "2006-002"}, {"2024-060", "2006-002"}, {"2023 060", "2006 002"}, {"2024  60", "2006 __2"},
		{"1999001", "2006002"}, {"1999050", "2006002"}, {"1999032", "2006002"}, {"1999059", "2006002"},
		{"1999060", "2006002"}, {"1999365", "2006002"}, {"2000060", "2006002"}, {"2000366", "2006002"},
		{"1999366", "2006002"}, {"2023-366", "2006-002"}, {"1900366", "2006002"}, {"2100 366", "2006 002"},
		{"1999000", "2006002"}, {"1999367", "2006002"}, {"1999777", "2006002"},
		{"25-03-14", "02-01-06"}, {"09.16", "01.02"}, {"August 2014", "January 2006"}, {"2014", "2006"},
		{"2024-02-29", "2006-01-02"}, {"2023-02-29", "2006-01-02"}, {"2024-2-9", "2006-01-02"},
		{"2024-2-9", "2006-1-2"}, {"2024-02-29 1:2:3", "2006-01-02 15:4:5"},
		{"2024-02-29 12:34:56.123456789", "2006-01-02 15:04:05.999999999"},
		{"2024-02-29 12:34:56.123456789999", "2006-01-02 15:04:05"},
		{"2024-02-29 12:34:56.120", "2006-01-02 15:04:05.000"},
		{"2024-02-29 12:34:56.12", "2006-01-02 15:04:05.000"},
		{"3:04PM", "3:04PM"}, {"12:04AM", "3:04PM"}, {"0 AM", "3 PM"}, {"15:04 PM", "15:04 PM"},
		{"Thu, 29 Feb 2024", "Mon, 02 Jan 2006"}, {"Fri, 29 Feb 2024", "Mon, 02 Jan 2006"},
		{"Feb 29 2024", "January 2 2006"}, {"February 29 2024", "January 2 2006"},
		{"2024-02-29T12:34:56Z", "2006-01-02T15:04:05Z07:00"},
		{"2024-02-29T12:34:56+05:30", "2006-01-02T15:04:05Z07:00"},
		{"2024-02-29 12:34:56 -0400", "2006-01-02 15:04:05 -0700"},
		{"2024-02-29 12:34:56 UTC", "2006-01-02 15:04:05 MST"},
		{"2024-02-29 12:34:56 EST", "2006-01-02 15:04:05 MST"},
		{"2024-02-29 12:34:56 XYZ", "2006-01-02 15:04:05 MST"},
		{"2024-02-29 12:34:60", "2006-01-02 15:04:05"},
		{"0000-02-29", "2006-01-02"}, {"0001-01-01", "2006-01-02"},
		{"literal", "literal"}, {"yesterday", "2006-01-02"},
	}
	for _, pair := range formattedInputs {
		add("formatted", pair[0], base, false, pair[1])
	}
	for _, current := range []string{"2026-01-01T00:00:00Z", "1980-01-01T00:00:00Z"} {
		for _, preference := range []uint8{0, 1, 2} {
			for _, input := range []string{"1/15/64", "1/15/24", "1/15/35", "1/15/69", "2/29/00", "1/1/26"} {
				cfg := base
				cfg.CurrentTime, cfg.PreferredDateSource = current, preference
				add("formatted", input, cfg, false, "1/2/06")
			}
		}
	}
	for _, day := range []uint8{0, 1, 2} {
		for _, month := range []uint8{0, 1, 2} {
			for _, pair := range [][2]string{{"2023", "2006"}, {"February 2023", "January 2006"}, {"2024-060", "2006-002"}} {
				cfg := base
				cfg.CurrentTime = "2024-01-31T12:00:00Z"
				cfg.PreferredDayOfMonth, cfg.PreferredMonthOfYear = day, month
				cfg.StrictParsing, cfg.ReturnTimeAsPeriod = true, true
				add("formatted", pair[0], cfg, false, pair[1])
			}
		}
	}
	for _, zone := range []string{"", "America/New_York", "Asia/Kathmandu"} {
		cfg := base
		cfg.CurrentTimezone, cfg.DefaultTimezone = "Europe/Berlin", zone
		add("formatted", "2024-03-10 02:30", cfg, false, "2006-01-02 15:04")
		add("formatted", "2024-11-03 01:30", cfg, false, "2006-01-02 15:04")
	}
	for _, order := range []string{"", "YMD", "YDM", "MDY", "MYD", "DMY", "DYM"} {
		for _, strict := range []bool{false, true} {
			for _, input := range []string{
				"20211011", "01022006", "202112", "202411", "20240229", "20230229", "240229", "000229", "690101", "990101",
				"202401011230", "20240101123045", "2024:01:01", "20240101:12abc", "123", "00000000", "99999999", " 20240101", "20240101 ", "202402301200",
			} {
				cfg := base
				cfg.CurrentTime = "2000-01-01T00:00:00Z"
				cfg.DateOrder, cfg.DateOrderIsExplicit = order, order != ""
				cfg.StrictParsing = strict
				add("nospace", input, cfg, false)
			}
		}
		cfg := base
		cfg.DateOrder, cfg.DateOrderIsExplicit = order, order != ""
		cfg.RequiredParts = []string{"year"}
		add("nospace", "20211011", cfg, false)
	}
	for _, source := range []uint8{0, 1, 2} {
		for _, precision := range []bool{false, true} {
			for _, input := range []string{
				"1 day ago", "in 2 day", "2 day", "yesterday", "1 days ago", "1 DAY ago", "1 Day ago", "0 day ago", "0 second ago", "1.5 day ago",
				"0.5 year ago", "0.1 year ago", "0.5 month ago", "1.25 week ago", "0.5 decade ago", "0.5 hour ago", "0.5 minute ago", "0.5 second ago", "0.4 second ago", "-0.5 second ago",
				"+2 day ago", "- 2 day ago", "1,5 hour ago", "1 day 2 day ago", "1 year 2 month 3 day ago", "1 week 2 day ago", "1 day ago 4 PM", "1 day ago 12:30:45.123456789", "1 hour ago 24:00", "1 day in ago",
				"1 day inside", "one day ago", "1.1.1 day ago", "1 day 5 minute ago", "1 YEAR ago", "(1 day) [ago]", "1\u00a0day ago", "1\u00a0 day ago", "", "ago",
			} {
				cfg := base
				cfg.PreferredDateSource, cfg.ReturnTimeAsPeriod = source, precision
				add("relative", input, cfg, false)
			}
		}
	}
	for _, current := range []string{"2024-03-31T12:30:45.987654321Z", "2023-03-31T12:30:45Z", "2024-02-29T12:30:45Z", "2000-01-31T12:30:45Z"} {
		for _, preserve := range []bool{false, true} {
			for _, input := range []string{"1 month ago", "in 1 month", "1 year ago", "in 1 year", "1 month 1 day ago", "0.5 month ago"} {
				cfg := base
				cfg.CurrentTime, cfg.PreserveEndOfMonth = current, preserve
				add("relative", input, cfg, false)
			}
		}
	}
	for _, current := range []string{"2024-03-10T16:00:00Z", "2024-11-03T17:00:00Z", "2024-03-31T12:00:00Z", "2024-10-27T12:00:00Z"} {
		for _, zone := range []string{"America/New_York", "Europe/Berlin", "Asia/Kathmandu"} {
			for _, input := range []string{"1 day ago", "24 hour ago", "in 1 day", "in 24 hour"} {
				cfg := base
				cfg.CurrentTime, cfg.CurrentTimezone, cfg.ReturnTimeAsPeriod = current, zone, true
				add("relative", input, cfg, false)
			}
		}
	}
	for _, zone := range []string{"", "America/New_York", "Asia/Kathmandu"} {
		for _, input := range []string{"1 day ago UTC", "1 day ago EST", "1 day ago +0530", "1 day ago UTC+05:45", "1 day ago (GMT)", "1 hour ago -0400", "in 1 month CET", "1 day ago 4 PM EST", "1 day ago Z", "1 day ago UTC UTC", "1 day ago \u00c9ST", "1 day ago UTC+25:00"} {
			cfg := base
			cfg.DefaultTimezone, cfg.ReturnTimeAsPeriod = zone, true
			add("relative", input, cfg, false)
		}
	}
	data.Cases = append(data.Cases, public...)
	encoded, err := json.MarshalIndent(data, "", "  ")
	if err != nil {
		panic(err)
	}
	encoded = append(encoded, '\n')
	if err := os.MkdirAll(filepath.Dir(*output), 0755); err != nil {
		panic(err)
	}
	if err := os.WriteFile(*output, encoded, 0644); err != nil {
		panic(err)
	}
	fmt.Printf("Wrote %d pinned Go reference cases to %s\n", len(data.Cases), *output)
}
