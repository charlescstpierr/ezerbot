# Contrôle Linux des pull requests

Le workflow [CI Linux](../.github/workflows/linux-pr.yml) lance le job `linux-build-test` sur chaque pull request et chaque push sur `main`. Le checkout par défaut teste le commit de fusion de la pull request. Aucun filtre de chemin ne limite ce contrôle.

Après le premier run, Charles marque `linux-build-test` comme contrôle requis dans la protection de `main`. Ce réglage est hors PR. Le job `compile` de [Coque macOS](../.github/workflows/macos-shell.yml) reste inchangé. Le choix de rendre ce contrôle macOS requis appartient aussi au réglage de protection, hors PR.

Le job utilise Ubuntu 24.04, Node 24 et le toolchain de `rust-toolchain.toml`. Il installe `lsof` pour les tests de webhooks, ainsi que `pkg-config`, `libgtk-3-dev` et `libwebkit2gtk-4.1-dev` pour compiler la coque Linux du workspace. Il vérifie le Chrome fourni par l'image Ubuntu avec `app/ui/chrome-binary.cjs`. Le cache contient les téléchargements Cargo et les dépendances compilées, avec une clé liée au système, à l'architecture, au toolchain, à `Cargo.lock` et au commit. Il ne contient ni journal ni données d'Ezerbot.

La restauration et la sauvegarde du cache sont séparées. Une compilation réussie permet la sauvegarde même si les tests échouent, sauf annulation. Une clé déjà restaurée à l'identique n'est pas sauvegardée à nouveau.

Les commandes principales sont les suivantes.

```sh
rustup show active-toolchain
npm ci --prefix app/web --no-audit --no-fund
df -BG "$GITHUB_WORKSPACE"
cargo build --workspace --locked
df -BG "$GITHUB_WORKSPACE"
cargo test --workspace --locked --no-fail-fast -- --skip a_command_that_reaches_the_data_without_naming_it_reads_nothing --exact
node --test --test-reporter=tap app/web/src/*.test.js
node --test --test-reporter=tap app/ui/*.test.cjs app/ui/panels/*.test.cjs
```

La seule exclusion est le test exact `a_command_that_reaches_the_data_without_naming_it_reads_nothing`, suivi dans [#528, bac à sable YOLO Linux](https://github.com/charlescstpierr/ezerbot/issues/528).

Les échecs Linux connus restent exécutés, sans filtre ni saut. Ils sont intermittents : chaque run n'en montre qu'une partie. Ils rendent le contrôle rouge s'ils se reproduisent. Chacun a un ticket de l'[épique Linux #600](https://github.com/charlescstpierr/ezerbot/issues/600).

| Test | Symptôme | Observé sur | Suivi |
| --- | --- | --- | --- |
| `state_scale::a_state_read_runs_the_same_statements_whatever_the_length_of_the_history` | Échec Rust | `linux-build-test`, suite nocturne | [#596](https://github.com/charlescstpierr/ezerbot/issues/596), PR [#602](https://github.com/charlescstpierr/ezerbot/pull/602) |
| `connectors::linear_draft_refuses_a_bad_target_without_a_card_and_an_unknown_issue_without_an_effect` | Panique `tests/support/mod.rs:666` | Diagnostic de la box | [#596](https://github.com/charlescstpierr/ezerbot/issues/596), PR [#602](https://github.com/charlescstpierr/ezerbot/pull/602) |
| `engine::pending_wake_is_delivered_by_tick_only_after_committed_resume` | Panique `wake not delivered` | `linux-build-test`, depuis #585 | [#594](https://github.com/charlescstpierr/ezerbot/issues/594), PR [#603](https://github.com/charlescstpierr/ezerbot/pull/603) |
| `engine::multibot_live_remove_stop_and_new_human_ignore_late_remote_events` | Échec Rust | Diagnostic de la box | [#594](https://github.com/charlescstpierr/ezerbot/issues/594), PR [#603](https://github.com/charlescstpierr/ezerbot/pull/603) |
| `app/ui/skill-folder-import.test.cjs:78`, « Importer un dossier skips a real symlink whose target is outside the folder » | Bouton d'import introuvable | `linux-build-test`, suite nocturne | [#594](https://github.com/charlescstpierr/ezerbot/issues/594), PR [#603](https://github.com/charlescstpierr/ezerbot/pull/603) |
| `app/ui/updates-browser.test.cjs:9`, « les mises à jour préservent le rail », variantes de thème et de taille | État connecté attendu absent | `linux-build-test`, suite nocturne | [#594](https://github.com/charlescstpierr/ezerbot/issues/594), PR [#603](https://github.com/charlescstpierr/ezerbot/pull/603) |
| `app/ui/panels/pages-browser.test.cjs:21` | Échec Node | Suite nocturne | [#594](https://github.com/charlescstpierr/ezerbot/issues/594), PR [#603](https://github.com/charlescstpierr/ezerbot/pull/603) |
| `app/ui/sidebar-rail.test.cjs:6` | Échec Node | Suite nocturne | [#594](https://github.com/charlescstpierr/ezerbot/issues/594), PR [#603](https://github.com/charlescstpierr/ezerbot/pull/603) |
| `app/web/src/generated-dialogs.test.js:27` | Échec Node | `linux-build-test`, suite nocturne | [#594](https://github.com/charlescstpierr/ezerbot/issues/594), PR [#603](https://github.com/charlescstpierr/ezerbot/pull/603) |

Les tests Rust et les deux groupes de tests Node utilisent `!cancelled()`. Ils s'exécutent même après un échec antérieur, sauf annulation. Chaque échec conserve le statut rouge du job.

Le job a une limite de 25 minutes. L'objectif est de terminer en moins de 25 minutes avec le cache chaud.

La [suite nocturne Linux](../.github/workflows/suite-complete-linux.yml) et sa réparation dans [#594](https://github.com/charlescstpierr/ezerbot/issues/594) ne sont pas modifiées.

Vérification locale du workflow.

```sh
actionlint .github/workflows/linux-pr.yml
```
