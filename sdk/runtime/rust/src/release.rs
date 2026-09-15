pub const DEFAULT_RUNTIME_VERSION: &str = "0.4.8-beta.2";
pub const DEFAULT_RELEASE_TAG: &str = "v0.4.8-beta.2";
pub const DEFAULT_REPOSITORY: &str = "kouryu339/orbit-runtime";
pub const ABI_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeArtifact {
    pub platform_id: &'static str,
    pub archive: &'static str,
    pub sha256: &'static str,
    pub library: &'static str,
}

pub const WINDOWS_X86_64: RuntimeArtifact = RuntimeArtifact {
    platform_id: "windows-x86_64",
    archive: "orbit-runtime-runtime-v0.4.8-beta.2-windows-x86_64.zip",
    sha256: "7dcef1d8d0d6dfdba7ff4a318f3b7d5e648c573fc248121d5032345d5d0ec9f4",
    library: "bin/agent_runtime.dll",
};

pub const LINUX_X86_64: RuntimeArtifact = RuntimeArtifact {
    platform_id: "linux-x86_64",
    archive: "orbit-runtime-runtime-v0.4.8-beta.2-linux-x86_64.zip",
    sha256: "52dde96d502032b0a94adc13a0b2d0a859f7fd080f4bd2bea8d7873fa5c1f67c",
    library: "lib/libagent_runtime.so",
};

pub fn current_platform_artifact() -> Option<RuntimeArtifact> {
    if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        Some(WINDOWS_X86_64)
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some(LINUX_X86_64)
    } else {
        None
    }
}

pub fn release_download_url(artifact: RuntimeArtifact) -> String {
    format!(
        "https://github.com/{}/releases/download/{}/{}",
        DEFAULT_REPOSITORY, DEFAULT_RELEASE_TAG, artifact.archive
    )
}
