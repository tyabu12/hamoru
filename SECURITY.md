# Security Policy

> **Note:** hamoru is under active development and is not production-ready.
> APIs, configuration formats, and security boundaries may change without notice.

## Supported Versions

| Version | Supported |
|---------|-----------|
| 0.1.x   | Yes       |

## Reporting a Vulnerability

If you discover a security vulnerability in hamoru, please report it responsibly through
[GitHub's private vulnerability reporting](https://github.com/tyabu12/hamoru/security/advisories/new).

**Please do NOT open a public issue for security vulnerabilities.**

You can expect an initial response within 7 days.

## Security Measures

- Dependencies are audited weekly via [cargo-deny](https://github.com/EmbarkStudios/cargo-deny) (RUSTSEC advisory database)
- All CI actions are pinned by commit SHA to prevent supply chain attacks
- Secret scanning with push protection is enabled on this repository
- API credentials are never stored in code — environment variables only
