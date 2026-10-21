# Réponses de chat réellement liées à OpenAI

Branche `cloud/gb-chat-openai-reel`, depuis `ffcd7e3` (`fix(chat): fiabiliser les réponses, l'arrêt et la reprise de connexion`).

## Chemin d'un message

1. L'interface macOS envoie `POST /api/send` (`app/ui/app.js`).
2. `crates/wrapper/src/features/engine.rs` enregistre le message, choisit `live` (`connected()` ou absence du mode simulation), puis `launch`.
3. `RunGatewayCall::stream` reconstruit l'historique (`history`) et le contrat d'instructions, puis appelle `ProviderGateway`.
4. `Provider::stream_run` aplatit le contenu utilisateur et poste `POST {api}/responses` en SSE (`store: false`, donc l'historique repart à chaque appel).
5. `gateway::stream_response` transforme les événements en `TextDelta`. Le texte affiché est la concaténation de ces deltas pour la réponse terminée (`projected_parts`).
6. Le mode par défaut est Codex : `https://chatgpt.com/backend-api/codex/responses`. Le partage d'abonnement utilise `https://api.openai.com/v1/responses`.

`GROKBOT_FAKE_OPENAI` n'existe que dans un binaire de debug et uniquement vers `http://127.0.0.1`. Le binaire release (`tools/build-app.sh`, `EZERBOT_RELEASE=1`) n'a pas `debug_assertions` : `simulation_allowed()` est faux, et un envoi sans jeton échoue avec « Connectez votre compte ChatGPT… » sans texte de démo.

## Causes

Les lignes ci-dessous sont celles de `ffcd7e3`, avant correction.

### 1. Le flux SSE mélange le commentaire Codex et la réponse

`crates/wrapper/src/gateway.rs:198-201` émettait chaque `response.output_text.delta`, sans regarder `phase`. Un item `commentary` (brouillon, raisonnement visible) était collé au `final_answer`. Le texte affiché n'était plus la réponse du modèle. À l'inverse, `response.output_text.done` et le texte d'un `response.output_item.done` de type message étaient ignorés : un flux sans delta produisait une réponse vide.

Correctif `08fa0cd`. Le texte visible est la concaténation exacte des deltas qui ne sont pas `commentary`. Le texte `done` n'est émis qu'une fois, et seulement si l'item n'a pas eu de delta. Un flux uniquement `commentary` est affiché une fois à `response.completed`, pour ne pas rester vide. Les deltas sans phase (ancien flux) restent émis tout de suite.

Rouge : `commentary_phase_is_not_concatenated_into_the_visible_answer` produisait `je réfléchis` + `17 × 23 = 391.` ; `message_text_present_only_in_done_events_is_emitted_once` ne produisait que `Completed`. Vert : les deux, plus l'absence de doublon `Bon` + `jour` + `done`.

### 2. La sonde Codex était une requête invalide, avec un modèle retiré

`crates/wrapper/src/provider/codex.rs:385-386` imposait `tool_choice: auto` et `parallel_tool_calls: true` même sans outils. La sonde et les appels internes (titre) partaient dans cette forme. `crates/wrapper/src/provider/connection.rs:207-216` repliait un catalogue Codex vide sur `gpt-5.5`, refusé par un vrai compte ChatGPT (« The 'gpt-5.5' model is not supported when using Codex with a ChatGPT account. »). Le catalogue statique normal commence à `gpt-6.1-sol` ; le repli ne servait que si la liste était vide, mais il rendait toute vérification future fausse.

Correctif `482b9a2`. `tool_choice` n'est posé que si `tools` est un tableau non vide. Le repli Codex est `codex::DEFAULT_MODEL` (`gpt-6.1-sol`).

Rouge : `body_omits_tool_choice_when_the_call_has_no_tools` voyait `tool_choice: auto` ; `empty_codex_catalog_probes_the_supported_default_not_a_retired_model` obtenait `gpt-5.5`. Vert : les deux.

### 3. « Connecté » voulait dire « un jeton est sur le disque »

`crates/wrapper/src/provider.rs:605-634` (`Session::connected` / `Session::status`) et le badge `app/ui/app.js:1071` traitaient la présence d'un jeton comme une connexion à OpenAI. La sonde (`POST /api/provider/test`) était manuelle et n'empêchait pas le chat.

Correctif `fix(chat): n'afficher Connecté qu'après un vrai appel OpenAI`. `signed_in` reste la présence d'un jeton. `connected` dans le JSON de statut n'est vrai qu'après une réponse `Completed` de la sonde Responses. Le premier `status()` d'une session signée démarre cette sonde s'il est encore `idle` (un échec ne boucle pas ; le bouton « Tester la connexion » relance). `launch` appelle `ensure_service` avant tout worker de modèle : si OpenAI n'a pas confirmé, le tour est en échec, sans delta et sans « Bien reçu ».

Le badge n'affiche « Codex » ou « ChatGPT » que lorsque `connected` est vrai. Jeton sans confirmation : « Vérification » pendant l'appel, sinon « Non connecté », avec le texte « Aucune réponse n'est produite ». Sans jeton, le mode développeur reste étiqueté « réponses simulées » ; un binaire release ne simule pas.

Rouge : `connected_requires_a_real_openai_call_and_a_failed_probe_never_replies` lisait `signed_in == null` (le champ n'existait pas) et `connected == true` dès le jeton. Vert : `signed_in` vrai et `connected` faux tant que la sonde est retenue ; puis `connected` vrai ; le corps du tour contient la question et pas la sonde ; avec `fail_probe`, le message d'échec contient « Aucune réponse n'est produite », le corps est vide, et aucun corps de chat n'est envoyé.

## Ce qui n'était pas le défaut

- L'historique et la question partent dans `input` (`engine.rs` `history`, `gateway.rs` `request_body`). Le test Codex `first_simple_question_uses_supported_default_and_persists_answer_text` exige déjà « Combien font 17 × 23 ? » dans le corps, modèle `gpt-6.1-sol`.
- `reply()` filtre `conversation_id` : la réponse d'un autre fil n'est pas projetée sur celui-ci.
- Le mode simulation de debug (`demo_reply`, `SimulatedGateway`) reste réservé aux tests sans compte. Il n'est pas présenté comme Codex.
- Le contenu assistant sur le fil reste une chaîne : Responses refuse `input_text` pour un message assistant.

## Protocole sur le Mac de Charles

Build release de cette branche, avec son compte ChatGPT habituel. Pas de `GROKBOT_FAKE_OPENAI`.

1. Lancer Ezerbot. Tant que le badge n'est pas « Codex » (ou « ChatGPT » si l'ancien partage est choisi), aucun envoi ne doit produire de phrase. Le libellé est « Vérification », puis « Codex » seulement après un appel réussi. S'il reste « Non connecté », le bouton « Tester la connexion » doit rester disponible et le fil doit expliquer qu'aucune réponse n'est produite.
2. Dans les logs réseau (proxy local ou journal de l'app, `logs/engine.log` à côté des données), le premier appel utile est `POST https://chatgpt.com/backend-api/codex/responses` avec `instructions` qui commence par `Test de connexion.` Le corps n'a pas `tool_choice`. `model` est un slug du catalogue (`gpt-6.1-sol` par défaut), jamais `gpt-5.5` ni `gpt-5.3-codex-spark`. La réponse SSE se termine par `response.completed`. Le badge passe alors à « Codex ». `logs/engine.log` peut contenir `upstream turn=… status=200`.
3. Nouveau fil. Envoyer exactement : `Combien font 17 × 23 ? Réponds seulement par le nombre.`
   - Réponse attendue : `391` (espaces ou gras autour du nombre acceptables ; pas de brouillon du type « je réfléchis » collé devant).
   - Réseau : un second `POST …/codex/responses`. Dans `input`, le dernier message utilisateur contient la question. `model` est le slug choisi. Les événements `response.output_text.delta` affichés ont `phase` absent ou `final_answer`. Un item `phase: commentary` ne doit pas apparaître dans la bulle.
4. Deuxième message dans le même fil : `Ajoute 9. Réponds seulement par le nombre.`
   - Réponse attendue : `400`.
   - Le corps `input` de ce POST contient à la fois la première question, la réponse `391` et « Ajoute 9 ». `store` vaut `false`.
5. Troisième message : `Quel est le nombre premier qui suit 10 ? Réponds seulement par le nombre.`
   - Réponse attendue : `11`.
6. Si une bulle contient un texte qui ne répond pas à la question, copier le SSE brut du POST correspondant (types d'événements, `item_id`, `phase`, deltas) et le modèle du corps. Ne pas juger la connexion au seul badge si l'étape 2 n'a pas montré un `response.completed`.
