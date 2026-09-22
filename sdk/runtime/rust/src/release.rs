pub const DEFAULT_RUNTIME_VERSION: &str = "0.4.8-beta.5";
pub const DEFAULT_RELEASE_TAG: &str = "v0.4.8-beta.5";
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
    archive: "orbit-runtime-runtime-v0.4.8-beta.5-windows-x86_64.zip",
    sha256: "eb08d7a946f8794386a92ed691b1be714b8475edb813277fc670411796a8444b",
    library: "bin/agent_runtime.dll",
};

pub const LINUX_X86_64: RuntimeArtifact = RuntimeArtifact {
    platform_id: "linux-x86_64",
    archive: "orbit-runtime-runtime-v0.4.8-beta.5-linux-x86_64.zip",
    sha256: "c8673f6b20d44a3716aaeafd303ba3aa055daded1a7089d31e5ce14e77105d44",
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
