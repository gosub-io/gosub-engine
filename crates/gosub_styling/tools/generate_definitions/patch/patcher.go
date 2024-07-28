package patch

import (
	"generate_definitions/utils"
	"os"
	"strings"
)

type PatchedFile struct {
	Name      string  `json:"name"`
	Sha       string  `json:"sha"`
	Patches   []Patch `json:"patches"`
	ResultSha string  `json:"resultSha"`
}

type Patch struct {
	Path string `json:"path"`
	Sha  string `json:"sha"`
}

func PatchFiles() ([]PatchedFile, error) {
	// TODO: we need to get the patchedFiles from the cache index, so we don't accidentally patch the same file twice

	var patchedFiles = make([]PatchedFile, 0)

	err := ApplyPatches(&patchedFiles, utils.CACHE_DIR+"/patches")
	if err != nil {
		return nil, err
	}

	err = ApplyPatches(&patchedFiles, utils.CUSTOM_PATCH_DIR)
	if err != nil {
		return nil, err
	}

	return patchedFiles, nil
}

func ApplyPatches(patchedFiles *[]PatchedFile, dir string) error {
	patches, err := os.ReadDir(dir)
	if err != nil {
		return err
	}

	for _, patch := range patches {
		if patch.IsDir() {
			continue
		}

		if !strings.HasSuffix(patch.Name(), ".patch") {
			continue
		}

		forFile := strings.TrimSuffix(patch.Name(), ".patch")

		filePath := utils.CACHE_DIR + "/" + forFile
		patchPath := dir + patch.Name()
		sha, err := utils.ComputeGitBlobSHA1(filePath)
		if err != nil {
			return err
		}
		shaPatch, err := utils.ComputeGitBlobSHA1(patchPath)
		if err != nil {
			return err
		}

		shaPatched, err := PatchFile(filePath, patchPath)
		if err != nil {
			return err
		}

		appendPatch(patchedFiles, forFile, sha, shaPatched, Patch{
			Path: patchPath,
			Sha:  shaPatch,
		})
	}

	return nil
}

func appendPatch(patchedFiles *[]PatchedFile, forFile string, sha string, shaPatched string, patch Patch) {
	for _, patchedFile := range *patchedFiles {
		if patchedFile.Name == forFile {
			patchedFile.Patches = append(patchedFile.Patches, patch)
			patchedFile.ResultSha = shaPatched
			return
		}
	}

	*patchedFiles = append(*patchedFiles, PatchedFile{
		Name:      forFile,
		Sha:       sha,
		Patches:   []Patch{patch},
		ResultSha: shaPatched,
	})

}
