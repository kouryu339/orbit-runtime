package runtimehost

import (
	"fmt"
	"runtime"
)

const DefaultRuntimeVersion = "0.4.8-beta.3"
const DefaultReleaseTag = "v0.4.8-beta.3"
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
		Archive:    "orbit-runtime-runtime-v0.4.8-beta.3-windows-x86_64.zip",
		SHA256:     "91a998b56192c3df29a8de9f15a51ea6a3aa3124403c45b13cf6b2e61985b11c",
		Library:    "bin/agent_runtime.dll",
	},
	"linux-x86_64": {
		PlatformID: "linux-x86_64",
		Archive:    "orbit-runtime-runtime-v0.4.8-beta.3-linux-x86_64.zip",
		SHA256:     "559e5a89abfb5f3f3f8bae29ed1a736c22772c808eb471f104a50b145655f6be",
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
