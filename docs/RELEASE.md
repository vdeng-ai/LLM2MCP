# LLM2MCP Release Guide

LLM2MCP uses a tag-driven GitHub Actions release pipeline. A tag such as `v0.2.0` must exactly match `package.version = "0.2.0"` in `Cargo.toml`.

## Release outputs

A successful release publishes these assets to one GitHub Release:

- Linux x86_64: `LLM2MCP_<version>_x86_64.AppImage`
- Windows x86_64: `LLM2MCP_<version>_x86_64-setup.exe` (NSIS, current-user install)
- macOS Apple Silicon: `LLM2MCP_<version>_aarch64.dmg`
- macOS Intel: `LLM2MCP_<version>_x86_64.dmg`
- macOS updater bundles: matching `*.app.tar.gz`
- updater signatures: `*.sig`
- `latest.json` for the in-app updater
- `SHA256SUMS`

The macOS `.app.tar.gz` files contain the app bundles used by the updater. The DMGs are the normal manual-install artifacts.

## 1. One-time updater signing-key setup

Updater signing is separate from Apple/Windows platform code signing. The updater key ensures that LLM2MCP installs only update payloads signed by this project.

Install the pinned packager used by CI:

```bash
cargo install cargo-packager --version 0.11.8 --locked
```

Generate the updater key pair once. The command asks for a password unless you explicitly provide an empty one; using a password is recommended:

```bash
cargo packager signer generate --path llm2mcp-updater.key
```

The command writes:

- `llm2mcp-updater.key` — private key; keep it secret and back it up securely.
- `llm2mcp-updater.key.pub` — public key; safe to expose.

Read the exact values that must be copied into GitHub:

Linux/macOS:

```bash
printf '%s\n' 'LLM2MCP_UPDATER_PUBKEY:'
cat llm2mcp-updater.key.pub
printf '%s\n' 'LLM2MCP_UPDATER_PRIVATE_KEY:'
cat llm2mcp-updater.key
```

Windows PowerShell:

```powershell
Write-Host 'LLM2MCP_UPDATER_PUBKEY:'
Get-Content -Raw .\llm2mcp-updater.key.pub
Write-Host 'LLM2MCP_UPDATER_PRIVATE_KEY:'
Get-Content -Raw .\llm2mcp-updater.key
```

Copy the **entire one-line/base64 value** printed for each file. Do not trim or decode it.

Configure the GitHub repository under **Settings → Secrets and variables → Actions**:

| GitHub location | Name | Value |
| --- | --- | --- |
| Variables → Repository variables | `LLM2MCP_UPDATER_PUBKEY` | exact complete contents of `llm2mcp-updater.key.pub` |
| Secrets → Repository secrets | `LLM2MCP_UPDATER_PRIVATE_KEY` | exact complete contents of `llm2mcp-updater.key` |
| Secrets → Repository secrets | `LLM2MCP_UPDATER_PRIVATE_KEY_PASSWORD` | the password you typed when generating the key pair |

If you deliberately generated the key without a password, `LLM2MCP_UPDATER_PRIVATE_KEY_PASSWORD` may be omitted. Never put the private key into a GitHub Variable; it must be an Actions Secret.

Do not commit the private key. A tagged release deliberately fails in the preflight job if the public or private updater key is missing.

The public key is embedded in release builds through `LLM2MCP_UPDATER_PUBKEY`. Local/dev builds without that variable simply disable the in-app updater.

## 2. Version and tag flow

1. Update `Cargo.toml` version.
2. Update user-facing release notes/docs as needed.
3. Run the normal checks:

```bash
cargo fmt -- --check
cargo test --all-targets
cargo clippy --all-targets --all-features -- -D warnings
```

4. Commit and push `main`.
5. Create and push the matching tag:

```bash
git tag v0.2.0
git push origin v0.2.0
```

The `Release` workflow then performs preflight validation, builds all platform packages, generates `latest.json` and `SHA256SUMS`, and creates the GitHub Release with generated release notes.

## 3. CI and release jobs

`.github/workflows/ci.yml` runs tests on Linux, Windows, and macOS for normal pushes and pull requests, plus format/Clippy checks on Linux.

`.github/workflows/release.yml` runs only for version tags and contains:

- `preflight` — tag/version and updater-key validation.
- `linux` — Ubuntu 22.04 x86_64 AppImage. Ubuntu 22.04 is intentionally used for a conservative glibc baseline.
- `windows` — Windows x86_64 NSIS installer.
- `macos` — native Apple Silicon (`macos-15`) and Intel (`macos-15-intel`) builds, producing `.app.tar.gz` updater bundles and DMGs.
- `publish` — validates required artifacts/signatures, builds `latest.json`, creates checksums, and creates or updates the GitHub Release.

`cargo-packager` is pinned to `0.11.8` in the workflow so packaging behavior does not silently change when a new packager release appears.

## 4. Automatic update protocol

The application checks:

```text
https://github.com/vdeng-ai/LLM2MCP/releases/latest/download/latest.json
```

The manifest maps the current OS/architecture to one signed updater package:

- `linux-x86_64` → AppImage
- `windows-x86_64` → NSIS installer
- `macos-aarch64` → `.app.tar.gz`
- `macos-x86_64` → `.app.tar.gz`

LLM2MCP verifies the downloaded package with the embedded updater public key before installation. An available update is shown in the GUI header. After installation the UI asks the user to restart LLM2MCP.

## 5. Platform trust signing

Updater signatures do **not** replace OS trust/code signing.

### macOS

The pipeline currently builds distributable `.app`/DMG artifacts, but Developer ID signing + Apple notarization require Apple Developer credentials and should be enabled before broad public distribution. `cargo-packager` supports Developer ID signing and notarization; keep those credentials in GitHub Actions secrets, never in the repository.

Recommended future secrets/settings:

- base64 Developer ID Application `.p12` certificate
- certificate password
- signing identity
- Apple notarization credentials (`APPLE_ID` + app-specific password + team ID, or App Store Connect API key)

### Windows

The NSIS installer works without Authenticode signing, but Windows SmartScreen reputation is significantly better with a trusted code-signing certificate. Add Authenticode signing when a certificate/provider is available.

These platform signatures are independent from the mandatory LLM2MCP updater key.

## 6. Local packaging smoke tests

Generate packaging icons:

```bash
python3 scripts/release/generate_icons.py
```

Linux:

```bash
cargo install cargo-packager --version 0.11.8 --locked
bash scripts/release/package-linux.sh
```

macOS:

```bash
cargo install cargo-packager --version 0.11.8 --locked
bash scripts/release/package-macos.sh
```

Windows PowerShell:

```powershell
cargo install cargo-packager --version 0.11.8 --locked
./scripts/release/package-windows.ps1
```

Unsigned local packaging is useful for smoke testing. A production Release workflow requires the updater signing secrets so that every updater asset has a matching `.sig`.
