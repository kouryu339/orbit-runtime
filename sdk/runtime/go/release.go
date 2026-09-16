package runtimehost

import (
	"fmt"
	"runtime"
)

const DefaultRuntimeVersion = "0.4.8-beta.5"
const DefaultReleaseTag = "v0.4.8-beta.5"
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
		Archive:    "orbit-runtime-runtime-v0.4.8-beta.5-windows-x86_64.zip",
		SHA256:     "beff6b2745172989f027458d0ef79218588c9dc8d073d352940223fb96dbb851",
		Library:    "bin/agent_runtime.dll",
	},
	"linux-x86_64": {
		PlatformID: "linux-x86_64",
		Archive:    "orbit-runtime-runtime-v0.4.8-beta.5-linux-x86_64.zip",
		SHA256:     "29ecf88ea9cd4e6cf5820b883614b07ae6e54c0d22d6d1bed9d23ee0977041f3",
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
