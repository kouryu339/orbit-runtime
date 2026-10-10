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
		SHA256:     "54a05d757573149914567cbfaeb1aaa164f35b3275904b3cc1291027ed3a6b72",
		Library:    "bin/agent_runtime.dll",
	},
	"linux-x86_64": {
		PlatformID: "linux-x86_64",
		Archive:    "orbit-runtime-runtime-v0.5.0-beta.1-linux-x86_64.zip",
		SHA256:     "0d9b318c53859b954bf6db78c3feed0afcc5f6bcd3c2eb7591711082e8f0c344",
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
