pub const DEFAULT_RUNTIME_VERSION: &str = "0.5.3";
pub const DEFAULT_RELEASE_TAG: &str = "v0.5.3";
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
    archive: "orbit-runtime-runtime-v0.5.3-windows-x86_64.zip",
    sha256: "d72c66bfffdb7766b360584f619ae2e4a665dec89163c08f791511d6d1c93b3e",
    library: "bin/agent_runtime.dll",
};

pub const LINUX_X86_64: RuntimeArtifact = RuntimeArtifact {
    platform_id: "linux-x86_64",
    archive: "orbit-runtime-runtime-v0.5.3-linux-x86_64.zip",
    sha256: "d12bcda3192405453fb0d2c214a4b150d8ad847c57941a696875506fd6a6cea4",
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
