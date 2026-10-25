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

## Réussite des tâches déléguées par tokens dépensés

Le test `crates/wrapper/tests/delegation_success.rs` mesure une tâche déléguée à un sous-bot : combien de tâches **réussissent**, pour combien de tokens. Tout tourne sur le `FakeOpenAi` en boucle locale (même vérification `http://127.0.0.1:` que le lanceur ci-dessus) : aucun compte, aucun réseau, aucun juge-modèle. Aucun réglage mesuré n’est modifié : budgets du moteur (12 appels de modèle, 24 appels d’outil, 600 s), descriptions d’outils, modèle et effort de raisonnement restent ceux de production. Le harnais règle seulement l’espace de l’essai : mode « Demander avant d’agir » (le mode YOLO, actif par défaut, lèverait les cartes), fichiers activés pour le bot enfant et dossier de l’essai accordé. Les approbations restent demandées : chaque essai doit lever la carte de lancement en fond, et `fichier` celle de l’écriture.

### Ce qui compte comme un succès

Un essai réussit si la tâche a été **terminée par `complete_task`** (l’arête de délégation finit `Done`) **et** si le correcteur de la tâche la juge correcte. Une bonne réponse écrite en texte libre ne termine pas la tâche : le test `a_task_that_never_calls_complete_task_fails_even_when_its_text_looks_done` le vérifie avec un enfant qui répond « Terminé : 391. » sans appeler `complete_task`. L’inverse échoue aussi : une tâche terminée mais fausse n’est pas un succès.

Le calculateur est une fonction pure, sans entrée-sortie : `report(&[Trial])` avec `Trial { task, completed, correct, tokens }`. Il imprime :

- le **taux de réussite** (succès ÷ essais) ;
- les **tokens par succès** (tokens totaux ÷ succès ; `—` s’il n’y a aucun succès) ;
- les **succès par million de tokens** ;
- par tâche, la **moyenne ± écart** du taux de réussite (en %) et des tokens, avec médiane, minimum, maximum et nombre de répétitions. L’écart est l’écart-type d’échantillon (n − 1), nul pour un seul essai.

Les tokens d’un essai sont la somme de `engine_response_usage` du run de l’**enfant** (entrée + sortie ; le cache est déjà compris dans l’entrée). Les tokens du bot parent, qui ne fait que déléguer puis relire le résultat, sont exclus : la mesure porte sur la tâche déléguée. Chaque appel de modèle de l’enfant, à l’essai *t* (0, 1, 2), déclare 300 + 100·*t* tokens d’entrée et 50 de sortie.

### Le corpus

Cinq tâches factices dans un registre (`CORPUS`), chacune avec son script de faux modèle et son correcteur déterministe. Le correcteur ne voit que l’état final observable : le résumé `Done`, les fichiers de l’espace de l’essai, les faits actifs du bot enfant. Il ne connaît jamais le comportement scripté.

| Tâche | Consigne | Correcteur |
| --- | --- | --- |
| `produit` | calculer 17 × 23 | le résumé contient `391` comme mot entier, sans signe ni décimales (`-391`, `391,5` sont faux) |
| `lecture` | lire `donnees.txt` (`code=7B42`) dans un dossier accordé et donner le code | le résumé contient `7B42` comme mot entier |
| `fichier` | créer `rapport.txt` contenant exactement `OK` | le fichier existe avec le contenu `OK` |
| `memoire` | retenir que la réunion d’équipe est le mardi | le bot enfant a un fait actif « La réunion d’équipe est le mardi. » |
| `dossier` | compter les `.txt` de `inventaire/` (3 fichiers) | le résumé contient `3 fichiers` comme mots entiers (`13 fichiers` est faux) |

Chaque tâche est rejouée trois fois. Le « bruit » est un `Behaviour` : la politique du faux modèle (et de la personne qui approuve) pour cet essai. `Solve` fait la tâche et appelle `complete_task` ; `WrongAnswer` termine avec une mauvaise valeur ; `TextOnly` répond en texte libre sans `complete_task` ; `Denied` refuse la carte d’approbation d’écriture, puis le faux modèle prétend quand même avoir réussi.

Chaque essai a son propre dossier temporaire, sa base, son faux serveur et son bot enfant ; l’espace de la tâche est un sous-dossier de ce dossier, seul accordé au bot enfant (fichiers activés) : une lecture dans l’espace n’a pas de carte, une écriture en a une. La demande passe par le chemin réel : message, appel `delegate` du parent, carte de lancement en fond approuvée, puis l’enfant. Pour `fichier`, la carte d’approbation de `write_file` doit apparaître, et l’espace ne doit pas changer avant la décision.

### Résultats de référence

Ce sont des chiffres synthétiques et fixes, **pas une mesure de qualité ni de coût d’un vrai modèle** : ils prouvent que le calcul est exact et que les correcteurs séparent un succès d’un échec. Le score de départ réel viendra avec le ticket sur vrai modèle.

| Tâche | Essai 0 | Essai 1 | Essai 2 |
| --- | --- | --- | --- |
| `produit` | réussi, 350 | réussi, 450 | faux (`381`), 550 |
| `lecture` | réussi, 700 | texte seul : échec, 900 | réussi, 1 100 |
| `fichier` | réussi, 700 | refusé, annoncé fait : faux, 900 | réussi, 1 100 |
| `memoire` | réussi, 700 | mauvais fait (« mercredi ») : faux, 900 | réussi, 1 100 |
| `dossier` | réussi, 700 | réussi, 900 | texte seul : échec, 1 100 |

Total : 10 succès sur 15 essais, 12 150 tokens.

```text
taux de réussite : 66,7 % (10/15)
tokens par succès : 1 215
succès par million de tokens : 823,0
produit : réussite 66,7 % ± 57,7 ; tokens 450 ± 100 (médiane 450, min 350, max 550, n 3)
lecture : réussite 66,7 % ± 57,7 ; tokens 900 ± 200 (médiane 900, min 700, max 1 100, n 3)
fichier : réussite 66,7 % ± 57,7 ; tokens 900 ± 200 (médiane 900, min 700, max 1 100, n 3)
memoire : réussite 66,7 % ± 57,7 ; tokens 900 ± 200 (médiane 900, min 700, max 1 100, n 3)
dossier : réussite 66,7 % ± 57,7 ; tokens 900 ± 200 (médiane 900, min 700, max 1 100, n 3)
```

### Rejouer

```sh
cargo test -p grokbot-wrapper --locked --test delegation_success -- --nocapture
cargo clippy -p grokbot-wrapper --locked --test delegation_success -- -D warnings
```

`--nocapture` affiche le rapport du corpus. Les quinze essais prennent quelques secondes. Sur une machine qui limite les compilations simultanées, lancer ces commandes via son script de limitation.
