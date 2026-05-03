#!/usr/bin/env bash
#
# Stcode .app 번들 빌드 스크립트.
#
# 사용:
#   bash scripts/build-app.sh                # release 빌드 + ad-hoc 코드사인
#   bash scripts/build-app.sh --debug        # debug 빌드 (디버깅용)
#   bash scripts/build-app.sh --no-codesign  # 코드사인 생략 (CI 등)
#
# 결과: dist/Stcode.app
#
# 사용자 머신 요구사항:
#   - macOS 13+
#   - Xcode + Metal toolchain
#   - rustup stable
#   - codex fork 빌드 (~/Documents/GitHub/codex-fork — 또는 STCODE_CODEX_BIN ENV)

set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$(pwd)"

PROFILE="release"
PROFILE_DIR="release"
SIGN=1
for arg in "$@"; do
    case "$arg" in
        --debug)
            PROFILE="dev"
            PROFILE_DIR="debug"
            ;;
        --no-codesign)
            SIGN=0
            ;;
        *)
            echo "알 수 없는 옵션: $arg" >&2
            exit 1
            ;;
    esac
done

echo "[1/4] cargo build --bin stcode (profile=$PROFILE)"
if [ "$PROFILE" = "release" ]; then
    cargo build --release --bin stcode
else
    cargo build --bin stcode
fi

BIN="$ROOT/target/$PROFILE_DIR/stcode"
if [ ! -x "$BIN" ]; then
    echo "빌드 결과 없음: $BIN" >&2
    exit 1
fi

VERSION="$(awk -F'"' '/^version[ ]*=/ { print $2; exit }' "$ROOT/crates/stcode-app/Cargo.toml")"
if [ -z "$VERSION" ]; then
    VERSION="$(awk -F'"' '/^version\.workspace/ { print "ws"; exit } /^version[ ]*=/ { print $2; exit }' "$ROOT/Cargo.toml")"
fi
[ -z "$VERSION" ] && VERSION="0.0.1"

DIST="$ROOT/dist"
APP="$DIST/Stcode.app"
echo "[2/4] .app 디렉터리 구성 → $APP"

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

# Info.plist — 템플릿의 __VERSION__ 치환.
sed "s/__VERSION__/$VERSION/g" "$ROOT/assets/Info.plist.template" > "$APP/Contents/Info.plist"

# 바이너리 복사 + 실행권한.
cp "$BIN" "$APP/Contents/MacOS/stcode"
chmod +x "$APP/Contents/MacOS/stcode"

# (옵션) 아이콘. 있으면 복사. v1엔 system default.
if [ -f "$ROOT/assets/AppIcon.icns" ]; then
    cp "$ROOT/assets/AppIcon.icns" "$APP/Contents/Resources/AppIcon.icns"
fi

if [ "$SIGN" = "1" ]; then
    echo "[3/4] ad-hoc 코드사인 (사내 배포 가정)"
    # --deep는 .app 내부 모든 binary에 사인. -s - 는 ad-hoc.
    codesign --force --deep --sign - "$APP"
    codesign --verify --verbose=2 "$APP" || {
        echo "코드사인 검증 실패" >&2
        exit 1
    }
else
    echo "[3/4] 코드사인 생략"
fi

echo "[4/4] 완료"
echo "  $APP"
echo "  open $APP   # 더블클릭 또는 이 명령으로 실행"
echo
echo "사내 배포: dist/Stcode.app 폴더를 zip 으로 압축 (예: ditto -c -k --keepParent dist/Stcode.app dist/Stcode.app.zip)"
echo "처음 실행 시 Gatekeeper 경고가 뜨면 Finder 우클릭 → 열기 → '열기' 한 번만."
