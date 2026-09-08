# Scrim

**Scrim** is a lightweight wrapper around command line tools offering version management and usage telemetry without the need for shell hooks. It can download and cache tools from the network, reducing system image maintenance toil.

> **Origin of the Name:**  
> The name *Scrim* refers to the thin, translucent fabric used in theater to create lighting effects or hide/reveal elements on stage. It represents a "thin" proxy that stays completely out of sight until needed.

## Key Features

- **Layered version management:** `scrim.yaml` files define precisely which version of each tool to invoke based on the current working directory. All found `scrim.yaml` files are layered, with more local configuration taking precedence. See [Configuration](#configuration).
- **Remote tool fetching:** Rather than maintain locally-installed tools, Scrim can fetch them from the network. It downloads the archive, unpacks it into a local cache, and executes the binary directly from the cache. Multiple versions of the same archive may be cached simultaneously. Downloads are hardened with a `sha256` hash, improving security and reproducibility.
- **Easy installation:** Scrim is a single statically-linked binary. Create a symlink in your path for each tool Scrim should wrap which points to the `scrim` binary. There are no shell hooks; this will work in any shell and without user configuration.
- **Non-blocking telemetry:** Get usage telemetry for the tools that are run, including full invocation information (coming soon). Telemetry is only performed once the proxied tool is called, removing telemetry from the critical path.
- **Low overhead:** Scrim adds less than 2 milliseconds to the tool invocation.

## Installation

1. Download the executable (e.g., `scrim-linux-amd64`) from the latest GitHub Release.
2. Install the downloaded binary to a stable directory in your `PATH` (e.g., `/usr/local/bin`):
   ```bash
   sudo install -m 755 scrim-linux-amd64 /usr/local/bin/scrim
   ```
3. Create `scrim.yaml` file(s) (keep reading).

## Configuration

Scrim is configured by `scrim.yaml` that may be placed in your project, in your home directory (`~/.config/scrim/`), and/or at the system level (`/etc/scrim/`). They layer, producing a composite configuration whereby more local elements take precedence. Scrim traverses up from the current working directory to discover `scrim.yaml` files, stopping at the repository boundary. This allows a system-wide version of a tool to be defined, but with user, project, and even project subdirectory overrides.

The only other requirement is that each tool needs to have a link (symbolic or hard) in the path which points to the installed `scrim` binary. (This is similar in principle to `busybox`.) This can be defined at the system level, although the configuration is resolved based in your current working directory.

```yaml
telemetry: true # disabled (false) by default
tools:
  node:
    system_path: /bin/node
  go:
    url: https://go.dev/dl/go1.21.5.linux-amd64.tar.gz
    sha256: 285c1f0624022839446d32839446d32839446d32839446d32839446d32839446
    archive_path: go/bin/go
  gofmt:
    template: go
    archive_path: go/bin/gofmt
```

## Usage

Run your command(s) normally; e.g.:
```bash
$ go version
# ... downloading and unpacking only if needed ...
go version go1.21.5 linux/amd64
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
| **On-Demand Fetching** | ✅ | ❌ | ❌ | ✅ | ✅ |
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
