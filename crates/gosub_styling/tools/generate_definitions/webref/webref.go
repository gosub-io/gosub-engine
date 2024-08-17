package webref

import (
	"encoding/json"
	"errors"
	"generate_definitions/utils"
	"io"
	"log"
	"net/http"
	"os"
	"path"
	"regexp"
	"strings"
	"sync"
)

var (
	skipList []string
)

type Data struct {
	Spec       Spec               `json:"spec"`
	Properties []WebRefProperties `json:"properties"`
	Values     []WebRefValue      `json:"values"`
	AtRules    []WebRefAtRule     `json:"atrules"`
	Selectors  []utils.Selector   `json:"selectors"`
}

type ParseData struct {
	Properties map[string]WebRefProperties
	Values     map[string]WebRefValue
	AtRules    map[string]WebRefAtRule
	Selectors  map[string]utils.Selector
}

type Spec struct {
	Title string `json:"title"`
	Url   string `json:"url"`
}

type WebRefValue struct {
	Name   string        `json:"name"`
	Syntax string        `json:"value"`
	Type   string        `json:"type"`   // Type of the value
	Values []WebRefValue `json:"values"` // Additional accompanied values
}

type WebRefProperties struct {
	Name      string                 `json:"name"`
	Syntax    string                 `json:"value"`
	NewSyntax string                 `json:"newValues"`
	Computed  []string               `json:"computed"`
	Initial   utils.StringMaybeArray `json:"initial"`
	Inherited string                 `json:"inherited"`
	Values    []WebRefValue          `json:"values"` // Additional accompanied values for this property
}

type WebRefAtRule struct {
	Name        string `json:"name"`
	Descriptors []struct {
		Name    string `json:"name"`
		Syntax  string `json:"value"`
		Initial string `json:"initial"`
	} `json:"descriptors"`
	Syntax string `json:"value"`
	Values []struct {
		Name   string `json:"name"`
		Value  string `json:"value,omitempty"`
		Values []struct {
			Name  string `json:"name"`
			Value string `json:"value"`
		}
	}
}

func GetWebRefFiles() []utils.DirectoryListItem {
	filesResp, err := http.Get("https://api.github.com/repos/" + utils.REPO + "/contents/" + utils.LOCATION + "?ref=" + utils.BRANCH)
	if err != nil {
		log.Panic(err)
	}

	defer filesResp.Body.Close()

	body, err := io.ReadAll(filesResp.Body)
	if err != nil {
		log.Panic(err)
	}

	var files []utils.DirectoryListItem
	if err := json.Unmarshal(body, &files); err != nil {
		log.Panic(err)
	}

	return files
}

func GetWebRefData() Data {
	//DownloadPatches() This is no longer needed
	files := GetWebRefFiles()

	wg := new(sync.WaitGroup)

	//s := specs2.GetSpecifications()

	parseData := ParseData{
		Properties: make(map[string]WebRefProperties),
		Values:     make(map[string]WebRefValue),
		AtRules:    make(map[string]WebRefAtRule),
		Selectors:  make(map[string]utils.Selector),
	}

	mu := new(sync.Mutex)

	for _, file := range files {
		if file.Type != "file" || !strings.HasSuffix(file.Name, ".json") {
			continue
		}

		wg.Add(1)
		go func() {
			defer wg.Done()
			content := DownloadFileContent(&file)
			if content == nil {
				return
			}

			shortname := strings.TrimSuffix(file.Name, ".json")
			if matched, err := regexp.Match(`\d+$`, []byte(shortname)); err != nil || matched {
				log.Println("Not matched our regexp: ", shortname)
				return
			}

			if !skip(shortname) {
				log.Println("Skipping non-W3C spec", shortname)
				return
			}

			mu.Lock()
			defer mu.Unlock()

			DecodeFileContent(content, &parseData)
		}()
	}

	wg.Wait()

	return Data{
		Properties: utils.MapToSlice(parseData.Properties),
		Values:     utils.MapToSlice(parseData.Values),
		AtRules:    utils.MapToSlice(parseData.AtRules),
		Selectors:  utils.MapToSlice(parseData.Selectors),
	}
}

func GetFileContent(file *utils.DirectoryListItem) ([]byte, error) {
	patchedPath := path.Join(utils.CACHE_DIR, "patched", file.Name)
	content, err := os.ReadFile(patchedPath)
	if err == nil {
		return content, nil
	}
	if !os.IsNotExist(err) {
		return nil, err
	}

	cachePath := path.Join(utils.CACHE_DIR, "specs2", file.Name)
	content, err = os.ReadFile(cachePath)
	if err != nil {
		return nil, err
	}

	return content, nil
}

func DownloadFileContent(file *utils.DirectoryListItem) []byte {
	cachePath := path.Join(utils.CACHE_DIR, "specs", file.Name)

	hash := ""
	content, err := os.ReadFile(cachePath)
	unexist := false
	if os.IsNotExist(err) {
		content = []byte{}
		unexist = true
	} else if err != nil {
		return nil
	} else {
		hash = utils.ComputeGitBlobSHA1Content(content)
	}

	if unexist || hash != file.Sha {
		log.Println("Cache file is outdated, downloading", file.Path)
		resp, err := http.Get(file.DownloadUrl)
		if err != nil {
			log.Panic(file.Path, " ", err)
		}

		body, err := io.ReadAll(resp.Body)
		resp.Body.Close()
		if err != nil {
			return nil
		}

		if err := os.MkdirAll(path.Dir(cachePath), 0755); err != nil {
			log.Panic("Failed to create cache directory ", path.Dir(cachePath), err)
		}

		if err := os.WriteFile(cachePath, body, 0644); err != nil {
			log.Panic("Failed to write cache file ", cachePath, err)
		}

		return body
	}

	return content
}

func DownloadPatches() {
	patchesResp, err := http.Get("https://api.github.com/repos/" + utils.REPO + "/contents/" + utils.PATCH_LOCATION)
	if err != nil {
		log.Panic(err)
	}

	defer patchesResp.Body.Close()

	body, err := io.ReadAll(patchesResp.Body)
	if err != nil {
		log.Panic(err)
	}

	var patches []utils.DirectoryListItem
	if err := json.Unmarshal(body, &patches); err != nil {
		log.Println(string(body))
		log.Panic(err)
	}

	for _, p := range patches {
		if p.Type != "file" || !strings.HasSuffix(p.Name, ".patch") {
			continue
		}

		patchPath := path.Join(utils.CACHE_DIR, "patches", p.Name)
		content, err := os.ReadFile(patchPath)
		if err == nil {
			hash := utils.ComputeGitBlobSHA1Content(content)
			if hash == p.Sha {
				continue
			}
		}

		resp, err := http.Get(p.DownloadUrl)
		if err != nil {
			log.Panic(err)
		}

		body, err := io.ReadAll(resp.Body)
		resp.Body.Close()
		if err != nil {
			log.Panic(err)
		}

		if err := os.WriteFile(patchPath, body, 0644); err != nil {
			log.Panic(err)
		}
	}
}

func DetectDuplicates(data *Data) {
	properties := make(map[string]WebRefProperties)
	values := make(map[string]WebRefValue)
	atRules := make(map[string]WebRefAtRule)
	selectors := make(map[string]utils.Selector)

	for _, property := range data.Properties {
		if p, ok := properties[property.Name]; ok {
			if p.Syntax != property.Syntax {
				log.Println("Different syntax for duplicated property", property.Name)
				log.Println("Old:", p.Syntax)
				log.Println("New:", property.Syntax)
			}
		}
		properties[property.Name] = property
	}

	for _, value := range data.Values {
		if _, ok := values[value.Name]; ok {
			log.Println("Duplicate value", value.Name)
		}
		values[value.Name] = value
	}

	for _, atRule := range data.AtRules {
		if _, ok := atRules[atRule.Name]; ok {
			log.Println("Duplicate at-rule", atRule.Name)
		}
		atRules[atRule.Name] = atRule
	}

	for _, selector := range data.Selectors {
		if _, ok := selectors[selector.Name]; ok {
			log.Println("Duplicate selector", selector.Name)
		}
		selectors[selector.Name] = selector
	}
}

func skip(shortname string) bool {
	for _, s := range skipList {
		if s == shortname {
			return false
		}
	}
	return true
}

// ProcessValue will process a single value (from either root values or property values) and add it
// to the ParseData if possible.
func ProcessValue(name string, type_ string, syntax string, pd *ParseData) {
	if name == syntax {
		// log.Println("name == syntax for ", name)
		return
	}

	// If value already exists, update the syntax if possible
	if v, ok := pd.Values[name]; ok {
		if v.Syntax == "" {
			v.Syntax = syntax
		}

		// Skip built-in values ie: (<integer> has syntax "<integer>", which results in a loop when resolving)
		if v.Syntax == v.Name {
			log.Println("name == syntax, skipping as this is an built-in value: ", name)
			return
		}

		// Not all values have the same syntax. It can change. We ignore this and get the latest one
		if v.Syntax != "" && syntax != "" && v.Syntax != syntax {
			log.Println("Different syntax for duplicated value", name)
			log.Println("Old:", v.Syntax)
			log.Println("New:", syntax)
			//log.Panic("Syntax mismatch")
		}

		pd.Values[name] = v
		return
	}

	// Values are always skipped
	if type_ == "value" {
		println("value type. Skipping: ", name)
		return
	}

	// Skip <integer> = syntax("<integer>")
	if syntax == name {
		println("value==name. Skipping: ", name)
		return
	}

	if syntax == "" {
		log.Println("empty value/syntax: ", name)
		return
	}

	pd.Values[name] = WebRefValue{
		Name:   name,
		Syntax: syntax,
	}
}

func DecodeFileContent(content []byte, pd *ParseData) {
	var fileData Data
	if err := json.Unmarshal(content, &fileData); err != nil {
		log.Panic(err)
	}

	for _, property := range fileData.Properties {
		if property.Name == "stop-color" || property.Name == "stop-opacity" {
			// Some hardcoded skips
			return
		}

		for _, v := range property.Values {
			ProcessValue(v.Name, v.Type, v.Syntax, pd)
			ProcessExtraValues(v.Values, pd)
		}

		if p, ok := pd.Properties[property.Name]; ok {
			if p.Syntax == "" {
				p.Syntax = property.Syntax
			} else if p.Syntax != property.Syntax && property.Syntax != "" {
				log.Println("Different syntax for duplicated property", property.Name)
				log.Println("Old:", p.Syntax)
				log.Println("New:", property.Syntax)
				//log.Panic("Syntax mismatch")
			}

			if p.NewSyntax != "" && p.Syntax != "" {
				p.Syntax += " | " + p.NewSyntax
				p.NewSyntax = ""
			}

			if property.NewSyntax != "" {
				if p.Syntax != "" {
					p.Syntax += " | " + property.NewSyntax
				} else if p.NewSyntax != "" {
					p.NewSyntax += " | " + property.NewSyntax
				} else {
					p.NewSyntax = property.NewSyntax
				}
			}

			pd.Properties[p.Name] = p
			continue
		}

		pd.Properties[property.Name] = property
	}

	ProcessExtraValues(fileData.Values, pd)

	for _, atRule := range fileData.AtRules {
		if a, ok := pd.AtRules[atRule.Name]; ok {
			if a.Syntax == "" {
				a.Syntax = atRule.Syntax
			}

			if a.Syntax != "" && atRule.Syntax != "" && a.Syntax != atRule.Syntax {
				log.Println("Different syntax for duplicated at-rule", atRule.Name)
				log.Println("Old:", a.Syntax)
				log.Println("New:", atRule.Syntax)
				//log.Panic("Syntax mismatch")
			}

			a.Values = append(a.Values, atRule.Values...)
			a.Descriptors = append(a.Descriptors, atRule.Descriptors...)

			pd.AtRules[a.Name] = a
			continue
		}
		pd.AtRules[atRule.Name] = atRule
	}

	for _, selector := range fileData.Selectors {
		pd.Selectors[selector.Name] = selector
	}
}

func ProcessExtraValues(values []WebRefValue, pd *ParseData) {
	for _, value := range values {
		ProcessValue(value.Name, value.Type, value.Syntax, pd)
		ProcessExtraValues(value.Values, pd)
	}
}

var PropertyAliasTable = map[string]string{
	"-webkit-align-content":              "align-content",
	"-webkit-align-items":                "align-items",
	"-webkit-align-self":                 "align-self",
	"-webkit-animation":                  "animation",
	"-webkit-animation-delay":            "animation-delay",
	"-webkit-animation-direction":        "animation-direction",
	"-webkit-animation-duration":         "animation-duration",
	"-webkit-animation-fill-mode":        "animation-fill-mode",
	"-webkit-animation-iteration-count":  "animation-iteration-count",
	"-webkit-animation-name":             "animation-name",
	"-webkit-animation-play-state":       "animation-play-state",
	"-webkit-animation-timing-function":  "animation-timing-function",
	"-webkit-appearance":                 "appearance",
	"-webkit-backface-visibility":        "backface-visibility",
	"-webkit-background-clip":            "background-clip",
	"-webkit-background-origin":          "background-origin",
	"-webkit-background-size":            "background-size",
	"-webkit-border-bottom-left-radius":  "border-bottom-left-radius",
	"-webkit-border-bottom-right-radius": "border-bottom-right-radius",
	"-webkit-border-radius":              "border-radius",
	"-webkit-border-top-left-radius":     "border-top-left-radius",
	"-webkit-border-top-right-radius":    "border-top-right-radius",
	"-webkit-box-align":                  "box-align",
	"-webkit-box-flex":                   "box-flex",
	"-webkit-box-ordinal-group":          "box-ordinal-group",
	"-webkit-box-orient":                 "box-orient",
	"-webkit-box-pack":                   "box-pack",
	"-webkit-box-shadow":                 "box-shadow",
	"-webkit-box-sizing":                 "box-sizing",
	"-webkit-filter":                     "filter",
	"-webkit-flex":                       "flex",
	"-webkit-flex-basis":                 "flex-basis",
	"-webkit-flex-direction":             "flex-direction",
	"-webkit-flex-flow":                  "flex-flow",
	"-webkit-flex-grow":                  "flex-grow",
	"-webkit-flex-shrink":                "flex-shrink",
	"-webkit-flex-wrap":                  "flex-wrap",
	"-webkit-justify-content":            "justify-content",
	"-webkit-mask":                       "mask",
	"-webkit-mask-box-image":             "mask-border",
	"-webkit-mask-box-image-outset":      "mask-border-outset",
	"-webkit-mask-box-image-repeat":      "mask-border-repeat",
	"-webkit-mask-box-image-slice":       "mask-border-slice",
	"-webkit-mask-box-image-source":      "mask-border-source",
	"-webkit-mask-box-image-width":       "mask-border-width",
	"-webkit-mask-clip":                  "mask-clip",
	"-webkit-mask-composite":             "mask-composite",
	"-webkit-mask-image":                 "mask-image",
	"-webkit-mask-origin":                "mask-origin",
	"-webkit-mask-position":              "mask-position",
	"-webkit-mask-repeat":                "mask-repeat",
	"-webkit-mask-size":                  "mask-size",
	"-webkit-order":                      "order",
	"-webkit-perspective":                "perspective",
	"-webkit-perspective-origin":         "perspective-origin",
	"-webkit-text-size-adjust":           "text-size-adjust",
	"-webkit-transform":                  "transform",
	"-webkit-transform-origin":           "transform-origin",
	"-webkit-transform-style":            "transform-style",
	"-webkit-transition":                 "transition",
	"-webkit-transition-delay":           "transition-delay",
	"-webkit-transition-duration":        "transition-duration",
	"-webkit-transition-property":        "transition-property",
	"-webkit-transition-timing-function": "transition-timing-function",
	"-webkit-user-select":                "user-select",
	"font-stretch":                       "font-width",
	"grid-column-gap":                    "column-gap",
	"grid-gap":                           "gap",
	"grid-row-gap":                       "row-gap",
}

func GetAlias(propName string) (string, error) {
	if PropertyAliasTable[propName] == "" {
		return "", errors.New("No alias syntax for property: " + propName)
	}

	return PropertyAliasTable[propName], nil
}
