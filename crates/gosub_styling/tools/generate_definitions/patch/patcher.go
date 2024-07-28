package patch

import (
	"encoding/json"
	"generate_definitions/utils"
	"log"
	"os"
	"path"
	"strings"
)

var (
	PATHS = []string{
		utils.CACHE_DIR + "/patches",
		utils.CUSTOM_PATCH_DIR,
	}
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

type FileListing struct {
	Files   *[]PatchedFile
	Patches *[]Patch
}

func (fl FileListing) FileNeedsReset(name string) bool {
	for _, file := range *fl.Files {
		if file.Name == name {
			return FileNeedsReset(&file, fl.Patches)
		}
	}
	return false
}

func (fl FileListing) FileNeedsPatch(name string) bool {
	for _, patch := range *fl.Patches {
		forFile := strings.TrimSuffix(patch.Path, ".patch")
		if forFile == name {
			return true
		}
	}
	return false
}

func (fl FileListing) PatchFile(name string) error {
	for _, patch := range *fl.Patches {
		forFile := strings.TrimSuffix(patch.Path, ".patch")
		if forFile == name {
			filePath := path.Join(utils.CACHE_DIR, name)

			if err := ApplyPatch(fl.Files, forFile, filePath, patch.Path); err != nil {
				return err
			}
		}
	}
	return nil
}

func GetFileListing() FileListing {
	patchedFiles, err := GetCachedPatches()
	if err != nil {
		log.Fatal(err)
	}

	patches, err := GetPatches()
	if err != nil {
		log.Fatal(err)
	}

	return FileListing{
		Files:   &patchedFiles,
		Patches: &patches,
	}
}

func GetPatches() ([]Patch, error) {
	var patches []Patch

	for _, dir := range PATHS {
		p, err := os.ReadDir(dir)
		if err != nil {
			return nil, err
		}

		for _, patch := range p {
			if patch.IsDir() {
				continue
			}

			if !strings.HasSuffix(patch.Name(), ".patch") {
				continue
			}

			patchPath := path.Join(dir, patch.Name())

			sha, err := utils.ComputeGitBlobSHA1(patchPath)
			if err != nil {
				return nil, err
			}

			patches = append(patches, Patch{
				Path: patchPath,
				Sha:  sha,
			})
		}
	}

	return patches, nil
}

func GetCachedPatches() ([]PatchedFile, error) {
	var patchedFiles []PatchedFile
	file, err := os.ReadFile(path.Join(utils.CACHE_INDEX_FILE, "index.json"))
	if err != nil {
		return nil, err
	}

	if err = json.Unmarshal(file, &patchedFiles); err != nil {
		return nil, err
	}

	return patchedFiles, nil
}

// PatchFilesWithCache
// If you call this function, you need to already have checked if you need to reset a file with the [FileNeedsReset] function
func PatchFilesWithCache(patchedFiles *[]PatchedFile) error {
	for _, dir := range PATHS {
		err := ApplyPatches(patchedFiles, dir)
		if err != nil {
			return err
		}

	}

	return nil
}

func ApplyPatches(patchedFiles *[]PatchedFile, dir string) error {
	//TODO: This function should take the [FileListing] struct as an argument, so we can't get out of sync with the cache
	//but this is really really unlikely to happen, so I'm not going to bother with it for now
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

		filePath := path.Join(dir, forFile)
		patchPath := dir + patch.Name()

		if err := ApplyPatch(patchedFiles, forFile, filePath, patchPath); err != nil {
			return err
		}
	}

	return nil
}

func ApplyPatch(patchedFiles *[]PatchedFile, forFile, file, patch string) error {
	sha, err := utils.ComputeGitBlobSHA1(file)
	if err != nil {
		return err
	}
	shaPatch, err := utils.ComputeGitBlobSHA1(patch)
	if err != nil {
		return err
	}

	shaPatched, err := PatchFile(file, patch)
	if err != nil {
		return err
	}

	appendPatch(patchedFiles, forFile, sha, shaPatched, Patch{
		Path: patch,
		Sha:  shaPatch,
	})

	return nil
}

func isAlreadyApplied(patchedFiles *[]PatchedFile, forFile, sha, patchFile, patchSha string) bool {
	for _, patchedFile := range *patchedFiles {
		if patchedFile.Name == forFile {
			if patchedFile.Sha == sha {
				return false // the file has not been modified, so we haven't applied any patches
			}

			for _, patch := range patchedFile.Patches {
				if patch.Path == patchFile {
					//The patch has been applied
					if patch.Sha != patchSha {
						// The patch has been applied, but its hash has changed
						log.Fatalf("Patch file %v has been modified", patchFile)
					}

					return true
				}
			}

			return false
		}
	}

	return false
}

func FileNeedsReset(file *PatchedFile, availablePatches *[]Patch) bool {
	sha, err := utils.ComputeGitBlobSHA1(path.Join(utils.CACHE_DIR, file.Name))
	if err != nil {
		log.Println("Failed to compute SHA1 for", file.Name)
		return true
	}

	if sha != file.ResultSha {
		return true
	}

	for _, patch := range file.Patches {
		for _, availablePatch := range *availablePatches {
			if patch.Path == availablePatch.Path {
				if patch.Sha != availablePatch.Sha {
					return true
				}
			}
		}
	}

	return false
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
