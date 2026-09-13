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

func runBenchmark(path, cohort string, passes int, provenance reference, correctionsPath string) {
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
	expectedReference := pinnedReference("v1.4.5")
	if data.Reference.Module != expectedReference.Module || data.Reference.Version != expectedReference.Version ||
		data.Reference.Commit != expectedReference.Commit || data.Reference.ModuleSum != expectedReference.ModuleSum ||
		data.Reference.GoVersion != provenance.GoVersion || data.Reference.TextVersion != provenance.TextVersion {
		panic("benchmark fixture does not match the pinned shared reference")
	}
	if data.Reference.WallYear != time.Now().Year() {
		panic("regenerate the Go fixture for the current wall-clock year")
	}
	historicalParsed := map[string]bool{}
	corrected := map[string]bool{}
	correctionsSHA256 := ""
	for _, item := range data.Cases {
		historicalParsed[item.ID] = item.Expected.Parsed
	}
	if correctionsPath != "" {
		encoded, err := os.ReadFile(correctionsPath)
		if err != nil {
			panic(err)
		}
		var corrections struct {
			Reference  reference         `json:"reference"`
			CoreSHA256 string            `json:"core_sha256"`
			Core       map[string]result `json:"core"`
			Python     struct {
				Version    string `json:"version"`
				Dateparser string `json:"dateparser"`
				Dateutil   string `json:"dateutil"`
			} `json:"python"`
		}
		if err := json.Unmarshal(encoded, &corrections); err != nil {
			panic(err)
		}
		if provenance.Version != "worktree" || corrections.Reference.Version != "worktree" ||
			corrections.Reference.Module != provenance.Module || corrections.Reference.SourceSHA256 != provenance.SourceSHA256 ||
			corrections.CoreSHA256 != fmt.Sprintf("%x", sha256.Sum256(contents)) ||
			corrections.Python.Version != "3.14.6" || corrections.Python.Dateparser != "1.4.3" || corrections.Python.Dateutil != "2.9.0.post0" {
			panic("benchmark corrections do not match the Python-qualified Go source and input fixture")
		}
		for index := range data.Cases {
			item := &data.Cases[index]
			if expected, exists := corrections.Core[item.ID]; exists {
				if item.Expected == expected {
					panic("redundant correction: " + item.ID)
				}
				item.Expected = expected
				corrected[item.ID] = true
				delete(corrections.Core, item.ID)
			}
		}
		if len(corrections.Core) != 0 {
			panic("correction references an unknown benchmark input")
		}
		correctionsSHA256 = fmt.Sprintf("%x", sha256.Sum256(encoded))
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
	goReference := map[string]string{
		"module": provenance.Module, "version": provenance.Version,
		"commit": provenance.Commit, "module_sum": provenance.ModuleSum,
	}
	if provenance.SourceSHA256 != "" {
		goReference["source_sha256"] = provenance.SourceSHA256
	}
	metadata := map[string]any{
		"cohort":         cohort,
		"cases":          len(prepared),
		"parsed":         expectedParsed,
		"fixture_sha256": fmt.Sprintf("%x", sha256.Sum256(contents)),
		"first_pass_ms":  firstPassMS,
		"go_reference":   goReference,
	}
	if correctionsSHA256 != "" {
		historicalCount := 0
		correctedCases := []string{}
		for _, item := range prepared {
			if historicalParsed[item.Case.ID] {
				historicalCount++
			}
			if corrected[item.Case.ID] {
				correctedCases = append(correctedCases, item.Case.ID)
			}
		}
		metadata["historical_parsed"] = historicalCount
		metadata["corrected_cases"] = correctedCases
		metadata["corrections_sha256"] = correctionsSHA256
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

var benchmarkSourceCommit, benchmarkSourceSHA256 string

func benchmarkSourceReference(provenance reference) reference {
	if len(benchmarkSourceCommit) != 40 || len(benchmarkSourceSHA256) != 64 {
		panic("local source provenance must be embedded by tools/benchmark.py at build time")
	}
	provenance.Version, provenance.ModuleSum = "worktree", ""
	provenance.Commit, provenance.SourceSHA256 = benchmarkSourceCommit, benchmarkSourceSHA256
	return provenance
}

func runFeatureBenchmark(path, cohort string, passes, iterations int, provenance reference) {
	if passes < 1 || iterations < 1 || !slices.Contains([]string{"search-auto", "search-split", "search-ngram", "time-span", "jalali", "hijri"}, cohort) {
		panic("feature benchmark requires a known cohort and positive passes/iterations")
	}
	if runtime.GOMAXPROCS(0) != 1 {
		panic("set GOMAXPROCS=1 for a single-thread comparison")
	}
	contents, err := os.ReadFile(path)
	if err != nil {
		panic(err)
	}
	var document struct {
		Reference struct {
			Versions map[string]string `json:"versions"`
		} `json:"reference"`
		Cases []featureCase `json:"cases"`
	}
	if err := json.Unmarshal(contents, &document); err != nil {
		panic(err)
	}
	if document.Reference.Versions["dateparser"] != "1.4.3" {
		panic("unexpected Python reference version")
	}
	type entry struct {
		Case          *featureCase
		Configuration *dps.Configuration
		Parser        *dps.Parser
	}
	var prepared []entry
	for index := range document.Cases {
		item := &document.Cases[index]
		if item.KnownDifference != "" || item.DetectorLanguages != nil {
			continue
		}
		selected := "search-split"
		switch {
		case item.Stage == "jalali" || item.Stage == "hijri":
			selected = item.Stage
		case item.Configuration.ReturnTimeSpan:
			selected = "time-span"
		case item.Configuration.SearchStrategy == "ngram":
			selected = "search-ngram"
		case item.Stage == "search" && len(item.Configuration.Languages) == 0 && len(item.Configuration.Locales) == 0:
			selected = "search-auto"
		}
		if selected != cohort {
			continue
		}
		item.Configuration.DateOrderIsExplicit = item.Configuration.DateOrder != ""
		prepared = append(prepared, entry{item, item.Configuration.public(), &dps.Parser{}})
	}
	if len(prepared) == 0 {
		panic("empty feature benchmark cohort")
	}
	calendars := cohort == "jalali" || cohort == "hijri"
	call := func(item entry) (string, []dps.SearchResult, error) {
		if calendars {
			parse := dps.ParseJalali
			if cohort == "hijri" {
				parse = dps.ParseHijri
			}
			parsed, err := parse(item.Configuration, item.Case.Input)
			if err != nil {
				return "", nil, err
			}
			return "", []dps.SearchResult{{Date: parsed, Text: item.Case.Input}}, nil
		}
		if item.Case.Stage == "search" {
			return item.Parser.Search(item.Configuration, item.Case.Input)
		}
		matches, err := item.Parser.SearchWithLanguage(item.Configuration, item.Case.Language, item.Case.Input)
		return item.Case.Language, matches, err
	}
	run := func(item entry) (detected string, matches []dps.SearchResult, err error, panicText string) {
		defer func() {
			if recovered := recover(); recovered != nil {
				panicText = fmt.Sprint(recovered)
			}
		}()
		detected, matches, err = call(item)
		return
	}
	historical := provenance.Version == "v1.4.3"
	expectedParsed, expectedMatches, expectedPanics := 0, 0, 0
	var differences []map[string]any
	outcomes := sha256.New()
	encoder := json.NewEncoder(outcomes)
	first := time.Now()
	for _, item := range prepared {
		detected, matches, err, panicText := run(item)
		matchesPython := panicText == "" && (!calendars || (err != nil) == (item.Case.Error != ""))
		if !calendars && len(matches) > 0 && detected != item.Case.Detected {
			matchesPython = false
		}
		actual := make([]featureMatch, 0, len(matches))
		for _, matched := range matches {
			date := featureDate(matched.Date)
			if !calendars {
				date.Locale, date.Period, date.Timezone = "", "", ""
			} else if len(item.Case.Matches) > 0 && item.Case.Matches[0].Date.Period == "Time" {
				if !slices.Contains([]string{"Hour", "Minute", "Second"}, date.Period) {
					matchesPython = false
				}
				date.Period = "Time"
			}
			actual = append(actual, featureMatch{matched.Text, date})
		}
		if !slices.Equal(actual, item.Case.Matches) {
			matchesPython = false
		}
		errorText := ""
		if err != nil {
			errorText = err.Error()
		}
		outcome := map[string]any{
			"id": item.Case.ID, "detected": detected, "matches": actual,
			"error": errorText, "panic": panicText,
		}
		if err := encoder.Encode(outcome); err != nil {
			panic(err)
		}
		if !matchesPython {
			if !historical {
				panic(fmt.Sprintf("%s %q: expected detected=%q matches=%+v error=%q, got %+v", item.Case.ID, item.Case.Input, item.Case.Detected, item.Case.Matches, item.Case.Error, outcome))
			}
			differences = append(differences, map[string]any{
				"input": item.Case.Input, "actual": outcome,
				"expected_detected": item.Case.Detected, "expected_matches": item.Case.Matches,
				"expected_error": item.Case.Error,
			})
		}
		if len(matches) > 0 {
			expectedParsed++
		}
		expectedMatches += len(matches)
		if panicText != "" {
			expectedPanics++
		}
	}
	identity := map[string]string{"module": provenance.Module, "version": provenance.Version, "commit": provenance.Commit, "module_sum": provenance.ModuleSum}
	if provenance.SourceSHA256 != "" {
		identity["source_sha256"] = provenance.SourceSHA256
	}
	metadata := map[string]any{
		"cohort": cohort, "cases": len(prepared), "parsed": expectedParsed,
		"matched_dates": expectedMatches, "iterations": iterations,
		"fixture_sha256":    fmt.Sprintf("%x", sha256.Sum256(contents)),
		"first_pass_ms":     float64(time.Since(first)) / float64(time.Millisecond),
		"go_reference":      identity,
		"python_matches":    len(prepared) - len(differences),
		"python_mismatches": len(differences), "python_differences": differences,
		"panics": expectedPanics, "outcome_sha256": fmt.Sprintf("%x", outcomes.Sum(nil)),
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
		parsedCount, matchCount, panicCount := 0, 0, 0
		start := time.Now()
		for range iterations {
			for _, item := range prepared {
				language, matches, err, panicText := run(item)
				if len(matches) > 0 {
					parsedCount++
				}
				matchCount += len(matches)
				if panicText != "" {
					panicCount++
				}
				runtime.KeepAlive(language)
				runtime.KeepAlive(matches)
				runtime.KeepAlive(err)
				runtime.KeepAlive(panicText)
			}
		}
		passMS = append(passMS, float64(time.Since(start))/float64(time.Millisecond))
		if parsedCount != expectedParsed*iterations || matchCount != expectedMatches*iterations || panicCount != expectedPanics*iterations {
			panic("feature benchmark result counts changed")
		}
	}
	emit("BENCHMARK_RESULT", map[string]any{"metadata": metadata, "pass_ms": passMS})
}
