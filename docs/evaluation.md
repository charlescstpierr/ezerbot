# Évaluation sur faux serveur — étape A

Le lanceur `replay_fake_scenario` rejoue une **liste JSON de messages utilisateur** dans une conversation Ezerbot dédiée. Il n’accepte aucun serveur ni compte : tous les endpoints sont ceux du `FakeOpenAi` en mémoire, sur loopback, et le lanceur refuse de démarrer si l’un d’eux sort de `http://127.0.0.1:`. Aucun accès réseau externe, navigateur, compte réel ou trousseau n’est nécessaire.

## Lancer

Le test est hermétique et tourne dans `cargo test`, sans variable d’environnement : il prépare lui-même un espace marqué dans un dossier temporaire, rejoue le scénario `crates/wrapper/tests/fixtures/eval-three-messages.json` avec un plafond fixé à 3 appels, vérifie que le plafond n’est pas atteint, puis supprime l’espace et le JSON de résultats qu’il a créés.

```sh
cargo test -p grokbot-wrapper --locked --test evaluation replay_fake_scenario
```

Sur une machine qui limite les compilations simultanées, lancer la commande via son script de limitation.

Les régressions de sécurité relancent ce même test dans un sous-processus (sentinelle `EZERBOT_EVAL_SUBPROCESS`, posée par elles seules) avec leurs propres espace, scénario et plafond lus dans l’environnement du sous-processus ; ce chemin impose aussi un `HOME` jetable. Le test hermétique, lui, ne lit jamais l’environnement.

Le dossier doit être absolu, neuf et ne contenir que le marqueur `.ezerbot-evaluation` (fichier ordinaire, pas un lien, contenu `ezerbot-evaluation-v1`). Le lanceur réclame atomiquement un sous-dossier `run`, y crée une base et une session **factice** (et, en sous-processus, impose un `HOME` jetable dans `run/home`). Il n’ouvre jamais une base existante. Un espace déjà utilisé est refusé : créer un autre dossier pour rejouer. L’espace garde les messages de test dans sa base locale ; ce n’est **pas** un résultat à publier. Ne jamais y placer de données réelles.

Le plafond d’appels est obligatoire, entier strictement positif (3 dans le test hermétique, `EZERBOT_EVAL_MAX_CALLS` pour les sous-processus). Les messages sont envoyés séquentiellement ; avec un plafond de 2 pour 3 messages, le troisième n’est jamais envoyé, les résultats sont écrits et le processus sort en erreur avec « Plafond atteint ». Le serveur factice impose aussi ce plafond à toute requête Responses, y compris auxiliaire. Les titres automatiques et la mémoire automatique sont désactivés pour cette mesure ; le budget de contexte synthétique est fixé à 2 000 000 tokens estimés pour éviter la compaction du scénario borné.

## Résultats

La ligne `EVAL_RESULT=...` indique `target/eval/fake-<date-Unix-ns>-<pid>.json`. Les valeurs sont **exclusivement numériques**, sur liste blanche :

- `input_tokens`, `output_tokens`, `cached_input_tokens` : sommes de `engine_response_usage` ; le cache est inclus dans l’entrée ;
- `model_calls`, `elapsed_ms` : compteurs durables de `engine_runs.usage` (durée active du moteur, pas durée totale du processus) ;
- `max_calls` : plafond demandé ;
- `limit_reached` : **1 = plafond atteint / scénario tronqué**, **0 = scénario terminé**.

Pour les trois messages fournis : entrée 300, sortie 60, entrée en cache 120, appels 3. Ce sont des chiffres synthétiques fixes, **pas une mesure de qualité ni de performance d’un vrai modèle**. Aucun identifiant, texte de conversation, jeton, chemin d’espace ou erreur brute n’est sérialisé dans les résultats. Les diagnostics du lanceur sont statiques et ne reproduisent pas le scénario.

## Vérifications ciblées

```sh
cargo test -p grokbot-wrapper --locked --test evaluation
cargo clippy -p grokbot-wrapper --locked --test evaluation -- -D warnings
cargo fmt --all -- --check
gitleaks detect --source . --no-banner
```

Les régressions lancent le même point d’entrée dans des sous-processus : scénario de trois messages, espace non marqué/non neuf ou lié symboliquement, plafond absent/invalide, arrêt à deux appels, valeurs-pièges absentes du JSON et des journaux, scénario invalide et plafond serveur auxiliaire. Le lanceur hermétique s’exécute avec elles, sans `#[ignore]`.

L’étape B (compte de test réel, mesures réelles et procédure associée) est hors périmètre.

Le [banc F1 de mémoire automatique](memory-auto-evaluation.md) rejoue séparément des sorties enregistrées synthétiques à travers l’extracteur, sans réseau ni compte. Il mesure les faits retenus et les garde-fous, pas les tokens ni la qualité d’un vrai modèle.
