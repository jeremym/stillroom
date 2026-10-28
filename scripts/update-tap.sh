#!/usr/bin/env bash
# Bump the stillroom cask in a local clone of jeremym/homebrew-tap. Shows the diff and stops;
# you review, commit and push with your own git (no tokens involved).
# Usage: scripts/update-tap.sh --tap PATH --version X.Y.Z [--sha256 HEX | --zip FILE]
set -euo pipefail
TAP=""; VERSION=""; SHA256=""; ZIP=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --tap) TAP="$2"; shift 2 ;;
        --version) VERSION="$2"; shift 2 ;;
        --sha256) SHA256="$2"; shift 2 ;;
        --zip) ZIP="$2"; shift 2 ;;
        *) echo "error: unknown argument '$1'" >&2; exit 2 ;;
    esac
done
[[ -n "$TAP" && -n "$VERSION" ]] || { echo "error: --tap and --version are required" >&2; exit 2; }
if [[ -z "$SHA256" ]]; then
    [[ -n "$ZIP" ]] || { echo "error: pass --sha256 or --zip" >&2; exit 2; }
    SHA256="$(awk '{print $1}' "$ZIP.sha256")"
fi
CASK="$TAP/Casks/stillroom.rb"
if [[ ! -f "$CASK" ]]; then
    echo "==> No cask yet; copying distribution/stillroom.rb into the tap"
    cp "$(dirname "$0")/../distribution/stillroom.rb" "$CASK"
fi
/usr/bin/sed -i '' -E "s/version \"[^\"]+\"/version \"$VERSION\"/" "$CASK"
/usr/bin/sed -i '' -E "s/sha256 \"[^\"]+\"/sha256 \"$SHA256\"/" "$CASK"
echo "==> $CASK:"
git -C "$TAP" --no-pager diff -- Casks/stillroom.rb || true
git -C "$TAP" status --short -- Casks/stillroom.rb
echo
echo "Validate (optional): brew audit --cask --online $CASK && brew style $CASK"
echo "Then: git -C \"$TAP\" add Casks/stillroom.rb && git -C \"$TAP\" commit -m \"stillroom $VERSION\" && git -C \"$TAP\" push"
