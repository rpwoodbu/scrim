# Scrim

**Scrim** is a lightweight, transparent proxy for developer tools, designed to provide context-aware tool versioning without requiring shell hooks, profile modifications, or environment variables.

> **Origin of the Name:**  
> The name *Scrim* refers to the thin, translucent fabric used in theater to create lighting effects or hide/reveal elements on stage. It represents a "thin" proxy that stays completely out of sight until needed.

---

## Key Features

- **Zero-Hook Integration:** Simply link Scrim to any tool name in your `PATH` (e.g., `node`, `go`). Scrim handles the rest based on your current directory.
- **Upward Search Resolution:** Scrim looks upwards from your current working directory to find the nearest `scrim.yaml` config file, stopping at repository (`.git`) boundaries to keep filesystem overhead near zero.
- **Dual Resolution Modes:**
  - **Local Path:** Instantly routes to a pre-installed local executable via `system_path`.
  - **URL Fetching & Caching:** Dynamically downloads a tool from a specified URL via `curl`, validates its integrity using `sha256sum`, unpacks archives automatically (if configured with `archive_path`), and caches it in `~/.cache/scrim/` for instant subsequent executions. Tools can also inherit fetch properties from other tools using `template`.
- **Non-Blocking Telemetry:** Fork-based telemetry runs execution reporting in a background child process, ensuring tool invocation latency remains completely unaffected.

---

## Configuration (`scrim.yaml`)

Define a `scrim.yaml` at the root of your project:

```yaml
telemetry: true # disabled (false) by default
tools:
  node:
    system_path: /usr/local/bin/node
  go:
    url: https://go.dev/dl/go1.21.5.linux-amd64.tar.gz
    sha256: 285c1f0624022839446d32839446d32839446d32839446d32839446d32839446
    archive_path: go/bin/go
  gofmt:
    template: go
    archive_path: go/bin/gofmt
```

---

## Installation & Setup

1. Build the Scrim executable:
   ```bash
   bazel build //:scrim
   ```
2. Copy the built `scrim` binary to a stable directory in your `PATH` (e.g., `~/.local/bin`):
   ```bash
   cp bazel-bin/src/scrim ~/.local/bin/scrim
   ```
3. Create symlinks for the tools you want to wrap pointing to the stable `scrim` binary:
   ```bash
   ln -s scrim ~/.local/bin/node
   ln -s scrim ~/.local/bin/go
   ```
4. Create a `scrim.yaml` in your project folder, and run your command normally:
   ```bash
   node app.js
   ```

---

## Development

### AI

This project is an experiment in AI-first development. Nearly every commit is AI-generated. However, the process is very design- and test-driven. Human review is focused on the design and tests, less so on the implementation. Read [DESIGN.md](DESIGN.md), especially the Core Design Principles.

### Building
```bash
bazel build //...
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
