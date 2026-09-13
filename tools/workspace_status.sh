#!/bin/sh
set -eu

# This script is called by Bazel's --workspace_status_command.
# Keys starting with STABLE_ will trigger rebuilds of stamped targets when they change.

GIT_COMMIT=$(git rev-parse HEAD 2>/dev/null || echo "")
echo "STABLE_GIT_COMMIT ${GIT_COMMIT}"
