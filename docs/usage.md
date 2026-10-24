# Usage et quotas

Dans **Paramètres → Usage**, Ezerbot distingue deux mesures :

- **Quota du forfait Codex** : lu chez OpenAI avec la session OAuth créée dans Ezerbot. Les fenêtres fournies par le forfait (par exemple 5 heures et une semaine) affichent le pourcentage restant et la réinitialisation en heure locale. Un avertissement apparaît dès **80 % utilisés**, dans le panneau et au-dessus du chat.
- **Tokens locaux** : entrée, sortie et total, par bot ou par source (chat, routine, groupe, délégation, entre bots). Les tokens en cache font déjà partie de l’entrée. Un enfant délégué n’est pas recompté dans son parent.

Le quota du forfait est partagé avec les autres applications Codex. Les tokens locaux ne permettent donc pas de le calculer exactement : ils servent seulement d'étalon pour l'estimation du coût d'une routine (ci-dessous). L’historique local couvre toutes les connexions utilisées dans cet espace, y compris les bots supprimés. Il compte les réponses principales du moteur, pas les appels auxiliaires de titres, mémoire automatique, vision ou résumés. Les exécutions sans télémétrie sont signalées comme non mesurées, jamais estimées.

## Coût d'une routine

`GET /api/routines` et l'état des routines renvoient `routine.cost`, en tokens et jamais en prix : moyenne des 10 derniers passages terminés et mesurés, total des 7 derniers jours (sous-tâches déléguées comprises, chacune une fois), prévision de l'horaire sur 7 jours (`forecast`, « impossible » pour GitHub, courriel, calendrier ou webhook) et part du quota hebdomadaire (`quota_share`). Cette part est une estimation : tokens mesurés ici depuis le début de la fenêtre de 7 jours ÷ son `used_percent`. Elle n'est donnée que si au moins 5 % du quota sont utilisés, 50 000 tokens mesurés et si le même compte Codex est connecté depuis le début de la fenêtre (Ezerbot note le compte qu'il voit à chaque cycle et à chaque requête envoyée au modèle, y compris celle qui suit un renouvellement de session ; un changement de compte, ou la première lecture après la mise à jour, rend la part « inconnue » jusqu'à la prochaine fenêtre) ; sinon `state` vaut `unknown`, avec sa raison, et seuls les tokens restent. Les autres applications Codex et les appels auxiliaires ne sont pas comptés ici : la valeur est approximative. La prévision suit le planificateur (jours ouvrés, changements d'heure, groupes) ; une routine en pause n'a aucun passage prévu.

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

## Évaluation sur faux serveur (opt-in)

Le [lanceur d’évaluation — étape A](evaluation.md) rejoue un scénario de messages dans un espace neuf marqué, impose `EZERBOT_EVAL_MAX_CALLS` et écrit uniquement des chiffres dans `target/eval/`. Il ne se connecte à aucun compte réel.

## Test de dictée Mac

Le test `french_audio_recognized_on_device_after_english` (crates/wrapper/tests/voice_ui.rs) tourne dans `cargo test` : il exige un Mac, les locales en-US/fr-CA installées et la permission Speech, accordée une fois au binaire de test `~/.local/share/ezerbot/dictation-fixture-test/ezerbot-dictation-fixture-test`. `tools/test-dictation-fixture.sh` l'installe, signé, et ne le recompile que si ses sources changent (une recompilation redemande la permission). Le premier lancement affiche l'invite macOS ; sans réponse en 45 s, ou si l'accès est refusé, le test échoue avec la marche à suivre. Seul ce test :

```sh
cargo test --locked -p grokbot-wrapper --test voice_ui french_audio_recognized_on_device_after_english -- --exact
```

`tools/test-dictation-fixture.sh --status` affiche l'état de la permission sans la demander.
