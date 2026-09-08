# Scrim

**Scrim** is a lightweight, transparent proxy for developer tools, designed to provide context-aware tool versioning without requiring shell hooks, profile modifications, or environment variables.

> **Origin of the Name:**  
> The name *Scrim* refers to the thin, translucent fabric used in theater to create lighting effects or hide/reveal elements on stage. It represents a "thin" proxy that stays completely out of sight until needed.

## Key Features

- **Zero-Hook Integration:** Simply link Scrim to any tool name in your `PATH` (e.g., `node`, `go`). Scrim handles the rest based on your current directory.
- **Upward Search Resolution:** Scrim looks upwards from your current working directory to find the nearest `scrim.yaml` config file, stopping at repository (`.git`) boundaries to keep filesystem overhead near zero.
- **Dual Resolution Modes:**
  - **Local Path:** Instantly routes to a pre-installed local executable via `system_path`.
  - **URL Fetching & Caching:** Dynamically downloads a tool from a specified URL via `curl`, validates its integrity using `sha256sum`, unpacks archives automatically (if configured with `archive_path`), and caches it in `~/.cache/scrim/` for instant subsequent executions. Tools can also inherit fetch properties from other tools using `template`.
- **Non-Blocking Telemetry:** Fork-based telemetry runs execution reporting in a background child process, ensuring tool invocation latency remains completely unaffected.

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
