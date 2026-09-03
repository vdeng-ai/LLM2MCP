# Contributing to LLM2MCP

Thanks for helping improve LLM2MCP. Focused bug reports, compatibility fixes, documentation improvements, tests, and well-scoped pull requests are welcome.

## Contributor License Agreement

Before submitting code or documentation, read [CLA.md](CLA.md).

By submitting a pull request, patch, commit, or other contribution to this repository, you agree to the Contributor License Agreement. You retain ownership of your contribution; the CLA grants the project maintainers the rights needed to distribute it under the project's MIT License and to preserve future relicensing, dual-licensing, and commercial-licensing options.

Do not submit third-party code, prompts, datasets, icons, fonts, or other material unless its license permits inclusion and you clearly identify the source and license in the pull request.

## Before opening an issue

- Search existing issues first.
- Include your operating system, LLM2MCP version, MCP host or coding agent, configured API compatibility target, and concise reproduction steps when relevant.
- Remove API keys, credentials, private repository content, and other secrets from logs and screenshots.
- For compatibility issues, state the host application and version whenever possible.

## Development workflow

Use a current Rust toolchain compatible with the repository's `Cargo.toml` and `Cargo.lock`.

Before submitting a pull request, run:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --release --locked
```

If your change affects packaging or installation, also test the relevant platform packaging flow documented in `docs/` and `.github/workflows/`.

## Pull requests

- Keep changes focused and explain the user-visible or protocol-visible behavior.
- Add or update tests for parsing, path sandboxing, configuration migration, job handling, MCP behavior, and client integration when those areas change.
- Preserve read-only workspace guarantees unless the change explicitly proposes and documents a security-model change.
- Update English and Simplified Chinese documentation when public behavior changes.
- Do not commit credentials, local model endpoints containing secrets, generated build output, or private workspace data.
- Call out new dependencies and their licenses in the pull request. If a direct dependency is added or removed, update [THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md).

## Licensing

LLM2MCP is currently distributed under the [MIT License](LICENSE). Accepted contributions become part of the public MIT-licensed project. The CLA does not revoke MIT rights from users of released versions.

The names and logos associated with LLM2MCP are handled separately from the source-code license; see [TRADEMARKS.md](TRADEMARKS.md).
