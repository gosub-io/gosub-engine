package main

import (
	"generate_definitions/webref"
)

type ExportType int

const (
	Both ExportType = iota
	SingleFile
	MultiFile
)

func main() {
	webrefData := webref.GetWebRefData()

	webref.DetectDuplicates(&webrefData)

	//mdnData := getMdnData()

	//for _, property := range webrefData.Properties {
	//	if property.Name == "margin" {
	//		log.Println(property.Syntax)
	//	}
	//}

}
