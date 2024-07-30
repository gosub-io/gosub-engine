package webref

import (
	"encoding/json"
	"generate_definitions/patch"
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
	skipList = []string{
		"css-borders", // not ready for impl
		"CSS",         // temp fix for duplicate properties
		"css-flexbox",
		"SVG",
		"svg-strokes",
		"css-position",
		"css-color-hdr",
		"css-content",
	}
)

type Data struct {
	Spec       Spec               `json:"spec"`
	Properties []WebRefProperties `json:"properties"`
	Values     []WebRefValue      `json:"values"`
	AtRules    []WebRefAtRule     `json:"atrules"`
	Selectors  []utils.Selector   `json:"selectors"`
}

type Spec struct {
	Title string `json:"title"`
	Url   string `json:"url"`
}

type WebRefValue struct {
	Name   string `json:"name"`
	Syntax string `json:"value"`
}

type WebRefProperties struct {
	Name      string                 `json:"name"`
	Syntax    string                 `json:"value"`
	NewSyntax string                 `json:"newValues"`
	Computed  []string               `json:"computed"`
	Initial   utils.StringMaybeArray `json:"initial"`
	Inherited string                 `json:"inherited"`
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
	filesResp, err := http.Get("https://api.github.com/repos/" + utils.REPO + "/contents/" + utils.LOCATION)
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
	DownloadPatches()
	files := GetWebRefFiles()

	wg := new(sync.WaitGroup)

	//s := specs.GetSpecifications()

	properties := make(map[string]WebRefProperties)
	values := make(map[string]WebRefValue)
	atRules := make(map[string]WebRefAtRule)
	selectors := make(map[string]utils.Selector)

	for _, file := range files {
		if file.Type != "file" || !strings.HasSuffix(file.Name, ".json") {
			continue
		}

		wg.Add(1)
		go func() {
			defer wg.Done()
			DownloadFileContent(&file)
		}()
	}

	wg.Wait()

	if err := patch.PatchFiles(); err != nil {
		log.Panic(err)
	}

	for _, file := range files {
		shortname := strings.TrimSuffix(file.Name, ".json")
		if matched, err := regexp.Match(`\d+$`, []byte(shortname)); err != nil || matched {
			continue
		}

		if !skip(shortname) {
			log.Println("Skipping non-W3C spec", shortname)
			continue
		}

		content, err := GetFileContent(&file)
		if err != nil {
			log.Panic(file.Path, " ", err)
		}

		if string(content) == "<deleted>" {
			continue
		}

		var fileData Data
		if err := json.Unmarshal(content, &fileData); err != nil {
			log.Println("content", string(content))
			log.Panic(file.Path, " ", err)
		}

		for _, property := range fileData.Properties {
			if p, ok := properties[property.Name]; ok {
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

				properties[p.Name] = p
				continue
			}

			properties[property.Name] = property
		}

		for _, value := range fileData.Values {
			if v, ok := values[value.Name]; ok {
				if v.Syntax == "" {
					v.Syntax = value.Syntax
				}

				if v.Syntax != "" && value.Syntax != "" && v.Syntax != value.Syntax {
					log.Println("Different syntax for duplicated value", value.Name)
					log.Println("Old:", v.Syntax)
					log.Println("New:", value.Syntax)
					//log.Panic("Syntax mismatch")
				}

				values[v.Name] = v
				continue
			}
			values[value.Name] = value
		}

		for _, atRule := range fileData.AtRules {
			if a, ok := atRules[atRule.Name]; ok {
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

				atRules[a.Name] = a
				continue
			}
			atRules[atRule.Name] = atRule
		}

		for _, selector := range fileData.Selectors {
			selectors[selector.Name] = selector
		}
	}

	return Data{
		Properties: utils.MapToSlice(properties),
		Values:     utils.MapToSlice(values),
		AtRules:    utils.MapToSlice(atRules),
		Selectors:  utils.MapToSlice(selectors),
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

	cachePath := path.Join(utils.CACHE_DIR, "specs", file.Name)
	content, err = os.ReadFile(cachePath)
	if err != nil {
		return nil, err
	}

	return content, nil
}

func DownloadFileContent(file *utils.DirectoryListItem) {
	cachePath := path.Join(utils.CACHE_DIR, "specs", file.Name)

	hash := ""
	content, err := os.ReadFile(cachePath)
	unexist := false
	if os.IsNotExist(err) {
		content = []byte{}
		unexist = true
	} else if err != nil {
		return
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
			return
		}

		if err := os.WriteFile(cachePath, body, 0644); err != nil {
			log.Panic("Failed to write cache file ", cachePath, err)
		}
	}
}

func DownloadPatches() {
	log.Println("Skipping patch download")
	return
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
