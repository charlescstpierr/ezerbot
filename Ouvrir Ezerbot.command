#!/bin/zsh
set -eu
project_dir="${0:A:h}"
if [[ ! -d "$project_dir/app/build/Ezerbot.app" ]]; then
  /bin/bash "$project_dir/tools/build-app.sh"
fi
exec /usr/bin/open "$project_dir/app/build/Ezerbot.app"
