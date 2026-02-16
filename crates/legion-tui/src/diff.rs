//! Git diff retrieval and parsing for ticket file changes

use std::path::Path;
use std::process::Command;
use anyhow::{Context, Result};

/// Parsed diff data for a ticket
#[derive(Debug, Clone)]
pub struct DiffData {
    pub files: Vec<DiffFile>,
    pub raw_diff: String,
}

/// A single file's diff information
#[derive(Debug, Clone)]
pub struct DiffFile {
    pub path: String,
    pub status: String,       // "A" (added), "M" (modified), "D" (deleted)
    pub additions: usize,
    pub deletions: usize,
    pub diff_lines: Vec<String>,  // The diff lines for this file
}

/// Get the leader reference for diff comparison
///
/// - Default session: uses the git default branch (main/master)
/// - Other sessions: uses "legion/<session>/leader" branch
pub fn get_leader_ref(project_path: &Path, session_name: &str, is_default: bool) -> String {
    if is_default {
        crate::worktree::default_branch(project_path)
    } else {
        crate::worktree::pane_branch_name(session_name, "Leader")
    }
}

/// Get diff for a worker worktree compared to leader ref
///
/// For Done/Error tickets: shows committed changes vs merge-base with leader
/// For Working tickets: also includes uncommitted changes (staged + unstaged)
pub fn get_worktree_diff(
    worktree_path: &Path,
    leader_ref: &str,
    include_uncommitted: bool,
) -> Result<DiffData> {
    // Get merge base
    let merge_base = get_merge_base(worktree_path, leader_ref)?;

    // Get committed diff: merge_base..HEAD
    let committed_diff = run_git_diff(worktree_path, &[&format!("{}..HEAD", merge_base), "--unified=3"])?;

    let mut raw_diff = committed_diff;

    // For Working tickets, also get uncommitted changes
    if include_uncommitted {
        // Staged changes
        let staged = run_git_diff(worktree_path, &["--cached", "--unified=3"])?;
        if !staged.is_empty() {
            if !raw_diff.is_empty() {
                raw_diff.push('\n');
            }
            raw_diff.push_str(&staged);
        }

        // Unstaged changes
        let unstaged = run_git_diff(worktree_path, &["--unified=3"])?;
        if !unstaged.is_empty() {
            if !raw_diff.is_empty() {
                raw_diff.push('\n');
            }
            raw_diff.push_str(&unstaged);
        }
    }

    let files = parse_diff(&raw_diff);

    Ok(DiffData { files, raw_diff })
}

/// Get merge base between leader ref and HEAD
fn get_merge_base(worktree_path: &Path, leader_ref: &str) -> Result<String> {
    let output = Command::new("git")
        .args(["merge-base", leader_ref, "HEAD"])
        .current_dir(worktree_path)
        .output()
        .context("Failed to run git merge-base")?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        // Fallback: if merge-base fails (e.g., no common ancestor), use leader_ref directly
        Ok(leader_ref.to_string())
    }
}

/// Run git diff with given arguments
fn run_git_diff(worktree_path: &Path, args: &[&str]) -> Result<String> {
    let mut cmd_args = vec!["diff"];
    cmd_args.extend(args);

    let output = Command::new("git")
        .args(&cmd_args)
        .current_dir(worktree_path)
        .output()
        .context("Failed to run git diff")?;

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Parse unified diff output into structured DiffFile entries
fn parse_diff(raw: &str) -> Vec<DiffFile> {
    let mut files: Vec<DiffFile> = Vec::new();
    let mut current_path = String::new();
    let mut current_status = String::from("M");
    let mut current_lines: Vec<String> = Vec::new();
    let mut additions: usize = 0;
    let mut deletions: usize = 0;

    for line in raw.lines() {
        if line.starts_with("diff --git") {
            // Save previous file if any
            if !current_path.is_empty() {
                files.push(DiffFile {
                    path: current_path.clone(),
                    status: current_status.clone(),
                    additions,
                    deletions,
                    diff_lines: current_lines.clone(),
                });
            }

            // Extract path from "diff --git a/path b/path"
            current_path = line
                .split(" b/")
                .nth(1)
                .unwrap_or("")
                .to_string();
            current_status = "M".to_string();
            current_lines = vec![line.to_string()];
            additions = 0;
            deletions = 0;
        } else if line.starts_with("new file") {
            current_status = "A".to_string();
            current_lines.push(line.to_string());
        } else if line.starts_with("deleted file") {
            current_status = "D".to_string();
            current_lines.push(line.to_string());
        } else if line.starts_with('+') && !line.starts_with("+++") {
            additions += 1;
            current_lines.push(line.to_string());
        } else if line.starts_with('-') && !line.starts_with("---") {
            deletions += 1;
            current_lines.push(line.to_string());
        } else {
            current_lines.push(line.to_string());
        }
    }

    // Save last file
    if !current_path.is_empty() {
        files.push(DiffFile {
            path: current_path,
            status: current_status,
            additions,
            deletions,
            diff_lines: current_lines,
        });
    }

    // Deduplicate: if same path appears multiple times (committed + uncommitted),
    // merge their stats and lines
    let mut merged: Vec<DiffFile> = Vec::new();
    for file in files {
        if let Some(existing) = merged.iter_mut().find(|f| f.path == file.path) {
            existing.additions += file.additions;
            existing.deletions += file.deletions;
            existing.diff_lines.push(String::new()); // separator
            existing.diff_lines.extend(file.diff_lines);
        } else {
            merged.push(file);
        }
    }

    merged
}

/// Rebuild DiffData from cached raw diff and file summaries.
/// Re-parses the raw diff to extract per-file diff lines,
/// but uses the provided summary for stats.
pub fn parse_raw_diff_with_summary(raw_diff: &str, summaries: Vec<DiffFile>) -> DiffData {
    let parsed = parse_diff(raw_diff);

    // Merge parsed diff lines into summary entries
    let files = summaries.into_iter().map(|mut s| {
        if let Some(parsed_file) = parsed.iter().find(|p| p.path == s.path) {
            s.diff_lines = parsed_file.diff_lines.clone();
        }
        s
    }).collect();

    DiffData {
        files,
        raw_diff: raw_diff.to_string(),
    }
}
