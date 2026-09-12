package main

import (
	"crypto/sha256"
	"fmt"
	"go/ast"
	"go/parser"
	"go/token"
	"os"
	"path/filepath"
	"slices"
	"strconv"
	"strings"

	"github.com/markusmobius/go-dateparser/internal/data"
	"github.com/markusmobius/go-dateparser/internal/digit"
	"github.com/markusmobius/go-dateparser/internal/language"
	"github.com/markusmobius/go-dateparser/internal/setting"
	"github.com/markusmobius/go-dateparser/internal/strutil"
)

type languageCase struct {
	Locale            string   `json:"locale"`
	Input             string   `json:"input"`
	KeepFormatting    bool     `json:"keep_formatting"`
	IgnoreSurrounding bool     `json:"ignore_surrounding"`
	SkipTokens        []string `json:"skip_tokens"`
	Normalized        string   `json:"normalized"`
	Simplified        string   `json:"simplified"`
	Tokens            []string `json:"tokens"`
	Applicable        bool     `json:"applicable"`
	Translations      []string `json:"translations"`
}

func languageCases(moduleDir string, provenance *reference) []languageCase {
	contents, err := os.ReadFile(filepath.Join(moduleDir, "internal", "language", "translate_test.go"))
	if err != nil {
		panic(err)
	}
	provenance.LanguageTestsSHA256 = fmt.Sprintf("%x", sha256.Sum256(contents))
	source, err := parser.ParseFile(token.NewFileSet(), "translate_test.go", contents, 0)
	if err != nil {
		panic(err)
	}
	pairs := [][2]string{}
	ast.Inspect(source, func(node ast.Node) bool {
		literal, ok := node.(*ast.CompositeLit)
		if !ok || len(literal.Elts) != 3 {
			return true
		}
		values := []string{}
		for _, element := range literal.Elts {
			value, ok := element.(*ast.BasicLit)
			if !ok || value.Kind != token.STRING {
				return true
			}
			decoded, err := strconv.Unquote(value.Value)
			if err != nil {
				panic(err)
			}
			values = append(values, decoded)
		}
		if _, exists := data.GetLocaleData(values[0]); exists {
			pairs = append(pairs, [2]string{values[0], values[1]})
		}
		return false
	})
	names := make([]string, 0, len(data.LocaleOrder))
	for name := range data.LocaleOrder {
		names = append(names, name)
	}
	slices.Sort(names)
	for _, name := range names {
		locale, _ := data.GetLocaleData(name)
		words := make([]string, 0, len(locale.Translations))
		for word := range locale.Translations {
			words = append(words, word)
		}
		slices.Sort(words)
		for _, word := range words {
			if slices.Contains(locale.Translations[word], "january") {
				pairs = append(pairs, [2]string{name, "14 " + word + " 2024"})
				break
			}
		}
		pairs = append(pairs, [2]string{name, "2024-02-29 12:34:56 +0530"})
	}
	pairs = append(pairs, [][2]string{
		{"en", "published: 14 January 2024 UTC footer"},
		{"en", "published: 14 January 2024 UTC"},
		{"en", "14 January 2024\u00a0"},
		{"en", "\u0662\u0660\u0662\u0664-\u0660\u0662-\u0662\u0669"},
		{"ru", "двадцать первого января 2024"},
		{"fi", "2 t sitten"},
	}...)
	seen := map[[2]string]bool{}
	results := []languageCase{}
	for _, pair := range pairs {
		if seen[pair] {
			continue
		}
		seen[pair] = true
		locale, _ := data.GetLocaleData(pair[0])
		for _, formatting := range []bool{false, true} {
			for _, ignore := range []bool{false, true} {
				cfg := &setting.Configuration{SkipTokens: []string{"t"}}
				normalized := strutil.NormalizeString(pair[1])
				simplified := language.Simplify(locale, digit.NormalizeString(strings.ToLower(strutil.NormalizeUnicode(pair[1]))))
				skipped := strutil.NewDict(cfg.SkipTokens...)
				if locale.Name == "fi" {
					skipped.Remove("t")
				}
				tokens := language.Split(locale, simplified, formatting, skipped)
				if tokens == nil {
					tokens = []string{}
				}
				translations := language.Translate(cfg, locale, pair[1], formatting, ignore)
				if translations == nil {
					translations = []string{}
				}
				results = append(results, languageCase{
					Locale: pair[0], Input: pair[1], KeepFormatting: formatting, IgnoreSurrounding: ignore, SkipTokens: cfg.SkipTokens,
					Normalized: normalized, Simplified: simplified, Tokens: tokens,
					Applicable: language.IsApplicable(cfg, locale, pair[1], false, ignore), Translations: translations,
				})
			}
		}
	}
	fmt.Printf("Generated %d locale translation cases from upstream examples and all locales\n", len(results))
	return results
}
