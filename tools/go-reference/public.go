package main

import (
	"crypto/sha256"
	"fmt"
	"go/ast"
	"go/parser"
	"go/token"
	"os"
	"path/filepath"
	"strconv"
	"time"

	dps "github.com/markusmobius/go-dateparser"
	"github.com/markusmobius/go-dateparser/internal/data"
)

func (cfg configuration) public() *dps.Configuration {
	internal := cfg.internal()
	result := &dps.Configuration{
		Locales: cfg.Locales, Languages: cfg.Languages, Region: cfg.Region, UseGivenOrder: cfg.UseGivenOrder,
		TryPreviousLocales: cfg.TryPreviousLocales, DefaultLanguages: cfg.DefaultLanguages, SkipTokens: cfg.SkipTokens,
		CurrentTime: internal.CurrentTime, DefaultTimezone: internal.DefaultTimezone,
		PreferredDayOfMonth: dps.PreferredDayOfMonth(cfg.PreferredDayOfMonth), PreferredMonthOfYear: dps.PreferredMonthOfYear(cfg.PreferredMonthOfYear),
		PreferredDateSource: dps.PreferredDateSource(cfg.PreferredDateSource), StrictParsing: cfg.StrictParsing,
		RequiredParts: cfg.RequiredParts, ReturnTimeAsPeriod: cfg.ReturnTimeAsPeriod, PreserveEndOfMonth: cfg.PreserveEndOfMonth,
		IgnoreSurroundingText: cfg.IgnoreSurroundingText,
	}
	if cfg.DateOrderIsExplicit {
		result.DateOrder = func(string) string { return cfg.DateOrder }
	}
	if len(cfg.DateOrderForLocales) > 0 {
		result.DateOrder = func(locale string) string { return cfg.DateOrderForLocales[locale] }
	}
	return result
}

func publicCases(moduleDir string, provenance *reference, translations []languageCase) []testCase {
	contents, err := os.ReadFile(filepath.Join(moduleDir, "parser_test.go"))
	if err != nil {
		panic(err)
	}
	provenance.ParserTestsSHA256 = fmt.Sprintf("%x", sha256.Sum256(contents))
	source, err := parser.ParseFile(token.NewFileSet(), "parser_test.go", contents, 0)
	if err != nil {
		panic(err)
	}
	base := configuration{CurrentTime: "2012-11-13T10:30:45.987654321Z"}
	results := []testCase{}
	sessions := map[string]*dps.Parser{}
	add := func(input string, cfg configuration, options testCase) {
		parser := &dps.Parser{}
		if options.Session != "" {
			if existing := sessions[options.Session]; existing != nil {
				parser = existing
			} else {
				sessions[options.Session] = parser
			}
		}
		for _, kind := range options.ParserTypes {
			parser.ParserTypes = append(parser.ParserTypes, dps.ParserType(kind))
		}
		if options.DetectorLanguages != nil {
			parser.DetectLanguagesFunction = func(input string) []string {
				options.DetectionInputs = append(options.DetectionInputs, input)
				return *options.DetectorLanguages
			}
		}
		parsed, err := parser.Parse(cfg.public(), input, options.Formats...)
		if parsed.Time.Location() == time.Local {
			parsed.Time = parsed.Time.UTC()
		}
		expected := result{Parsed: !parsed.IsZero()}
		if err != nil {
			expected.Error = err.Error()
		}
		if expected.Parsed {
			_, expected.Offset = parsed.Time.Zone()
			expected.UnixSeconds, expected.Nanosecond = parsed.Time.Unix(), parsed.Time.Nanosecond()
			expected.Period, expected.Timezone, expected.Locale = parsed.Period.String(), parsed.Time.Location().String(), parsed.Locale
		}
		options.ID, options.Stage = fmt.Sprintf("public-%04d", len(results)+1), "public"
		options.Input, options.Configuration, options.Expected = input, cfg, expected
		results = append(results, options)
	}
	for _, declaration := range source.Decls {
		function, ok := declaration.(*ast.FuncDecl)
		if !ok || function.Name.Name != "TestParser_Parse" {
			continue
		}
		ast.Inspect(function.Body, func(node ast.Node) bool {
			literal, ok := node.(*ast.CompositeLit)
			if !ok || len(literal.Elts) != 2 {
				return true
			}
			value, ok := literal.Elts[0].(*ast.BasicLit)
			if !ok || value.Kind != token.STRING {
				return true
			}
			input, err := strconv.Unquote(value.Value)
			if err != nil {
				panic(err)
			}
			add(input, base, testCase{})
			htmlDate := base
			htmlDate.PreferredDateSource, htmlDate.StrictParsing = 1, true
			add(input, htmlDate, testCase{ParserTypes: []int{3, 4}})
			return false
		})
	}
	seen := map[[2]string]bool{}
	checkedOrders := map[string]bool{}
	for _, example := range translations {
		key := [2]string{example.Locale, example.Input}
		if seen[key] {
			continue
		}
		seen[key] = true
		cfg := base
		cfg.Locales = []string{example.Locale}
		add(example.Input, cfg, testCase{})
		locale, _ := data.GetLocaleData(example.Locale)
		if !checkedOrders[example.Locale] && len(locale.DateOrder) != 3 {
			checkedOrders[example.Locale] = true
			for _, input := range []string{"20211011", "2018-04-12", "12:34", "202411"} {
				for _, parserTypes := range [][]int{{4}, {5}} {
					add(input, cfg, testCase{ParserTypes: parserTypes})
				}
			}
		}
	}
	for _, languages := range [][]string{{"en"}, {"fr"}, {"fr", "en"}, {"en", "fr"}} {
		for _, ordered := range []bool{false, true} {
			for _, input := range []string{"02/03/2014", "2018-04-12", "02-2024", "2024", "20 fevrier 2012", "2 days ago"} {
				cfg := base
				cfg.Languages, cfg.UseGivenOrder = languages, ordered
				add(input, cfg, testCase{})
				cfg.RequiredParts = []string{"year"}
				add(input, cfg, testCase{})
			}
		}
	}
	for _, locales := range [][]string{{"en-GB"}, {"en-US"}, {"en", "en-GB"}, {"en", "en"}, {"fr-PF"}, {"not-real"}} {
		cfg := base
		cfg.Locales, cfg.Languages, cfg.Region = locales, []string{"unknown"}, "XX"
		add("02/03/2014", cfg, testCase{})
		add("2024-02-29", cfg, testCase{Formats: []string{"2006-01-02"}})
	}
	for _, region := range []string{"GB", " us ", "001", "XX"} {
		cfg := base
		cfg.Languages, cfg.Region = []string{"en", "fr"}, region
		add("02/03/2014", cfg, testCase{})
	}
	for _, order := range []string{"YMD", "YDM", "MDY", "MYD", "DMY", "DYM"} {
		cfg := base
		cfg.Languages, cfg.DateOrder, cfg.DateOrderIsExplicit = []string{"fr"}, order, true
		for _, input := range []string{"2018-04-12", "01/02/03", "20211011"} {
			add(input, cfg, testCase{})
		}
	}
	for _, callback := range []map[string]string{{"fr": "YMD", "en": "DMY"}, {"fr": "invalid"}} {
		cfg := base
		cfg.Languages, cfg.DateOrderForLocales = []string{"fr"}, callback
		add("2018-04-12", cfg, testCase{})
	}
	for _, zone := range []string{"", "America/New_York", "Asia/Kathmandu"} {
		for _, input := range []string{"2024-03-10 02:30", "2024-11-03 01:30", "12 August 2021 EST", "12 August 2021 UTC+05:45", "2 days ago", "1700000000123", "published: 14 January 2024 UTC", "article title 14 January 2024 footer"} {
			cfg := base
			cfg.CurrentTimezone, cfg.DefaultTimezone = "America/New_York", zone
			cfg.IgnoreSurroundingText, cfg.ReturnTimeAsPeriod = true, true
			add(input, cfg, testCase{})
		}
	}
	for _, detected := range [][]string{{"fr"}, {"en"}, {"unknown"}, {}} {
		cfg := base
		cfg.DefaultLanguages = []string{"en"}
		add("12 August 2021", cfg, testCase{DetectorLanguages: &detected})
		cfg.Locales = []string{"fr"}
		add("20 Fevrier 2012", cfg, testCase{DetectorLanguages: &detected})
	}
	for _, previous := range []bool{false, true} {
		cfg := base
		cfg.TryPreviousLocales = previous
		session := fmt.Sprintf("previous-%t", previous)
		for _, input := range []string{"20 fevrier 2012", "02/03/2014", "12 August 2021", "03/04/2014"} {
			add(input, cfg, testCase{Session: session})
		}
	}
	for _, input := range []string{"", "!", "\n", "unknown \"date\"", "12 Aug. 2021", "on: 12 August 2021:", "1\u00a0day ago"} {
		add(input, base, testCase{})
	}
	fmt.Printf("Generated %d public Parse cases including HtmlDate configuration\n", len(results))
	return results
}
