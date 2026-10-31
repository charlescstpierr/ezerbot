# Dossier de run et étapes isolées de la preuve Linux, sourcés par prove-linux.sh (et les tests).
# Le dossier du run (chemin court pour le socket Chrome) contient HOME, XDG_* et TMPDIR ; le trap
# le supprime et tue le groupe de processus de l'étape en cours, en réussite comme en échec.
run=$(mktemp -d /tmp/ezv.XXXXXX)
mkdir -p "$run/home" "$run/config" "$run/cache" "$run/data" "$run/runtime" "$run/tmp"
child=''
kill_group() {
  kill -TERM -- "-$1" 2>/dev/null || true
  for ((i = 0; i < 100; i++)); do kill -0 -- "-$1" 2>/dev/null || break; sleep 0.05; done
  kill -KILL -- "-$1" 2>/dev/null || true
}
cleanup() {
  local status=$?
  trap - EXIT INT TERM
  if [[ -n $child ]]; then
    kill_group "$child"
    wait "$child" 2>/dev/null || true
  fi
  rm -rf -- "$run"
  [[ ! -e $run ]] && printf 'nettoyage: %s supprimé\n' "$run"
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
# Lance une étape dans son propre groupe, puis tue ce qui en reste (Xvfb, Chrome, wrapper, enfants).
step() {
  local directory=$1; shift
  (cd "$directory" && exec setsid "$@") &
  child=$!
  local status=0
  wait "$child" || status=$?
  kill_group "$child"
  child=''
  return "$status"
}
