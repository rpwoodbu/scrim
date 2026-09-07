# Scrim: Design & Architecture

**Scrim** is a lightweight, transparent proxy for developer tools, designed to provide context-aware tool versioning without the overhead of environment manipulation.

## Core Design Principles

- **Performance First**: The critical path (resolving and executing a tool) must be as close to zero-overhead as possible.
- **Transparency**: Users should interact with their tools normally; Scrim stays behind the curtain.
- **Reliability**: Failures in non-critical paths (like telemetry or fetching) must never block tool execution.
- **Portability**: A single static binary with minimal dependencies.

## Technical Stack

- **Language**: Rust (for memory safety, speed, and static linking).
- **Build System**: Bazel (for reproducible and scalable builds).
- **Distribution**: Single static binary.

## Architecture

### 1. The Proxy Mechanism
Scrim works by acting as a shim for various developer tools. 
- A single directory (e.g., `~/.scrim/bin`) is added to the user's `PATH`.
- This directory contains symlinks or small "shim" binaries that all point back to the main `scrim` binary.
- `scrim` uses the `arg[0]` (the command name) to determine which tool it is proxying.

### 2. Resolution Logic
When a command (e.g., `node`) is invoked, Scrim follows this resolution order:

1.  **Repository Override**: Search upwards from the current working directory (CWD) for a configuration file (e.g., `.scrim.yaml` or `.tool-versions`).
2.  **User/System Default**: If no repository override is found, use a pre-configured global default.
3.  **Path Resolution**:
    - If the resolved version is a **Path**, execute it directly.
    - If the resolved version needs to be **Fetched**, check `~/.cache/scrim/` (or an overridden cache path). If missing, fetch it synchronously (with a progress indicator) and then execute.

### 3. Execution
To minimize overhead, Scrim will use `execve` (on Unix) to replace the current process with the target tool process. This ensures there is no "parent" Scrim process hanging around during tool execution.

### 4. Telemetry Hook
Telemetry is gathered to track tool usage patterns.
- **Constraint**: Must never block or cause the tool to fail.
- **Implementation**: 
    - Scrim will `fork()` before executing the target tool.
    - The **Parent** process will immediately `execve()` the target tool to preserve the original PID and environment.
    - The **Child** process will handle telemetry gathering and reporting in the background, then exit silently.
    - This ensures telemetry is completely decoupled from the tool's execution latency.

## Build & Project Structure
- Use Bazel with `rules_rust` for building the project.
- The project will be structured to keep the "hot path" (proxying) separate from management logic (adding tools, fetching).

---

## Testing & Performance

Performance is a primary design goal. To ensure Scrim remains thin and fast, we will implement rigorous performance monitoring.

### 1. Performance Thresholds
- **Hot Path Overhead**: The time added by Scrim when a tool is already resolved (no fetch required) should be **< 2ms**.
- **Cold Path Overhead**: (First resolution in a session) should be **< 10ms** (excluding network I/O for fetching).

### 2. Microbenchmarks
- We will use `criterion` or a similar Rust benchmarking suite to measure:
    - Config resolution time (searching parent directories).
    - Parsing time for the configuration file.
    - Forking and `execve` overhead.
- Benchmarks will be integrated into the Bazel build pipeline.

### 3. Testing Strategy
- **Unit Tests**: Every core module (resolution, parsing, environment handling) must have high test coverage.
- **Integration Tests**: Bazel `sh_test` or `rust_test` targets that simulate:
    - Deeply nested project structures.
    - Missing configurations (falling back to global defaults).
    - Failed fetches and fallback behavior.
- **Telemetry Validation**: Tests to ensure that telemetry failure never impacts the main execution flow.

## Open Design Decisions (Resolved)
- **Shim Strategy**: Single binary with `argv[0]` detection. Symlinks will point to the `scrim` binary.
- **Configuration Format**: Scrim-specific configuration only (e.g., `.scrim.json`). We will prioritize simplicity and speed over native support for other tools' config files.
- **Fetching Mechanism**: Use external binaries (like `curl`) for downloads to keep the Scrim binary size to a minimum.
