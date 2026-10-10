package runtimehost

import (
	"fmt"
	"runtime"
)

const DefaultRuntimeVersion = "0.5.0-beta.1"
const DefaultReleaseTag = "v0.5.0-beta.1"
const DefaultRepository = "kouryu339/orbit-runtime"
const RuntimeABIVersion = 1

type RuntimeArtifact struct {
	PlatformID string
	Archive    string
	SHA256     string
	Library    string
}

var runtimeArtifacts = map[string]RuntimeArtifact{
	"windows-x86_64": {
		PlatformID: "windows-x86_64",
		Archive:    "orbit-runtime-runtime-v0.5.0-beta.1-windows-x86_64.zip",
		SHA256:     "dfb2e1ee486b7d8d359bdda9d65637ca7611702242fd597c198dfd042e35c7cd",
		Library:    "bin/agent_runtime.dll",
	},
	"linux-x86_64": {
		PlatformID: "linux-x86_64",
		Archive:    "orbit-runtime-runtime-v0.5.0-beta.1-linux-x86_64.zip",
		SHA256:     "2df61c0824b6a18f2ed115b08139778368cab9f584245352b0b1f9d3fc824e30",
		Library:    "lib/libagent_runtime.so",
	},
}

func RuntimePlatformID() (string, error) {
	if runtime.GOARCH != "amd64" {
		return "", fmt.Errorf("unsupported runtime architecture: %s", runtime.GOARCH)
	}
	switch runtime.GOOS {
	case "windows":
		return "windows-x86_64", nil
	case "linux":
		return "linux-x86_64", nil
	default:
		return "", fmt.Errorf("unsupported runtime OS: %s", runtime.GOOS)
	}
}

func CurrentRuntimeArtifact() (RuntimeArtifact, error) {
	platformID, err := RuntimePlatformID()
	if err != nil {
		return RuntimeArtifact{}, err
	}
	artifact, ok := runtimeArtifacts[platformID]
	if !ok {
		return RuntimeArtifact{}, fmt.Errorf("unsupported runtime platform: %s", platformID)
	}
	return artifact, nil
}

func RuntimeReleaseURL(artifact RuntimeArtifact) string {
	return fmt.Sprintf(
		"https://github.com/%s/releases/download/%s/%s",
		DefaultRepository,
		DefaultReleaseTag,
		artifact.Archive,
	)
}
