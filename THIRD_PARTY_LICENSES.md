# Third-Party Software and Licenses

LLM2MCP itself is distributed under the [MIT License](LICENSE). It also uses third-party software whose licenses remain governed by their respective authors and license texts.

This file records the project's intentional direct Rust dependencies as a compliance aid. The exact transitive dependency set for a release is defined by `Cargo.lock` and can change when dependencies are updated. Binary release preparation should therefore generate and review a complete notice set from the exact locked dependency graph rather than treating this document as the only license inventory.

## Direct Rust dependencies

| Dependency | Declared/upstream license family |
| --- | --- |
| `anyhow` | MIT OR Apache-2.0 |
| `clap` | MIT OR Apache-2.0 |
| `directories` | MIT OR Apache-2.0 |
| `eframe` | MIT OR Apache-2.0 |
| `fs2` | MIT OR Apache-2.0 |
| `cargo-packager-updater` | MIT and/or Apache-2.0; see package license files for the resolved release |
| `ignore` | MIT OR Unlicense |
| `reqwest` | MIT OR Apache-2.0 |
| `serde` | MIT OR Apache-2.0 |
| `serde_json` | MIT OR Apache-2.0 |
| `sha2` | MIT OR Apache-2.0 |
| `time` | MIT OR Apache-2.0 |
| `toml_edit` | MIT OR Apache-2.0 |

Versions are intentionally not duplicated here; `Cargo.toml` defines requested versions and `Cargo.lock` is the release source of truth for resolved versions.

## Release compliance checklist

Before publishing a commercial or public binary release:

1. Build from a committed `Cargo.lock` using `--locked`.
2. Generate a complete dependency/license report from the resolved graph (for example with `cargo-deny`, `cargo-about`, or another audited Rust license tool).
3. Review copyleft, attribution, notice, cryptography, updater, installer, and platform-specific obligations separately.
4. Bundle any license or notice text required by the dependencies actually distributed in the release.
5. Review newly added non-code assets such as icons, fonts, screenshots, prompts, example data, and bundled configuration; they may have licensing terms independent of Rust crates.
6. For installer and updater releases, also review third-party software introduced by the packaging toolchain or bundled system libraries.

## Updating this file

When a direct dependency is added, removed, or materially relicensed, update this document in the same pull request. A release process may additionally generate a machine-produced full inventory or SBOM; generated output should not be assumed to replace required upstream license texts.

This document is provided for project compliance tracking and is not legal advice. Upstream license files and applicable law control if this summary differs from an upstream package's actual license terms.
