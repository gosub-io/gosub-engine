package patch

import (
	"fmt"
	"generate_definitions/utils"
	"os"
	"os/exec"
)

func PatchFile(path string, patchPath string) (string, error) {
	cmd := exec.Command("patch", path, patchPath)

	output, err := cmd.CombinedOutput()
	if err != nil {
		return "", fmt.Errorf("Failed to apply patch %v: %v\nOUTPUT:\n%v", path, err, string(output))
	}

	if _, err := os.Stat(path); os.IsNotExist(err) {
		return "<deleted>", nil
	}

	return utils.ComputeGitBlobSHA1(path)

}
