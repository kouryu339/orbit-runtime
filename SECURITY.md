# Security Policy

Please do not report security vulnerabilities in a public issue.

Use GitHub's private vulnerability reporting for
[`kouryu339/orbit-runtime`](https://github.com/kouryu339/orbit-runtime/security/advisories/new).
Include affected versions, reproduction steps, impact, and any suggested
mitigation. Do not include live credentials or private user data.

The project does not guarantee security fixes for unreleased development
branches. Supported release lines and remediation timelines are stated in the
corresponding advisory.

## Release Integrity

The Git tag source is the trust anchor for each release. Native runtime
packages are official only when they are attached to a GitHub Release under
[`kouryu339/orbit-runtime`](https://github.com/kouryu339/orbit-runtime).

Release packages include SHA-256 checksums next to the zip assets. Users should
verify those checksums before loading the native runtime library. Third-party
repackaged binaries are not official project artifacts.

The release process is expected to grow toward CI verification that package
contents match the tagged source, SBOM publication, and signed release tags.
