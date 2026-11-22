"""Bazel configuration transition for release-representative benchmarks."""

def _opt_musl_transition_impl(_settings, _attr):
    return {
        "//command_line_option:compilation_mode": "opt",
        "//command_line_option:platforms": ["//:musl_host"],
    }

opt_musl_transition = transition(
    implementation = _opt_musl_transition_impl,
    inputs = [],
    outputs = [
        "//command_line_option:compilation_mode",
        "//command_line_option:platforms",
    ],
)

def _opt_musl_binary_impl(ctx):
    src = ctx.attr.target[0]
    src_exe = src[DefaultInfo].files_to_run.executable
    out = ctx.actions.declare_file(ctx.label.name)
    ctx.actions.symlink(
        output = out,
        target_file = src_exe,
        is_executable = True,
    )
    runfiles = src[DefaultInfo].default_runfiles.merge(ctx.runfiles(files = [out, src_exe]))
    return [
        DefaultInfo(
            files = depset([out]),
            executable = out,
            runfiles = runfiles,
        ),
    ]

opt_musl_binary = rule(
    implementation = _opt_musl_binary_impl,
    attrs = {
        "target": attr.label(
            cfg = opt_musl_transition,
            executable = True,
            mandatory = True,
        ),
        "_allowlist_function_transition": attr.label(
            default = "@bazel_tools//tools/allowlists/function_transition_allowlist",
        ),
    },
    executable = True,
)
