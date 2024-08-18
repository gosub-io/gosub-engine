package main

import (
	"bytes"
	"encoding/json"
	"generate_definitions/mdn"
	"generate_definitions/utils"
	"generate_definitions/webref"
	"log"
	"os"
	"path"
	"sort"
)

type ExportType int

const (
	Both ExportType = iota
	SingleFile
	MultiFile
)

const (
	exportType      = Both
	ResourcePath    = ".output"
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

	sort.Slice(webrefData.Properties, func(i, j int) bool {
		return webrefData.Properties[i].Name < webrefData.Properties[j].Name
	})

	for _, property := range webrefData.Properties {
		if property.Syntax == "" {
			// Skip any empty syntax, as they will be filled later with alias table
			continue
		}

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

		for _, property := range webrefData.Properties {
			if property.Syntax != "" {
				// Skip properties with syntax
				continue
			}

			alias, err := webref.GetAlias(property.Name)
			if err != nil {
				log.Panic("failed to get alias syntax for property: " + property.Name)
			}

			data.PropAliases = append(data.PropAliases, utils.PropAlias{
				Name: property.Name,
				For:  alias,
			})
		}
	}

	data.Selectors = webrefData.Selectors

	log.Printf(
		"Collected data: %d properties, %d values, %d at-rules, %d selectors",
		len(data.Properties), len(data.Values), len(data.AtRules), len(data.Selectors),
	)

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
	ExportData(data.PropAliases, path.Join(MultiFileDir, MultiFilePrefix+"prop-aliases.json"))

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
