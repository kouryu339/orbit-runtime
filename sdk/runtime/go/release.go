package runtimehost

import (
	"fmt"
	"runtime"
)

const DefaultRuntimeVersion = "0.4.8-beta.1"
const DefaultReleaseTag = "v0.4.8-beta.1"
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
		Archive:    "orbit-runtime-runtime-v0.4.8-beta.1-windows-x86_64.zip",
		SHA256:     "c932a55e956095d8c89956e1e2506dfdfde88e273b11f0280f36c1671bbc397c",
		Library:    "bin/agent_runtime.dll",
	},
	"linux-x86_64": {
		PlatformID: "linux-x86_64",
		Archive:    "orbit-runtime-runtime-v0.4.8-beta.1-linux-x86_64.zip",
		SHA256:     "55ac5b7ca2dac3d7a3979c2104fabe238540e48da95983f56a5247e9a508464d",
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
