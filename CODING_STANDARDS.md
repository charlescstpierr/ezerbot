# Conventions de code

## Rust

Workspace Cargo à la racine (`crates/*`). `tools/verify-rust.py` enchaîne le formatage, les tests et Clippy en mode strict :

```sh
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

Les avertissements Clippy sont des erreurs (`-D warnings`). Les tests passent sur le workspace verrouillé (`--locked`).

Un test Rust qui a besoin d'un dossier ou d'une base temporaire utilise `grokbot_test_support::TestDir` (crate `crates/test-support`, en `[dev-dependencies]`) : un sous-dossier unique de `$TMPDIR`, supprimé en entier à la fin du test, échec compris. Jamais `std::env::temp_dir().join(..)` (le test `no_raw_temp_dir` le refuse) : une base posée directement dans `$TMPDIR` fuit au moindre échec et fait balayer tout `$TMPDIR` par `Vault::open`. Déclarer le `TestDir` avant l'application ou la connexion qu'il contient, ou en dernier champ d'une fixture, pour qu'il soit supprimé après eux. Un test qui utilise le fournisseur lance des fils de fond (sonde de connexion, catalogue de modèles, quota, retour OAuth) qui écrivent dans ce dossier : il garde le `Provider` dans une garde `Drop` qui appelle `wait_for_background_work()` (bornée, `Err` si un fil ne finit pas), déclarée après le faux serveur et le `TestDir` pour qu'elle s'exécute avant eux, y compris pendant l'unwinding d'une assertion qui échoue ; jamais une attente plus longue.

## JavaScript

Les tests d'interface utilisent le runner natif Node (`node:test`, `node:assert/strict`). Pas de Vitest ni de Jest.

```sh
node --test app/web/src/*.test.js
node --test app/ui/provider-ui.test.cjs app/ui/panels/*.test.cjs
```

`app/web` est un module ESM. Les tests de `app/ui` sont en CommonJS (`.cjs`).
