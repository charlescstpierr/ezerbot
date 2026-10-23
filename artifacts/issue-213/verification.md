# Vérification de #213

Captures 820×560 : `sources-clair.png`, `sources-sombre.png`, `source-introuvable.png`.
Reproduction : démarrer Vite avec `app/web` comme racine, puis `PLAYWRIGHT_MODULE=/chemin/playwright/index.mjs node --test tools/verify-sources.mjs` (Chromium configurable avec `CHROMIUM_PATH`). Le scénario vérifie clic, métadonnées, ouverture du bon identifiant, compte déconnecté, retour du focus avec Échap et source barrée.

Les six tests Node de `sources.test.js`, les 16 tests Rust de bibliothèque et les 157 tests des panneaux passent au premier passage. Un second passage et la relance isolée rencontrent un délai Chromium dans `à 1140 px, chaque nom de pastille du groupe s’affiche en entier`. Le build web, `cargo build --workspace --locked`, le formatage et Clippy strict passent avec Rust 1.99.1 installé.
La suite JS complète signale quatre échecs de `delegation-ui.test.js` (« Unexpected token 'function' »), reproduits sur le fichier `app.js` de HEAD, et le test de temps de recherche de `thread-find-rating.test.js` (« une longue réponse à beaucoup de correspondances se traite en temps linéaire »), dont le code n'est pas modifié ici.
Le premier passage Rust a échoué dans `every_transport_hang_ends_with_a_persisted_french_error_and_retry_works` et `slow_compaction_is_foreground_work_and_logs_the_user_run` ; les 22 tests de `chat_reliability` passent au second passage.

Le diff source reste inférieur à 300 lignes ; `app/ui/chat.js` et `app/ui/chat.css` sont les bundles embarqués régénérés.
La relance complète `cargo test --workspace --locked --no-fail-fast` passe : 1 867 tests réussis, aucun échec.
