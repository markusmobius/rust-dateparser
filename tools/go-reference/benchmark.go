package main

import (
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"os"
	"runtime"
	"slices"
	"time"

	dps "github.com/markusmobius/go-dateparser"
)

func runBenchmark(path, cohort string, passes int, provenance reference) {
	if passes < 1 || !slices.Contains([]string{"auto", "explicit", "htmldate"}, cohort) {
		panic("benchmark requires a known cohort and at least one measured pass")
	}
	if runtime.GOMAXPROCS(0) != 1 {
		panic("set GOMAXPROCS=1 for a single-thread comparison")
	}
	contents, err := os.ReadFile(path)
	if err != nil {
		panic(err)
	}
	var data fixture
	if err := json.Unmarshal(contents, &data); err != nil {
		panic(err)
	}
	if data.Reference.Module != provenance.Module || data.Reference.Version != provenance.Version ||
		data.Reference.Commit != provenance.Commit || data.Reference.ModuleSum != provenance.ModuleSum ||
		data.Reference.GoVersion != provenance.GoVersion || data.Reference.TextVersion != provenance.TextVersion {
		panic("benchmark fixture does not match the pinned Go reference")
	}
	if data.Reference.WallYear != time.Now().Year() {
		panic("regenerate the Go fixture for the current wall-clock year")
	}
	type entry struct {
		Case          *testCase
		Configuration *dps.Configuration
		Parser        *dps.Parser
	}
	var prepared []entry
	expectedParsed := 0
	for index := range data.Cases {
		item := &data.Cases[index]
		if item.Stage != "public" || item.Session != "" || item.DetectorLanguages != nil {
			continue
		}
		selected := "auto"
		if slices.Equal(item.ParserTypes, []int{3, 4}) {
			selected = "htmldate"
		} else if len(item.Configuration.Locales) > 0 || len(item.Configuration.Languages) > 0 {
			selected = "explicit"
		}
		if selected != cohort {
			continue
		}
		parser := &dps.Parser{}
		for _, kind := range item.ParserTypes {
			parser.ParserTypes = append(parser.ParserTypes, dps.ParserType(kind))
		}
		prepared = append(prepared, entry{item, item.Configuration.public(), parser})
		if item.Expected.Parsed {
			expectedParsed++
		}
	}
	if len(prepared) == 0 {
		panic("benchmark cohort is empty")
	}
	first := time.Now()
	for _, item := range prepared {
		parsed, err := item.Parser.Parse(item.Configuration, item.Case.Input, item.Case.Formats...)
		if parsed.Time.Location() == time.Local {
			parsed.Time = parsed.Time.UTC()
		}
		actual := result{Parsed: !parsed.IsZero()}
		if err != nil {
			actual.Error = err.Error()
		}
		if actual.Parsed {
			_, actual.Offset = parsed.Time.Zone()
			actual.UnixSeconds, actual.Nanosecond = parsed.Time.Unix(), parsed.Time.Nanosecond()
			actual.Period, actual.Timezone, actual.Locale = parsed.Period.String(), parsed.Time.Location().String(), parsed.Locale
		}
		if actual != item.Case.Expected {
			panic(fmt.Sprintf("%s %q: expected %+v, got %+v", item.Case.ID, item.Case.Input, item.Case.Expected, actual))
		}
	}
	firstPassMS := float64(time.Since(first)) / float64(time.Millisecond)
	metadata := map[string]any{
		"cohort":         cohort,
		"cases":          len(prepared),
		"parsed":         expectedParsed,
		"fixture_sha256": fmt.Sprintf("%x", sha256.Sum256(contents)),
		"first_pass_ms":  firstPassMS,
	}
	emit := func(prefix string, value any) {
		encoded, err := json.Marshal(value)
		if err != nil {
			panic(err)
		}
		fmt.Printf("%s %s\n", prefix, encoded)
	}
	emit("BENCHMARK_READY", metadata)
	passMS := make([]float64, 0, passes)
	for range passes {
		parsedCount := 0
		start := time.Now()
		for _, item := range prepared {
			parsed, err := item.Parser.Parse(item.Configuration, item.Case.Input, item.Case.Formats...)
			if !parsed.IsZero() {
				parsedCount++
			}
			runtime.KeepAlive(parsed)
			runtime.KeepAlive(err)
		}
		passMS = append(passMS, float64(time.Since(start))/float64(time.Millisecond))
		if parsedCount != expectedParsed {
			panic("benchmark parsed count changed")
		}
	}
	emit("BENCHMARK_RESULT", map[string]any{"metadata": metadata, "pass_ms": passMS})
}
