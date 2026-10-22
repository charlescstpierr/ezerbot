#!/usr/bin/env bash
# Teste scripts/verify-factory.sh sans cargo, clippy ni gitleaks réels.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
SCRIPT="$ROOT/scripts/verify-factory.sh"
fail_count=0
keep=()

cleanup() {
  local dir
  for dir in "${keep[@]+"${keep[@]}"}"; do
    /bin/rm -rf "$dir"
  done
}
trap cleanup EXIT

# PATH sans le vrai gh : seuls les outils du test et les doublures sont visibles.
SAFE=$(mktemp -d)
keep+=("$SAFE")
for tool in git tee date mktemp sed awk dirname basename mkdir cp rm env bash grep diff head tail chmod cat python3; do
  if [[ -x "/usr/bin/$tool" ]]; then
    ln -s "/usr/bin/$tool" "$SAFE/$tool"
  elif [[ -x "/bin/$tool" ]]; then
    ln -s "/bin/$tool" "$SAFE/$tool"
  else
    printf 'outil manquant: %s\n' "$tool" >&2
    exit 1
  fi
done

ok() {
  printf 'ok: %s\n' "$1" >&2
}

bad() {
  printf 'ECHEC: %s\n' "$1" >&2
  fail_count=$((fail_count + 1))
}

setup_repo() {
  REPO=$(mktemp -d)
  BIN=$(mktemp -d)
  keep+=("$REPO" "$BIN")
  CARGO_STUB_LOG="$REPO/cargo-invocations"
  : >"$CARGO_STUB_LOG"
  export CARGO_STUB_LOG
  unset RUSTC_STUB_VERSION GH_ISSUE_BODY GH_STUB_FAIL || true

  cat >"$BIN/cargo" <<'EOF'
#!/bin/bash
printf 'cargo %s\n' "$*" >>"${CARGO_STUB_LOG:?}"
if [[ "${CARGO_STUB_FAIL:-}" == "${1:-}" ]]; then
  printf '%s\n' 'echec volontaire' >&2
  exit 1
fi
case "${1:-}" in
  test)
    printf '%s\n' 'test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s'
    printf '%s\n' 'test result: ok. 2 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.01s'
    ;;
  build|clippy)
    printf '%s\n' '    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.01s'
    ;;
esac
exit 0
EOF
  cat >"$BIN/rustc" <<'EOF'
#!/bin/bash
printf '%s\n' "${RUSTC_STUB_VERSION:-rustc 1.99.1 (test)}"
EOF
  cat >"$BIN/gitleaks" <<'EOF'
#!/bin/bash
printf '%s\n' '1:00AM INF no leaks found'
exit 0
EOF
  chmod +x "$BIN/cargo" "$BIN/rustc" "$BIN/gitleaks"

  git -C "$REPO" init -b main >/dev/null
  git -C "$REPO" config user.email test@example.com
  git -C "$REPO" config user.name test
  printf 'a\n' >"$REPO/readme"
  git -C "$REPO" add readme
  git -C "$REPO" commit -m a >/dev/null
  git -C "$REPO" update-ref refs/remotes/origin/main HEAD
  printf 'b\n' >>"$REPO/readme"
  git -C "$REPO" add readme
  git -C "$REPO" commit -m b >/dev/null
  SHA=$(git -C "$REPO" rev-parse HEAD)
  export PATH="$BIN:$SAFE"
}

install_gh() {
  cat >"$BIN/gh" <<'EOF'
#!/bin/bash
if [[ "${1:-}" == "api" ]]; then
  method="GET"
  input=""
  shift
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --paginate) shift ;;
      --method) method=$2; shift 2 ;;
      --input) input=$2; shift 2 ;;
      --jq) shift 2 ;;
      *) shift ;;
    esac
  done
  state="${GH_STATE:?}"
  mkdir -p "$state"
  if [[ "$method" == "GET" ]]; then
    if [[ -f "$state/posted" ]]; then
      printf '%s\n' '[{"id":77,"body":"<!-- factory-proof -->\nprécédent"}]'
    else
      printf '%s\n' '[]'
    fi
    exit 0
  fi
  cp "$input" "$state/last.json"
  printf '%s\n' "$method" >>"$state/methods"
  printf '77\n' >"$state/posted"
  printf '%s\n' '{"id":77}'
  exit 0
fi
if [[ "${GH_STUB_FAIL:-}" == 1 ]]; then
  printf '%s\n' 'gh stub echec' >&2
  exit 1
fi
printf '%s\n' "${GH_ISSUE_BODY:-}"
EOF
  chmod +x "$BIN/gh"
}

run_script() {
  local caller=$PWD
  cd "$REPO"
  set +e
  OUTPUT=$(bash "$SCRIPT" "$@" 2>&1)
  STATUS=$?
  set -e
  cd "$caller"
}

expect_status() {
  local want=$1 label=$2
  if [[ "$STATUS" -ne "$want" ]]; then
    printf 'statut %s au lieu de %s (%s)\n%s\n' "$STATUS" "$want" "$label" "$OUTPUT" >&2
    bad "$label"
  else
    ok "$label"
  fi
}

require_grep() {
  local pattern=$1 label=$2
  if ! printf '%s\n' "$OUTPUT" | grep -F -q -- "$pattern"; then
    printf 'manque [%s] dans (%s)\n%s\n' "$pattern" "$label" "$OUTPUT" >&2
    bad "$label"
  else
    ok "$label"
  fi
}

require_absent() {
  local pattern=$1 label=$2
  if printf '%s\n' "$OUTPUT" | grep -F -q -- "$pattern"; then
    printf 'présent à tort [%s] (%s)\n%s\n' "$pattern" "$label" "$OUTPUT" >&2
    bad "$label"
  else
    ok "$label"
  fi
}

# Le script porte les commandes fixes et le mode strict.
for literal in \
  'set -euo pipefail' \
  'cargo build --workspace --locked' \
  'cargo fmt --check' \
  'cargo test --workspace --locked' \
  'cargo clippy --workspace --all-targets --locked -- -D warnings' \
  'gitleaks detect --redact --log-opts=origin/main..HEAD' \
  'gh issue view' \
  '<!-- factory-proof -->' \
  '--comment'
do
  if ! grep -F -q -- "$literal" "$SCRIPT"; then
    bad "littéral $literal"
  else
    ok "littéral $literal"
  fi
done

# 1. Arguments refusés.
setup_repo
run_script
expect_status 2 "sans argument"

run_script abc
expect_status 2 "issue non numérique"

run_script 1 aaaa fichier-en-trop
expect_status 2 "trois arguments dont le second n'est pas un SHA"

# 2. Barrière verte, fichier de critères, SHA attendu.
setup_repo
cat >"$REPO/criteres.md" <<'EOF'
```verify
echo critere-ok
```
EOF
run_script 137 "$SHA" "$REPO/criteres.md"
expect_status 0 "barrière verte"
first=$(printf '%s\n' "$OUTPUT" | head -n 1)
if [[ "$first" == '<!-- factory-proof -->' ]]; then
  ok "marqueur en tête"
else
  printf 'première ligne [%s]\n' "$first" >&2
  bad "marqueur en tête"
fi
require_grep "SHA \`${SHA}\`" "SHA dans le résumé"
log="$REPO/target/verify-factory/137-${SHA}.log"
if [[ -f "$log" ]]; then
  ok "journal présent"
else
  bad "journal présent"
fi
if [[ -f "$log" ]] && diff -u "$log" <(printf '%s\n' "$OUTPUT") >/dev/null; then
  ok "journal identique à la sortie"
else
  bad "journal identique à la sortie"
fi
require_grep "horodatage=" "horodatage"
require_grep "rustc 1.99.1 (test)" "version rustc"
require_grep "+ cargo build --workspace --locked" "build"
require_grep "+ cargo fmt --check" "fmt"
require_grep "+ cargo test --workspace --locked" "test workspace"
require_grep "+ cargo clippy --workspace --all-targets --locked -- -D warnings" "clippy"
require_grep "+ gitleaks detect --redact --log-opts=origin/main..HEAD" "gitleaks"
require_grep "critères: fichier ${REPO}/criteres.md" "source fichier"
require_grep "+ echo critere-ok" "commande verify"
require_grep "critere-ok" "sortie verify"
require_grep "verify-factory: ok" "verdict ok"
require_grep '- `cargo build --workspace --locked` — exit 0 — Finished '"'"'dev'"'"' profile [unoptimized + debuginfo] target(s) in 0.01s' "résultat build"
require_grep '- `cargo fmt --check` — exit 0 — ok' "résultat fmt"
require_grep '- `cargo test --workspace --locked` — exit 0 — test result: ok. 12 passed; 0 failed; 1 ignored; 0 measured' "résultat tests"
require_grep '- `gitleaks detect --redact --log-opts=origin/main..HEAD` — exit 0 — no leaks found' "résultat gitleaks"
require_grep '- `echo critere-ok` — exit 0 — critere-ok' "résultat critère"
require_grep '<summary>Journal complet</summary>' "journal replié"
if [[ "$(printf '%s\n' "$OUTPUT" | grep -c '^horodatage=')" -eq 1 ]]; then
  ok "un seul horodatage"
else
  bad "un seul horodatage"
fi
if printf '%s\n' "$OUTPUT" | grep -Eq '^horodatage=[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$'; then
  ok "horodatage UTC"
else
  bad "horodatage UTC"
fi

# 3. Sans commande verify : la barrière fixe suffit.
setup_repo
printf 'pas de bloc\n' >"$REPO/criteres.md"
run_script 12 "$REPO/criteres.md"
expect_status 0 "sans commande verify"
require_grep "critères: aucune commande" "aucune commande"

# 4. Une commande fausse n'empêche pas la suivante.
setup_repo
cat >"$REPO/criteres.md" <<'EOF'
```verify
false
echo ensuite
```
EOF
run_script 9 "$REPO/criteres.md"
expect_status 1 "commande verify en échec"
require_grep "ensuite" "commande suivante exécutée"
require_grep "verify-factory: échec" "verdict échec verify"

# 5. Échec de fmt : on n'enchaîne pas les tests.
setup_repo
printf '```verify\necho ne-doit-pas-tourner\n```\n' >"$REPO/criteres.md"
export CARGO_STUB_FAIL=fmt
run_script 3 "$REPO/criteres.md"
expect_status 1 "fmt en échec"
require_grep "verify-factory: échec" "verdict échec fmt"
if grep -F -q 'cargo test' "$CARGO_STUB_LOG"; then
  bad "cargo test non lancé après fmt"
else
  ok "cargo test non lancé après fmt"
fi
require_absent "+ echo ne-doit-pas-tourner" "critères non lancés après fmt"
unset CARGO_STUB_FAIL

# 6. SHA différent de HEAD.
setup_repo
old=$(git -C "$REPO" rev-parse origin/main)
printf '```verify\necho non\n```\n' >"$REPO/criteres.md"
run_script 4 "$old" "$REPO/criteres.md"
expect_status 1 "SHA différent"
require_grep "différent du SHA demandé ${old}" "message SHA différent"
if [[ -s "$CARGO_STUB_LOG" ]]; then
  bad "cargo non lancé si SHA différent"
else
  ok "cargo non lancé si SHA différent"
fi

# 7. SHA inconnu.
setup_repo
printf '```verify\necho non\n```\n' >"$REPO/criteres.md"
run_script 4 aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa "$REPO/criteres.md"
expect_status 1 "SHA inconnu"
require_grep "SHA inconnu: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" "message SHA inconnu"

# 8. gh fournit le bloc verify et le fichier est ignoré.
setup_repo
install_gh
export GH_ISSUE_BODY=$'texte\n```verify\necho depuis-gh\n```\n'
printf '```verify\necho depuis-fichier\n```\n' >"$REPO/criteres.md"
run_script 137 "$SHA" "$REPO/criteres.md"
expect_status 0 "bloc verify via gh"
require_grep "critères: gh issue view 137" "source gh"
require_grep "depuis-gh" "commande gh"
require_absent "depuis-fichier" "fichier ignoré si gh a un bloc"

# 9. gh sans bloc : le fichier passé en argument est lu.
setup_repo
install_gh
export GH_ISSUE_BODY='Critères en prose, sans bloc.'
printf '```verify\necho depuis-fichier\n```\n' >"$REPO/criteres.md"
run_script 137 "$REPO/criteres.md"
expect_status 0 "repli fichier"
require_grep "aucun bloc verify sur le ticket" "mention du repli"
require_grep "depuis-fichier" "commande du fichier"

# 10. gh en échec.
setup_repo
install_gh
export GH_STUB_FAIL=1
printf '```verify\necho non\n```\n' >"$REPO/criteres.md"
run_script 137 "$REPO/criteres.md"
expect_status 1 "gh en échec"
require_grep "gh issue view 137 a échoué" "message gh"
require_absent "+ cargo build --workspace --locked" "pas de build si gh échoue"
unset GH_STUB_FAIL

# 11. Ni gh ni fichier.
setup_repo
run_script 137
expect_status 2 "gh absent sans fichier"
require_grep "gh absent: passer le corps du ticket en fichier" "message gh absent"

# 12. Mauvais rustc.
setup_repo
export RUSTC_STUB_VERSION='rustc 1.83.0 (old)'
printf '```verify\necho non\n```\n' >"$REPO/criteres.md"
run_script 8 "$REPO/criteres.md"
expect_status 1 "mauvais rustc"
require_grep "rustc 1.99.1 stable requis" "message rustc"
if [[ -s "$CARGO_STUB_LOG" ]]; then
  bad "cargo non lancé si rustc incorrect"
else
  ok "cargo non lancé si rustc incorrect"
fi

# 13. Commentaire et ligne vide ignorés, deux blocs exécutés.
setup_repo
cat >"$REPO/criteres.md" <<'EOF'
```verify
# commentaire
echo un

echo deux
```
texte
```verify
echo trois
```
EOF
run_script 15 "$REPO/criteres.md"
expect_status 0 "deux blocs verify"
require_grep "un" "première commande"
require_grep "deux" "deuxième commande"
require_grep "trois" "troisième commande"
require_absent "+ # commentaire" "commentaire ignoré"

# 14. --comment crée, puis met à jour le commentaire marqué.
setup_repo
install_gh
export GH_STATE="$REPO/gh-state"
git -C "$REPO" remote add origin https://github.com/exemple/ezerbot.git
printf '```verify\necho critere-ok\n```\n' >"$REPO/criteres.md"
run_script --comment 138 137 "$SHA" "$REPO/criteres.md"
expect_status 0 "commentaire créé"
if [[ "$(cat "$GH_STATE/methods")" == "POST" ]]; then
  ok "création POST"
else
  printf 'méthodes [%s]\n' "$(cat "$GH_STATE/methods")" >&2
  bad "création POST"
fi
if python3 -c 'import json,sys; body=json.load(open(sys.argv[1],encoding="utf-8"))["body"]; full=open(sys.argv[2],encoding="utf-8").read(); sys.exit(0 if body==full and "<!-- factory-proof -->" in body else 1)' "$GH_STATE/last.json" "$REPO/target/verify-factory/137-${SHA}.log"; then
  ok "corps publié"
else
  bad "corps publié"
fi
require_absent "commentaire tronqué" "pas de troncature si le journal tient"
run_script --comment 138 137 "$SHA" "$REPO/criteres.md"
expect_status 0 "commentaire mis à jour"
if [[ "$(cat "$GH_STATE/methods")" == $'POST\nPATCH' ]]; then
  ok "mise à jour PATCH"
else
  printf 'méthodes [%s]\n' "$(cat "$GH_STATE/methods")" >&2
  bad "mise à jour PATCH"
fi
unset GH_STATE

# 15. --comment sans dépôt origin.
setup_repo
install_gh
export GH_STATE="$REPO/gh-state"
printf '```verify\necho critere-ok\n```\n' >"$REPO/criteres.md"
run_script --comment 138 137 "$REPO/criteres.md"
expect_status 1 "commentaire sans origin"
require_grep "origin introuvable: --comment impossible" "message origin"
unset GH_STATE

# 16. --comment sans gh.
setup_repo
printf '```verify\necho critere-ok\n```\n' >"$REPO/criteres.md"
run_script --comment 9 12 "$REPO/criteres.md"
expect_status 1 "commentaire sans gh"
require_grep "gh absent: --comment impossible" "message commentaire sans gh"

# 17. Journal trop long : le fichier reste complet, le commentaire tient dans la limite.
setup_repo
install_gh
export GH_STATE="$REPO/gh-state"
git -C "$REPO" remote add origin https://github.com/exemple/ezerbot.git
cat >"$REPO/criteres.md" <<'EOF'
```verify
python3 -c 'print("\n".join(["ligne-journal"]*8000))'
```
EOF
run_script --comment 138 21 "$REPO/criteres.md"
expect_status 0 "commentaire long"
long_log="$REPO/target/verify-factory/21-${SHA}.log"
if [[ "$(grep -c 'ligne-journal' "$long_log")" -ge 8000 ]]; then
  ok "journal local complet"
else
  bad "journal local complet"
fi
if grep -F -q 'Journal tronqué' "$long_log"; then
  bad "journal local non tronqué"
else
  ok "journal local non tronqué"
fi
if python3 -c '
import json, sys
body = json.load(open(sys.argv[1], encoding="utf-8"))["body"]
if len(body) > 65536:
    raise SystemExit(1)
need = ("<!-- factory-proof -->", "verify-factory: ok", "[… milieu omis …]", "ligne-journal", "Journal tronqué")
raise SystemExit(0 if all(part in body for part in need) else 1)
' "$GH_STATE/last.json"; then
  ok "commentaire dans la limite"
else
  bad "commentaire dans la limite"
fi
unset GH_STATE

# 18. Option --comment mal formée.
setup_repo
run_script --comment
expect_status 2 "--comment sans numéro"
run_script --comment abc 1
expect_status 2 "--comment non numérique"

if [[ "$fail_count" -ne 0 ]]; then
  printf '%s échec(s)\n' "$fail_count" >&2
  exit 1
fi
printf '%s\n' 'verify-factory.test: ok' >&2
