package mdn

import (
	"encoding/json"
	"generate_definitions/utils"
	"io"
	"log"
	"net/http"
)

const (
	MDN_PROPERTIES = "https://raw.githubusercontent.com/mdn/data/main/css/properties.json"
)

type MdnItem struct {
	Syntax   string                 `json:"syntax"`
	Initial  utils.StringMaybeArray `json:"initial"`
	Computed utils.StringMaybeArray `json:"computed"`
}

func GetMdnData() map[string]MdnItem {
	mdnResp, err := http.Get(MDN_PROPERTIES) //TODO: this should probably also be cached
	if err != nil {
		log.Panic(err)
	}

	defer mdnResp.Body.Close()

	body, err := io.ReadAll(mdnResp.Body)
	if err != nil {
		log.Panic(err)
	}

	var mdnData map[string]MdnItem

	if err := json.Unmarshal(body, &mdnData); err != nil {
		log.Panic(err)
	}

	return mdnData
}
