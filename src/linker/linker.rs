use config::Config;
use logger::{scrim_error, scrim_progress};
use resolver::resolve_tool;
use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Component, Path, PathBuf};

pub fn path_relative_from(target: &Path, base: &Path) -> PathBuf {
    let target_components: Vec<_> = target.components().collect();
    let base_components: Vec<_> = base.components().collect();
    let mut i = 0;
    while i < target_components.len()
        && i < base_components.len()
        && target_components[i] == base_components[i]
    {
        i += 1;
    }
    let mut rel = PathBuf::new();
    for comp in &base_components[i..] {
        if matches!(comp, Component::Normal(_)) {
            rel.push("..");
        }
    }
    for comp in &target_components[i..] {
        rel.push(comp.as_os_str());
    }
    if rel.as_os_str().is_empty() {
        rel.push(".");
    }
    rel
}

pub fn create_links(
    target_dir: &Path,
    scrim_exe: &Path,
    config: &Config,
    dry_run: bool,
    prune: bool,
) -> Result<bool, Box<dyn std::error::Error>> {
    if !target_dir.is_dir() {
        return Err(format!(
            "Target directory '{}' does not exist or is not a directory",
            target_dir.display()
        )
        .into());
    }

    // Step 0: Check for any semantic errors across the aggregated configuration.
    // Fails immediately with a non-zero exit status if any semantic error exists across the aggregated configuration, making no filesystem changes.
    let mut validation_errors = Vec::new();
    for (tool_name, tool_config) in &config.tools {
        if let Err(e) = tool_config.validate(tool_name) {
            validation_errors.push(e);
        } else if let Some(template_name) = &tool_config.template {
            if let Some(template_config) = config.tools.get(template_name) {
                if let Err(e) = template_config.validate(template_name) {
                    validation_errors.push(e);
                } else if template_config.template.is_some() {
                    validation_errors.push(format!(
                        "Tool '{}' references template '{}', which is also a template. Template chaining is not allowed.",
                        tool_name, template_name
                    ));
                }
            } else {
                validation_errors.push(format!(
                    "Tool '{}' references template '{}', which was not found.",
                    tool_name, template_name
                ));
            }
        }
    }
    if !validation_errors.is_empty() {
        for err in &validation_errors {
            scrim_error!("{}", err);
        }
        return Err("One or more tool configurations are invalid".into());
    }

    let canonical_target_dir = fs::canonicalize(target_dir).unwrap_or_else(|_| target_dir.to_path_buf());
    let canonical_scrim_exe = fs::canonicalize(scrim_exe).unwrap_or_else(|_| scrim_exe.to_path_buf());

    let rel_scrim_exe = path_relative_from(&canonical_scrim_exe, &canonical_target_dir);

    if dry_run {
        scrim_progress!("Updating links in dry-run mode");
    }

    // 1. Create or update symlinks for configured tools on host platform
    let mut tool_names: Vec<_> = config.tools.keys().cloned().collect();
    tool_names.sort();

    let mut host_configured_tools = BTreeSet::new();
    let mut had_errors = false;

    for tool in &tool_names {
        match resolve_tool(tool, config) {
            Ok(Some(_)) => {
                host_configured_tools.insert(tool.clone());
            }
            Ok(None) => {
                // Not configured for current host platform, skip
            }
            Err(e) => {
                scrim_error!("{}", e);
                had_errors = true;
            }
        }
    }

    let mut modified = false;

    for tool in &host_configured_tools {
        let link_path = target_dir.join(tool);

        if let Ok(meta) = fs::symlink_metadata(&link_path) {
            if meta.file_type().is_symlink() {
                if let Ok(target) = fs::read_link(&link_path) {
                    if target == rel_scrim_exe {
                        continue;
                    }
                }
                scrim_error!("'{}' already exists and points elsewhere", link_path.display());
                had_errors = true;
                continue;
            } else {
                scrim_error!("'{}' already exists", link_path.display());
                had_errors = true;
                continue;
            }
        }

        if !dry_run {
            symlink(&rel_scrim_exe, &link_path)?;
            scrim_progress!("Linked {} -> {}", tool, rel_scrim_exe.display());
        } else {
            scrim_progress!("Would link {} -> {}", tool, rel_scrim_exe.display());
        }
        modified = true;
    }

    // 2. Remove obsolete symlinks pointing to scrim if prune is enabled
    if prune {
        match fs::read_dir(target_dir) {
            Ok(entries) => {
                for entry in entries {
                    let entry = match entry {
                        Ok(e) => e,
                        Err(e) => {
                            scrim_error!(
                                "Failed to read directory entry in '{}': {}",
                                target_dir.display(),
                                e
                            );
                            had_errors = true;
                            continue;
                        }
                    };
                    let entry_path = entry.path();
                    let file_name = match entry_path.file_name().and_then(|s| s.to_str()) {
                        Some(name) => name.to_string(),
                        None => continue,
                    };

                    // If this file is a tool configured for the host platform, it was just processed above
                    if host_configured_tools.contains(&file_name) {
                        continue;
                    }

                    // Check if entry is a symlink pointing to the scrim binary
                    if let Ok(meta) = fs::symlink_metadata(&entry_path) {
                        if meta.file_type().is_symlink() {
                            let is_scrim_link = if let Ok(target) = fs::read_link(&entry_path) {
                                let resolved_target = if target.is_relative() {
                                    canonical_target_dir.join(&target)
                                } else {
                                    target.clone()
                                };

                                let canonical_resolved = fs::canonicalize(&resolved_target)
                                    .unwrap_or(resolved_target);

                                canonical_resolved == canonical_scrim_exe
                            } else {
                                false
                            };

                            if is_scrim_link {
                                if !dry_run {
                                    if let Err(e) = fs::remove_file(&entry_path) {
                                        scrim_error!(
                                            "Failed to remove obsolete link '{}': {}",
                                            file_name,
                                            e
                                        );
                                        had_errors = true;
                                    } else {
                                        scrim_progress!("Removed obsolete link {}", file_name);
                                        modified = true;
                                    }
                                } else {
                                    scrim_progress!("Would remove obsolete link {}", file_name);
                                    modified = true;
                                }
                            }
                        }
                    }
                }
            }
            Err(e) => {
                scrim_error!("Failed to read directory '{}': {}", target_dir.display(), e);
                had_errors = true;
            }
        }
    }

    if !had_errors && !modified {
        scrim_progress!("All links are correct");
    }

    if had_errors {
        return Err("One or more links could not be created".into());
    }

    Ok(modified)
}

