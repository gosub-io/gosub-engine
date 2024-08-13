package specs

import (
	"encoding/json"
	"generate_definitions/utils"
	"io"
	"log"
	"net/http"
	"os"
)

const (
	SPEC_INDEX = "ed/index.json"
	CACHE_FILE = utils.CACHE_DIR + "/specs2/index.json"
)

type W3CSpec struct {
	Url       string `json:"url"`
	Shortname string `json:"shortname"`
	Series    struct {
		Shortname string `json:"shortname"`
	}
	Release struct {
		Url      string `json:"url"`
		Status   string `json:"status"`
		Filename string `json:"filename"`
	} `json:"release"`
	Title string `json:"title"`
}

type SpecList struct {
	Specs map[string]struct{}
}

func (s *SpecList) Contains(spec string) bool {
	_, ok := s.Specs[spec]
	return ok
}

func GetSpecifications() SpecList {
	specList := SpecList{
		Specs: make(map[string]struct{}),
	}

	specFile, err := GetSpecFileContent()
	if err != nil {
		log.Panic(err)
	}

	var spec struct {
		Results []W3CSpec `json:"results"`
	}

	if err := json.Unmarshal(specFile, &spec); err != nil {
		log.Panic(err)
	}

	for _, spec := range spec.Results {
		if spec.Release.Status == "" || spec.Release.Url == "" || spec.Release.Filename == "" {
			continue
		}

		specList.Specs[spec.Shortname] = struct{}{}
		specList.Specs[spec.Series.Shortname] = struct{}{}
	}

	return specList
}

func GetSpecFileContent() ([]byte, error) {
	if content, err := os.ReadFile(CACHE_FILE); err == nil {
		resp, err := http.Get("https://api.github.com/repos/w3c/webref/contents/" + SPEC_INDEX)
		if err != nil {
			return nil, err
		}

		defer resp.Body.Close()

		body, err := io.ReadAll(resp.Body)
		if err != nil {
			return nil, err
		}

		var item utils.DirectoryListItem

		if err := json.Unmarshal(body, &item); err != nil {
			return nil, err
		}

		sha := utils.ComputeGitBlobSHA1Content(body)

		if sha == item.Sha {
			return content, nil
		}
	}

	downloadUrl := "https://raw.githubusercontent.com/w3c/webref/main/" + SPEC_INDEX

	resp, err := http.Get(downloadUrl)
	if err != nil {
		return nil, err
	}

	defer resp.Body.Close()

	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return nil, err
	}

	if err := os.WriteFile(CACHE_FILE, body, 0644); err != nil {
		return nil, err
	}

	return body, nil
}
