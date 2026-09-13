package main

import (
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"go/ast"
	"go/parser"
	"go/token"
	"os"
	"os/exec"
	"path/filepath"
	"slices"
	"strconv"
	"strings"
	"time"

	dps "github.com/markusmobius/go-dateparser"
	"github.com/markusmobius/go-dateparser/date"
	"github.com/markusmobius/go-dateparser/internal/data"
	"github.com/markusmobius/go-dateparser/internal/language"
	"github.com/markusmobius/go-dateparser/internal/setting"
)

type featureMatch struct {
	Text string `json:"text"`
	Date result `json:"date"`
}

type featureCase struct {
	ID                     string         `json:"id"`
	Stage                  string         `json:"stage"`
	Input                  string         `json:"input"`
	Language               string         `json:"language,omitempty"`
	Configuration          configuration  `json:"configuration"`
	UnspecifiedCurrentTime bool           `json:"unspecified_current_time,omitempty"`
	ParserTypes            []int          `json:"parser_types,omitempty"`
	DetectorLanguages      *[]string      `json:"detector_languages,omitempty"`
	DetectionInputs        []string       `json:"detection_inputs,omitempty"`
	DateOrderInputs        []string       `json:"date_order_inputs,omitempty"`
	Detected               string         `json:"detected,omitempty"`
	Error                  string         `json:"error,omitempty"`
	UpstreamPanic          string         `json:"upstream_panic,omitempty"`
	KnownDifference        string         `json:"known_difference,omitempty"`
	Matches                []featureMatch `json:"matches"`
	Translations           []string       `json:"translations"`
	Originals              []string       `json:"originals"`
}

func featureDate(parsed date.Date) result {
	if parsed.Time.Location() == time.Local {
		parsed.Time = parsed.Time.UTC()
	}
	expected := result{Parsed: !parsed.IsZero()}
	if expected.Parsed {
		_, expected.Offset = parsed.Time.Zone()
		expected.UnixSeconds, expected.Nanosecond = parsed.Time.Unix(), parsed.Time.Nanosecond()
		expected.Period, expected.Timezone, expected.Locale = parsed.Period.String(), parsed.Time.Location().String(), parsed.Locale
	}
	return expected
}

func featureString(expression ast.Expr) (string, bool) {
	switch value := expression.(type) {
	case *ast.BasicLit:
		if value.Kind == token.STRING {
			text, err := strconv.Unquote(value.Value)
			return text, err == nil
		}
	case *ast.BinaryExpr:
		if value.Op == token.ADD {
			left, leftOK := featureString(value.X)
			right, rightOK := featureString(value.Y)
			return left + right, leftOK && rightOK
		}
	}
	return "", false
}

func exportFeatures(output string, provenance reference) {
	encoded, err := exec.Command("go", "list", "-m", "-json", provenance.Module).Output()
	if err != nil {
		panic(err)
	}
	var module struct {
		Dir, Version string
		Replace      *json.RawMessage
	}
	if err := json.Unmarshal(encoded, &module); err != nil {
		panic(err)
	}
	if module.Version != provenance.Version || module.Replace != nil {
		panic("unexpected feature reference")
	}
	document := struct {
		Reference          reference         `json:"reference"`
		SourceSHA256       map[string]string `json:"source_sha256"`
		CalendarDataSHA256 string            `json:"calendar_data_sha256"`
		Cases              []featureCase     `json:"cases"`
	}{Reference: provenance, SourceSHA256: map[string]string{}}
	document.CalendarDataSHA256 = exportCalendarData(module.Dir)
	for _, file := range []string{"LICENSE", "data/locales.json"} {
		contents, err := os.ReadFile(filepath.Join("../..", file))
		if err != nil {
			panic(err)
		}
		digest := fmt.Sprintf("%x", sha256.Sum256(contents))
		if file == "LICENSE" {
			document.Reference.ProjectLicenseSHA256 = digest
		} else {
			document.Reference.LocaleDataSHA256 = digest
		}
	}
	base := configuration{CurrentTime: "2000-01-01T00:00:00Z"}
	add := func(test featureCase) {
		test.ID = fmt.Sprintf("%s-%04d", test.Stage, len(document.Cases)+1)
		test.Matches, test.Translations, test.Originals = []featureMatch{}, []string{}, []string{}
		defer func() {
			if recovered := recover(); recovered != nil {
				test.UpstreamPanic = fmt.Sprint(recovered)
				fmt.Printf("Recorded upstream panic: %s %s %q: %s\n", test.ID, test.Language, test.Input, test.UpstreamPanic)
			}
			document.Cases = append(document.Cases, test)
		}()
		cfg := test.Configuration.public()
		if test.UnspecifiedCurrentTime {
			cfg.CurrentTime = time.Time{}
		}
		if test.Stage == "translation" {
			locale, _ := data.GetLocaleData(test.Language)
			translated, original := language.TranslateSearch(&setting.Configuration{SkipTokens: []string{"t"}}, locale, test.Input)
			test.Translations, test.Originals = append(test.Translations, translated...), append(test.Originals, original...)
		} else if test.Stage == "jalali" || test.Stage == "hijri" {
			if cfg.DateOrder != nil {
				resolve := cfg.DateOrder
				cfg.DateOrder = func(locale string) string {
					test.DateOrderInputs = append(test.DateOrderInputs, locale)
					return resolve(locale)
				}
			}
			parse := dps.ParseJalali
			if test.Stage == "hijri" {
				parse = dps.ParseHijri
			}
			parsed, err := parse(cfg, test.Input)
			if err != nil {
				test.Error = err.Error()
			}
			if !parsed.IsZero() {
				test.Matches = append(test.Matches, featureMatch{test.Input, featureDate(parsed)})
			}
		} else {
			engine := &dps.Parser{}
			for _, kind := range test.ParserTypes {
				engine.ParserTypes = append(engine.ParserTypes, dps.ParserType(kind))
			}
			if test.DetectorLanguages != nil {
				engine.DetectLanguagesFunction = func(input string) []string {
					test.DetectionInputs = append(test.DetectionInputs, input)
					return *test.DetectorLanguages
				}
			}
			var matches []dps.SearchResult
			var err error
			if test.Stage == "search" {
				test.Detected, matches, err = engine.Search(cfg, test.Input)
			} else {
				matches, err = engine.SearchWithLanguage(cfg, test.Language, test.Input)
			}
			if err != nil {
				test.Error = err.Error()
			}
			for _, match := range matches {
				test.Matches = append(test.Matches, featureMatch{match.Text, featureDate(match.Date)})
			}
		}
	}
	for _, file := range []string{"internal/language/translate-search_test.go", "internal/language/detector_test.go", "search_test.go", "internal/parser/jalali/parser_test.go", "internal/parser/hijri/parser_test.go"} {
		contents, err := os.ReadFile(filepath.Join(module.Dir, file))
		if err != nil {
			panic(err)
		}
		document.SourceSHA256[file] = fmt.Sprintf("%x", sha256.Sum256(contents))
		source, err := parser.ParseFile(token.NewFileSet(), file, contents, 0)
		if err != nil {
			panic(err)
		}
		ast.Inspect(source, func(node ast.Node) bool {
			literal, ok := node.(*ast.CompositeLit)
			if !ok {
				return true
			}
			if strings.HasPrefix(file, "internal/parser/") && len(literal.Elts) == 2 {
				input, valid := featureString(literal.Elts[0])
				if _, timeCall := literal.Elts[1].(*ast.CallExpr); valid && timeCall {
					stage := strings.Split(file, "/")[2]
					add(featureCase{Stage: stage, Input: input, Configuration: base})
					return false
				}
			}
			if file == "internal/language/detector_test.go" && len(literal.Elts) == 2 {
				code, codeOK := featureString(literal.Elts[0])
				input, inputOK := featureString(literal.Elts[1])
				if _, exists := data.GetLocaleData(code); exists && codeOK && inputOK {
					add(featureCase{Stage: "search", Language: code, Input: input, Configuration: base})
					return false
				}
			}
			if file == "internal/language/translate-search_test.go" && len(literal.Elts) == 3 {
				code, codeOK := featureString(literal.Elts[0])
				input, inputOK := featureString(literal.Elts[1])
				if _, exists := data.GetLocaleData(code); exists && codeOK && inputOK {
					add(featureCase{Stage: "translation", Language: code, Input: input, Configuration: base})
					return false
				}
			}
			fields := map[string]ast.Expr{}
			for _, entry := range literal.Elts {
				pair, ok := entry.(*ast.KeyValueExpr)
				if !ok {
					continue
				}
				if name, ok := pair.Key.(*ast.Ident); ok {
					fields[name.Name] = pair.Value
				}
			}
			code, codeOK := featureString(fields["Language"])
			input, inputOK := featureString(fields["Text"])
			if codeOK && inputOK {
				test := featureCase{Stage: "search_with_language", Language: code, Input: input, Configuration: base}
				if call, ok := fields["CurrentTime"].(*ast.CallExpr); ok {
					parts := []int{2000, 1, 1, 0, 0, 0}
					valid := len(call.Args) <= len(parts)
					for index, argument := range call.Args {
						value, ok := argument.(*ast.BasicLit)
						if !ok || value.Kind != token.INT || index >= len(parts) {
							valid = false
							break
						}
						parts[index], err = strconv.Atoi(value.Value)
						if err != nil {
							valid = false
							break
						}
					}
					if valid {
						test.Configuration.CurrentTime = time.Date(parts[0], time.Month(parts[1]), parts[2], parts[3], parts[4], parts[5], 0, time.UTC).Format(time.RFC3339Nano)
					}
				}
				if kinds, ok := fields["ParserTypes"].(*ast.CompositeLit); ok {
					for _, item := range kinds.Elts {
						selector, ok := item.(*ast.SelectorExpr)
						if !ok {
							panic("unexpected search parser type")
						}
						kind, exists := map[string]int{"Timestamp": 0, "NegativeTimestamp": 1, "RelativeTime": 2, "CustomFormat": 3, "AbsoluteTime": 4, "NoSpacesTime": 5}[selector.Sel.Name]
						if !exists {
							panic("unknown search parser type")
						}
						test.ParserTypes = append(test.ParserTypes, kind)
					}
				}
				add(test)
				test.Stage = "search"
				test.Configuration.Languages = []string{code}
				add(test)
				test.Configuration.SearchStrategy = "ngram"
				add(test)
				return false
			}
			return true
		})
	}
	names := make([]string, 0, len(data.LocaleOrder))
	for name := range data.LocaleOrder {
		names = append(names, name)
	}
	slices.Sort(names)
	for _, code := range names {
		for _, stage := range []string{"translation", "search_with_language"} {
			add(featureCase{Stage: stage, Language: code, Input: "2024-02-29 12:34:56 UTC", Configuration: base})
		}
	}
	add(featureCase{Stage: "search_with_language", Language: "en", Input: "25th march 2015 , i need this report today.", Configuration: base, UnspecifiedCurrentTime: true})
	add(featureCase{Stage: "search_with_language", Language: "not-real", Input: "4 October 1957", Configuration: base})
	for _, strategy := range []string{"split", "ngram"} {
		for _, input := range []string{"", "Hello world nothing here at all", "Chapter 12, page 3", "The satellite was launched on 4 October 1957", "The client arrived in March 3rd, 2004 and returned on May 6th 2004", "posted 10 minutes ago", "updated 1700000000 UTC"} {
			cfg := base
			cfg.SearchStrategy, cfg.Languages = strategy, []string{"en"}
			add(featureCase{Stage: "search", Input: input, Configuration: cfg})
		}
		for _, current := range []string{"2025-01-31T17:30:45.123456789Z", "2024-03-11T16:30:45.123456789Z"} {
			for _, start := range []string{"monday", "sunday"} {
				for _, days := range []int{28, 30} {
					for _, input := range []string{"for the past month", "during the previous week", "in the next week", "past 2 days", "next 2 days", "past 2 weeks", "next 2 weeks", "past 1 months", "next 1 months", "next month", "past 0 days", "past 99999999999999999999 weeks", "next week and last month"} {
						cfg := base
						cfg.CurrentTime, cfg.CurrentTimezone = current, "America/New_York"
						cfg.SearchStrategy, cfg.Languages, cfg.ReturnTimeSpan = strategy, []string{"en"}, true
						cfg.DefaultStartOfWeek, cfg.DefaultDaysInMonth = start, days
						add(featureCase{Stage: "search", Input: "Messages received " + input, Configuration: cfg})
					}
				}
			}
		}
	}
	for _, detected := range [][]string{{"fr"}, {"en", "fr"}, {}, {"not-real"}} {
		for _, locales := range [][]string{nil, {"fr"}} {
			cfg := base
			cfg.Locales = locales
			add(featureCase{Stage: "search", Input: "Publie le 11 juillet 2016", Configuration: cfg, DetectorLanguages: &detected})
		}
	}
	for _, cfg := range []configuration{
		{CurrentTime: base.CurrentTime, Languages: []string{"en", "fr", "es", "pt", "de", "it", "ar"}, StrictParsing: true},
		{CurrentTime: base.CurrentTime, Locales: []string{"en-US", "en-GB"}},
		{CurrentTime: base.CurrentTime, Languages: []string{"not-real"}},
		{CurrentTime: base.CurrentTime, Locales: []string{"not-real"}},
		{CurrentTime: base.CurrentTime, DefaultLanguages: []string{"en"}},
		{CurrentTime: base.CurrentTime, SearchStrategy: "unknown"},
		{CurrentTime: base.CurrentTime, DefaultStartOfWeek: "friday"},
	} {
		for _, input := range []string{"Date de facture 23 juillet 2020 Condition Redevable livraison FR", "###!!!", "4 October 1957"} {
			add(featureCase{Stage: "search", Input: input, Configuration: cfg})
		}
	}
	for _, year := range []int{0, 1, 8, 9, 37, 38, 198, 199, 425, 426, 685, 686, 755, 756, 817, 818, 1110, 1111, 1180, 1181, 1209, 1210, 1387, 1394, 1403, 1634, 1635, 2059, 2060, 2096, 2097, 2191, 2192, 2261, 2262, 2323, 2324, 2393, 2394, 2455, 2456, 3177, 3178} {
		for _, month := range []int{1, 6, 7, 11, 12} {
			for _, day := range []int{1, 29, 30, 31} {
				add(featureCase{Stage: "jalali", Input: fmt.Sprintf("%d/%d/%04d", day, month, year), Configuration: base})
			}
		}
	}
	for year := 1356; year <= 1500; year++ {
		for month := 1; month <= 12; month++ {
			for _, day := range []int{1, 29, 30} {
				add(featureCase{Stage: "hijri", Input: fmt.Sprintf("%d/%d/%04d", day, month, year), Configuration: base})
			}
		}
	}
	for _, year := range []int{0, 1, 1355, 1501, 1502, 9999} {
		for _, month := range []int{1, 12} {
			add(featureCase{Stage: "hijri", Input: fmt.Sprintf("1/%d/%04d", month, year), Configuration: base})
		}
	}
	calendarSettings := []configuration{base}
	for _, order := range []string{"DMY", "DYM", "MDY", "MYD", "YMD", "YDM"} {
		cfg := base
		cfg.DateOrder, cfg.DateOrderIsExplicit = order, true
		calendarSettings = append(calendarSettings, cfg)
	}
	for _, orders := range []map[string]string{{"": "YMD"}, {"": "invalid"}, {"fa": "YMD"}} {
		cfg := base
		cfg.DateOrderForLocales = orders
		calendarSettings = append(calendarSettings, cfg)
	}
	for source := uint8(0); source <= 2; source++ {
		for month := uint8(0); month <= 2; month++ {
			for day := uint8(0); day <= 2; day++ {
				cfg := base
				cfg.PreferredDateSource, cfg.PreferredMonthOfYear, cfg.PreferredDayOfMonth = source, month, day
				calendarSettings = append(calendarSettings, cfg)
			}
		}
	}
	for _, cfg := range []configuration{
		{CurrentTime: base.CurrentTime, StrictParsing: true},
		{CurrentTime: base.CurrentTime, RequiredParts: []string{"year", "month"}},
		{CurrentTime: base.CurrentTime, RequiredParts: []string{"DAY", "Month"}},
		{CurrentTime: base.CurrentTime, RequiredParts: []string{"hour"}},
		{CurrentTime: base.CurrentTime, ReturnTimeAsPeriod: true},
		{CurrentTime: base.CurrentTime, IgnoreSurroundingText: true},
		{CurrentTime: base.CurrentTime, SearchStrategy: "invalid", DateOrderForLocales: map[string]string{"": "YMD"}},
		{CurrentTime: "2024-03-11T00:30:45.123456789Z", CurrentTimezone: "America/New_York", DefaultTimezone: "Asia/Kolkata", ReturnTimeAsPeriod: true},
		{CurrentTime: "2024-11-03T00:30:45.123456789Z", CurrentTimezone: "Asia/Kolkata", DefaultTimezone: "America/New_York", PreferredDateSource: 2},
	} {
		calendarSettings = append(calendarSettings, cfg)
	}
	for _, stage := range []string{"jalali", "hijri"} {
		for _, cfg := range calendarSettings {
			for _, input := range []string{"", " \t ", "today", "year", "00", "10", "01/02", "1444", "01/1444", "Farvardin 1444", "1/1/1444", "1444/1/2", "1/1/89", "1/1/90", "30/12/1444", "30/02/1444", "1/1/1444 16:50 pm", "1/1/1444 09:10:11.123456789 UTC", "1/1/1444 03:05 EST", "1/1/1444 +05:30", "23:30", "24:00", "1/1/1444 unknown"} {
				add(featureCase{Stage: stage, Input: input, Configuration: cfg})
			}
		}
		for _, current := range []string{"1937-03-13T23:59:59Z", "1937-03-14T00:00:00Z", "2077-11-16T23:59:59Z", "2077-11-17T00:00:00Z"} {
			for _, input := range []string{"01/02", "1/1/1444", "23:30"} {
				cfg := base
				cfg.CurrentTime = current
				add(featureCase{Stage: stage, Input: input, Configuration: cfg})
			}
		}
	}
	for _, strategy := range []string{"split", "ngram"} {
		for _, input := range []string{"xyzz", "CET", "March 2014 CET", "march 2014", "now PST", "on twenty first of March 2014", "\u0441 12 \u043c\u0430\u0440\u0442\u0430 2024 \u043f\u043e 14 \u043c\u0430\u0440\u0442\u0430 2024"} {
			cfg := base
			cfg.SearchStrategy, cfg.SkipTokens = strategy, []string{"t", "xyzz"}
			cfg.Languages = []string{"en", "fr", "ru"}
			add(featureCase{Stage: "search", Input: input, Configuration: cfg})
		}
	}
	for _, stage := range []string{"jalali", "hijri"} {
		add(featureCase{
			Stage: stage, Input: "1/1/1444 GMT+05:30", Configuration: base,
		})
	}
	contents, err := json.MarshalIndent(document, "", "  ")
	if err != nil {
		panic(err)
	}
	if err := os.MkdirAll(filepath.Dir(output), 0755); err != nil {
		panic(err)
	}
	if err := os.WriteFile(output, append(contents, '\n'), 0644); err != nil {
		panic(err)
	}
	fmt.Printf("Exported %d supplementary feature cases to %s\n", len(document.Cases), output)
}

func exportCalendarData(root string) string {
	type translations struct {
		Replacement string   `json:"replacement"`
		Aliases     []string `json:"aliases"`
	}
	document := map[string]any{}
	sources := map[string]string{}
	file := "internal/parser/calendars/data.json"
	contents, err := os.ReadFile(filepath.Join(root, file))
	if err != nil {
		panic(err)
	}
	sources["github.com/markusmobius/go-dateparser/"+file] = fmt.Sprintf("%x", sha256.Sum256(contents))
	if err := os.WriteFile("../../data/calendar-conversions.json", contents, 0644); err != nil {
		panic(err)
	}
	file = "internal/parser/jalali/translate.go"
	contents, err = os.ReadFile(filepath.Join(root, file))
	if err != nil {
		panic(err)
	}
	sources["github.com/markusmobius/go-dateparser/"+file] = fmt.Sprintf("%x", sha256.Sum256(contents))
	source, err := parser.ParseFile(token.NewFileSet(), file, contents, 0)
	if err != nil {
		panic(err)
	}
	patterns := map[string]string{}
	ast.Inspect(source, func(node ast.Node) bool {
		value, valid := node.(*ast.ValueSpec)
		if !valid || len(value.Names) != 1 || len(value.Values) != 1 {
			return true
		}
		name := value.Names[0].Name
		if strings.HasPrefix(name, "rx") {
			call, valid := value.Values[0].(*ast.CallExpr)
			if !valid || len(call.Args) != 1 {
				panic("unexpected Jalali pattern")
			}
			pattern, valid := featureString(call.Args[0])
			if !valid {
				panic("unexpected Jalali pattern value")
			}
			patterns[name] = rustPattern(pattern)
			return false
		}
		key := map[string]string{"monthNames": "jalali_months", "weekdayNames": "jalali_weekdays", "dayNumbers": "jalali_days"}[name]
		if key == "" {
			return true
		}
		literal := value.Values[0].(*ast.CompositeLit)
		words := []translations{}
		for _, entry := range literal.Elts {
			pair := entry.(*ast.CompositeLit)
			replacement, valid := featureString(pair.Elts[0])
			if !valid {
				number, valid := pair.Elts[0].(*ast.BasicLit)
				if !valid || number.Kind != token.INT {
					panic("unexpected Jalali translation")
				}
				replacement = number.Value
			}
			aliases := []string{}
			for _, alias := range pair.Elts[1].(*ast.CompositeLit).Elts {
				text, valid := featureString(alias)
				if !valid {
					panic("unexpected Jalali alias")
				}
				aliases = append(aliases, text)
			}
			words = append(words, translations{replacement, aliases})
		}
		document[key] = words
		return false
	})
	document["jalali_patterns"], document["source_sha256"] = patterns, sources
	if len(patterns) != 6 || document["jalali_months"] == nil || document["jalali_weekdays"] == nil || document["jalali_days"] == nil {
		panic("incomplete calendar export")
	}
	encoded, err := json.MarshalIndent(document, "", "  ")
	if err != nil {
		panic(err)
	}
	encoded = append(encoded, '\n')
	if err := os.WriteFile("../../data/calendars.json", encoded, 0644); err != nil {
		panic(err)
	}
	fmt.Printf("Exported calendar vocabulary and native conversion tables\n")
	return fmt.Sprintf("%x", sha256.Sum256(encoded))
}
