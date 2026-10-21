# Conventions de code

## Rust

Workspace Cargo à la racine (`crates/*`). `tools/verify-rust.py` enchaîne le formatage, les tests et Clippy en mode strict :

```sh
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

Les avertissements Clippy sont des erreurs (`-D warnings`). Les tests passent sur le workspace verrouillé (`--locked`).

## JavaScript

Les tests d'interface utilisent le runner natif Node (`node:test`, `node:assert/strict`). Pas de Vitest ni de Jest.

```sh
node --test app/web/src/*.test.js
node --test app/ui/provider-ui.test.cjs app/ui/panels/*.test.cjs
```

`app/web` est un module ESM. Les tests de `app/ui` sont en CommonJS (`.cjs`).
