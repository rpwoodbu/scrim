### Coding Approach
Always read and understand the `DESIGN.md` file before proceeding with any tasks or modifications.

### Review Process
All changes need human review before they are committed. Review your own work before seeking human review. Consult these guidelines and ensure they have not been violated.

When participating in a design discussion, you must first propose changes to `DESIGN.md` and explicitly ask the user for review. You must NOT proceed to make any code changes until the user (a human) has explicitly reviewed and approved the design changes in `DESIGN.md`.

### Repository Hygiene
Keep the history clean. Follow these rules to achieve this:
* Commits must be complete, including any test or documentation changes relevant to the change.
* Yet commits should be small and targeted. Separate concerns deserve separate commits.
* All tests should pass on each commit.
* It is appropriate to amend unpushed commits to incorporate fixes that should have been included originally.

### Bazel
Bazel builds are hermetic and fully specified. The local environment may not contain toolchains. Do not expect, e.g., `rustc` to be in the `PATH`. Keep the build hermetic.

Bazel may be configured to run in "Build without the Bytes" (BwtB) mode, meaning that its output directory may not contain artifacts. Build with the flag `--remote_download_toplevel` when it is necessary to get the artifact(s) for the requested build target(s). Do not use this gratuitously.
