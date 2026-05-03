//! Turn 단위 자동 git commit + 되돌리기.
//!
//! 컨셉: 사용자 폴더가 git 저장소가 아니면 자동 init한다(.git만 만든다 — 의식조차 못하게).
//! 매 turn 시작 직전 HEAD oid를 snapshot 으로 잡고, turn이 정상 종료되면 변경된 파일을
//! 자동으로 add+commit한다. 사용자가 "되돌리기"를 누르면 snapshot 시점으로 hard reset.
//!
//! 정책:
//! - commit author는 codex 기본 git config 사용. 없으면 "Stcode <stcode@local>" fallback
//! - .gitignore 같은 사용자 의도는 존중 (status 호출이 알아서 제외)
//! - working tree에 변화가 없으면 commit skip (None 반환). 빈 commit은 만들지 않음.
//! - 사용자가 직접 만든 미커밋 변경은 첫 turn 직전 base commit으로 함께 들어간다 — 의도적.
//!   바이브 코더는 git을 모르므로 "내가 폴더에 둔 것 + AI가 만든 것"이 한 덩어리로 보존돼야 함.

use std::path::Path;

use git2::{IndexAddOption, Repository, ResetType, Signature};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GitError {
    #[error("git 저장소 열기 실패: {0}")]
    Open(#[source] git2::Error),
    #[error("git init 실패: {0}")]
    Init(#[source] git2::Error),
    #[error("작업 트리 상태 확인 실패: {0}")]
    Status(#[source] git2::Error),
    #[error("스테이지 실패: {0}")]
    Stage(#[source] git2::Error),
    #[error("커밋 실패: {0}")]
    Commit(#[source] git2::Error),
    #[error("HEAD 조회 실패: {0}")]
    Head(#[source] git2::Error),
    #[error("되돌리기 대상 commit 조회 실패: {0}")]
    LookupRevert(#[source] git2::Error),
    #[error("되돌리기 reset 실패: {0}")]
    Reset(#[source] git2::Error),
}

/// turn이 만들어낸 새 commit + 되돌릴 베이스 정보.
#[derive(Debug, Clone)]
pub struct TurnCommit {
    /// 새로 만든 commit oid (40자 hex).
    pub commit_oid: String,
    /// turn 직전 HEAD — 되돌리기 시 reset 대상. 첫 commit이면 None.
    pub revert_to: Option<String>,
    /// 사용자에게 보여줄 한 줄 (commit 메시지 첫 줄).
    pub summary: String,
}

/// 폴더에 .git이 없으면 init한다. 이미 있으면 조용히 false 반환.
/// 바이브 코더는 git을 모르니 자동 init이 우리 정책.
pub fn ensure_repo(path: &Path) -> Result<bool, GitError> {
    match Repository::open(path) {
        Ok(_) => Ok(false),
        Err(_) => {
            Repository::init(path).map_err(GitError::Init)?;
            tracing::info!("git init 자동 수행: {}", path.display());
            Ok(true)
        }
    }
}

/// 현재 HEAD commit oid (없는 빈 repo면 None).
pub fn current_head(path: &Path) -> Result<Option<String>, GitError> {
    let repo = Repository::open(path).map_err(GitError::Open)?;
    head_oid(&repo)
}

fn head_oid(repo: &Repository) -> Result<Option<String>, GitError> {
    match repo.head() {
        Ok(r) => match r.target() {
            Some(oid) => Ok(Some(oid.to_string())),
            None => Ok(None),
        },
        // unborn head (빈 repo) — 정상.
        Err(e)
            if e.code() == git2::ErrorCode::UnbornBranch
                || e.code() == git2::ErrorCode::NotFound =>
        {
            Ok(None)
        }
        Err(e) => Err(GitError::Head(e)),
    }
}

/// turn 직후: working tree 변화가 있으면 add+commit, summary로 메시지 만든다.
/// `prev_oid`는 turn 시작 직전 HEAD (snapshot). revert 시 hard reset 대상.
pub fn auto_commit_turn(
    path: &Path,
    user_prompt: &str,
    prev_oid: Option<&str>,
) -> Result<Option<TurnCommit>, GitError> {
    let repo = Repository::open(path).map_err(GitError::Open)?;

    if !has_changes(&repo)? {
        return Ok(None);
    }

    // 모든 변경 (untracked 포함) 스테이지.
    {
        let mut idx = repo.index().map_err(GitError::Stage)?;
        idx.add_all(["*"].iter(), IndexAddOption::DEFAULT, None)
            .map_err(GitError::Stage)?;
        idx.write().map_err(GitError::Stage)?;
    }

    let summary = build_summary(user_prompt);
    let sig = signature_for(&repo);
    let tree_oid = {
        let mut idx = repo.index().map_err(GitError::Stage)?;
        idx.write_tree().map_err(GitError::Commit)?
    };
    let tree = repo.find_tree(tree_oid).map_err(GitError::Commit)?;

    // parent 결정. unborn HEAD면 빈 부모.
    let parents: Vec<git2::Commit> = match repo.head() {
        Ok(h) => match h.target() {
            Some(p_oid) => {
                let parent = repo.find_commit(p_oid).map_err(GitError::Commit)?;
                vec![parent]
            }
            None => vec![],
        },
        Err(e)
            if e.code() == git2::ErrorCode::UnbornBranch
                || e.code() == git2::ErrorCode::NotFound =>
        {
            vec![]
        }
        Err(e) => return Err(GitError::Head(e)),
    };
    let parent_refs: Vec<&git2::Commit> = parents.iter().collect();

    let commit_oid = repo
        .commit(Some("HEAD"), &sig, &sig, &summary, &tree, &parent_refs)
        .map_err(GitError::Commit)?;

    Ok(Some(TurnCommit {
        commit_oid: commit_oid.to_string(),
        revert_to: prev_oid.map(|s| s.to_string()),
        summary,
    }))
}

/// hard reset to oid. None이면 빈 repo의 unborn 상태로 — working tree만 비움
/// (현실적으론 첫 turn 되돌리기는 거의 없음). v1엔 None인 경우 에러로.
pub fn revert_to(path: &Path, oid: Option<&str>) -> Result<(), GitError> {
    let oid_str = oid.ok_or_else(|| {
        GitError::Reset(git2::Error::from_str("첫 turn 이전으로는 되돌릴 수 없어요"))
    })?;
    let repo = Repository::open(path).map_err(GitError::Open)?;
    let oid_parsed = git2::Oid::from_str(oid_str).map_err(GitError::LookupRevert)?;
    let obj = repo
        .find_object(oid_parsed, Some(git2::ObjectType::Commit))
        .map_err(GitError::LookupRevert)?;
    repo.reset(&obj, ResetType::Hard, None)
        .map_err(GitError::Reset)?;
    Ok(())
}

fn has_changes(repo: &Repository) -> Result<bool, GitError> {
    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        // .git, .DS_Store 등은 status가 자체적으로 무시 (ignore 룰 적용)
        .include_ignored(false);
    let statuses = repo.statuses(Some(&mut opts)).map_err(GitError::Status)?;
    Ok(!statuses.is_empty())
}

fn build_summary(prompt: &str) -> String {
    // 첫 줄 + 너무 길면 자른다. git commit 첫 줄 관습(≤72자) 따르되 한국어 가독성 위해 60.
    let first_line = prompt.lines().next().unwrap_or("").trim();
    let trimmed: String = first_line.chars().take(60).collect();
    if trimmed.is_empty() {
        "stcode: 빈 prompt".into()
    } else {
        format!("stcode: {trimmed}")
    }
}

fn signature_for(repo: &Repository) -> Signature<'static> {
    // user.name/email 우선 — git2의 Signature::now는 'a 라이프타임이라 to_owned 필요.
    if let Ok(cfg) = repo.config() {
        let name = cfg
            .get_string("user.name")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Stcode".into());
        let email = cfg
            .get_string("user.email")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "stcode@local".into());
        if let Ok(sig) = Signature::now(&name, &email) {
            return sig;
        }
    }
    Signature::now("Stcode", "stcode@local").expect("정적 signature 생성 실패")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write_file(path: &Path, body: &str) {
        fs::write(path, body).expect("test file write");
    }

    #[test]
    fn ensure_repo_initializes_non_git_dir() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path();

        assert!(!path.join(".git").exists());
        assert!(ensure_repo(path).expect("init repo"));
        assert!(path.join(".git").exists());
        assert!(!ensure_repo(path).expect("already repo"));
        assert_eq!(current_head(path).expect("head"), None);
    }

    #[test]
    fn auto_commit_skips_when_worktree_is_clean() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path();
        ensure_repo(path).expect("init repo");

        let commit = auto_commit_turn(path, "아무 것도 안 바꿈", None).expect("auto commit");

        assert!(commit.is_none());
        assert_eq!(current_head(path).expect("head"), None);
    }

    #[test]
    fn auto_commit_tracks_untracked_and_modified_files() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path();
        ensure_repo(path).expect("init repo");

        write_file(&path.join("hello.txt"), "one\n");
        let first = auto_commit_turn(path, "파일 만들어", None)
            .expect("first commit")
            .expect("commit created");
        assert_eq!(first.summary, "stcode: 파일 만들어");
        assert_eq!(first.revert_to, None);
        assert_eq!(
            current_head(path).expect("head"),
            Some(first.commit_oid.clone())
        );

        write_file(&path.join("hello.txt"), "two\n");
        let prev = current_head(path).expect("head").expect("prev head");
        let second = auto_commit_turn(path, "파일 바꿔", Some(&prev))
            .expect("second commit")
            .expect("commit created");

        assert_eq!(second.summary, "stcode: 파일 바꿔");
        assert_eq!(second.revert_to, Some(prev));
        assert_eq!(current_head(path).expect("head"), Some(second.commit_oid));
    }

    #[test]
    fn revert_to_previous_head_restores_tracked_content() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path();
        ensure_repo(path).expect("init repo");

        let file = path.join("hello.txt");
        write_file(&file, "one\n");
        let first = auto_commit_turn(path, "처음", None)
            .expect("first commit")
            .expect("commit created");

        write_file(&file, "two\n");
        let second = auto_commit_turn(path, "두번째", Some(&first.commit_oid))
            .expect("second commit")
            .expect("commit created");

        revert_to(path, second.revert_to.as_deref()).expect("revert");

        assert_eq!(fs::read_to_string(&file).expect("read file"), "one\n");
        assert_eq!(current_head(path).expect("head"), Some(first.commit_oid));
    }

    #[test]
    fn reverting_before_first_turn_returns_friendly_error() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path();
        ensure_repo(path).expect("init repo");

        write_file(&path.join("hello.txt"), "one\n");
        let first = auto_commit_turn(path, "처음", None)
            .expect("first commit")
            .expect("commit created");

        let err = revert_to(path, first.revert_to.as_deref()).expect_err("first turn revert fails");

        assert!(err.to_string().contains("첫 turn 이전"));
    }
}
