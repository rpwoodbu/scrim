### Coding Approach
You MUST read and understand the `DESIGN.md` file before proceeding with any tasks or modifications.

### Review Process
All changes MUST receive explicit human review BEFORE they are committed. You MUST review your own work, consult these guidelines, and ensure they have not been violated before seeking human review.

When participating in a design discussion, you MUST first propose changes to `DESIGN.md` and explicitly ask the user for review. You MUST NOT proceed to make any code changes until the user (a human) has explicitly reviewed and approved the design changes in `DESIGN.md`.

### Repository Hygiene
Keep the history clean. Follow these rules to achieve this:
* Commits MUST be complete, including any test or documentation changes relevant to the change. You MUST NOT commit code without ensuring its behavior is tested.
* When modifying dependencies (e.g., `MODULE.bazel`), you MUST run a build/test and include any resulting lockfile updates (e.g., `MODULE.bazel.lock`) in the same commit.
* Yet commits should be small and targeted. Separate concerns deserve separate commits.
* All tests MUST pass on each commit.
* It is appropriate to amend unpushed commits to incorporate fixes that should have been included originally.
* Commit messages MUST be descriptive and clearly explain the 'why' and 'how' of the change.

### Bazel
Bazel builds are hermetic and fully specified. The local environment may not contain toolchains. Do not expect, e.g., `rustc` to be in the `PATH`. Keep the build hermetic.

Bazel may be configured to run in "Build without the Bytes" (BwtB) mode, meaning that its output directory may not contain artifacts. Build with the flag `--remote_download_toplevel` when it is necessary to get the artifact(s) for the requested build target(s). Do not use this gratuitously.
