package patch

import (
	"encoding/json"
	"fmt"
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
	FilePath string `json:"path"`
	Name     string `json:"name"`
	Sha      string `json:"sha"`
}

type FileListing struct {
	Files   *[]PatchedFile
	Patches *[]Patch
}

func (fl FileListing) Save() error {
	file, err := json.Marshal(fl.Files)
	if err != nil {
		return err
	}

	if err = os.WriteFile(path.Join(utils.CACHE_INDEX_FILE, "index.json"), file, 0644); err != nil {
		return err

	}

	return nil
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
		forFile := strings.TrimSuffix(patch.Name, ".patch")
		if forFile == name {
			return true
		}
	}
	return false
}

func (fl FileListing) FilePatchesApplied(name string) bool {
	var neededPatches []Patch
	for _, patch := range *fl.Patches {
		forFile := strings.TrimSuffix(patch.Name, ".patch")
		if forFile == name {
			neededPatches = append(neededPatches, patch)
		}
	}

	for _, file := range *fl.Files {
		if file.Name == name {
			if len(file.Patches) == len(neededPatches) {
				return true
			}
			if len(file.Patches) == 0 {
				return false
			}

			log.Panic("File has been patched, but not all patches have been applied")
		}
	}

	if len(neededPatches) == 0 {
		return true
	}

	return false

}

func (fl FileListing) PatchFile(name string) error {
	log.Println("Patching file", name)

	for _, patch := range *fl.Patches {
		forFile := strings.TrimSuffix(patch.Name, ".patch")
		if forFile == name {
			filePath := path.Join(utils.CACHE_DIR, name)

			if err := ApplyPatch(fl.Files, forFile, filePath, patch.FilePath, patch.Name); err != nil {
				return err
			}
		}
	}
	return nil
}

func GetFileListing() FileListing {
	patchedFiles, err := GetCachedPatches()
	if err != nil {
		log.Panic(err)
	}

	patches, err := GetPatches()
	if err != nil {
		log.Panic(err)
	}

	for _, p := range patches {
		log.Println(p.FilePath, p.Sha)
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
				FilePath: patchPath,
				Name:     patch.Name(),
				Sha:      sha,
			})
		}
	}

	return patches, nil
}

func GetCachedPatches() ([]PatchedFile, error) {
	var patchedFiles []PatchedFile
	file, err := os.ReadFile(utils.CACHE_INDEX_FILE)
	if os.IsNotExist(err) {
		return patchedFiles, nil
	}
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

func PatchFiles() error {
	patchedFiles, err := GetCachedPatches()
	if err != nil {
		return err
	}

	err = PatchFilesWithCache(&patchedFiles)
	if err != nil {
		return err
	}

	marshaled, err := json.Marshal(patchedFiles)
	if err != nil {
		return err
	}

	if err = os.WriteFile(path.Join(utils.CACHE_INDEX_FILE), marshaled, 0644); err != nil {
		return err
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
			log.Println("Skipping directory", patch.Name())
			continue
		}

		if !strings.HasSuffix(patch.Name(), ".patch") {
			log.Println("Skipping non-patch file", patch.Name())
			continue
		}

		forFile := strings.TrimSuffix(patch.Name(), ".patch")

		filePath := path.Join(utils.CACHE_DIR, "patched", forFile)

		{
			cacheFilePath := path.Join(utils.CACHE_DIR, "specs2", forFile)
			content, err := os.ReadFile(cacheFilePath)
			if err != nil {
				return err
			}

			if err = os.WriteFile(filePath, content, 0644); err != nil {
				return err
			}
		}

		patchPath := path.Join(dir, patch.Name())

		if err := ApplyPatch(patchedFiles, forFile, filePath, patchPath, patch.Name()); err != nil {
			return err
		}
	}

	return nil
}

func ApplyPatch(patchedFiles *[]PatchedFile, forFile, file, patch, patchName string) error {
	sha, err := utils.ComputeGitBlobSHA1(file)
	if os.IsNotExist(err) {
		log.Printf("Failed to patch file %v: file does not exist", file)
		return nil
	}
	if err != nil {
		return fmt.Errorf("Failed to compute SHA1 for %v: %v", file, err)
	}
	shaPatch, err := utils.ComputeGitBlobSHA1(patch)
	if os.IsNotExist(err) {
		log.Printf("Failed to patch file %v: patch does not exist", patch)
		return nil
	}
	if err != nil {
		return fmt.Errorf("Failed to compute SHA1 for %v: %v", patch, err)
	}

	shaPatched, err := PatchFile(file, patch)
	if os.IsNotExist(err) {
		log.Printf("Failed to patch file %v: file does not exist", file)
		return nil
	}
	if err != nil {
		return fmt.Errorf("Failed to apply patch %v to %v: %v", patch, file, err)
	}

	appendPatch(patchedFiles, forFile, sha, shaPatched, Patch{
		FilePath: patch,
		Name:     patchName,
		Sha:      shaPatch,
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
				if patch.FilePath == patchFile {
					//The patch has been applied
					if patch.Sha != patchSha {
						// The patch has been applied, but its hash has changed
						log.Panicf("Patch file %v has been modified", patchFile)
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
			if patch.FilePath == availablePatch.FilePath {
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
