# Scrim: Design & Architecture

**Scrim** is a lightweight, transparent proxy for developer tools, designed to provide context-aware tool versioning without requiring shell hooks or environment manipulation.

> [!NOTE]
> The name **Scrim** refers to the thin, translucent fabric used in theater to create lighting effects or hide/reveal elements on stage. It reflects our goal of being a "thin" proxy that stays behind the curtain until needed.

> [!IMPORTANT]
> All changes to the project must be reflected in this document to ensure the architecture remains transparent and auditable.

## Core Design Principles

- **Test-Driven Rigor**: All functionality must be covered by tests.
- **Performance**: The critical path (resolving and executing a tool) must be as close to zero-overhead as possible.
- **Transparency**: Users should interact with their tools normally; Scrim stays behind the curtain.
- **Reliability**: Failures in telemetry or other auxiliary tasks must never block or prevent tool execution.
- **YAGNI (You Aren't Gonna Need It)**: Favor simplicity and minimal configuration. Avoid preemptive abstractions until they are strictly required.

## Technical Stack

- **Language**: Rust (for memory safety, speed, and static linking).
- **Build System**: Bazel (for reproducible builds).
- **Distribution**: Single static binary.

## Architecture

### 1. The Proxy Mechanism
Scrim works by acting as a drop-in shim for developer tools. Unlike tools that require shell hooks, Scrim requires **zero shell configuration**. 
- Users create symlinks or hardlinks (for slightly better performance) named as the commands they wish to wrap (e.g., `node`, `go`) in a directory already in their `PATH` (e.g., `/usr/local/bin` or `~/.local/bin`).
- These links all point to the single `scrim` binary.
- `scrim` uses the `argv[0]` (the command name) to determine which tool it is proxying.

### 2. Resolution Logic
When a command (e.g., `node`) is invoked, Scrim follows this resolution order:

1.  **Repository Override**: Search upwards from the current working directory (CWD) for a configuration file (`scrim.yaml`). 
    - **Note**: The configuration uses YAML and separates global settings (like `telemetry`) from tool-specific configurations.
    
    Example `scrim.yaml`:
    ```yaml
    telemetry: true
    tools:
      node:
        path: /usr/local/bin/node
      go:
        url: https://go.dev/dl/go1.21.5.linux-amd64.tar.gz
        sha256: 285c1f0624022839446d32
        archive_bin: go/bin/go
    ```
2.  **User/System Default**: If no repository override is found, Scrim falls back to a user-level configuration (e.g., `~/.config/scrim/scrim.yaml`) or a system-level configuration (e.g., `/etc/scrim/scrim.yaml`).
3.  **Path Resolution**:
    - If the resolved version is a **Path**, execute it directly.
    - If the resolved version needs to be **Fetched**, check `~/.cache/scrim/` (or an overridden cache path). If missing, fetch it synchronously (with a progress indicator) and then execute.

### 3. Upward Search Heuristics
To minimize filesystem overhead during resolution:
- Scrim will search upwards from the CWD for a configuration file.
- **Future Optimization**: To prevent unnecessary `stat` calls in large directory trees, Scrim can stop the search at known boundaries (e.g., the user's home directory or the first `.git` directory encountered).

### 4. Execution
To minimize overhead, Scrim will use `execve` (on Unix) to replace the current process with the target tool process. This ensures there is no "parent" Scrim process hanging around during tool execution.

### 5. Telemetry Hook
Telemetry is gathered to track tool usage patterns.
- **Constraint**: Must never block or cause the tool to fail.
- **Implementation**: 
    - Scrim will `fork()` before executing the target tool.
    - The **Parent** process will immediately `execve()` the target tool to preserve the original PID and environment.
    - The **Child** process will handle telemetry gathering and reporting in the background, then exit silently.
    - This ensures telemetry is completely decoupled from the tool's execution latency.

## Build & Project Structure
- Use Bazel with `rules_rust` for building the project.
- **Modular Build Files**: To maintain a clean architecture, avoid a single overarching `BUILD.bazel` file at the repository root. Prefer individual `BUILD.bazel` files distributed within each logical segment of the project.

### Directory Layout
The project enforces the following prescriptive layout:
- `/src/`: Contains the core Rust source code and library logic.
- `/tests/`: Contains standard unit and integration tests as well as validation tests for benchmarks thresholds.
    - Benchmark threshold tests must be marked as `manual` as they are not correctness tests and are subject to flakiness.
- `/benches/`: Contains the statistical microbenchmark harness code.

---

## Testing & Performance

Performance is a primary design goal. To ensure Scrim remains thin and fast, we will implement rigorous performance monitoring.

### 1. Performance Thresholds
- **Hot Path Overhead**: The time added by Scrim when a tool is already resolved (no fetch required) should be **< 2ms**. This ensures that even when tools are called in rapid succession by scripts, the cumulative overhead remains negligible.

### 2. Microbenchmarks
- We will use `criterion` or a similar Rust benchmarking suite to measure:
    - Config resolution time (searching parent directories).
    - Parsing time for the configuration file.
    - End-to-End (E2E) hot path shim overhead (exercising the full proxy flow).
- Benchmarks will be integrated into the Bazel build pipeline.

### 3. Testing Strategy
- **Unit Tests**: Every core module (resolution, parsing, environment handling) must have high test coverage.
- **Integration Tests**: Bazel `sh_test` or `rust_test` targets that simulate:
    - Deeply nested project structures.
    - Missing configurations (falling back to global defaults).
    - Failed fetches and fallback behavior.
- **Telemetry Validation**: Tests to ensure that telemetry failure never impacts the main execution flow.
- **Performance Threshold Validation**: There must be a separate "manual" test target which will run the benchmarks such that they generate proper data and will automatically fail if the required performance thresholds (e.g., < 2ms overhead) are not met.

### 4. Bug Regression Testing
- **Regression Prevention**: If a bug is discovered, a dedicated regression test must be written that clearly reproduces and elucidates the bug, ensuring it never returns.

## Future Work
- **Cache Management**: Commands to clean or inspect the `~/.cache/scrim` directory.
- **Global Config Fallbacks**: Implement support for user-level (`~/.config/scrim/scrim.yaml`) and system-level (`/etc/scrim/scrim.yaml`) default configurations when no repository `scrim.yaml` is found.
- **Cache Override Support**: Support environment variable overrides (e.g., `SCRIM_CACHE_DIR`) to configure the cache directory dynamically.
- **Home Directory Search Boundary**: Stop upward directory traversal for `scrim.yaml` at `$HOME` to prevent scanning system directories when outside of a repository.
