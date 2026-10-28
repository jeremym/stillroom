#!/usr/bin/env bash
#
# Create the long-lived, self-signed "Stillroom Release" code-signing identity and
# (optionally) export a password-protected .p12 backup.
#
# Why self-signed: there is no Apple Developer ID for this project (by choice). Two things key
# off the signature's designated requirement (`identifier "com.jeremym.stillroom" and certificate
# leaf = H"<sha1>"`): Gatekeeper's one-time approval and the login keychain's access list for the
# API keys Stillroom stores. Both stay stable across releases ONLY if this one certificate is
# reused forever. Ad-hoc signing pins to the build's cdhash, so every update would prompt again.
# Create it once; never lose the .p12.
#
# ⚠️  PASSWORD-PROMPT STEP. Expect these macOS dialogs, in order:
#     1. (maybe) "security wants to import key … into the login keychain" → login password
#     2. "security wants to modify Trust Settings"                        → login password
#     Both come from /usr/bin/security, run by this script, touching ONLY the login keychain.
#     Nothing is uploaded anywhere. Run it attended, never from an unattended agent.
#
# Usage:
#   scripts/create-release-cert.sh [--out PATH] [--p12-password PASS] [--no-backup]
#   --no-backup: skip the .p12 export; export one later from Keychain Access
#                (right-click the "Stillroom Release" private key → Export).

set -euo pipefail

IDENTITY="Stillroom Release"
KEYCHAIN="$HOME/Library/Keychains/login.keychain-db"
OUT="Stillroom-Release"
P12_PASSWORD=""
NO_BACKUP=0

while [[ $# -gt 0 ]]; do
    case "$1" in
        --out) OUT="$2"; shift 2 ;;
        --p12-password) P12_PASSWORD="$2"; shift 2 ;;
        --no-backup) NO_BACKUP=1; shift ;;
        *) echo "error: unknown argument '$1'" >&2; exit 2 ;;
    esac
done

if security find-identity -v -p codesigning 2>/dev/null | grep -qF "$IDENTITY"; then
    echo "Identity '$IDENTITY' already exists in the keychain. Not creating a second one."
    security find-identity -v -p codesigning | grep -F "$IDENTITY"
    exit 0
fi

if [[ "$NO_BACKUP" == "0" && -z "$P12_PASSWORD" ]]; then
    read -r -s -p "Choose a password to protect the exported .p12: " P12_PASSWORD
    echo
    [[ -n "$P12_PASSWORD" ]] || { echo "error: a .p12 password is required." >&2; exit 2; }
fi

WORKDIR="$(mktemp -d)"
trap 'rm -rf "$WORKDIR"' EXIT

cat > "$WORKDIR/cert.cnf" <<CNF
[ req ]
distinguished_name = dn
x509_extensions    = ext
prompt             = no
[ dn ]
CN = $IDENTITY
[ ext ]
basicConstraints     = critical,CA:false
keyUsage             = critical,digitalSignature
extendedKeyUsage     = critical,codeSigning
CNF

echo "==> Generating self-signed certificate (valid 20 years)"
openssl req -x509 -newkey rsa:2048 -sha256 -days 7300 -nodes \
    -keyout "$WORKDIR/key.pem" -out "$WORKDIR/cert.pem" -config "$WORKDIR/cert.cnf" 2>/dev/null

# The Security framework only accepts PKCS#12 built with legacy SHA1/3DES.
echo "==> Packaging as PKCS#12 for the keychain"
openssl pkcs12 -export -inkey "$WORKDIR/key.pem" -in "$WORKDIR/cert.pem" \
    -out "$WORKDIR/cert.p12" -name "$IDENTITY" \
    -macalg sha1 -keypbe PBE-SHA1-3DES -certpbe PBE-SHA1-3DES -passout pass:temp 2>/dev/null

echo "==> Importing into the login keychain (dialog 1, may prompt)"
# -T grants codesign access to the key without a per-use prompt.
security import "$WORKDIR/cert.p12" -k "$KEYCHAIN" -P temp -A \
    -T /usr/bin/codesign -T /usr/bin/security >/dev/null

echo "==> Trusting it for code signing (dialog 2: 'modify Trust Settings')"
security add-trusted-cert -r trustRoot -p codeSign -k "$KEYCHAIN" "$WORKDIR/cert.pem"

if [[ "$NO_BACKUP" == "0" ]]; then
    echo "==> Exporting password-protected backup to $OUT.p12"
    openssl pkcs12 -export -inkey "$WORKDIR/key.pem" -in "$WORKDIR/cert.pem" \
        -out "$OUT.p12" -name "$IDENTITY" \
        -macalg sha1 -keypbe PBE-SHA1-3DES -certpbe PBE-SHA1-3DES -passout pass:"$P12_PASSWORD" 2>/dev/null
fi

LEAF_SHA1="$(openssl x509 -in "$WORKDIR/cert.pem" -noout -fingerprint -sha1 | sed 's/.*=//; s/://g' | tr '[:upper:]' '[:lower:]')"
LEAF_SHA256="$(openssl x509 -in "$WORKDIR/cert.pem" -noout -fingerprint -sha256 | sed 's/.*=//; s/://g' | tr '[:upper:]' '[:lower:]')"

echo
echo "Done."
echo "  Identity:            $IDENTITY (login keychain, 20 years)"
if [[ "$NO_BACKUP" == "0" ]]; then
    echo "  Backup:              $OUT.p12  → password manager, then delete from disk"
else
    echo "  Backup:              none written. Export from Keychain Access when ready."
fi
echo "  Cert leaf (SHA-1):   $LEAF_SHA1   (what codesign -d -r- prints)"
echo "  Cert leaf (SHA-256): $LEAF_SHA256"
echo
echo "Record the SHA-1 in README.md → 'Verifying a download'. Next: scripts/build-release.sh --version X.Y.Z"
