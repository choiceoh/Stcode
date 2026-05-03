//! codex / git / network 에러 raw 메시지를 바이브 코더가 알아듣게 한국어로 변환.
//!
//! 정책: substring 매칭으로 충분. 매칭되는 첫 패턴 사용. 안 맞으면 원문 + 일반 안내.
//! 매핑은 specific → general 순서 — 위에 있을수록 먼저 매치돼야 정확.

/// raw 에러 텍스트를 친화적 한국어로 변환.
/// 매칭되는 패턴이 없으면 원문 그대로 (디버그 단서 보존).
pub fn translate(raw: &str) -> String {
    for (needles, friendly) in PATTERNS {
        if needles.iter().all(|n| raw.contains(n)) {
            return (*friendly).to_string();
        }
    }
    raw.to_string()
}

/// (substring AND-조건들, 친화 메시지). 모든 substring이 raw에 있어야 매치.
/// 가장 구체적인 패턴을 위쪽에 두기.
const PATTERNS: &[(&[&str], &str)] = &[
    // ─── codex 프로토콜 mismatch ─────────────────────────
    (
        &["unknown variant", "approval"],
        "⚠ codex 프로토콜 버전이 안 맞아요. codex fork를 다시 빌드해 주세요.",
    ),
    (
        &["Invalid request", "unknown variant"],
        "⚠ codex 프로토콜 버전이 안 맞아요. codex fork를 다시 빌드해 주세요.",
    ),
    // ─── 모델 / 네트워크 ─────────────────────────────────
    (
        &["Connection refused"],
        "⚠ vLLM 서버에 연결할 수 없어요. 서버가 켜져 있는지 확인해 주세요.",
    ),
    (
        &["tcp connect"],
        "⚠ vLLM 서버에 연결할 수 없어요. 네트워크 또는 서버 상태를 확인해 주세요.",
    ),
    (
        &["timed out"],
        "⚠ 모델 응답이 너무 오래 걸려요. 서버 상태를 확인하거나 다시 시도해 주세요.",
    ),
    (
        &["stream disconnected"],
        "⚠ 모델 응답이 중간에 끊겼어요. 다시 시도해 주세요.",
    ),
    (
        &["rate limit"],
        "⚠ 잠깐 너무 많이 호출했어요. 1분쯤 기다린 뒤 다시 시도해 주세요.",
    ),
    (
        &["429"],
        "⚠ 잠깐 너무 많이 호출했어요. 1분쯤 기다린 뒤 다시 시도해 주세요.",
    ),
    (
        &["401"],
        "⚠ API 키가 만료된 것 같아요. 설정에서 갱신해 주세요.",
    ),
    (
        &["unauthorized"],
        "⚠ 인증에 실패했어요. API 키를 확인해 주세요.",
    ),
    // ─── codex 내부 ─────────────────────────────────────
    (
        &["No such file", "codex"],
        "⚠ codex 바이너리를 못 찾았어요. `brew install codex` 또는 fork 빌드 후 다시 시도해 주세요.",
    ),
    (
        &["NotFound"],
        "⚠ codex 바이너리를 못 찾았어요. fork 빌드 위치를 확인해 주세요.",
    ),
    // ─── git 안전망 ─────────────────────────────────────
    (
        &["not a repository"],
        "⚠ 폴더가 git 저장소가 아니에요. 다른 폴더를 골라주세요.",
    ),
    (
        &["첫 turn 이전"],
        "⚠ 첫 변경 이전으로는 되돌릴 수 없어요.",
    ),
    (
        &["nothing to commit"],
        "ℹ 저장할 변경이 없어요.",
    ),
    // ─── 일반 ──────────────────────────────────────────
    (
        &["Permission denied"],
        "⚠ 파일/폴더 권한이 없어요. macOS 권한 설정을 확인해 주세요.",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn untranslated_passes_through() {
        let raw = "some random error text";
        assert_eq!(translate(raw), raw);
    }

    #[test]
    fn matches_connection_refused() {
        let out = translate("error: Connection refused (os error 61)");
        assert!(out.contains("vLLM"));
    }

    #[test]
    fn matches_unknown_variant() {
        let out = translate("Invalid request: unknown variant `onRequest`");
        assert!(out.contains("프로토콜"));
    }

    #[test]
    fn first_match_wins() {
        // "rate limit" 패턴이 "429"보다 위에 있으면 그게 우선되어야.
        let out = translate("rate limit exceeded (HTTP 429)");
        assert!(out.contains("1분쯤"));
    }

    #[test]
    fn git_revert_first_commit() {
        let out = translate("첫 turn 이전으로는 되돌릴 수 없어요");
        assert!(out.contains("되돌릴"));
    }
}
