# Scrim: Design & Architecture

**Scrim** is a lightweight, transparent proxy for developer tools, designed to provide context-aware tool versioning without requiring shell hooks or environment manipulation.

> [!NOTE]
> The name **Scrim** refers to the thin, translucent fabric used in theater to create lighting effects or hide/reveal elements on stage. It reflects our goal of being a "thin" proxy that stays behind the curtain until needed.

> [!IMPORTANT]
> All changes to the project must be reflected in this document to ensure the architecture remains transparent and auditable.

## Core Design Principles

- **Design-Driven Rigor**: All user-visible functionality and significant implementation details MUST be specified in this design document. Other implementation details MAY also be specified here.
- **Test-Driven Rigor**: All functionality must be covered by tests. All bugs must be proven with a test included with the fix.
- **Performance**: The critical path (resolving and executing a tool) must be as close to zero-overhead as possible.
- **Backward Compatibility**: Once Scrim reaches GA (v1), configuration and behavior MUST be backward-compatible, as Scrim is bootstrapping other tools.
- **Transparency**: Users should interact with their tools normally; Scrim stays behind the curtain.
- **Actionable UX**: Scrim must never fail silently; errors and progress must be explicit and clear.
- **Reliability**: Failures in telemetry or other auxiliary tasks must never block or prevent tool execution.
- **YAGNI (You Aren't Gonna Need It)**: Favor simplicity and minimal configuration. Avoid preemptive abstractions until they are strictly required.
- **DRY (Don't Repeat Yourself)**: Avoid duplicating logic or configuration; centralize shared behavior.

## UX

- **Actionable**: Errors must be explicit and clear. Scrim must never fail silently.
- **Attribution**: All error output must clearly indicate that it originates from Scrim.
- **Visibility**: Proactively inform the user during long-running operations (e.g., unpacking archives).

### Management CLI

When invoked directly as `scrim` (i.e., `argv[0]` is `scrim`), Scrim provides a management CLI rather than proxying a tool. Running `scrim` without any arguments, or with invalid arguments, will display a short message guiding the user how to get help. The CLI supports the following commands:
- `help`: Displays usage information and available commands.
- `version`: Displays the version of Scrim.
- `config`: Reports the resultant aggregated configuration after resolving all configuration layers. Unspecified fields and empty collections are omitted from the output to reduce noise.

### Configuration

Example `scrim.yaml`:
```yaml
telemetry: true # disabled (false) by default
tools:
  node:
    system_path: /bin/node
  go:
    url: https://go.dev/dl/go1.21.5.linux-amd64.tar.gz
    sha256: 285c1f0624022839446d32
    archive_path: go/bin/go
  gofmt:
    template: go
    archive_path: go/bin/gofmt
```

## Architecture

### Technical Stack

- **Language**: Rust (for memory safety, speed, and static linking).
- **Build System**: Bazel (for reproducible builds).
- **Distribution**: Single static binary.

### The Proxy Mechanism
Scrim works by acting as a drop-in shim for developer tools. Unlike tools that require shell hooks, Scrim requires **zero shell configuration**. 
- Users create symlinks or hardlinks (for slightly better performance) named as the commands they wish to wrap (e.g., `node`, `go`) in a directory already in their `PATH` (e.g., `/usr/local/bin` or `~/.local/bin`).
- These links all point to the single `scrim` binary.
- `scrim` uses the `argv[0]` (the command name) to determine which tool it is proxying.
- **Exception**: If `argv[0]` is `scrim` itself, it presents a management CLI.


### Resolution Logic
When a command (e.g., `node`) is invoked, Scrim follows this resolution logic:

1.  **Configuration Layering**: Scrim resolves configurations by aggregating them across multiple layers. The resultant configuration includes all tools found across all layers. In cases where the same tool or global setting is defined in multiple layers, the layer with the highest precedence wins (i.e., precedence is evaluated on a per-tool basis). The configuration layers, in increasing order of precedence, are:
    1. **System-Level Configuration**: `/etc/scrim/scrim.yaml`
    2. **User-Level Configuration**: `~/.config/scrim/scrim.yaml`
    3. **Recursive Search**: Search upwards from the current working directory (CWD) for `scrim.yaml` files. To minimize filesystem overhead, the search automatically stops when it encounters a known repository boundary (e.g., a `.git` directory). Configuration files found in child directories take precedence over those found in their parent directories.
2.  **Configuration Rules**:
    - **Note**: The configuration uses YAML and separates global settings (like `telemetry`) from tool-specific configurations.
    - **Tool Templates**: A tool can specify a `template: <tool_name>` property to inherit the `url` and `sha256` of another tool configuration, minimizing repetition and preventing mismatches within toolchains. Template chains are not allowed; a templated entry must directly reference a concrete tool configuration.
    
    - **Configuration Validation**: To enforce the actionable UX principles, Scrim will explicitly reject invalid configurations with clear error messages rather than silently ignoring properties. Specifically:
        1. YAML parsing must be strict: unknown or misspelled fields must be rejected with an error rather than silently ignored.
        2. `system_path` is mutually exclusive with all fetch-related properties (`url`, `sha256`, `template`, `archive_path`).
        3. `template` is mutually exclusive with `url` and `sha256`.
        4. If a tool requires fetching, both `url` and `sha256` must be present (neither can be provided without the other).
        5. `archive_path` cannot be provided if the tool doesn't resolve to a fetchable archive (i.e., no `url` or `template` is provided).
        6. Empty tool configurations are invalid. Every tool must specify a `system_path`, a `url` (with `sha256`), or a `template`.

3.  **Execution**:
    - If the resolved version is a **Local Path** (via `system_path`), execute it directly.
    - If the resolved version needs to be **Fetched**, check `~/.cache/scrim/tools/<sha256>/`. Caching purely by the `sha256` digest (omitting the tool name) maximizes cache hits when multiple repositories or templated aliases refer to the same payload.
    - If missing, fetch it synchronously, verify the digest, extract the archive (if applicable), and then execute.

### Execution
To minimize overhead, Scrim will use `execve` (on Unix) to replace the current process with the target tool process. This ensures there is no "parent" Scrim process hanging around during tool execution.

### Telemetry Hook
Telemetry is gathered to track tool usage patterns.
- **Constraint**: Must never block or cause the tool to fail.
- **Implementation**: 
    - Scrim will `fork()` before executing the target tool.
    - The **Parent** process will immediately `execve()` the target tool to preserve the original PID and environment.
    - The **Child** process will handle telemetry gathering and reporting in the background, then exit silently. Currently, telemetry is simply written to a hardcoded log file at `/tmp/scrim_telemetry.log`.
    - This ensures telemetry is completely decoupled from the tool's execution latency.

### Logging & User Communication
Scrim avoids heavy external logging crates. Instead, it uses a lightweight, internal logging module (e.g., `src/logger.rs`) with custom macros (e.g., `scrim_error!`, `scrim_progress!`) to ensure consistent attribution.

## Build & Project Structure
- Use Bazel with `rules_rust` for building the project.
- **Modular Build Files**: To maintain a clean architecture, avoid a single overarching `BUILD.bazel` file at the repository root. Prefer individual `BUILD.bazel` files distributed within each logical segment of the project.

### CI/CD
Releases are automated via GitHub Actions.
- The pipeline triggers on pushes to version tags (e.g., `v*`).
- It runs all tests, including unit, integration, and manual performance validation tests, ensuring regressions are not published.
- It builds a statically linked Linux amd64 release binary using Bazel.
- The binary is published as a GitHub Release artifact alongside the source code.

### Directory Layout
The project enforces the following prescriptive layout:
- `/src/`: Contains the core Rust source code and library logic.
- `/tests/`: Contains standard unit and integration tests as well as validation tests for benchmarks thresholds.
    - Benchmark threshold tests must be marked as `manual` as they are not correctness tests and are subject to flakiness.
- `/benches/`: Contains the statistical microbenchmark harness code.

## Testing & Performance

Performance is a primary design goal. To ensure Scrim remains thin and fast, we will implement rigorous performance monitoring.

### Performance Thresholds
- **Hot Path Overhead**: The time added by Scrim when a tool is already resolved (no fetch required) should be **< 2ms**. This ensures that even when tools are called in rapid succession by scripts, the cumulative overhead remains negligible.

### Microbenchmarks
- We will use `criterion` or a similar Rust benchmarking suite to measure:
    - Config resolution time (searching parent directories).
    - Parsing time for the configuration file.
    - End-to-End (E2E) hot path shim overhead (exercising the full proxy flow).
- Benchmarks will be integrated into the Bazel build pipeline.

### Testing Strategy
- **Unit Tests**: Every core module (resolution, parsing, environment handling) must have high test coverage.
- **Integration Tests**: Bazel `sh_test` or `rust_test` targets that simulate:
    - Deeply nested project structures.
    - Missing configurations (falling back to global defaults).
    - Failed fetches and fallback behavior.
- **Telemetry Validation**: Tests to ensure that telemetry failure never impacts the main execution flow.
- **Performance Threshold Validation**: There must be a separate "manual" test target which will run the benchmarks such that they generate proper data and will automatically fail if the required performance thresholds (e.g., < 2ms overhead) are not met.

### Bug Regression Testing
- **Regression Prevention**: If a bug is discovered, a dedicated regression test must be written that clearly reproduces and elucidates the bug, ensuring it never returns.

## Future Work
- **Cache Management**: Commands to clean or inspect the `~/.cache/scrim` directory.
- **Cache Override Support**: Support environment variable overrides (e.g., `SCRIM_CACHE_DIR`) to configure the cache directory dynamically.
- **Home Directory Search Boundary**: Stop upward directory traversal for `scrim.yaml` at `$HOME` to prevent scanning system directories when outside of a repository.
- **Concurrency Safety**: Safely support simultaneous concurrent access, notably when fetching the tool.
- **Native Fetching**: Replace external shell command dependencies (`curl`, `sha256sum`, `tar`, `unzip`) with native Rust crates to make the static binary truly self-contained.
- **Configurable Telemetry**: Support configuring the telemetry behavior and output destination (rather than hardcoding `/tmp/scrim_telemetry.log`).
- **Cross-Platform Releases**: Support for building and releasing binaries for other architectures and operating systems (e.g., ARM64, macOS).
