//! 세션 단위 git worktree/branch 관리.
//!
//! 병렬 세션은 원본 프로젝트 폴더를 직접 건드리면 서로 충돌한다. 이 모듈은 세션 시작 시
//! 원본 repo HEAD에서 작업용 worktree와 branch를 만들고, 세션 종료 시 변경이 없는
//! 임시 branch/worktree를 안전하게 정리하는 낮은 레이어다.

use std::collections::hash_map::DefaultHasher;
use std::ffi::{OsStr, OsString};
use std::hash::{Hash, Hasher};
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum WorktreeError {
    #[error("git 명령 실행 실패: {0}")]
    Spawn(#[source] io::Error),
    #[error("git 명령 실패: {command}\n{stderr}")]
    GitCommand { command: String, stderr: String },
    #[error("worktree root 생성 실패: {0}")]
    CreateRoot(#[source] io::Error),
    #[error("세션 worktree가 이미 있어요: {0}")]
    WorktreeAlreadyExists(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionWorktree {
    /// 사용자가 선택한 원본 프로젝트 경로.
    pub source_path: PathBuf,
    /// 원본 git repo 최상위 경로.
    pub repo_root: PathBuf,
    /// 세션이 실제로 작업할 격리 worktree 경로.
    pub worktree_path: PathBuf,
    /// 세션 전용 branch 이름.
    pub branch: String,
    /// 세션 branch를 만들 때 기준이 된 commit.
    pub base_oid: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeCleanup {
    pub removed_worktree: bool,
    pub deleted_branch: bool,
    pub kept_branch_reason: Option<String>,
}

/// 기본 worktree 보관 위치.
///
/// macOS에서는 `~/Library/Application Support/Stcode/worktrees`가 된다.
pub fn default_worktrees_root() -> Option<PathBuf> {
    dirs::config_dir().map(|p| p.join("Stcode").join("worktrees"))
}

/// 세션 id에서 추적 가능한 branch 이름을 만든다.
pub fn session_branch_name(session_id: &str) -> String {
    format!("stcode/{}", sanitize_component(session_id))
}

/// 원본 repo HEAD에서 세션용 worktree/branch를 만든다.
pub fn prepare_session_worktree(
    source_path: &Path,
    session_id: &str,
    worktrees_root: &Path,
) -> Result<SessionWorktree, WorktreeError> {
    let source_path = source_path.to_path_buf();
    let repo_root = discover_repo_root(&source_path)?;
    let base_oid = git_output(&repo_root, ["rev-parse", "--verify", "HEAD"])?;
    let branch = session_branch_name(session_id);
    let worktree_path = worktree_path_for(worktrees_root, &repo_root, session_id);

    if worktree_path.exists() {
        return Err(WorktreeError::WorktreeAlreadyExists(worktree_path));
    }
    if let Some(parent) = worktree_path.parent() {
        std::fs::create_dir_all(parent).map_err(WorktreeError::CreateRoot)?;
    }

    if branch_exists(&repo_root, &branch)? {
        git_output_os(
            &repo_root,
            [
                OsString::from("worktree"),
                OsString::from("add"),
                worktree_path.as_os_str().to_os_string(),
                OsString::from(&branch),
            ],
        )?;
    } else {
        git_output_os(
            &repo_root,
            [
                OsString::from("worktree"),
                OsString::from("add"),
                OsString::from("-b"),
                OsString::from(&branch),
                worktree_path.as_os_str().to_os_string(),
                OsString::from(&base_oid),
            ],
        )?;
    }

    Ok(SessionWorktree {
        source_path,
        repo_root,
        worktree_path,
        branch,
        base_oid,
    })
}

/// 세션 worktree를 안전하게 정리한다.
///
/// worktree에 미커밋 변경이 있으면 삭제하지 않는다. branch는 세션 시작 기준 commit에서
/// 움직이지 않은 경우에만 지운다. 즉 실제 작업물이 있는 branch는 "사용하지 않음"으로
/// 간주하지 않는다.
pub fn cleanup_session_worktree(
    worktree: &SessionWorktree,
) -> Result<WorktreeCleanup, WorktreeError> {
    let mut cleanup = WorktreeCleanup {
        removed_worktree: false,
        deleted_branch: false,
        kept_branch_reason: None,
    };

    if worktree.worktree_path.exists() {
        if worktree_has_changes(&worktree.worktree_path)? {
            cleanup.kept_branch_reason = Some("worktree에 아직 저장되지 않은 변경이 있어요".into());
            return Ok(cleanup);
        }
        git_output_os(
            &worktree.repo_root,
            [
                OsString::from("worktree"),
                OsString::from("remove"),
                worktree.worktree_path.as_os_str().to_os_string(),
            ],
        )?;
        cleanup.removed_worktree = true;
    }

    if branch_exists(&worktree.repo_root, &worktree.branch)? {
        let head = git_output(
            &worktree.repo_root,
            ["rev-parse", "--verify", &worktree.branch],
        )?;
        if head == worktree.base_oid {
            git_output(&worktree.repo_root, ["branch", "-D", &worktree.branch])?;
            cleanup.deleted_branch = true;
        } else {
            cleanup.kept_branch_reason = Some("branch에 세션 작업 commit이 남아 있어요".into());
        }
    }

    Ok(cleanup)
}

fn discover_repo_root(path: &Path) -> Result<PathBuf, WorktreeError> {
    Ok(PathBuf::from(git_output(
        path,
        ["rev-parse", "--show-toplevel"],
    )?))
}

fn worktree_path_for(root: &Path, repo_root: &Path, session_id: &str) -> PathBuf {
    let repo_name = repo_root
        .file_name()
        .and_then(OsStr::to_str)
        .map(sanitize_component)
        .unwrap_or_else(|| "repo".into());
    let mut hasher = DefaultHasher::new();
    repo_root.to_string_lossy().hash(&mut hasher);
    let repo_key = format!("{repo_name}-{:016x}", hasher.finish());
    root.join(repo_key).join(sanitize_component(session_id))
}

fn branch_exists(repo: &Path, branch: &str) -> Result<bool, WorktreeError> {
    let reference = format!("refs/heads/{branch}");
    let status = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["show-ref", "--verify", "--quiet", &reference])
        .status()
        .map_err(WorktreeError::Spawn)?;
    Ok(status.success())
}

fn worktree_has_changes(worktree_path: &Path) -> Result<bool, WorktreeError> {
    Ok(!git_output(worktree_path, ["status", "--porcelain"])?.is_empty())
}

fn git_output<I, S>(repo: &Path, args: I) -> Result<String, WorktreeError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    git_output_os(
        repo,
        args.into_iter()
            .map(|arg| arg.as_ref().to_os_string())
            .collect::<Vec<_>>(),
    )
}

fn git_output_os<I, S>(repo: &Path, args: I) -> Result<String, WorktreeError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let args: Vec<OsString> = args
        .into_iter()
        .map(|arg| arg.as_ref().to_os_string())
        .collect();
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(&args)
        .output()
        .map_err(WorktreeError::Spawn)?;
    if !output.status.success() {
        return Err(WorktreeError::GitCommand {
            command: render_git_command(repo, &args),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn render_git_command(repo: &Path, args: &[OsString]) -> String {
    let rendered_args = args
        .iter()
        .map(|arg| arg.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");
    format!("git -C {} {rendered_args}", repo.display())
}

fn sanitize_component(raw: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in raw.chars() {
        let next = if ch.is_ascii_alphanumeric() || ch == '_' || ch == '.' {
            Some(ch.to_ascii_lowercase())
        } else if ch == '-' {
            Some('-')
        } else {
            Some('-')
        };
        if let Some(ch) = next {
            if ch == '-' {
                if last_dash {
                    continue;
                }
                last_dash = true;
            } else {
                last_dash = false;
            }
            out.push(ch);
        }
    }
    let trimmed = out.trim_matches(&['-', '.'][..]).to_string();
    if trimmed.is_empty() {
        "session".into()
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn git(repo: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .output()
            .expect("git command spawn");
        assert!(
            output.status.success(),
            "git -C {} {} failed\nstdout:\n{}\nstderr:\n{}",
            repo.display(),
            args.join(" "),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_stdout(repo: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .output()
            .expect("git command spawn");
        assert!(
            output.status.success(),
            "git -C {} {} failed\nstdout:\n{}\nstderr:\n{}",
            repo.display(),
            args.join(" "),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn init_repo(path: &Path) {
        git(path, &["init", "-b", "main"]);
        git(path, &["config", "user.name", "Stcode Test"]);
        git(path, &["config", "user.email", "stcode-test@example.local"]);
        fs::write(path.join("README.md"), "# test\n").expect("write readme");
        git(path, &["add", "README.md"]);
        git(path, &["commit", "-m", "initial"]);
    }

    #[test]
    fn branch_name_is_safe_and_prefixed() {
        assert_eq!(session_branch_name("S 1/한글"), "stcode/s-1");
        assert_eq!(session_branch_name("..."), "stcode/session");
    }

    #[test]
    fn prepare_creates_isolated_worktree_and_branch() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let repo = tmp.path().join("repo");
        fs::create_dir(&repo).expect("repo dir");
        init_repo(&repo);
        let root = tmp.path().join("worktrees");

        let worktree = prepare_session_worktree(&repo, "s1", &root).expect("prepare worktree");

        assert_eq!(
            worktree.repo_root,
            repo.canonicalize().expect("canonical repo")
        );
        assert_eq!(worktree.branch, "stcode/s1");
        assert!(worktree.worktree_path.exists());
        assert!(worktree.worktree_path.join("README.md").exists());
        assert_eq!(
            git_stdout(&worktree.worktree_path, &["branch", "--show-current"]),
            "stcode/s1"
        );
        assert_eq!(
            git_stdout(&worktree.repo_root, &["branch", "--show-current"]),
            "main"
        );
    }

    #[test]
    fn cleanup_removes_unchanged_worktree_and_branch() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let repo = tmp.path().join("repo");
        fs::create_dir(&repo).expect("repo dir");
        init_repo(&repo);
        let root = tmp.path().join("worktrees");
        let worktree = prepare_session_worktree(&repo, "s2", &root).expect("prepare worktree");

        let cleanup = cleanup_session_worktree(&worktree).expect("cleanup");

        assert!(cleanup.removed_worktree);
        assert!(cleanup.deleted_branch);
        assert!(!worktree.worktree_path.exists());
        assert!(!branch_exists(&worktree.repo_root, &worktree.branch).expect("branch exists"));
    }

    #[test]
    fn cleanup_keeps_branch_with_committed_session_work() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let repo = tmp.path().join("repo");
        fs::create_dir(&repo).expect("repo dir");
        init_repo(&repo);
        let root = tmp.path().join("worktrees");
        let worktree = prepare_session_worktree(&repo, "s3", &root).expect("prepare worktree");

        fs::write(worktree.worktree_path.join("feature.txt"), "done\n").expect("write feature");
        git(&worktree.worktree_path, &["add", "feature.txt"]);
        git(&worktree.worktree_path, &["commit", "-m", "session work"]);

        let cleanup = cleanup_session_worktree(&worktree).expect("cleanup");

        assert!(cleanup.removed_worktree);
        assert!(!cleanup.deleted_branch);
        assert_eq!(
            cleanup.kept_branch_reason.as_deref(),
            Some("branch에 세션 작업 commit이 남아 있어요")
        );
        assert!(branch_exists(&worktree.repo_root, &worktree.branch).expect("branch exists"));
    }

    #[test]
    fn cleanup_does_not_remove_dirty_worktree() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let repo = tmp.path().join("repo");
        fs::create_dir(&repo).expect("repo dir");
        init_repo(&repo);
        let root = tmp.path().join("worktrees");
        let worktree = prepare_session_worktree(&repo, "s4", &root).expect("prepare worktree");

        fs::write(worktree.worktree_path.join("dirty.txt"), "not committed\n")
            .expect("write dirty");

        let cleanup = cleanup_session_worktree(&worktree).expect("cleanup");

        assert!(!cleanup.removed_worktree);
        assert!(!cleanup.deleted_branch);
        assert_eq!(
            cleanup.kept_branch_reason.as_deref(),
            Some("worktree에 아직 저장되지 않은 변경이 있어요")
        );
        assert!(worktree.worktree_path.exists());
        assert!(branch_exists(&worktree.repo_root, &worktree.branch).expect("branch exists"));
    }
}
