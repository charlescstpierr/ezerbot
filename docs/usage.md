# Usage et quotas

Dans **Paramètres → Usage**, Ezerbot distingue deux mesures :

- **Quota du forfait Codex** : lu chez OpenAI avec la session OAuth créée dans Ezerbot. Les fenêtres fournies par le forfait (par exemple 5 heures et une semaine) affichent le pourcentage restant et la réinitialisation en heure locale. Un avertissement apparaît dès **80 % utilisés**, dans le panneau et au-dessus du chat.
- **Tokens locaux** : entrée, sortie et total, par bot ou par source (chat, routine, groupe, délégation, entre bots). Les tokens en cache font déjà partie de l’entrée. Un enfant délégué n’est pas recompté dans son parent.

Le quota du forfait est partagé avec les autres applications Codex. Les tokens locaux ne permettent donc pas de le calculer. L’historique local couvre toutes les connexions utilisées dans cet espace, y compris les bots supprimés. Il compte les réponses principales du moteur, pas les appels auxiliaires de titres, mémoire automatique, vision ou résumés. Les exécutions sans télémétrie sont signalées comme non mesurées, jamais estimées.

## Coût d'une routine

`GET /api/routines` et l'état des routines renvoient `routine.cost`, en tokens et jamais en prix : moyenne des 10 derniers passages terminés et mesurés, total des 7 derniers jours (sous-tâches déléguées comprises, chacune une fois) et prévision de l'horaire sur 7 jours (`forecast`, « impossible » pour GitHub, courriel, calendrier ou webhook). La prévision suit le planificateur (jours ouvrés, changements d'heure, groupes) ; une routine en pause n'a aucun passage prévu. La part du quota hebdomadaire (`quota_share`) vaut toujours `{"state":"unknown","reason":"quota_not_attributed"}`, sans nombre : les tokens locaux peuvent venir d'un autre compte Codex que celui dont le quota est lu, tant que chaque réponse ne note pas son compte.

## Lecture et confidentialité

`GET /backend-api/wham/usage` utilise le Bearer et l’en-tête `chatgpt-account-id` de la session Codex propre à Ezerbot. Aucun fichier d’identifiants de pi, de Codex CLI ou d’un autre outil n’est lu. Le transport ne suit pas les redirections ; toute réponse hors 2xx est une erreur. Seuls les champs validés du quota atteignent l’interface ; ni jeton, ni corps d’erreur brut ne sont exposés.

La lecture est faite en arrière-plan, coalescée et mise en cache 60 secondes. **Actualiser** peut la relancer après 5 secondes. Une réponse 401 entraîne au plus un renouvellement et une nouvelle lecture. Une déconnexion invalide les lectures en vol. Après une erreur, aucune jauge fictive n’est affichée. Une fenêtre dont la réinitialisation est échue devient seule indisponible (`null`, listée dans `quota.unavailable`) ; les autres restent affichées. Si la dernière lecture précède cette réinitialisation, le quota est relu après 5 secondes. Le partage d’abonnement, sans connexion Codex sélectionnée, indique que la jauge n’est pas disponible.

Panneau ouvert, un changement de backend, de connexion ou d’e-mail recharge l’historique. Toutes les 60 secondes, le panneau relit seulement l’usage, sans repeindre l’espace ni ce qui n’a pas changé ; cette lecture est sautée pendant un envoi ou quand la fenêtre est masquée. **Actualiser** pressé pendant un chargement est mis en file et relancé une fois à la fin.

API locale : `GET /api/usage` et `POST /api/usage/refresh`. La première renvoie la ventilation durable et le dernier état du quota, sans attendre le réseau OpenAI.

## Vérification sans compte externe

```sh
cargo test -p grokbot-wrapper --test usage --test codex --test quota --test gateway --test http
node --test app/ui/panels/usage.test.cjs
cargo clippy --workspace --all-targets -- -D warnings
```

Les tests utilisent uniquement de faux serveurs et des sessions éphémères propres à Ezerbot. Ils couvrent OAuth/renouvellement, absence de fuite d’identifiants, validation des fenêtres, erreurs, déconnexion pendant une lecture, déduplication des tokens, sources et conservation après suppression/redémarrage. Les tests UI couvrent aussi la borne de 80 %, l’actualisation du quota quand la base locale reste inactive, l’intervalle de 60 secondes, le saut pendant un envoi, la mise en file d’**Actualiser** et le rechargement après un changement de compte.

## Taux de cache de prompt : rejeu hors ligne

```sh
python3 tools/prompt_cache.py replay --output /tmp/ezerbot-prompt-cache.json
python3 -m unittest discover -s tools -p test_prompt_cache.py
cargo test --locked -p grokbot-wrapper --test prompt_cache --test prompt_cache_replay
```

Le script exige Python 3 et Cargo. Il lance uniquement le test de rejeu sur
`FakeOpenAi` (serveur en boucle locale), avec une base et une session factices
éphémères, sans compte externe ni lancement de l'application installée. Deux fils
font chacun cinq tours : suivi ordinaire et changement de mémoire au troisième
tour. Le préfixe, l'ordre des outils et la clé de cache ne sont pas modifiés.

Le JSON et la sortie standard contiennent le rapport global, par conversation
(numérotée, sans identifiant brut), par tour et par réponse fournisseur :

- `measured` = somme des tokens en cache ÷ somme des tokens d'entrée, **hors première
  réponse enregistrée de chaque conversation** (la plus ancienne ligne de
  `engine_response_usage`, pas le premier tour entier si un tour fait plusieurs
  appels). Ce n'est pas la moyenne des pourcentages ;
- `cacheable` applique en plus le seuil d'entrée **≥ 1 024** ; `below_1024` donne
  séparément les réponses mesurées sous ce seuil. Chaque réponse porte aussi son
  indicateur `below_1024`, y compris la première, exclue du calcul ;
- `excluded_first` identifie l'échauffement. Un dénominateur nul produit `null`,
  jamais un taux inventé. Les runs sans télémétrie sont comptés dans
  `unmeasured_runs`, sans remplacer les tokens absents par zéro ; si le premier run
  d'une conversation n'a aucune télémétrie, c'est la première réponse enregistrée
  ensuite qui est exclue ;
- `compaction_count` compte les résumés durables dans `engine_summaries`, globalement
  et par conversation (même s'ils ont été invalidés depuis). Une compaction peut
  changer le préfixe et baisser le taux ; le rejeu court n'en déclenche normalement
  pas. Ce compteur ne date pas la compaction à un tour précis.

La lecture utilise `engine_response_usage` et l'ordre d'insertion des runs, puis
l'ordinal de réponse fournisseur dans chaque run. Elle ne lit ni les messages, ni
les enregistrements de runs, ni les textes des résumés. Le fichier de résultats ne
contient aucun texte de message, secret ou identifiant de conversation brut.
Les appels auxiliaires (titre, mémoire automatique, résumé) n'entrent pas dans le
dénominateur. Pour relire une base de rejeu `FakeOpenAi` conservée séparément,
sans réseau (le rapport est toujours marqué `synthetic: true`) :

```sh
python3 tools/prompt_cache.py report --db /tmp/replay.sqlite --output /tmp/cache.json
```

**Ces taux sont synthétiques.** Le faux serveur renvoie des tokens fixes
(11 008 en cache sur 12 340 en entrée), indépendamment du contenu et de la
compaction. Ils valident le calcul, pas le vrai taux de cache ; les repères 70 %
(« bon ») et > 90 % (« excellent ») ne sont pas des critères de sortie ici.
Le rejeu réel et les essais d'ordre du préfixe restent un travail séparé.

## Erreur d'estimation du contexte : mesure hors ligne

À chaque réponse du modèle, `engine_response_usage` consigne trois entiers à côté
des tokens fournisseur, sans aucun texte :

- `context_characters` : le nombre exact que le déclencheur de compaction compare au
  seuil pour cet envoi (historique éligible, instructions, texte du résumé en
  vigueur). C'est la même valeur, calculée une seule fois ;
- `characters_per_token` : le réglage utilisé pour passer des caractères aux tokens
  (4 aujourd'hui) ;
- `tool_characters` : la taille JSON des définitions d'outils envoyées avec la
  requête. **Elles ne sont pas comptées** dans `context_characters`.

Les anciennes lignes restent à `NULL` (inconnu, jamais zéro), comme une réponse sans
télémétrie. Les tokens de l'appel de résumé ne sont pas dans cette table. Les envois
des conversations de groupe, qui ne se compactent jamais, ont eux aussi un nombre.

```sh
python3 tools/token_estimate.py replay --output /tmp/ezerbot-token-estimate.json
python3 tools/token_estimate.py replay --counter o200k --output /tmp/ezerbot-token-estimate.json
python3 tools/token_estimate.py report --db /tmp/replay.sqlite --output /tmp/estimate.json
python3 -m unittest discover -s tools -p test_token_estimate.py
```

`replay` lance le test `token_estimate_replay` sur `FakeOpenAi` (six conversations
de longueurs différentes, dont une compactée au budget minimum et une de groupe). Sans `--counter`,
`input_tokens` suit une formule fixe du test. Avec `--counter o200k`, un compteur
`tiktoken` (`pip install tiktoken`) le calcule à partir du corps de chaque requête :
c'est un proxy du tokenizer fournisseur, pas le fournisseur lui-même.

Le rapport donne, par conversation (numérotée, sans identifiant brut) et globalement,
l'erreur signée `(tokens estimés - tokens réels) ÷ tokens réels` avec
`n`, `mean`, `median`, `pstdev`, `min`, `max`, `worst` (l'erreur de plus grande valeur
absolue, signée) et `underestimates`. Un nombre négatif est une sous-estimation, le
côté dangereux. `signed_error_with_tools` refait le calcul en ajoutant
`tool_characters`. `tool_definitions_counted` vaut `false` et `notice` l'explique.
Les lignes sans estimation ou à zéro token d'entrée vont dans `unmeasured_responses`.
Le fichier ne contient ni texte de message, ni secret, ni identifiant brut.

**Ces erreurs sont synthétiques** : `input_tokens` vient d'un compteur de test, pas
du fournisseur. Elles mesurent l'écart de l'estimation, sans régler
`characters_per_token` ni `compaction_percent`.

## Journal des tours

Chaque tour terminé laisse une ligne JSON dans la table `wrapper_turn_log` (`run_id`, `at_ms`, `entry`) de la base de l’espace ; rien ne quitte le Mac. Un tour n’est écrit qu’une fois : sa première ligne reste, même s’il est arrêté ou régénéré ensuite.

Par tour : `at_ms`, `bot_id`, `source` (chat, routine, group, delegation, bot_to_bot, comme `/api/usage`), `turn` (empreinte de 16 caractères, la même que `turn=` dans `logs/engine.log`), `model`, `effort`, `status`, `error_kind`, `duration_ms`, `model_calls`, `tool_calls`, `tokens` (`input`, `output`, `cached`, ou `null` si le fournisseur n’a rien mesuré) et `tools`.

Par appel d’outil, dans l’ordre : `name`, `started_ms`, `duration_ms`, `result` et `code`. La durée va de l’entrée de l’appel à son résultat ; un appel suspendu pour approbation est chronométré à sa reprise, sans l’attente humaine. Un appel jamais exécuté, repris après redémarrage ou d’effet incertain n’a ni début ni durée (`null`). Un nom venu du modèle (`mcp_<connecteur>_<outil>` pour un outil MCP) n’est gardé que s’il tient en 64 octets de `A-Z a-z 0-9 _ . -` et si l’appel a été validé ; sinon, et pour `unknown_tool` et `not_executed`, il devient `-`. Un résultat d’avant le journal, sans `code`, est déduit de son statut (`ok` ou `failed`) et son nom devient `-`. Un tour arrêté en attente d’approbation est journalisé à l’arrêt ; une tâche déléguée suspendue puis annulée avec son parent ne l’est pas.

| `code` | `result` |
| --- | --- |
| `ok` | `ok` |
| `failed`, `uncertain` | `erreur` |
| `invalid_arguments`, `unknown_tool`, `follow_up_pending`, `approval_pending` | `refusé` |
| `approval_denied` | `approbation refusée` |
| `not_executed` | `annulé` |

**Jamais dans le journal** : texte de message, argument, sortie ou raison d’un outil, secret du coffre, jeton, en-tête, `request_id`, titre de conversation. Que des compteurs, des codes fixes et des identifiants opaques.

**Bornes.** 64 appels détaillés par tour au plus (`tool_calls` garde le vrai total), 20 000 tours au plus (les plus anciens partent), 30 jours : les lignes plus vieilles sont supprimées au démarrage puis une fois par jour, sans toucher aux autres tables. Une écriture qui échoue ne change pas l’issue du tour.

**Résumé d’une journée**, en heure locale du Mac (minuit à minuit, changements d’heure compris), par la route ou la commande, qui renvoient le même JSON ; un jour qui n’est pas `AAAA-MM-JJ` donne une erreur 400 (code de sortie 1 en ligne de commande) :

```sh
curl -s 'http://127.0.0.1:<port>/api/turns/summary?day=2026-10-24'
grokbot-wrapper --turns-summary 2026-10-24 [--db chemin.sqlite]
```

`turns` ; `tokens` (`input`, `output`, `cached` sur les tours mesurés, `unmeasured_turns` pour les autres, jamais comptés comme zéro) ; `most_used` (`name`, `calls`) ; `slowest` (`name`, `calls` chronométrés, `median_ms`, `p95_ms`, du plus lent au plus rapide selon `p95_ms`) ; `errors` (`name`, `code`, `count` pour tout résultat autre que `ok`). Médiane et p95 se calculent au rang le plus proche : la valeur de rang `ceil(n × p ÷ 100)` sur `n` durées triées. Les égalités se départagent par nom, puis par code.

## Évaluation sur faux serveur (opt-in)

Le [lanceur d’évaluation — étape A](evaluation.md) rejoue un scénario de messages dans un espace neuf marqué, impose un plafond d’appels et écrit uniquement des chiffres dans `target/eval/`. Il ne se connecte à aucun compte réel.

## Test de dictée Mac

Le test `french_audio_recognized_on_device_after_english` (crates/wrapper/tests/voice_ui.rs) tourne dans `cargo test` : il exige un Mac, les locales en-US/fr-CA installées et la permission Speech, accordée une fois au binaire de test `~/.local/share/ezerbot/dictation-fixture-test/ezerbot-dictation-fixture-test`. `tools/test-dictation-fixture.sh` l'installe, signé, et ne le recompile que si ses sources changent (une recompilation redemande la permission). Le premier lancement affiche l'invite macOS ; sans réponse en 45 s, ou si l'accès est refusé, le test échoue avec la marche à suivre. Seul ce test :

```sh
cargo test --locked -p grokbot-wrapper --test voice_ui french_audio_recognized_on_device_after_english -- --exact
```

`tools/test-dictation-fixture.sh --status` affiche l'état de la permission sans la demander.
