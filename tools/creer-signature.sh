#!/bin/bash
# À lancer volontairement par le propriétaire du Mac. Les tests doivent TOUJOURS
# passer un trousseau jetable explicite ; ce script ne change ni confiance ni liste
# de recherche, n'ouvre aucun autre trousseau et n'autorise jamais toutes les apps.
# Apostrophes typographiques intentionnelles dans les messages français.
# shellcheck disable=SC1112
set -euo pipefail
set +x
umask 077
identity='Ezerbot Signature Locale'
if [[ $# -gt 1 || "${1:-}" == '--help' || "${1:-}" == '-h' ]]; then
  printf '%s\n' 'Usage : tools/creer-signature.sh [chemin-du-trousseau]' 'Sans chemin, utilise le trousseau par défaut. Déverrouillez le trousseau avant de continuer.'
  [[ $# -le 1 ]] && exit 0 || exit 1
fi
if [[ "$(uname -s)" != Darwin ]]; then
  printf '%s\n' 'La signature locale nécessite macOS.' >&2
  exit 1
fi
keychain="${1:-}"
if [[ -z "$keychain" ]]; then
  keychain="$(/usr/bin/security default-keychain -d user | /usr/bin/xargs)"
fi
if [[ -z "$keychain" || ! -f "$keychain" ]]; then
  printf '%s\n' 'Trousseau introuvable. Indiquez le chemin d’un trousseau existant et déverrouillé.' >&2
  exit 1
fi
identities="$(/usr/bin/security find-identity -p codesigning "$keychain" 2>/dev/null || true)"
existing=false
if [[ "$identities" == *"\"$identity\""* ]]; then
  existing=true
  printf '%s\n' 'L’identité « Ezerbot Signature Locale » existe déjà. Aucun certificat ne sera recréé.'
fi
printf '%s' 'Mot de passe du trousseau choisi (saisie masquée) : ' >&2
if ! IFS= read -r -s keychain_password; then
  printf '\n%s\n' 'Saisie annulée. Aucun certificat n’a été créé.' >&2
  exit 1
fi
printf '\n' >&2
if ! /usr/bin/security unlock-keychain -p "$keychain_password" "$keychain" >/dev/null 2>&1; then
  unset keychain_password
  printf '%s\n' 'Impossible de déverrouiller le trousseau choisi. Vérifiez son mot de passe.' >&2
  exit 1
fi
work="$(mktemp -d "${TMPDIR:-/tmp}/ezerbot-signature.XXXXXXXX")"
trap 'rm -rf "$work"' EXIT
if [[ "$existing" == false ]]; then
cat > "$work/certificat.cnf" <<'CONFIG'
[req]
prompt = no
distinguished_name = sujet
x509_extensions = signature
[sujet]
CN = Ezerbot Signature Locale
[signature]
basicConstraints = critical,CA:false
keyUsage = critical,digitalSignature
extendedKeyUsage = critical,codeSigning
subjectKeyIdentifier = hash
CONFIG
if ! /usr/bin/openssl req -new -x509 -newkey rsa:2048 -nodes -sha256 -days 3650 \
  -config "$work/certificat.cnf" -keyout "$work/cle.pem" -out "$work/certificat.pem" > "$work/openssl.log" 2>&1; then
  printf '%s\n' 'Impossible de créer le certificat de signature local. Aucune identité n’a été importée.' >&2
  exit 1
fi
password="$(/usr/bin/openssl rand -hex 24)"
if ! /usr/bin/openssl pkcs12 -export -name "$identity" -inkey "$work/cle.pem" -in "$work/certificat.pem" \
  -out "$work/identite.p12" -passout "pass:$password" > "$work/export.log" 2>&1; then
  printf '%s\n' 'Impossible de préparer l’identité de signature. Aucune identité n’a été importée.' >&2
  exit 1
fi
# Autorisation limitée à l’outil Apple codesign, clé non exportable après import.
# Ne jamais utiliser -A, add-trusted-cert, ou modifier les ACL des autres clés.
if ! /usr/bin/security import "$work/identite.p12" -k "$keychain" -f pkcs12 -P "$password" \
  -x -T /usr/bin/codesign > "$work/import.log" 2>&1; then
  printf '%s\n' 'Import impossible. Déverrouillez le trousseau choisi puis relancez ce script. Aucun autre trousseau n’a été modifié.' >&2
  exit 1
fi
unset password
fi
# Ne sélectionner que notre propre clé privée de signature, jamais toutes les
# clés du trousseau de l’utilisateur. Aucune modification des réglages de confiance.
if ! /usr/bin/security set-key-partition-list -S apple-tool:,apple:,codesign: -s -t private \
  -l "$identity" -k "$keychain_password" "$keychain" > "$work/autorisation.log" 2>&1; then
  unset keychain_password
  printf '%s\n' 'L’identité est conservée, mais son autorisation pour codesign n’a pas pu être configurée. Relancez ce script avec le bon trousseau et son mot de passe.' >&2
  exit 1
fi
unset keychain_password
printf '%s\n' 'Identité « Ezerbot Signature Locale » prête dans le trousseau choisi.' \
  'Les prochaines reconstructions pourront utiliser cette même identité.' \
  'Conservez ce trousseau : cette identité protège la continuité d’accès aux secrets d’Ezerbot.'
