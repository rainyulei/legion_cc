//! Git worktree management for squad sessions
//!
//! Each session creates isolated worktrees per pane:
//!   ../<project>-legion/<session>/<pane-label>/
//! Branch naming: legion/<session>/<pane-label>

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

/// Compute the worktree root: <parent>/<project-name>-legion/
pub fn legion_root(project_path: &Path) -> PathBuf {
    let parent = project_path.parent().unwrap_or(project_path);
    let name = project_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("project");
    parent.join(format!("{}-legion", name))
}

/// Compute worktree path for a pane: ../<project>-legion/<session>/<label-slug>/
pub fn pane_worktree_path(project_path: &Path, session_name: &str, pane_label: &str) -> PathBuf {
    let label_slug = pane_label.to_lowercase().replace(' ', "-");
    legion_root(project_path).join(session_name).join(label_slug)
}

/// Compute git branch name: legion/<session>/<label-slug>
pub fn pane_branch_name(session_name: &str, pane_label: &str) -> String {
    let label_slug = pane_label.to_lowercase().replace(' ', "-");
    format!("legion/{}/{}", session_name, label_slug)
}

/// Create a git worktree for a pane
///
/// If the worktree already exists, returns its path.
/// If the branch already exists (leftover from incomplete cleanup), reuses it.
pub fn create_worktree(
    project_path: &Path,
    session_name: &str,
    pane_label: &str,
) -> Result<PathBuf> {
    let wt_path = pane_worktree_path(project_path, session_name, pane_label);
    let branch = pane_branch_name(session_name, pane_label);

    // Already exists — reuse
    if worktree_exists(&wt_path) {
        return Ok(wt_path);
    }

    if let Some(parent) = wt_path.parent() {
        std::fs::create_dir_all(parent).context("Failed to create worktree parent directory")?;
    }

    // Try creating with new branch
    let output = Command::new("git")
        .args(["worktree", "add", &wt_path.to_string_lossy(), "-b", &branch])
        .current_dir(project_path)
        .output()
        .context("Failed to run git worktree add")?;

    if output.status.success() {
        return Ok(wt_path);
    }

    // Branch might already exist (leftover) — try using existing branch
    let output2 = Command::new("git")
        .args(["worktree", "add", &wt_path.to_string_lossy(), &branch])
        .current_dir(project_path)
        .output()
        .context("Failed to run git worktree add with existing branch")?;

    if output2.status.success() {
        return Ok(wt_path);
    }

    let stderr = String::from_utf8_lossy(&output2.stderr);
    anyhow::bail!("git worktree add failed: {}", stderr.trim())
}

/// Check if a worktree path exists and is valid
pub fn worktree_exists(path: &Path) -> bool {
    path.is_dir() && path.join(".git").exists()
}

/// Remove a worktree and its branch
pub fn remove_worktree(
    project_path: &Path,
    session_name: &str,
    pane_label: &str,
    force: bool,
) -> Result<()> {
    let wt_path = pane_worktree_path(project_path, session_name, pane_label);
    let branch = pane_branch_name(session_name, pane_label);

    let mut args = vec!["worktree", "remove"];
    if force {
        args.push("--force");
    }
    let wt_str = wt_path.to_string_lossy().to_string();
    args.push(&wt_str);

    let output = Command::new("git")
        .args(&args)
        .current_dir(project_path)
        .output()
        .context("Failed to run git worktree remove")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::warn!("git worktree remove failed: {}", stderr.trim());
        if wt_path.exists() {
            std::fs::remove_dir_all(&wt_path).ok();
        }
    }

    let flag = if force { "-D" } else { "-d" };
    let _ = Command::new("git")
        .args(["branch", flag, &branch])
        .current_dir(project_path)
        .output();

    Ok(())
}

/// Merge a pane's branch into current branch
pub fn merge_branch(
    project_path: &Path,
    session_name: &str,
    pane_label: &str,
) -> Result<()> {
    let branch = pane_branch_name(session_name, pane_label);

    let output = Command::new("git")
        .args([
            "merge",
            &branch,
            "--no-ff",
            "-m",
            &format!(
                "Merge legion session: {} ({})",
                session_name, pane_label
            ),
        ])
        .current_dir(project_path)
        .output()
        .context("Failed to run git merge")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git merge failed (conflicts?): {}", stderr.trim());
    }

    Ok(())
}

/// Get the default branch name (main or master)
pub fn default_branch(project_path: &Path) -> String {
    let output = Command::new("git")
        .args(["symbolic-ref", "refs/remotes/origin/HEAD", "--short"])
        .current_dir(project_path)
        .output();

    if let Ok(out) = output {
        if out.status.success() {
            let branch = String::from_utf8_lossy(&out.stdout).trim().to_string();
            return branch.rsplit('/').next().unwrap_or("main").to_string();
        }
    }

    let check_main = Command::new("git")
        .args(["rev-parse", "--verify", "main"])
        .current_dir(project_path)
        .output();

    if check_main.map(|o| o.status.success()).unwrap_or(false) {
        "main".into()
    } else {
        "master".into()
    }
}

/// Create all worktrees for a session (leader + N workers)
pub fn create_session_worktrees(
    project_path: &Path,
    session_name: &str,
    worker_count: u16,
) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::with_capacity(1 + worker_count as usize);
    paths.push(create_worktree(project_path, session_name, "Leader")?);
    for i in 1..=worker_count {
        paths.push(create_worktree(
            project_path,
            session_name,
            &format!("Worker {}", i),
        )?);
    }
    Ok(paths)
}

/// Create worktrees for a default session (Leader uses main repo, workers get worktrees)
pub fn create_default_session_worktrees(
    project_path: &Path,
    session_name: &str,
    worker_count: u16,
) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::with_capacity(1 + worker_count as usize);
    // Leader path = project_path itself (no worktree)
    paths.push(project_path.to_path_buf());
    for i in 1..=worker_count {
        paths.push(create_worktree(
            project_path,
            session_name,
            &format!("Worker {}", i),
        )?);
    }
    Ok(paths)
}

/// Remove all worktrees for a session
pub fn remove_session_worktrees(
    project_path: &Path,
    session_name: &str,
    worker_count: u16,
    force: bool,
) -> Result<()> {
    remove_worktree(project_path, session_name, "Leader", force)?;
    for i in 1..=worker_count {
        remove_worktree(project_path, session_name, &format!("Worker {}", i), force)?;
    }

    let session_dir = legion_root(project_path).join(session_name);
    if session_dir.exists() {
        std::fs::remove_dir(&session_dir).ok();
    }

    Ok(())
}

/// Remove worktrees for a default session (only workers, Leader = main repo is untouched)
pub fn remove_default_session_worktrees(
    project_path: &Path,
    session_name: &str,
    worker_count: u16,
    force: bool,
) -> Result<()> {
    // Skip Leader — it's the main repo
    for i in 1..=worker_count {
        remove_worktree(project_path, session_name, &format!("Worker {}", i), force)?;
    }

    let session_dir = legion_root(project_path).join(session_name);
    if session_dir.exists() {
        std::fs::remove_dir(&session_dir).ok();
    }

    Ok(())
}

/// Get the current branch name (None if detached HEAD)
pub fn current_branch(project_path: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["branch", "--show-current"])
        .current_dir(project_path)
        .output()
        .ok()?;
    if output.status.success() {
        let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if branch.is_empty() { None } else { Some(branch) }
    } else {
        None
    }
}

/// Get the current HEAD commit SHA (short)
pub fn current_commit(project_path: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(project_path)
        .output()
        .ok()?;
    if output.status.success() {
        let sha = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if sha.is_empty() { None } else { Some(sha) }
    } else {
        None
    }
}

/// Check if a local branch exists
pub fn branch_exists(project_path: &Path, branch: &str) -> bool {
    Command::new("git")
        .args(["rev-parse", "--verify", branch])
        .current_dir(project_path)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// List all local branch names
pub fn list_local_branches(project_path: &Path) -> Vec<String> {
    let output = Command::new("git")
        .args(["branch", "--format=%(refname:short)"])
        .current_dir(project_path)
        .output();
    match output {
        Ok(o) if o.status.success() => {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .filter(|l| !l.starts_with("legion/")) // exclude worktree branches
                .collect()
        }
        _ => vec![],
    }
}

/// Sanitize a branch name for use as session name (replace / with -)
pub fn sanitize_branch_name(branch: &str) -> String {
    branch.replace('/', "-")
}
