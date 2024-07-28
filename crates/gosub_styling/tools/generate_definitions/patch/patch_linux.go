package patch

import (
	"generate_definitions/utils"
	"os/exec"
)

func PatchFile(path string, patchPath string) (string, error) {

	cmd := exec.Command("patch", path, patchPath)

	err := cmd.Run()
	if err != nil {
		return "", err
	}

	return utils.ComputeGitBlobSHA1(path)

}
