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
- **Actionable UX**: Usually Scrim is silent, but when messages are appropriate, they should be concise and helpful.
- **Reliability**: Failures in telemetry or other auxiliary tasks must never block or prevent tool execution.
- **YAGNI (You Aren't Gonna Need It)**: Favor simplicity and minimal configuration. Avoid preemptive abstractions until they are strictly required.
- **DRY (Don't Repeat Yourself)**: Avoid duplicating logic or configuration; centralize shared behavior.
- **No Global State**: Library modules MUST NOT read or mutate process-global state (e.g., environment variables). All global state MUST be captured at the application entrypoint and passed to internal logic via explicit parameters to ensure modularity, thread safety, and testability.

## UX

- **Actionable**: Errors must be explicit, clear, and provide enough context for the user to take corrective action. Scrim must never fail silently.
- **Attribution**: All error output must clearly indicate that it originates from Scrim.
- **Visibility**: Proactively inform the user during long-running operations (e.g., unpacking archives).

### Management CLI

When invoked directly as `scrim` (i.e., `argv[0]` is `scrim`), Scrim provides a management CLI rather than proxying a tool. CLI arguments are parsed using `clap`. Running `scrim` without any arguments will display usage information and available commands. Scrim will fail with an error if the user provides any invalid arguments or undefined flags. The main help output also documents supported environment variables (e.g., `XDG_CACHE_HOME`, `XDG_CONFIG_HOME`). The CLI supports the following commands:
- `help`: Displays usage information and available commands.
- `version`: Displays the version of Scrim. If built with workspace status stamping (e.g., release builds), it outputs the release commit hash alongside the semantic version in the format `scrim <version> (<commit_hash>)`. In unstamped builds, it outputs `scrim <version>`.
- `config`: Reports the resultant aggregated configuration after resolving all configuration layers, or reports a specific tool's configuration if an optional tool name argument is provided. Unspecified fields and empty collections are omitted from the output to reduce noise.
- `links`: Updates links in a specified target directory for all defined tools to point to the `scrim` binary.
- `run`: Runs a defined tool directly without requiring a link.
- `which`: Locates and prints the absolute path of the binary that would be executed for a tool without running it.

#### Config Command (`scrim config [TOOL]`)

The `scrim config` command reports the resultant aggregated configuration after resolving all configuration layers, or reports the configuration of a specific tool if `<TOOL>` is provided. Unspecified fields and empty collections are omitted from the output to reduce noise.

- **Arguments**: Optional `<TOOL>` (name of the tool whose configuration to report).
- **Behavior**:
  1. Resolves aggregated configuration across system, user, and workspace layers.
  2. If `<TOOL>` is omitted, serializes and prints the entire aggregated configuration in YAML format, with tools sorted in ascending lexicographical order by tool name.
  3. If `<TOOL>` is provided, looks up `<TOOL>` in the aggregated configuration. If found, serializes and prints only that tool's configuration in YAML format (omitting unspecified fields).
- **Error Handling**:
  - Exits with non-zero status and an attributed error message if `<TOOL>` is provided but not found in the aggregated configuration (`[Scrim] Error: Tool '<TOOL>' not found in config`), or if invalid flags or additional arguments are provided.

#### Links Command (`scrim links [OPTIONS] <DIR>`)

The `scrim links` command creates or updates links in the specified directory `<DIR>` to point to the current `scrim` executable for all tools declared across resolved configuration layers.

- **Options**:
  - `-n, --dry-run`: Performs a dry run without creating or removing any symlinks. Reports the actions that would be performed and exits with a status code indicating whether any modifications are needed. The exit code behavior is documented in the command's help message.
- **Arguments**: `<DIR>` (path to directory where links should be created/updated).
- **Behavior**:
  1. Resolves aggregated configuration across system, user, and workspace layers to get the set of defined tool names.
  2. Resolves the executable path of the current `scrim` binary.
  3. In dry-run mode, logs that Scrim is updating links in dry-run mode (`[Scrim] Updating links in dry-run mode`).
  4. For each tool in the aggregated configuration:
     - If `<DIR>/<tool_name>` already exists as a symbolic link pointing to the relative path of the `scrim` binary, it is skipped.
     - If an existing file or symlink pointing elsewhere is present, reports an error, skips it, and continues.
     - Otherwise, in normal mode, creates a symbolic link at `<DIR>/<tool_name>` pointing to the relative path of the `scrim` binary and logs progress (`[Scrim] Linked <tool_name> -> <rel_path>`). In dry-run mode, logs what link would be created (`[Scrim] Would link <tool_name> -> <rel_path>`) without modifying the filesystem.
  5. Scans `<DIR>` for any existing symbolic links pointing to the `scrim` binary whose command names are not in the aggregated tool configuration:
     - In normal mode, removes the obsolete symlink and logs progress (`[Scrim] Removed obsolete link <file_name>`).
     - In dry-run mode, logs what link would be removed (`[Scrim] Would remove obsolete link <file_name>`) without modifying the filesystem.
  6. If all links are already correct and no changes would be made, logs that fact (`[Scrim] All links are correct`).
- **Exit Status**:
  - In normal mode: exits with status `0` on success.
  - In dry-run mode:
    - Exits with status `0` if all links are already correct (no changes would be made).
    - Exits with status `2` if changes would have been made (links to create or obsolete links to remove).
  - On error (in either mode): exits with non-zero status (`1`) and an error message if `<DIR>` is omitted, if `<DIR>` does not exist or is not a directory, if the `scrim` executable path cannot be determined, if conflicting files or symlinks prevent link creation, or if filesystem operations fail.

#### Run Command (`scrim run <TOOL> [args...]`)

The `scrim run` command executes any tool defined in the aggregated configuration directly, without requiring a symlink or hardlink to `scrim`.

- **Arguments**: `<TOOL>` (name of the tool to run), followed by any optional arguments `[args...]` to pass through to the executed tool.
- **Behavior**: Behaves identically to executing `<TOOL>` via a link. Following the convention of tools like `sudo`, `--` is used to delimit the end of `scrim` options; subsequent options and arguments are passed to the command.
- **Error Handling**: Exits with non-zero status and an error message if `<TOOL>` is omitted, if invalid flags are passed to `scrim`, or if tool resolution/execution fails.

#### Which Command (`scrim which <TOOL>`)

The `scrim which` command outputs the precise absolute path of the binary that would be executed for a given tool according to the [Resolution Logic](#resolution-logic), but does not execute the binary or trigger downloads.

- **Arguments**: `<TOOL>` (name of the tool to look up).
- **Behavior**: Resolves `<TOOL>` using Scrim's resolution rules and prints the target executable's absolute path to standard output without executing it or fetching missing tools.
- **Error Handling**: Exits with non-zero status (`1`) and an attributed error message if `<TOOL>` is omitted (`[Scrim] Error: tool name argument required for which command`), if `<TOOL>` is not found in the aggregated configuration (`[Scrim] Error: Tool '<TOOL>' not found in config`), or if configuration/resolution validation fails.



### Configuration

Example `scrim.yaml`:
```yaml
telemetry: true # disabled (false) by default
imports:
  - base-tools.yaml
  - path: ../overrides/tools.yaml
    optional: true
    precedence: after
tools:
  node:
    system_path: /bin/node
  go:
    # Uses system_path if it exists on disk; falls back to fetching via url if absent
    system_path: /usr/local/go/bin/go
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
    2. **User-Level Configuration**: `<resolved_config_dir>/scrim/scrim.yaml`. Following the [XDG Base Directory Specification][xdg-basedir], `<resolved_config_dir>` is determined by:
        1. Environment variable: `XDG_CONFIG_HOME`, if set, non-empty, and an absolute path.
        2. Default location: `~/.config` (falling back to `./.config` if `$HOME` is unset).
    3. **Recursive Search**: Search upwards from the current working directory (CWD) for `scrim.yaml` files. To minimize filesystem overhead, the search automatically stops when it encounters a known repository boundary (e.g., a `.git` directory). Configuration files found in child directories take precedence over those found in their parent directories.
2.  **Configuration Rules**:
    - **Note**: The configuration uses YAML and separates global settings (like `telemetry`) from tool-specific configurations.
    - **Configuration Imports**: A configuration file may specify an `imports` list to include definitions from other configuration files.
        - **Syntax**: Entries in `imports` can be either a string shorthand representing the relative or absolute path (e.g., `- base-tools.yaml` or `- shared/tools.yaml`) or a structured object:
          ```yaml
          imports:
            - path: <path>
              optional: false     # default: false (fails if file does not exist)
              precedence: before  # default: before; options are "before" or "after"
          ```
        - **Path Resolution**: Relative paths specified in `path` (or string shorthand) MUST be resolved relative to the directory containing the configuration file declaring the import, never relative to the current working directory.
        - **Missing Files**: If an imported file does not exist:
          - If `optional` is `false` (the default), Scrim MUST reject the configuration with an actionable error.
          - If `optional` is `true`, Scrim silently ignores the missing file and proceeds.
        - **Precedence Order**:
          - Imports with `precedence: before` (the default) are evaluated before the importing file's own tool and setting definitions, acting as defaults that the importing file can override.
          - Imports with `precedence: after` are evaluated after the importing file's own definitions, taking precedence over and overriding definitions in the importing file.
          - When multiple imports share the same precedence category (`before` or `after`), they are evaluated sequentially in the order listed, with later imports taking precedence over earlier imports within that category.
          - Concretely, definitions within a single configuration file are assembled in the order:
            1. All `precedence: before` imports (in declared sequence).
            2. The current configuration file's own definitions.
            3. All `precedence: after` imports (in declared sequence).
        - **Transitive Imports & Cycle Detection**: Imported files may themselves contain `imports`. Scrim tracks the canonical paths of configuration files currently in the active import stack; if a circular import is detected, Scrim MUST abort with an actionable error indicating the cycle.
    - **Supported Digests**: Scrim supports cryptographic checksums using `sha256` or `sha512`.
    - **Tool Templates**: A tool can specify a `template: <tool_name>` property to inherit the `url` and configured digest of another tool configuration, minimizing repetition and preventing mismatches within toolchains. Template chains are not allowed; a templated entry must directly reference a concrete tool configuration.
    
    - **Configuration Validation**: To enforce the actionable UX principles, Scrim will explicitly reject invalid configurations with clear error messages rather than silently ignoring properties. Specifically:
        1. YAML parsing must be strict: unknown or misspelled fields must be rejected with an error rather than silently ignored.
        2. `system_path` may be specified alongside fetch-related properties (`url` or `template`, along with `archive_path`). In this mode, `system_path` is preferred if the target executable exists on the system at execution time; if the file at `system_path` does not exist, Scrim falls back to fetching the tool. However, `system_path` cannot be combined with partial fetch definitions (all required fetch properties must still be validly specified).
        3. `template` is mutually exclusive with `url` and digests.
        4. Exactly one digest must be specified when fetching (digest algorithms are mutually exclusive).
        5. If a tool specifies `url` or a digest, both `url` and a digest must be present (neither can be provided without the other).
        6. `archive_path` cannot be provided if the tool doesn't resolve to a fetchable archive (i.e., no `url` or `template` is provided). If `archive_path` is omitted, Scrim defaults to `<tool_name>` at the root of the extracted directory for standard archives (`.tar.gz`, `.zip`, `.tar.xz`, etc.), and to `AppRun` at the root of the extracted directory for AppImage packages (`.AppImage`, `.appimage`).
        7. `system_path` MUST be an absolute path. Relative paths in `system_path` are rejected with an actionable error.
        8. Empty tool configurations are invalid. Every tool must specify a `system_path`, a `url` (with a digest), or a `template`.

3.  **Execution**:
    - If the tool specifies both a `system_path` and fetch configuration (or template):
      - At execution time, Scrim checks whether the executable at `system_path` exists on the filesystem. If it exists, Scrim uses the local executable directly.
      - If the file at `system_path` does not exist, Scrim falls back to fetching the tool as described below.
    - If only `system_path` is specified, execute it directly.
    - If only fetch configuration (or `template`) is specified (or when falling back from a missing `system_path`), the resolved version needs to be **Fetched**: check the tools cache directory (`<resolved_cache_dir>/tools/`). Following the [XDG Base Directory Specification][xdg-basedir], the base cache directory `<resolved_cache_dir>` is `<resolved_xdg_cache_home>/scrim`, where `<resolved_xdg_cache_home>` is resolved as:
        1. Environment variable: `XDG_CACHE_HOME`, if set, non-empty, and an absolute path.
        2. Default location: `~/.cache` (falling back to `./.cache` if `$HOME` is unset).
      Two entries must exist in `<resolved_cache_dir>/tools/` for a cache hit: a subdirectory named `<digest_algorithm>:<digest_value>`, and a file named `<digest_algorithm>:<digest_value>.url` which must contain the specified URL. Caching by both `<digest_algorithm>:<digest_value>` and `url` ensures that updating the tool's `url` in `scrim.yaml` without changing the digest will not continue executing the old version.
    - If missing or if the cached URL does not match the configured `url`, Scrim continues to download the tool from the specified `url`. During download, Scrim computes the actual digest in a streaming manner using the specified digest algorithm. If the computed digest does not match the configured digest, Scrim aborts execution and reports an actionable verification error displaying the algorithm, expected digest, and actual digest, facilitating easy configuration updates. If the digest matches, Scrim records the URL in `<resolved_cache_dir>/tools/<digest_algorithm>:<digest_value>.url` (supporting multiple verified source URLs, such as mirrors, on separate lines), extracts the archive (if applicable), and executes the tool. Fetching is implemented entirely using native Rust crates (`ureq`, `sha2`, `tar`, `flate2`, `zip`, `lzma-rs`, `backhand`) to eliminate external shell command dependencies (`curl`, digest utilities, `tar`, `unzip`, `xz`, `unsquashfs`, FUSE) and ensure Scrim is a truly self-contained binary.
        - **Concurrency Safety & Cache Poisoning**: To prevent multiple Scrim processes from corrupting the cache or using a partially downloaded file, downloads are written to a temporary file (`tempfile::NamedTempFile`) and only atomically persisted to the final cache destination upon successful verification.
        - **Streaming Hash Optimization**: To maximize performance during fetching, the cryptographic digest is computed in memory via a custom streaming writer concurrently with the download stream, completely eliminating the penalty of double-reading the payload from disk.
        - **IPv4 & IPv6 Dual-Stack Support**: The HTTP fetching client supports both IPv4 and IPv6 network connectivity, resolving dual-stack endpoints and handling IPv6 literal addresses in URLs (e.g., `http://[::1]:<port>/...`).
        - **Archive Format Support**: Scrim supports compressed archives and packages for fetched tools, automatically recognized by file extension (ignoring URL query parameters):
            - **Gzip-compressed tar archives**: `.tar.gz`, `.tgz` (decompressed via `flate2` and extracted via `tar`).
            - **Zip archives**: `.zip` (extracted via `zip`).
            - **XZ-compressed tar archives**: `.tar.xz`, `.txz`, `.xz` (decompressed via pure-Rust `lzma-rs` and extracted via `tar`).
            - **AppImage packages**: `.AppImage`, `.appimage`. Scrim supports Type 2 AppImages by natively parsing the ELF headers to locate the embedded SquashFS filesystem offset, and extracting the filesystem using pure Rust without requiring external tools or FUSE (`libfuse`). Type 1 AppImages (ISO 9660 based) are deprecated and explicitly rejected with an actionable error. If `archive_path` is omitted in the tool configuration, Scrim automatically defaults to executing `AppRun` at the root of the extracted image.
            All archives are unpacked into a temporary directory before atomic relocation to the cache path, and target binaries are verified to exist, checked not to be symlinks outside of the extracted directory, and marked executable (`0o755`).

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
Scrim avoids heavy external logging crates. Instead, it uses a lightweight, internal logging module with custom macros (e.g., `scrim_error!`, `scrim_progress!`) to ensure consistent attribution.

## Build & Project Structure
- Use Bazel with `rules_rust` for building the project.
- **Static Linking via musl**: Bazel and `rules_rust` are configured to target `x86_64-unknown-linux-musl` to eliminate runtime dynamic C library dependencies and ensure universal portability across Linux distributions.
- **Modular Build Files & Packaging**: To maintain a clean architecture, avoid monolithic build files or libraries. Logical domains are partitioned into distinct Bazel packages with their own `BUILD.bazel` files.
- **Visibility Protection**: Packages must enforce least privilege through Bazel target visibilities. Targets should only be visible to packages that strictly require them, protecting architectural boundaries from inappropriate coupling.
- **Binary Version Stamping**: Release binaries are stamped with the Git commit hash using Bazel workspace status stamping (`--workspace_status_command`). A workspace status script (`tools/workspace_status.sh`) provides `STABLE_GIT_COMMIT`, which `rules_rust` embeds into the binary during stamped builds (`--stamp`). In unstamped builds, the commit hash is omitted.

### CI/CD
CI and releases are automated via GitHub Actions.
- The pipeline triggers on pushes to the `main` branch as well as on version tags (e.g., `v*`).
- Pushes to `main` build and test the codebase to validate commits and maintain warm Bazel caches (Bazelisk, repository, and disk cache) on the default branch scope.
- It runs all tests, including unit, integration, and manual performance validation tests, ensuring regressions are not published.
- It builds a statically linked Linux amd64 release binary using Bazel with workspace stamping enabled (`--stamp`).
- The binary is published as a GitHub Release artifact alongside the source code only when triggered by a version tag.
- Build status for `main` is tracked via a workflow status badge in `README.md`.

### Directory Layout
The project enforces the following prescriptive layout:
- `/src/`: Contains the executable binary target package (`/src/bin/`) and modular library subpackages corresponding to distinct architectural concerns (e.g., CLI parsing, configuration, fetching, linking, logging, resolution, telemetry, versioning).
- `/tests/`: Contains standard unit and integration tests as well as validation tests for benchmarks thresholds.
    - Benchmark threshold tests must be marked as `manual` and `exclusive` as they are not correctness tests and require dedicated machine resources to minimize timing noise and flakiness.
- `/benches/`: Contains the statistical microbenchmark harness code.
- `/tools/`: Contains build and workspace automation scripts (e.g., workspace status script for binary stamping).

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
- **Unit Tests**: Every core module must have high test coverage, maintained via localized unit tests within each package.
- **Integration Tests**: Bazel `sh_test` or `rust_test` targets that simulate:
    - Deeply nested project structures.
    - Missing configurations (falling back to global defaults).
    - Failed fetches and fallback behavior.
- **Telemetry Validation**: Tests to ensure that telemetry failure never impacts the main execution flow.
- **Performance Threshold Validation**: There must be a separate "manual" test target (tagged `manual` and `exclusive` to ensure isolated execution without noise from concurrent actions) which will run the benchmarks such that they generate proper data and will automatically fail if the required performance thresholds (e.g., < 2ms overhead) are not met.

### Bug Regression Testing
- **Regression Prevention**: If a bug is discovered, a dedicated regression test must be written that clearly reproduces and elucidates the bug, ensuring it never returns.

## Future Work
- **Cache Management**: Commands to clean or inspect the cache directory.
- **Home Directory Search Boundary**: Stop upward directory traversal for `scrim.yaml` at `$HOME` to prevent scanning system directories when outside of a repository.
- **Configurable Telemetry**: Support configuring the telemetry behavior and output destination (rather than hardcoding `/tmp/scrim_telemetry.log`).
- **Cross-Platform Releases**: Support for building and releasing binaries for other architectures and operating systems (e.g., ARM64, macOS).
- **Per-Platform Configuration**: Configuration needs to allow per-platform specification.

[xdg-basedir]: https://specifications.freedesktop.org/basedir/0.8/ "XDG Base Directory Specification (version 0.8)"
