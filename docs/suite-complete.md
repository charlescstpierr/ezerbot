# Suite complète et contrôle de fusion

## Exécution nocturne Linux

Le workflow `suite-complete-linux.yml` démarre chaque jour à 07:23 UTC et sur
`workflow_dispatch`. Le job `suite-complete-linux` teste le SHA immuable de
l'événement. Un lancement manuel sur une branche vérifie cette branche. Il ne
constitue pas une preuve pour `main`.

Aucun runner auto-hébergé n'était enregistré au relevé du 25 octobre 2026.
Le workflow utilise donc Ubuntu 24.04 hébergé par GitHub. Chaque passe restaure
les seuls dossiers de compilation dans `target/debug`. Aucun dossier utilisateur,
journal de test ou fichier de données ne fait partie du cache. Les fixtures Node
cherchent le binaire dans `target/debug`, ce qui impose `CARGO_TARGET_DIR=target`.
Le runner limite Cargo à trois tâches, désactive sccache et refuse une compilation
avec moins de 20 Gio libres. Une nouvelle exécution attend alors de disposer
d'assez de disque.

Le runner Python exécute, dans cet ordre, les commandes suivantes. Il poursuit
les phases de tests après un échec et ne relance aucun test.

```sh
npm ci --prefix app/web --no-audit --no-fund
cargo test --workspace --locked --no-fail-fast -- --skip a_command_that_reaches_the_data_without_naming_it_reads_nothing --exact
node --test --test-reporter=tap app/web/src/*.test.js
node --test --test-reporter=tap app/ui/*.test.cjs app/ui/panels/*.test.cjs
```

L'exclusion exacte du test YOLO est autorisée par le brief de ce ticket et reste
visible dans le résultat. [#528](https://github.com/charlescstpierr/ezerbot/issues/528)
décrit l'absence de bac à sable Linux. Le test reste dans le dépôt. Le test voisin
`the_keychain_is_refused_by_the_kernel_however_the_command_is_spelled` est déjà
absent sous Linux par `cfg`. Sur Mac, son retour anticipé faute de trousseau lisible
reste une exclusion de couverture connue liée à #528. Un succès de ce test ne
prouve donc pas, à lui seul, le refus du trousseau.

Le test vocal `french_audio_recognized_on_device_after_english` est naturellement
absent sous Linux. L'exclusion macOS de la PR #355 n'ajoute donc aucun filtre ici.
La commande hebdomadaire sur le Mac conserve ce test, avec la permission Speech.
Aucune exemption ne concerne [#376](https://github.com/charlescstpierr/ezerbot/issues/376)
ou [#387](https://github.com/charlescstpierr/ezerbot/issues/387). #376 reste ouvert
au relevé initial. #387 est fermé. Tout autre échec rend la suite rouge.

## Inventaire généré des différences de plateforme

`tools/suite-platform-inventory.py` lit les sources Rust suivies par Git au SHA
demandé. Il distingue les attributs de fichier, de module, de test et de bloc,
en ignorant les commentaires et les littéraux. Une forme conditionnelle non
prise en charge provoque une erreur plutôt qu'un inventaire incomplet.

```sh
python3 tools/suite-platform-inventory.py --ref 884d8cd
python3 tools/suite-platform-inventory.py --ref origin/main
```

La référence générée est `docs/suite-platform-baseline.json`. Au commit historique
`884d8cd` et au commit `2ef1a02333a0516804e6899321a868c88874740b`, elle donne les mêmes
28 tests Rust absents et les mêmes 2 tests avec blocs `#[cfg]` partiels.
Le relevé de l'issue comptait 27 tests. Il omettait le test de
`crates/forms/src/vault/keychain.rs`, dont la condition vient du `mod keychain`
dans `crates/forms/src/vault.rs`. Ce test était déjà absent au commit historique.

| Fichier | Tests absents |
| --- | ---: |
| `crates/wrapper/tests/keychain.rs` | 9 |
| `crates/wrapper/tests/routines_background.rs` | 9 |
| `crates/wrapper/tests/connectors.rs` | 3 |
| `crates/wrapper/tests/yolo.rs` | 2 |
| `crates/attachments/src/document_tests.rs` | 2 |
| `crates/forms/src/vault/keychain.rs` | 1 |
| `crates/wrapper/tests/library.rs` | 1 |
| `crates/wrapper/tests/voice_ui.rs` | 1 |

Les deux tests partiels sont `generation::tests::limits_and_errors_are_explicit`
et `standing_rule_is_honoured_and_internal_data_writes_are_rejected` dans
`crates/wrapper/tests/generated.rs`.
Deux autres tests suivent un chemin PDF différent via `cfg!`, déjà au commit
historique. Le rapport les distingue dans `runtime_branches`.
Il s'agit de `generation::tests::all_formats_are_readable_with_french_accents`
et de `all_formats_are_persistent_valid_attachments_and_counted_by_storage`
dans `crates/wrapper/tests/generated.rs`.

Chaque passe recalcule les listes. Un ajout, retrait ou renommage apparaît dans
`inventory_delta` et rend le résultat rouge jusqu'à la révision de la référence.
Les deux tests Node `native-compile.test.js` et `signing-scripts.test.js` déclarent
aussi un saut sous Linux. Ils comptent dans `ignored`, séparément des 28 absences
Rust. Un saut Node supplémentaire ou un test Rust ignoré rend le résultat rouge.

## Contrat du résultat

Le seul artefact publié contient `result.json`. Son nom est
`suite-complete-linux-<run_id>-<run_attempt>`, avec une conservation de 14 jours.
Les journaux bruts restent privés au runner et ne sont jamais publiés.
Les noms d'échecs sont des identifiants Rust validés ou des emplacements de tests
Node relatifs au dépôt. Le résultat ne copie ni diagnostic ni titre Node dynamique.

Les champs du JSON version 1 sont les suivants.

| Champ | Type et sens |
| --- | --- |
| `schema_version` | Entier `1`. |
| `context` | `suite-complete-linux` ou `suite-macos-hebdo`. |
| `sha` | SHA Git complet du code testé. |
| `run_id`, `run_attempt` | Chaînes GitHub Actions, vides pour une passe locale. |
| `started_at`, `finished_at` | Horodatages UTC RFC 3339. `finished_at` vaut `null` tant que la passe n'est pas terminée. |
| `status` | `vert` ou `rouge`. |
| `counts` | Entiers `passed`, `failed`, `ignored`, `absent_linux`, `partial_linux`, `runtime_branches_linux`, `excluded`. |
| `failures` | Tableau des identifiants d'échecs et des erreurs de phases. |
| `known_exclusions` | Tableau d'objets `{"issue":528,"platform":"linux","test":"a_command_that_reaches_the_data_without_naming_it_reads_nothing"}`. Vide sur Mac. |
| `platform_inventory` | Tableaux `absent`, `partial`, `runtime_branches`, `ignored`, avec des identifiants `chemin::module::test`. |
| `inventory_delta` | Objets `absent`, `partial`, `runtime_branches`, chacun avec les tableaux `added` et `removed`. |
| `phases` | Objets `npm`, `rust`, `web`, `ui`. `exit_code` est un entier ou `null` si la commande n'a pas démarré. Les entiers `passed`, `failed`, `ignored` comptent ses tests. |

Les compteurs additionnent les résultats des phases exécutées. Ils ne mesurent
pas des scénarios uniques, car un test Rust peut lancer un test Node.
Une compilation ratée peut avoir `failed=0` et `status=rouge`.
Le code de sortie du runner est `0` uniquement pour `vert`.
Une erreur de préparation, un résumé absent, une modification suivie du dépôt
ou de `Cargo.lock`, ou un inventaire invalide empêche le vert.

Le signal GitHub est le **check Actions natif** `suite-complete-linux`, associé
au SHA testé. Ce n'est pas un statut REST dans `/commits/<sha>/status`.
Créer ce dernier demanderait `statuses: write`. Le workflow conserve uniquement
`contents: read`, sans secret configuré. Les permissions et les réponses des
API sont décrites dans la [syntaxe des workflows](https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-syntax)
et la [référence des exécutions Actions](https://docs.github.com/en/rest/actions/workflow-runs).

## Règle de refus pour pret

`pret` reste hors dépôt. Son intégration doit appliquer toutes les conditions
suivantes avant une fusion.

1. Lire la dernière exécution du workflow `suite-complete-linux.yml` sur `main`,
   parmi les événements `schedule` et `workflow_dispatch`, sans filtrer les succès.
   Utiliser `GET /repos/charlescstpierr/ezerbot/actions/workflows/suite-complete-linux.yml/runs?branch=main`.
   Considérer la dernière tentative de cette exécution.
2. Refuser si elle est en attente, en cours, annulée, expirée ou en échec.
   Ne jamais remplacer une dernière exécution rouge par un succès plus ancien.
3. Télécharger son artefact exact pour cette tentative. Refuser un artefact
   absent, expiré, invalide ou dont `schema_version` n'est pas `1`.
4. Vérifier `context`, `run_id`, `run_attempt`, et `sha == head_sha` de l'exécution.
   Vérifier que le SHA testé appartient à l'historique actuel de `main`.
   Les lancements sur d'autres branches ne satisfont jamais cette règle.
5. Exiger `status=vert`, `conclusion=success` de l'exécution et du job
   `suite-complete-linux`, `failures=[]` et aucun écart d'inventaire.
6. Calculer l'âge depuis `started_at`, avec `started_at <= finished_at <= maintenant`.
   Refuser si l'âge dépasse 86 400 secondes. Cette règle inclut la durée des tests
   et accepte exactement 24 heures. Ne pas mesurer l'âge depuis le téléchargement.

Le refus nomme le SHA, l'âge en heures et les identifiants de `failures`.
Si le rapport manque, il nomme aussi la conclusion GitHub et l'absence du rapport.
Un `main` qui avance depuis la nuit ne nécessite pas un SHA identique, mais le SHA
testé doit rester son ancêtre. La suite nocturne contrôle la santé récente de
`main`. La CI par PR reste la preuve du changement proposé.
Le refus strict ne peut être déclaré installé par cette PR. L'intégration de
`pret` et la résolution de #376 restent à faire du côté de leurs propriétaires.

## Passe hebdomadaire sur le Mac

Sur le Mac connecté à la session de Charles, installer le navigateur et accorder
la permission Speech au test de dictée. La commande unique lance la suite complète,
y compris les 28 tests absents sous Linux et toutes les assertions partielles.

```sh
CARGO_BUILD_JOBS=3 CARGO_TARGET_DIR=target RUSTC_WRAPPER= python3 tools/run-complete-suite.py --macos --output "$HOME/Library/Logs/Ezerbot/suite-macos-hebdo/result.json"
```

Le mode Mac n'exclut aucun test et produit le même contrat avec
`context=suite-macos-hebdo`. Il ne publie rien sur GitHub.
`pret` peut lire ce fichier local et signaler un rouge ou un âge supérieur à
691 200 secondes, soit 8 jours, sans bloquer au départ. La publication éventuelle
d'un statut GitHub macOS reste hors de cette PR, avec les mêmes contraintes
de permissions que le statut Linux.
