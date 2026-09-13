package main

import (
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"reflect"
	"runtime/debug"
	"time"

	dps "github.com/markusmobius/go-dateparser"
	"github.com/markusmobius/go-dateparser/date"
	"github.com/markusmobius/go-dateparser/internal/parser/absolute"
	"github.com/markusmobius/go-dateparser/internal/parser/formatted"
	"github.com/markusmobius/go-dateparser/internal/parser/nospace"
	"github.com/markusmobius/go-dateparser/internal/parser/relative"
	"github.com/markusmobius/go-dateparser/internal/parser/timestamp"
	"github.com/markusmobius/go-dateparser/internal/timezone"
)

var correctionsDateutilVersion = "v2.9.0"

func exportCorrections(source, output string) {
	source, err := filepath.Abs(source)
	if err != nil {
		panic(err)
	}
	info, ok := debug.ReadBuildInfo()
	if !ok || info.GoVersion != "go1.27.1" {
		panic("unverified Go toolchain")
	}
	var dateutilSum string
	switch correctionsDateutilVersion {
	case "v2.9.0":
		dateutilSum = "h1:5XFuxvOZNPUAlx50mQZTSJPPAmk030dm6qz9WkO5Fms="
	case "v2.9.1":
		dateutilSum = "h1:5g4V8s1vg/EAmhu/g14VQcHgocNf8H8VD4IzOfZg+1Y="
	default:
		panic("unverified Dateutil version")
	}
	verifiedSource, verifiedDateutil := false, false
	for _, dependency := range info.Deps {
		switch dependency.Path {
		case "github.com/markusmobius/go-dateparser":
			verifiedSource = dependency.Replace != nil && filepath.Clean(dependency.Replace.Path) == source
		case "github.com/markusmobius/go-dateutil/v2":
			verifiedDateutil = dependency.Replace == nil && dependency.Version == correctionsDateutilVersion && dependency.Sum == dateutilSum
		}
	}
	if !verifiedSource || !verifiedDateutil {
		panic("corrections require the explicit Go worktree and published Dateutil")
	}
	provenance := benchmarkSourceReference(pinnedReference("v1.4.5"))
	document := struct {
		Reference      reference                 `json:"reference"`
		CoreSHA256     string                    `json:"core_sha256"`
		FeaturesSHA256 string                    `json:"features_sha256"`
		Core           map[string]result         `json:"core"`
		Features       map[string][]featureMatch `json:"features"`
	}{Reference: provenance, Core: map[string]result{}, Features: map[string][]featureMatch{}}
	contents, err := os.ReadFile("../../testdata/go-core.json")
	if err != nil {
		panic(err)
	}
	document.CoreSHA256 = fmt.Sprintf("%x", sha256.Sum256(contents))
	var core fixture
	if err := json.Unmarshal(contents, &core); err != nil {
		panic(err)
	}
	sessions := map[string]*dps.Parser{}
	for _, test := range core.Cases {
		var parsed date.Date
		var parseError error
		switch test.Stage {
		case "timestamp":
			parsed = timestamp.Parse(test.Configuration.internal(), test.Input, test.Negative)
			parsed.Time = parsed.Time.UTC()
		case "relative":
			parsed = relative.Parse(test.Configuration.internal(), test.Input)
		case "absolute":
			parsed, _ = absolute.Parse(test.Configuration.internal(), test.Input, timezone.OffsetData{})
		case "formatted":
			parsed = formatted.Parse(test.Configuration.internal(), test.Input, test.Formats...)
		case "nospace":
			cfg := test.Configuration.internal()
			cfg.DateOrder = test.Configuration.DateOrder
			parsed, _ = nospace.Parse(cfg, test.Input)
		case "public":
			engine := &dps.Parser{}
			if test.Session != "" {
				if existing := sessions[test.Session]; existing != nil {
					engine = existing
				} else {
					sessions[test.Session] = engine
				}
			}
			for _, kind := range test.ParserTypes {
				engine.ParserTypes = append(engine.ParserTypes, dps.ParserType(kind))
			}
			if test.DetectorLanguages != nil {
				engine.DetectLanguagesFunction = func(string) []string { return *test.DetectorLanguages }
			}
			parsed, parseError = engine.Parse(test.Configuration.public(), test.Input, test.Formats...)
		default:
			panic("unknown core stage")
		}
		actual := featureDate(parsed)
		if parseError != nil {
			actual.Error = parseError.Error()
		}
		if actual != test.Expected {
			document.Core[test.ID] = actual
		}
	}
	contents, err = os.ReadFile("../../testdata/go-features.json")
	if err != nil {
		panic(err)
	}
	document.FeaturesSHA256 = fmt.Sprintf("%x", sha256.Sum256(contents))
	var features struct {
		Cases []featureCase `json:"cases"`
	}
	if err := json.Unmarshal(contents, &features); err != nil {
		panic(err)
	}
	for _, test := range features.Cases {
		if test.Stage != "search" && test.Stage != "search_with_language" {
			continue
		}
		cfg := test.Configuration.public()
		if test.UnspecifiedCurrentTime {
			cfg.CurrentTime = time.Time{}
		}
		engine := &dps.Parser{}
		for _, kind := range test.ParserTypes {
			engine.ParserTypes = append(engine.ParserTypes, dps.ParserType(kind))
		}
		detectionInputs := []string{}
		if test.DetectorLanguages != nil {
			engine.DetectLanguagesFunction = func(input string) []string {
				detectionInputs = append(detectionInputs, input)
				return *test.DetectorLanguages
			}
		}
		var found []dps.SearchResult
		var detected string
		var parseError error
		if test.Stage == "search" {
			detected, found, parseError = engine.Search(cfg, test.Input)
		} else {
			found, parseError = engine.SearchWithLanguage(cfg, test.Language, test.Input)
		}
		errorText := ""
		if parseError != nil {
			errorText = parseError.Error()
		}
		if errorText != test.Error || detected != test.Detected || len(detectionInputs) != len(test.DetectionInputs) {
			panic("unexpected non-date feature change: " + test.ID)
		}
		for index, input := range detectionInputs {
			if input != test.DetectionInputs[index] {
				panic("detector input changed")
			}
		}
		matches := []featureMatch{}
		for _, match := range found {
			matches = append(matches, featureMatch{match.Text, featureDate(match.Date)})
		}
		if !reflect.DeepEqual(matches, test.Matches) {
			document.Features[test.ID] = matches
		}
	}
	encoded, err := json.MarshalIndent(document, "", "  ")
	if err != nil {
		panic(err)
	}
	if err := os.WriteFile(output, append(encoded, '\n'), 0644); err != nil {
		panic(err)
	}
	fmt.Printf("Recorded %d core and %d feature corrections from the reviewed Go worktree\n", len(document.Core), len(document.Features))
}
