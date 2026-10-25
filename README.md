# Scrim

[![CI/CD](https://github.com/rpwoodbu/scrim/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/rpwoodbu/scrim/actions/workflows/ci.yml?query=branch%3Amain)

**Scrim** is a lightweight wrapper around command line tools offering version management and usage telemetry without the need for shell hooks. It can download and cache tools from the network, reducing system image maintenance toil.

> **Origin of the Name:**  
> The name *Scrim* refers to the thin, translucent fabric used in theater to create lighting effects or hide/reveal elements on stage. It represents a "thin" proxy that stays completely out of sight until needed.

## Key Features

- **Layered version management:** `scrim.yaml` files define precisely which version of each tool to invoke based on the current working directory. All found `scrim.yaml` files are layered, with more local configuration taking precedence. See [Configuration](#configuration).
- **Remote tool fetching:** Rather than maintain locally-installed tools, Scrim can fetch them from the network. It downloads the archive or package (`.tar.gz`, `.tgz`, `.zip`, `.tar.xz`, `.txz`, `.xz`, `.AppImage`) or standalone binary, unpacks it into a local cache (extracting Type 2 AppImages natively without requiring FUSE), and executes the binary directly from the cache. Multiple versions of the same tool may be cached simultaneously. Downloads are hardened with a cryptographic digest (`sha256` or `sha512`), improving security and reproducibility.
- **Easy installation:** Scrim is a single statically-linked binary. Tool links in your path can be automatically populated and kept in sync using `scrim links <DIR>`. There are no shell hooks; this will work in any shell and without user configuration.
- **Non-blocking telemetry:** Get usage telemetry for the tools that are run, including full invocation information (coming soon). Telemetry is only performed once the proxied tool is called, removing telemetry from the critical path.
- **Low overhead:** Scrim adds less than 2 milliseconds to the tool invocation.

## Installation

1. Download the executable (e.g., `scrim-linux-x86_64` or `scrim-linux-aarch64`) from the latest GitHub Release.
2. Install the downloaded binary to a stable directory in your `PATH` (e.g., `/usr/local/bin`):
   ```bash
   sudo install -m 755 scrim-linux-x86_64 /usr/local/bin/scrim
   ```
3. Create `scrim.yaml` file(s) (keep reading).
4. Populate tool links in a directory in your `PATH` (e.g., `/usr/local/bin`):
   ```bash
   sudo scrim links /usr/local/bin
   ```
   > **Note on `sudo`:** `sudo` typically resets `$HOME` to `/root`. If you want `scrim links` to resolve your user configuration at `~/.config/scrim/scrim.yaml`, preserve `HOME` explicitly:
   > ```bash
   > sudo --preserve-env=HOME scrim links /usr/local/bin
   > ```

## Configuration

Scrim is configured by `scrim.yaml` that may be placed in your project, in your home directory (`~/.config/scrim/`), and/or at the system level (`/etc/scrim/`). They layer, producing a composite configuration whereby more local elements take precedence. Scrim traverses up from the current working directory to discover `scrim.yaml` files, stopping at the repository boundary. This allows a system-wide version of a tool to be defined, but with user, project, and even project subdirectory overrides.

The only other requirement is that each tool needs to have a link (symbolic or hard) in the path which points to the installed `scrim` binary. (This is similar in principle to `busybox`.) You can create and synchronize these links automatically with `scrim links <DIR>`.

```yaml
telemetry: true # disabled (false) by default
default_platform: linux-x86_64 # optional file-level default platform
imports:
  - base-tools.yaml
  - path: ../overrides/tools.yaml
    optional: true
    precedence: after
tools:
  # Uses default_platform:
  node:
    system_path: /bin/node
    url: https://nodejs.org/dist/v20.10.0/node-v20.10.0-linux-x64.tar.gz
    sha256: 285c1f0624022839446d32839446d32839446d32839446d32839446d32839446
    archive_path: node-v20.10.0-linux-x64/bin/node

  # Explicit multi-platform matrix:
  go:
    archive_path: go/bin/go # default archive_path across platforms
    platforms:
      linux-x86_64:
        # Uses system_path if it exists on disk; falls back to fetching via url if absent
        system_path: /usr/local/go/bin/go
        url: https://go.dev/dl/go1.21.5.linux-amd64.tar.gz
        sha256: 285c1f0624022839446d32839446d32839446d32839446d32839446d32839446
      linux-aarch64:
        system_path: /usr/local/go/bin/go
        url: https://go.dev/dl/go1.21.5.linux-arm64.tar.gz
        sha256: 9e06e39958dc1800cfc244490f2beeeff3dbb2bc61e389d70231cfb565a973bb
      macos-aarch64:
        system_path: /opt/homebrew/bin/go
        url: https://go.dev/dl/go1.21.5.darwin-arm64.tar.gz
        sha256: 7d6c5c8f285c1f0624022839446d32839446d32839446d32839446d32839446d

  # Universal tool / script:
  local-helper:
    platforms:
      any:
        system_path: /usr/local/bin/local-helper

  # Templated tool:
  gofmt:
    template: go
    archive_path: go/bin/gofmt

  # AppImage package:
  vim:
    platforms:
      linux-x86_64:
        url: https://github.com/vim/vim-appimage/releases/download/v9.2.1225/Vim-v9.2.1225.glibc2.34-x86_64.AppImage
        sha256: fb78530264806ee19eb014d6e5344d42e3434b12f646d661f68042227ff34057
  xxd:
    template: vim
    archive_path: usr/bin/xxd
```

For fetched archives, `archive_path` defaults to `<tool_name>` at the root of the archive if omitted, or to `AppRun` for `.AppImage` / `.appimage` packages. Tools can also configure both `system_path` and fetch properties (`url` or `template`), in which case Scrim checks at execution time whether the local executable at `system_path` exists on disk; if present, it is executed directly, and if absent, Scrim falls back to downloading and caching the tool from the network.

### Configuration Imports

Configuration files can import definitions from other files using the top-level `imports` list. Imports can be specified either as a shorthand file path string or as a structured object:

- **Relative Path Resolution**: Relative paths are resolved relative to the directory of the file declaring the import.
- **Precedence**: `precedence: before` (the default) imports definitions as defaults that can be overridden by the importing file. `precedence: after` imports definitions as overrides that take precedence over the importing file.
- **Optional Imports**: Set `optional: true` to silently ignore missing files; by default (`optional: false`), missing files trigger an error.

## Usage

Run your command(s) normally; e.g.:
```bash
$ go version
# ... downloading and unpacking only if needed ...
go version go1.21.5 linux/amd64
```

Alternatively, you can run any configured tool directly without a link:
```bash
scrim run go version
```

Run `scrim help` to see administrative functions.

## Comparison to Alternatives

There are many excellent tools in the ecosystem for managing environment variables and tool versions (such as `direnv`, `asdf`, and `mise`). If you are a solo developer looking for a feature-rich local environment manager, `mise` is an outstanding choice.

However, Scrim is specifically designed to solve **enterprise fleet deployment** problems where traditional environment managers introduce friction. Scrim's unique value proposition is built on three pillars:

1. **Invisible Fleet Deployment (System-Wide)**: Traditional workflows require developers to opt-in by modifying their shell profiles (e.g., `~/.bashrc`). While modern tools like `mise` *can* be configured by administrators for system-wide shim deployments, they are fundamentally designed as complex, user-level environment managers. Scrim is purpose-built for the fleet: it uses fast symlinks pointing to a compiled binary in a standard system path (like `/usr/local/bin`), meaning administrators can drop it across thousands of machines and it instantly works for all users in all shells with **zero user configuration**.
2. **Declarative Generic Fetching**: Supporting a custom or proprietary tool historically required writing and hosting custom Bash plugins (as seen in `asdf`). While `mise` now supports declarative fetching via its `http` backend, Scrim treats it as the core primitive. You can securely distribute any tool by providing just a URL, a mandatory SHA256 hash, and the binary's path within the archive in `scrim.yaml`.
3. **Enterprise Telemetry**: Scrim is built from the ground up to gather tool invocation telemetry. It safely `fork()`s a background reporting process on every execution without blocking the critical path, allowing organizations to track internal tool usage globally.

| Feature | Scrim | [`direnv`](https://direnv.net/) | [`asdf`](https://asdf-vm.com/) | [`mise`](https://mise.jdx.dev/)<br>*(Shims Only)* | [`mise`](https://mise.jdx.dev/)<br>*(Shell Hooks)* |
| :--- | :---: | :---: | :---: | :---: | :---: |
| **Version Management** | ✅ | ✅<br>*(via `$PATH`)* | ✅ | ✅ | ✅ |
| **Shell Env Var Management** | ❌ | ✅ | ❌ | ❌ | ✅ |
| **Fetches Binaries** | ✅ | ❌ | ✅ | ✅ | ✅ |
| **Works Without Plugins** | ✅ | N/A | ❌ | ✅ | ✅ |
| **On-Demand Fetching** | ✅ | ❌<br>*(eager on directory entry)* | ❌<br>*(requires `asdf install`)* | ✅ | ✅ |
| **Works Without Shell Hooks** | ✅ | ❌ | ✅ | ✅ | ❌ |
| **Uses Shims in `PATH`** | ✅<br>*(Symlink to Rust binary)* | ❌ | ✅<br>*(Bash script to Go binary)* | ✅<br>*(Symlink to Rust binary)* | ❌ |
| **System-wide Multi-User Setup** | ✅ | ❌ | ✅ | ✅ | ❌ |
| **Invocation Telemetry** | ✅ | ❌ | ❌ | ❌ | ❌ |
| **Directory Nav. Overhead** | Zero | Low | Zero | Zero | Low |
| **Execution Overhead** | Very Low *(<2ms)* | Zero | Medium | Low | Zero |

### The "Hooks vs. Shims" Trade-off

Scrim is fundamentally built on a **pure shim architecture** because it must intercept every single execution to provide its core enterprise feature: **invocation telemetry**. If Scrim simply modified your shell's `$PATH` (like `mise activate` or `direnv` do), the OS would execute the binary directly. This would bypass Scrim entirely, making reliable telemetry gathering challenging. 

Because Scrim is forced to sit in the critical path of every tool invocation, **relentless optimization for speed is existential**. This is why Scrim is written in Rust, uses native OS symlinks instead of Bash scripts, and safely `fork()`s its telemetry reporting into the background. It is aggressively engineered to add fewer than 2 milliseconds of latency per execution.

However, a shim architecture carries an inherent, cumulative trade-off. If you use Scrim to manage a high-frequency compiler (like `gcc` or `rustc`) that a build tool might invoke 10,000 times during compilation, that tiny 2ms overhead will still add 20 seconds to your build time. While noticeable, this is drastically better than the multiple minutes of overhead introduced by other, slower shim architectures. Because Scrim's overhead is so tightly constrained, it remains tolerable.

The primary value of Scrim is providing seamless versioning and telemetry to end users, IDEs, agents, and less capable build systems. The overhead is worth it to get these abilities in a consistent manner across these disparate contexts. More capable build systems can (and should) manage their toolchains hermetically and provide their own telemetry.

## Development

### AI

This project is an experiment in AI-first development. Nearly every commit is AI-generated. However, the process is very design- and test-driven. Human review is focused on the design and tests, less so on the implementation. Read [DESIGN.md](DESIGN.md), especially the Core Design Principles.

### Building
```bash
bazel build //:scrim
```

### Running Tests
The test suite includes Rust unit tests and E2E integration tests simulating network downloads and telemetry forks:
```bash
bazel test //...
```

### IDE Support (rust-analyzer)
Because this project strictly uses Bazel as its build system, `rust-analyzer` needs a `rust-project.json` file to understand the project structure and dependencies.

You can generate this file by running:
```bash
bazel run @rules_rust//tools/rust_analyzer:gen_rust_project
```
This will place a `rust-project.json` at the root of the repository, enabling features like autocomplete, type hints, and go-to-definition in editors like VSCode or Neovim. You may need to restart your IDE's language server after generating it.
