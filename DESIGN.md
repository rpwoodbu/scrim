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
- **Implementation**: Telemetry data will be handed off to a background process or written to a non-blocking queue/file for later processing. The main execution path should not wait for network I/O.

## Build & Project Structure
- Use Bazel with `rules_rust` for building the project.
- The project will be structured to keep the "hot path" (proxying) separate from management logic (adding tools, fetching).

---

## Open Design Questions
- **Shim Strategy**: Should we use a single binary that detects its name from `argv[0]`, or should we generate tiny, tool-specific shims that call into a central `scrim` daemon? (Current plan: `argv[0]` detection).
- **Configuration Format**: Should we support existing files like `.nvmrc` or `go.mod` natively, or require a specific `scrim` config file?
- **Fetching Mechanism**: Should Scrim handle the actual downloads (e.g., using `reqwest`), or delegate to system tools like `curl`/`wget` to keep the binary small?
