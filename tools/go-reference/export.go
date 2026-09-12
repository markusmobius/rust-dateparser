package main

import (
	"bytes"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"go/ast"
	"go/parser"
	"go/token"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"regexp/syntax"
	"slices"
	"strconv"
	"strings"
	"time"
	"unicode"

	"github.com/markusmobius/go-dateparser/internal/data"
	"github.com/markusmobius/go-dateparser/internal/digit"
	"github.com/markusmobius/go-dateparser/internal/timezone"
)

func exportData(provenance *reference) ([]languageCase, []testCase) {
	command := exec.Command("go", "list", "-m", "-json", provenance.Module)
	encoded, err := command.Output()
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
	if module.Dir == "" || module.Version != provenance.Version || module.Replace != nil {
		panic("unexpected source module for data export")
	}
	root := filepath.Join("..", "..")
	projectLicense, err := os.ReadFile(filepath.Join(root, "LICENSE"))
	if err != nil {
		panic(err)
	}
	provenance.ProjectLicenseSHA256 = fmt.Sprintf("%x", sha256.Sum256(projectLicense))
	for _, filename := range []string{"LICENSE", "LICENSE_dateparser.txt"} {
		data, err := os.ReadFile(filepath.Join(module.Dir, filename))
		if err != nil {
			panic(err)
		}
		digest := fmt.Sprintf("%x", sha256.Sum256(data))
		if filename == "LICENSE" {
			provenance.LicenseSHA256 = digest
		} else {
			provenance.PythonLicenseSHA256 = digest
		}
	}
	data, err := os.ReadFile(filepath.Join(module.Dir, "internal", "timezone", "timezones.go"))
	if err != nil {
		panic(err)
	}
	provenance.TimezoneSourceSHA256 = fmt.Sprintf("%x", sha256.Sum256(data))
	source, err := parser.ParseFile(token.NewFileSet(), "timezones.go", data, 0)
	if err != nil {
		panic(err)
	}
	names := map[string]string{}
	ast.Inspect(source, func(node ast.Node) bool {
		literal, ok := node.(*ast.CompositeLit)
		if !ok {
			return true
		}
		mapping, ok := literal.Type.(*ast.MapType)
		if !ok {
			return true
		}
		keyType, keyOK := mapping.Key.(*ast.Ident)
		valueType, valueOK := mapping.Value.(*ast.Ident)
		if !keyOK || !valueOK || keyType.Name != "string" || valueType.Name != "int" {
			return true
		}
		for _, item := range literal.Elts {
			pair, ok := item.(*ast.KeyValueExpr)
			if !ok {
				panic("unexpected timezone entry")
			}
			key, ok := pair.Key.(*ast.BasicLit)
			if !ok || key.Kind != token.STRING {
				panic("unexpected timezone name")
			}
			name, err := strconv.Unquote(key.Value)
			if err != nil {
				panic(err)
			}
			upper := strings.ToUpper(name)
			if _, exists := names[upper]; !exists {
				names[upper] = name
			}
		}
		return false
	})
	keys := make([]string, 0, len(names))
	for name := range names {
		keys = append(keys, name)
	}
	slices.Sort(keys)
	var output bytes.Buffer
	fmt.Fprintf(&output, "#[cfg(test)]\npub(crate) const SOURCE_SHA256: &str =\n    %q;\n\n#[rustfmt::skip]\npub(crate) const TIMEZONES: &[(&str, &str, i32)] = &[\n", provenance.TimezoneSourceSHA256)
	for _, key := range keys {
		name := names[key]
		location, err := timezone.Load(name)
		if err != nil {
			panic(err)
		}
		_, offset := time.Unix(0, 0).In(location).Zone()
		fmt.Fprintf(&output, "    (%s, %s, %d),\n", rustString(key), rustString(name), offset)
	}
	output.WriteString("];\n")
	exportTimezoneMatchers(&output, source)
	if err := os.MkdirAll(filepath.Join(root, "src"), 0755); err != nil {
		panic(err)
	}
	if err := os.WriteFile(filepath.Join(root, "src", "timezone_data.rs"), output.Bytes(), 0644); err != nil {
		panic(err)
	}
	fmt.Printf("Exported %d timezone names from %s\n", len(keys), provenance.Version)
	exportLocales(root, module.Dir, provenance)
	translations := languageCases(module.Dir, provenance)
	return translations, publicCases(module.Dir, provenance, translations)
}

func rustPattern(pattern string) string {
	expression, err := syntax.Parse(pattern, syntax.Perl)
	if err != nil {
		panic(err)
	}
	var render func(*syntax.Regexp) string
	render = func(expression *syntax.Regexp) string {
		switch expression.Op {
		case syntax.OpWordBoundary:
			return `(?-u:\b)`
		case syntax.OpNoWordBoundary:
			return `(?-u:\B)`
		case syntax.OpConcat, syntax.OpAlternate:
			children := make([]string, len(expression.Sub))
			for index, child := range expression.Sub {
				children[index] = render(child)
			}
			if expression.Op == syntax.OpConcat {
				return strings.Join(children, "")
			}
			return "(?:" + strings.Join(children, "|") + ")"
		case syntax.OpCapture:
			prefix := "("
			if expression.Name != "" {
				prefix += "?P<" + expression.Name + ">"
			}
			return prefix + render(expression.Sub[0]) + ")"
		case syntax.OpStar, syntax.OpPlus, syntax.OpQuest, syntax.OpRepeat:
			quantifier := map[syntax.Op]string{syntax.OpStar: "*", syntax.OpPlus: "+", syntax.OpQuest: "?"}[expression.Op]
			if expression.Op == syntax.OpRepeat {
				if expression.Max < 0 {
					quantifier = fmt.Sprintf("{%d,}", expression.Min)
				} else {
					quantifier = fmt.Sprintf("{%d,%d}", expression.Min, expression.Max)
				}
			}
			if expression.Flags&syntax.NonGreedy != 0 {
				quantifier += "?"
			}
			return "(?:" + render(expression.Sub[0]) + ")" + quantifier
		default:
			return expression.String()
		}
	}
	return render(expression)
}

type localeReplacement struct {
	Pattern     int    `json:"pattern"`
	Replacement string `json:"replacement"`
}

type localeData struct {
	Name                  string              `json:"name"`
	DateOrder             string              `json:"date_order"`
	NoWordSpacing         bool                `json:"no_word_spacing"`
	SentenceSplitterGroup int                 `json:"sentence_splitter_group"`
	Charset               string              `json:"charset"`
	Abbreviations         []string            `json:"abbreviations"`
	Simplifications       []localeReplacement `json:"simplifications"`
	Translations          map[string][]string `json:"translations"`
	RelativeType          map[string]string   `json:"relative_type"`
	RelativeTypeRegexes   []localeReplacement `json:"relative_type_regexes"`
	Combined              int                 `json:"combined"`
	ExactCombined         int                 `json:"exact_combined"`
	KnownWords            []string            `json:"known_words"`
}

func exportLocales(root, moduleDir string, provenance *reference) {
	document := struct {
		Module         string            `json:"module"`
		Version        string            `json:"version"`
		Commit         string            `json:"commit"`
		SourceSHA256   map[string]string `json:"source_sha256"`
		LanguageOrder  map[string]int    `json:"language_order"`
		LocaleOrder    map[string]int    `json:"locale_order"`
		Patterns       []string          `json:"patterns"`
		DigitMappings  [][2]uint32       `json:"digit_mappings"`
		UnicodeVersion string            `json:"unicode_version"`
		Locales        []localeData      `json:"locales"`
	}{Module: provenance.Module, Version: provenance.Version, Commit: provenance.Commit, SourceSHA256: map[string]string{}, LanguageOrder: data.LanguageOrder, LocaleOrder: data.LocaleOrder}
	paths, err := filepath.Glob(filepath.Join(moduleDir, "internal", "data", "*.go"))
	if err != nil {
		panic(err)
	}
	for _, path := range paths {
		contents, err := os.ReadFile(path)
		if err != nil {
			panic(err)
		}
		document.SourceSHA256[filepath.Base(path)] = fmt.Sprintf("%x", sha256.Sum256(contents))
	}
	digitSource, err := os.ReadFile(filepath.Join(moduleDir, "internal", "digit", "digit.go"))
	if err != nil {
		panic(err)
	}
	document.SourceSHA256["../digit/digit.go"] = fmt.Sprintf("%x", sha256.Sum256(digitSource))
	document.UnicodeVersion = unicode.Version
	for character := rune(0); character <= unicode.MaxRune; character++ {
		if !unicode.IsDigit(character) {
			continue
		}
		normalized := []rune(digit.NormalizeString(string(character)))
		if len(normalized) != 1 {
			panic("digit normalization changed character count")
		}
		if normalized[0] != character {
			document.DigitMappings = append(document.DigitMappings, [2]uint32{uint32(character), uint32(normalized[0])})
		}
	}
	names := make([]string, 0, len(data.LocaleOrder))
	for name := range data.LocaleOrder {
		names = append(names, name)
	}
	slices.Sort(names)
	patternIndexes := map[string]int{}
	patternSources := map[string]int{}
	internPattern := func(pattern string) int {
		if index, exists := patternSources[pattern]; exists {
			return index
		}
		normalized := rustPattern(pattern)
		index, exists := patternIndexes[normalized]
		if !exists {
			index = len(document.Patterns)
			document.Patterns = append(document.Patterns, normalized)
			patternIndexes[normalized] = index
		}
		patternSources[pattern] = index
		return index
	}
	replacements := func(entries []data.ReplacementData) []localeReplacement {
		results := make([]localeReplacement, 0, len(entries))
		for _, entry := range entries {
			results = append(results, localeReplacement{Pattern: internPattern(entry.Rx.String()), Replacement: entry.Replacement})
		}
		return results
	}
	for _, name := range names {
		locale, exists := data.GetLocaleData(name)
		if !exists {
			panic("locale order contains an unknown locale: " + name)
		}
		entry := localeData{
			Name: locale.Name, DateOrder: locale.DateOrder, NoWordSpacing: locale.NoWordSpacing,
			SentenceSplitterGroup: locale.SentenceSplitterGroup, Charset: string(locale.Charset),
			Abbreviations: locale.Abbreviations, Simplifications: replacements(locale.Simplifications),
			Translations: locale.Translations, RelativeType: locale.RelativeType,
			RelativeTypeRegexes: replacements(locale.RelativeTypeRegexes), KnownWords: locale.KnownWords,
			Combined: -1, ExactCombined: -1,
		}
		if entry.Translations == nil {
			entry.Translations = map[string][]string{}
		}
		if entry.RelativeType == nil {
			entry.RelativeType = map[string]string{}
		}
		if entry.KnownWords == nil {
			entry.KnownWords = []string{}
		}
		if locale.RxCombined != nil {
			entry.Combined = internPattern(locale.RxCombined.String())
		}
		if locale.RxExactCombined != nil {
			entry.ExactCombined = internPattern(locale.RxExactCombined.String())
		}
		document.Locales = append(document.Locales, entry)
	}
	encoded, err := json.Marshal(document)
	if err != nil {
		panic(err)
	}
	encoded = append(encoded, '\n')
	provenance.LocaleDataSHA256 = fmt.Sprintf("%x", sha256.Sum256(encoded))
	if err := os.MkdirAll(filepath.Join(root, "data"), 0755); err != nil {
		panic(err)
	}
	if err := os.WriteFile(filepath.Join(root, "data", "locales.json"), encoded, 0644); err != nil {
		panic(err)
	}
	fmt.Printf("Exported %d locales across %d languages (%d bytes)\n", len(document.Locales), len(document.LanguageOrder), len(encoded))
}

func rustString(value string) string {
	var output strings.Builder
	output.WriteByte('"')
	for _, character := range value {
		switch character {
		case '\\':
			output.WriteString(`\\`)
		case '"':
			output.WriteString(`\"`)
		default:
			if character >= 32 && character < 127 {
				output.WriteRune(character)
			} else {
				fmt.Fprintf(&output, `\u{%x}`, character)
			}
		}
	}
	output.WriteByte('"')
	return output.String()
}

func literalString(expression ast.Expr) string {
	if parenthesized, ok := expression.(*ast.ParenExpr); ok {
		return literalString(parenthesized.X)
	}
	literal, ok := expression.(*ast.BasicLit)
	if !ok || literal.Kind != token.STRING {
		panic("unexpected timezone string expression")
	}
	value, err := strconv.Unquote(literal.Value)
	if err != nil {
		panic(err)
	}
	return value
}

func exportTimezoneMatchers(output *bytes.Buffer, source *ast.File) {
	output.WriteString("\n#[rustfmt::skip]\npub(crate) const MATCHERS: &[(&str, i32, &str)] = &[\n")
	searchPatterns := map[string]struct{}{}
	ast.Inspect(source, func(node ast.Node) bool {
		literal, ok := node.(*ast.CompositeLit)
		if !ok {
			return true
		}
		var patterns, names []string
		var alternatives [][2]string
		for _, element := range literal.Elts {
			pair, ok := element.(*ast.KeyValueExpr)
			if !ok {
				continue
			}
			field, ok := pair.Key.(*ast.Ident)
			if !ok {
				continue
			}
			value, ok := pair.Value.(*ast.CompositeLit)
			if !ok {
				continue
			}
			switch field.Name {
			case "RegexPatterns":
				for _, item := range value.Elts {
					patterns = append(patterns, literalString(item))
				}
			case "Timezones":
				for _, item := range value.Elts {
					names = append(names, literalString(item.(*ast.KeyValueExpr).Key))
				}
			case "AlternativePatterns":
				for _, item := range value.Elts {
					alternative := item.(*ast.KeyValueExpr)
					call := alternative.Key.(*ast.CallExpr)
					alternatives = append(alternatives, [2]string{literalString(call.Args[0]), literalString(alternative.Value)})
				}
			}
		}
		if len(patterns) == 0 || len(names) == 0 {
			return true
		}
		emit := func(name, pattern string) {
			location, err := timezone.Load(name)
			if err != nil {
				panic(err)
			}
			_, offset := time.Unix(0, 0).In(location).Zone()
			normalized, err := syntax.Parse("(?i)"+pattern, syntax.Perl)
			if err != nil {
				panic(err)
			}
			var validate func(*syntax.Regexp)
			validate = func(expression *syntax.Regexp) {
				if expression.Op == syntax.OpWordBoundary || expression.Op == syntax.OpNoWordBoundary {
					panic("timezone pattern needs explicit ASCII word-boundary conversion")
				}
				for _, child := range expression.Sub {
					validate(child)
				}
			}
			validate(normalized)
			fmt.Fprintf(output, "    (%s, %d, %s),\n", rustString(name), offset, rustString(normalized.String()))
		}
		for _, pattern := range patterns {
			for _, name := range names {
				search := regexp.QuoteMeta(name)
				searchPatterns[search] = struct{}{}
				emit(name, fmt.Sprintf(pattern, search))
			}
			for _, alternative := range alternatives {
				replacer := regexp.MustCompile(alternative[0])
				for _, name := range names {
					search := replacer.ReplaceAllString(regexp.QuoteMeta(name), alternative[1])
					searchPatterns[search] = struct{}{}
					emit(name, fmt.Sprintf(pattern, search))
				}
			}
		}
		return false
	})
	output.WriteString("];\n")
	searches := make([]string, 0, len(searchPatterns))
	for search := range searchPatterns {
		searches = append(searches, search)
	}
	slices.Sort(searches)
	fmt.Fprintf(output, "\npub(crate) const TOKEN_PATTERN: &str =\n    %s;\n", rustString(rustPattern("(?i)^(?:"+strings.Join(searches, "|")+")$")))
}
