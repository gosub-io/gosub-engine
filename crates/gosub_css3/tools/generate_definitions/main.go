package main

import (
	"bytes"
	"cmp"
	"encoding/json"
	"generate_definitions/mdn"
	"generate_definitions/utils"
	"generate_definitions/webref"
	"log"
	"os"
	"path"
	"slices"
)

type ExportType int

const (
	Both ExportType = iota
	SingleFile
	MultiFile
)

const (
	exportType      = Both
	ResourcePath    = ".output/definitions"
	SingleFilePath  = ResourcePath + "/definitions.json"
	MultiFileDir    = ResourcePath
	MultiFilePrefix = "definitions_"
)

func main() {
	webrefData := webref.GetWebRefData()

	webref.DetectDuplicates(&webrefData)

	mdnData := mdn.GetMdnData()

	var data utils.Data

	log.Printf(
		"Webref data: %d properties, %d values, %d at-rules, %d selectors",
		len(webrefData.Properties), len(webrefData.Values), len(webrefData.AtRules), len(webrefData.Selectors),
	)

	for _, property := range webrefData.Properties {
		prop := utils.Property{
			Name:      property.Name,
			Syntax:    property.Syntax,
			Computed:  []string{},
			Initial:   property.Initial,
			Inherited: property.Inherited == "yes",
		}

		mdnProp, ok := mdnData[property.Name]
		if ok {
			if len(mdnProp.Computed.Array) == 0 {
				prop.Computed = []string{mdnProp.Computed.String}
			} else {
				prop.Computed = mdnProp.Computed.Array
			}

			prop.Initial = mdnProp.Initial
		}

		data.Properties = append(data.Properties, prop)
	}

	for _, value := range webrefData.Values {
		data.Values = append(data.Values, utils.Value{
			Name:   value.Name,
			Syntax: value.Syntax,
		})
	}

	for _, atRule := range webrefData.AtRules {
		descriptors := make([]utils.AtRuleDescriptor, len(atRule.Descriptors))

		for i, descriptor := range atRule.Descriptors {

			initial := descriptor.Initial
			{
				// remove "n/a" or "N/A" initial values
				if len(initial) >= 3 {
					if initial[0] == 'n' || initial[0] == 'N' &&
						initial[1] == '/' &&
						initial[2] == 'a' || initial[2] == 'A' {
						initial = ""
					}
				}
			}

			descriptors[i] = utils.AtRuleDescriptor{
				Name:    descriptor.Name,
				Syntax:  descriptor.Syntax,
				Initial: initial,
			}
		}

		data.AtRules = append(data.AtRules, utils.AtRule{
			Name:        atRule.Name,
			Descriptors: descriptors,
			Values:      atRule.Values,
		})
	}

	data.Selectors = webrefData.Selectors

	log.Printf(
		"Collected data: %d properties, %d values, %d at-rules, %d selectors",
		len(data.Properties), len(data.Values), len(data.AtRules), len(data.Selectors),
	)

	// Sort elements, so that the output is deterministic and we have less issues with version control
	slices.SortFunc(data.Properties, func(a, b utils.Property) int {
		return cmp.Compare(a.Name, b.Name)
	})
	slices.SortFunc(data.Values, func(a, b utils.Value) int {
		return cmp.Compare(a.Name, b.Name)
	})
	slices.SortFunc(data.AtRules, func(a, b utils.AtRule) int {
		return cmp.Compare(a.Name, b.Name)
	})
	slices.SortFunc(data.Selectors, func(a, b utils.Selector) int {
		return cmp.Compare(a.Name, b.Name)
	})

	switch exportType {
	case SingleFile:
		ExportSingleFile(&data)
		break

	case MultiFile:
		ExportMultiFile(&data)
		break

	case Both:
		ExportMultiFile(&data)
		ExportSingleFile(&data)
		break
	}
}

func ExportSingleFile(data *utils.Data) {
	ExportData(data, SingleFilePath)
}

func ExportMultiFile(data *utils.Data) {
	if _, err := os.Stat(MultiFileDir); os.IsNotExist(err) {
		if err := os.MkdirAll(MultiFileDir, 0755); err != nil {
			log.Panic(err)
		}
	}

	ExportData(data.Properties, path.Join(MultiFileDir, MultiFilePrefix+"properties.json"))
	ExportData(data.Values, path.Join(MultiFileDir, MultiFilePrefix+"values.json"))
	ExportData(data.AtRules, path.Join(MultiFileDir, MultiFilePrefix+"at-rules.json"))
	ExportData(data.Selectors, path.Join(MultiFileDir, MultiFilePrefix+"selectors.json"))
}

func ExportData(data any, path string) {
	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	enc.SetEscapeHTML(false)
	enc.SetIndent("", "  ")

	if err := enc.Encode(data); err != nil {
		log.Panic(err)
	}

	if err := os.WriteFile(path, buf.Bytes(), 0644); err != nil {
		log.Panic(err)
	}
}
