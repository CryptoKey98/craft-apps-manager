# Documentation

Choose a guide for what you want to do. Downloaded releases do not require a Rust toolchain.

## Install and use

| Guide | What you will find |
| --- | --- |
| [Quick start](../README.md#get-started) | Downloads, package choices, and first launch |
| [Using the manager](user-guide/using-the-manager.md) | Installations, updates, Activity, custom apps, builds, backups, and storage |
| [Linux](user-guide/linux.md) | Native packages, portable archives, tray behavior, and required tools |
| [macOS](user-guide/macos.md) | DMG and ZIP setup, first-launch approval, updates, and recovery |
| [Changelog](../CHANGELOG.md) | Changes in each release |

## Build and contribute

| Guide | What you will find |
| --- | --- |
| [Building and testing](development/building.md) | Prerequisites, updater choices, tests, and local packages |
| [Windows packaging](development/windows-packaging.md) | MSI layout, WiX, portable packaging, and upgrade identity |

## Publish and maintain

| Guide | What you will find |
| --- | --- |
| [Release preparation](development/releases.md) | Version preparation, CI, publication, revisions, and catalog-only recovery |
| [Updater operations](development/updater-operations.md) | Configuration, signing, hosting, renewal, and recovery rules |
| [Production catalog inputs](../update-service/catalog/README.md) | Checked-in policy files and production configuration |

## Understand the architecture

- [Per-app installations](architecture/per-app-installation.md): inventory and selection.
- [Operations](architecture/operations.md): lifecycle records, cancellation, and cross-window observation.
- [Windows batch approval](architecture/windows-batch-admin.md): elevated installer batches.
- [Linux batch approval](architecture/linux-batch-admin.md): PolicyKit and native package batches.

Older links directly under `docs/` remain as short redirects to these guides.

## Editing and wiki previews

These files are the maintained source for the GitHub Wiki mirror. Make corrections here so the repository and wiki do not become two separate sets of instructions. See [Wiki documentation preview](development/building.md#wiki-documentation-preview) to generate and test changes locally. The documentation workflow publishes approved changes from main when wiki syncing is enabled.
