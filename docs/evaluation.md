# Évaluation sur faux serveur — étape A

Le lanceur opt-in `replay_fake_scenario` rejoue une **liste JSON de messages utilisateur** dans une conversation Ezerbot dédiée. Il n’accepte aucun serveur ni compte : tous les endpoints sont ceux du `FakeOpenAi` en mémoire, sur loopback. Aucun accès réseau externe, navigateur, compte réel ou trousseau n’est nécessaire.

## Lancer

Depuis la racine du dépôt :

```sh
space=$(mktemp -d)
printf 'ezerbot-evaluation-v1\n' > "$space/.ezerbot-evaluation"
EZERBOT_EVAL_SPACE="$space" \
EZERBOT_EVAL_SCENARIO="$PWD/crates/wrapper/tests/fixtures/eval-three-messages.json" \
EZERBOT_EVAL_MAX_CALLS=3 \
cargo test -p grokbot-wrapper --locked --test evaluation replay_fake_scenario -- --ignored --exact --nocapture
```

Sur le Mac partagé, préfixer chaque commande Cargo par `/Users/cstpierrecharles/firstmate/data/build-slot.sh`.

Le dossier doit être absolu, neuf et ne contenir que le marqueur ci-dessus (fichier ordinaire, pas un lien). Le lanceur réclame atomiquement un sous-dossier `run`, y crée une base et une session **factice**, et impose un `HOME` jetable dans `run/home`. Il n’ouvre jamais une base existante. Un espace déjà utilisé est refusé : créer un autre dossier pour rejouer. L’espace garde les messages de test dans sa base locale ; ce n’est **pas** un résultat à publier. Ne jamais y placer de données réelles.

`EZERBOT_EVAL_MAX_CALLS` est obligatoire, entier strictement positif. Les messages sont envoyés séquentiellement ; avec un plafond de 2 pour 3 messages, le troisième n’est jamais envoyé, les résultats sont écrits et le processus sort en erreur avec « Plafond atteint ». Le serveur factice impose aussi ce plafond à toute requête Responses, y compris auxiliaire. Les titres automatiques et la mémoire automatique sont désactivés pour cette mesure ; le budget de contexte synthétique est fixé à 2 000 000 tokens estimés pour éviter la compaction du scénario borné.

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

Les régressions lancent le même point d’entrée dans des sous-processus : scénario de trois messages, espace non marqué/non neuf ou lié symboliquement, plafond absent/invalide, arrêt à deux appels, valeurs-pièges absentes du JSON et des journaux, scénario invalide et plafond serveur auxiliaire. Le lanceur lui-même reste ignoré par défaut.

L’étape B (compte de test réel, mesures réelles et procédure associée) est hors périmètre.
