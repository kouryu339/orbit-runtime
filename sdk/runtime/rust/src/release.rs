pub const DEFAULT_RUNTIME_VERSION: &str = "0.4.5";
pub const DEFAULT_RELEASE_TAG: &str = "v0.4.5";
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
    archive: "orbit-runtime-runtime-v0.4.5-windows-x86_64.zip",
    sha256: "37f1da612f1d4e2aef25656f6b39448e00c2489481527e80fdf83db42b328b19",
    library: "bin/agent_runtime.dll",
};

pub const LINUX_X86_64: RuntimeArtifact = RuntimeArtifact {
    platform_id: "linux-x86_64",
    archive: "orbit-runtime-runtime-v0.4.5-linux-x86_64.zip",
    sha256: "32207752353b952484f24b49905958bc9e9c85d0a139c7b1413fc3a08fd97850",
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
