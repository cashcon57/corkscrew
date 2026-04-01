# Security Policy

## Reporting a Vulnerability

If you discover a security vulnerability in Corkscrew, please report it responsibly.

**Do NOT open a public GitHub issue for security vulnerabilities.**

Instead, please email **cashconway@icloud.com** with:

- A description of the vulnerability
- Steps to reproduce
- Potential impact
- Any suggested fixes

You should receive a response within 48 hours. We will work with you to understand and address the issue before any public disclosure.

## Scope

The following are in scope:

- Path traversal in archive extraction or mod deployment
- Credential exposure (OAuth tokens, API keys, signing keys)
- Command injection via mod names, file paths, or user input
- Unauthorized file access outside the game directory or staging area
- Vulnerabilities in the auto-updater or code signing pipeline

The following are out of scope:

- Vulnerabilities in third-party dependencies (report these upstream)
- Issues requiring physical access to the user's machine
- Social engineering attacks

## Supported Versions

Only the latest release is supported with security updates.

| Version | Supported |
|---------|-----------|
| Latest  | Yes       |
| Older   | No        |

## Security Practices

- All paths from external sources (archives, user input) are validated against traversal attacks
- OAuth tokens are stored with restricted file permissions (mode 0600)
- The auto-updater verifies signatures before applying updates
- Sentry telemetry is opt-in, strips PII, and sends no file paths or mod lists
- The Tauri signing key is never committed to the repository
