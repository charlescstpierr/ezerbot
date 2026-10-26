#!/bin/bash
set -euo pipefail
E=/Users/cstpierrecharles/.treehouse/pm/_orch/pages/evidence/i561/r2
source "$E/env.sh"
git rev-parse HEAD > "$E/test-head.txt"
EZERBOT_GIT_COMMIT="$(git rev-parse HEAD)" /Users/cstpierrecharles/.treehouse/pm/_orch/bin/cargo-gate /Users/cstpierrecharles/.cargo/bin/cargo build --locked -p grokbot-wrapper > "$E/build-final.log" 2>&1
/Users/cstpierrecharles/.treehouse/pm/_orch/bin/test-gate node --test --test-concurrency=1 app/ui/select-bot.test.cjs app/ui/sidebar-rail.test.cjs app/ui/updates-browser.test.cjs app/ui/panels/updates.test.cjs app/ui/panels/topics.test.cjs app/ui/panels/memory-space.test.cjs app/ui/panels/memory-add.test.cjs app/ui/panels/memory-detail.test.cjs app/ui/panels/memory-filter.test.cjs app/ui/panels/memory-graph.test.cjs app/ui/panels/memory-links.test.cjs app/ui/panels/memory-review.test.cjs app/ui/panels/memory-used.test.cjs app/ui/panels/pages.test.cjs app/ui/panels/pages-browser.test.cjs app/ui/panels/pages-app-browser.test.cjs app/ui/panels/routines.test.cjs app/ui/iphone.test.cjs app/ui/iphone-panel.test.cjs app/web/src/reviewer-ui.test.js app/web/src/deep-link-navigation.test.js app/web/src/notifications-ui.test.js app/ui/composer-bar.test.cjs app/web/src/generated-dialogs.test.js > "$E/final-tests.log" 2>&1
echo 'Tests R2 terminés.'
