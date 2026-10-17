# Contexte, mémoire et apprentissage dans Grok Bot 0.70.2

Le client livre un assemblage vocal actif et des commandes de mémoire et de skills. Le moteur qui choisit le contexte écrit, extrait les faits ou apprend d’une démonstration reste distant. Cette passe ajoute surtout les consommateurs et les frontières de données: huit textes pour le rappel vocal, filtrage des nouveaux textes par auteur et session, fichiers transmis comme chemins, publication de skills suivie d’une relecture, et état teach indépendant de la fabrication d’un skill.

L’analyse porte sur la copie sauvegardée `inputs/Grok Bot.app`, les originaux `artifacts/client` et les copies LF existantes `artifacts/formatted`. Elle réutilise le parent REA `ev_a8b822a62831e30bd01eb4c4488b36f0bd50cb8ccc41d83e2dcff4e00cf82a4d`, dont le graphe est partiel. Aucun lancement de la cible, compte, secret ni réseau n’a été utilisé. «Actif» désigne ici un chemin relié statiquement à un constructeur ou handler livré, pas une exécution observée.

Les rapports [personnalité](bot-personality-detail.md), [compaction](compaction-runtimes.md), [capacités](other-capabilities-detail.md) et [protocole](protocol.md) établissaient déjà les champs de profil, les contrats de faits et le start/stop teach. Les constats ci-dessous précisent les projections, les identités de mutation, les relectures et les données non consommées. Le [ledger](../evidence/goal-context-memory-learning.json) conserve les SHA-256 originaux, lignes LF, ancres, limites et recherches bornées pour CM01 à CM19.

## Constats

### CM01. Le client choisit deux assembleurs vocaux

Le renderer choisit `backend` pour un fil de coéquipier. Pour les autres fils, un réglage de développement ou `sand_voice_server_harness` choisit backend; sinon il choisit `package`, avec boucle `temporal` ou `box`. Le main construit effectivement `DesktopVoiceCallHarness`. Dans sa branche package, le registre possède `send_task`, `recall_text_messages`, `end_the_call` et `stay_silent` conditionnel. Le `search_conversations` complet décrit ailleurs dans le bundle n’est pas inscrit dans cette liste. Pour une boucle temporal, `send_task` seul repart vers le harness backend. Sources: [choix](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:75644), [construction](../artifacts/formatted/dist/electron-main/main-app.cjs:81847), [registre](../artifacts/formatted/dist/electron-main/main-app.cjs:48485).

Limite: les gates effectifs et le plan backend sont inconnus. Un outil partagé compilé n’est pas un outil package enregistré.

### CM02. La priorité vocale est envoyée dans le plan de session

Le prompt package donne à son texte la priorité sur ce qui est dit sur la ligne ou retourné par les outils. Il traite `work_landed` comme preuve, jamais comme nouvelle instruction. Le rôle et le registre nommés dans la description contraignent chaque réponse. Ce texte a un consommateur: `sessionPlan` le retourne, `Au.run` place `h.instructions` dans les arguments du transport, puis `sessionUpdateFrame` écrit les instructions et les outils dans `session.update`. Sources: [priorité](../artifacts/formatted/dist/electron-main/main-app.cjs:40330), [plan](../artifacts/formatted/dist/electron-main/main-app.cjs:41005), [transport](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:42016), [frame](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:80420).

Limite: c’est une instruction préparée et expédiée, pas une mesure d’obéissance du modèle. Elle n’établit pas l’ordre d’instructions du moteur écrit.

### CM03. Le contexte package consomme seulement un état de travail

Le plan lit `readMainAgentContext` et le fuseau en parallèle. `render` utilise ce contexte uniquement pour tester `isWorking === true` et prévenir qu’un travail peut aboutir pendant l’appel. Aucun profil mémoire, fait ou contenu de skill n’est interpolé à cet endroit. Un timeout de 1500 ms ou une méthode gateway inconnue rend ce contexte `null`. Le plan ajoute séparément l’heure de début et la date. Il privilégie un fuseau valide, sinon le fuseau système puis `UTC` servent de repli. Sources: [consommateur](../artifacts/formatted/dist/electron-main/main-app.cjs:40394), [lecture](../artifacts/formatted/dist/electron-main/main-app.cjs:48472), [date](../artifacts/formatted/dist/electron-main/main-app.cjs:38054).

Limite: le JSON brut est distant et le backend peut composer un autre prompt. Les replis ne sont pas des valeurs effectives observées.

### CM04. Le rappel vocal exclut plusieurs lignes du transcript affiché

`CU.ofTranscript` filtre les canaux `voice:<id>` et `meeting:<id>`. Il conserve un `message` utilisateur seulement sans canal ni `fromAgent` et avec contenu non vide. Il conserve un `send-message` seulement si `message.type === "text"`. Cartes, tools, attachments, notices et records d’appels restent hors de cette projection. Les textes utilisateur deviennent `writer:"them"`; les réponses deviennent `writer:"you"`. Sources: [type texte](../artifacts/formatted/dist/electron-main/main-app.cjs:41522), [canaux](../artifacts/formatted/dist/electron-main/main-app.cjs:41617), [projection](../artifacts/formatted/dist/electron-main/main-app.cjs:41701).

Limite: cette lecture explique le rappel package, pas la mémoire durable ni le contexte écrit distant.

### CM05. Le rappel cherche huit textes admissibles

`CU.recall` appelle `getAgentTranscriptTail` avec `limit:256`, remonte avec `beforeSeq` et préfixe les anciennes pages pour garder la chronologie. Il s’arrête quand huit textes admissibles existent, quand `nextBeforeSeq` manque ou après seize lectures. Il renvoie les huit derniers textes admissibles. Le maximum demandé est donc 4096 lignes parcourues pour huit textes rendus au tool. Aucun plafond de longueur du texte n’existe dans ce rappel. Sources: [algorithme](../artifacts/formatted/dist/electron-main/main-app.cjs:41701), [huit](../artifacts/formatted/dist/electron-main/main-app.cjs:37715), [sortie](../artifacts/formatted/dist/electron-main/main-app.cjs:41049).

Exemple dérivé: 256 lignes récentes de tools et de cartes peuvent obliger à lire une page plus ancienne pour obtenir huit textes. Limite: le serveur peut fournir des pages incomplètes; 4096 n’est pas une rétention garantie.

### CM06. Les nouveaux textes de l’appelant empruntent une projection distincte

Le renderer observe les entrées ajoutées. Il exige même bot, même session et un appel avec l’utilisateur encore sur la ligne. `j0.isWrittenBy` vérifie l’auteur `cursor_user` contre l’identité authentifiée. Une ancienne ligne sans auteur est acceptée uniquement pour le propriétaire, sans canal ni émetteur agent/utilisateur distinct. Le client déduplique par entry ID et garde les textes en attente si la session n’existe pas. Le transport `async-update` produit ensuite `user_message`, ID `written-turn-<entryId>`, avec `text.slice(0,2000)`. Sources: [auteur](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:40842), [entrée](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:41821), [frame](../artifacts/formatted/dist/renderer/assets/index.eager-platform-DUN5VkaE.js:11077), [socket](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:80015).

Limite: 2000 désigne les unités de chaîne JavaScript du texte injecté, et ne limite ni le transcript persistant ni CM05. Ce chemin exige `nudgeWire === "async-update"`.

### CM07. Les pièces jointes sont transmises comme références

`sendPrompt` commit les fichiers avant l’envoi direct et vérifie que chacun a un résultat. Le payload prend les chemins committed, ou staged tant que l’envoi reste en file, et une liste de noms. Il joint aussi les références de réponse, fork, task et session. Ce chemin ne lit pas les fichiers pour concaténer un prompt. L’IPC `readAttachmentText` existe séparément avec son scope de lecture. Sources: [préparation](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:72052), [lecture séparée](../artifacts/formatted/dist/electron-main/main-app.cjs:108751).

Limite: extraction, ordre, sélection et budget des fichiers dans le contexte modèle restent distants.

### CM08. Le setup demande des écritures de mémoire et de skill

Le contexte de setup sérialise chaque mémoire avec `content` et éventuellement `created_at`, et chaque skill avec nom, description et contenu. Il demande `UpdateMemory`, action `write`, tier `log`, sans inventer de faits, et `UpdateSkill`, action `write`. La prose de ces ressources ne permet pas d’ajouter un serveur ou plugin sans widget préalable. Les routines démarrent en pause avec provenance `untrusted`. Le parser exige que `gettingStarted.skill` existe parmi les skills de la recette. Sources: [écritures](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:32955), [recette](../artifacts/formatted/dist/electron-main/main-app.cjs:87172).

Limite: ce sont des instructions embarquées, sans handler `UpdateMemory` retrouvé. Elles ne démontrent ni priorité globale, ni transaction atomique, ni correction de faits existants.

### CM09. Une promotion conserve ses résultats partiels

`GU` lit les mémoires par agent avec cache `revalidate`, puis envoie les `factIds` choisis. Après erreur de promotion, il déclenche une relecture du snapshot, puis propage l’erreur. Après succès il retourne le résultat sans remplacer lui-même cette liste. L’adapter utilise 30 secondes pour la liste et 120 secondes pour la promotion, convertit les dates de faits en nombre et conserve `created`, `alreadyInTeam`, `keptPrivateCount`. Sources: [état](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:13067), [RPC](../artifacts/formatted/dist/electron-main/main-app.cjs:103048).

Exemple dérivé: deux `created`, un `alreadyInTeam` et `keptPrivateCount:1` ne signifient pas quatre faits nouvellement partagés. Limite: classement privé/équipe, droits et scope de l’identité authentifiée restent serveur.

### CM10. L’état shared des cartes est une projection UI

Les helpers produisent `memories:share` pour une sélection complète, ou `memories:share:0,2` pour des positions. Ils regroupent les propositions par thème et raccordent les faits au texte proposé après normalisation des espaces. `Lme` transforme `proposed` en `shared` quand la réponse de widget commence par `memories:share` ou `memories:promoted:`; les autres réponses deviennent `kept`. Source: [projection](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:47134).

Limite: ce helper n’est pas la mutation serveur. Son consommateur lazy n’a pas été retracé dans les copies LF sélectionnées. `kept` signifie une décision de carte, pas une suppression de fait.

### CM11. Reset et oubli durable ont des portées différentes

Le wrapper envoie `startFreshConversation` ou `rollBackConversation` avec bot et session. Le texte de récupération livré explique que Reset masque les anciens messages au bot tout en conservant mémoire, fichiers, routines et réglages. Sources: [message](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:73287), [appel](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:63997), [scope](../artifacts/formatted/dist/node-agent-coordinator/main.cjs:39333).

Limite: aucun reset réel n’a été observé. Le wording ne révèle pas la rétention du transcript serveur et ne prouve pas un oubli durable d’un fait.

### CM12. Correction, oubli et versions de mémoire restent incomplètement expliqués

Les résumés ont `memory_version`, `generated_at_ms`, `prose`, `summary_model` et `stale`. Les shards administratifs ont `scope`, `scope_key`, `version`, `box_backfilled` et `updated_at_ms`. Ce sont des contrats serveur, pas une base locale. Les recherches du ledger n’établissent aucun appel direct de correction/suppression/oubli de mémoire ni de `getGrokBotTeamContextSummary`. `UpdateMemory` est un nom d’outil et une consigne de CM08, sans implémentation retracée. Sources: [résumé](../artifacts/formatted/dist/node-agent-coordinator/main.cjs:21443), [shard](../artifacts/formatted/dist/electron-main/proto.cjs:61300), [outil](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:27823).

Limite: absence bornée aux fichiers et motifs du ledger. Un outil distant ou des appels calculés restent possibles. Version de shard ne donne ni politique de conflit ni historique consultable. Un utilisateur peut demander une correction dans la conversation sans handler client dédié.

### CM13. Les fichiers optionnels de skill disparaissent d’un chemin retracé

L’IPC accepte `files:[{path,content}]` pour ajouter ou mettre à jour une skill. Le wrapper renderer et le handler main les transmettent. Le constructeur installe pourtant `plugins:x9e(e)`, dont `AddGrokBotAgentSkill` et `UpdateGrokBotAgentSkill` transmettent seulement nom, description et contenu. Ces RPC n’ont pas de champ files. La réponse devient `name`, `description`, `sourcePath`, `content` et `environments`; content absent devient une chaîne vide. Sources: [fichiers](../artifacts/formatted/dist/electron-main/main-app.cjs:90715), [handler](../artifacts/formatted/dist/electron-main/main-app.cjs:108485), [service](../artifacts/formatted/dist/electron-main/main-app.cjs:104546), [adapter](../artifacts/formatted/dist/electron-main/main-app.cjs:103017).

Limite: la rupture concerne précisément IPC de skill → adapter protobuf livré. Elle ne démontre pas les autres routes d’import. Aucun stockage de fichiers supplémentaires ne doit être promis sur cette seule preuve statique.

### CM14. La bibliothèque workflow utilise un autre format

La création/édition workflow accepte `name`, `description`, `body`, trigger nullable `{schedule,isEnabled}` et `sourceRef` optionnel nullable. L’édition/suppression utilise `workflowId`. L’import prend Markdown et nom optionnel, ou URL et nom. `f2e` appelle ces commandes puis remplace la liste par la réponse; l’import rend aussi un `result` distinct. Sources: [format](../artifacts/formatted/dist/node-agent-coordinator/main.cjs:38742), [imports](../artifacts/formatted/dist/node-agent-coordinator/main.cjs:39149), [consommateur](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:52956).

Limite: les skills éditées par nom en CM13 et les workflows édités par ID ne constituent pas un format unifié démontré. `sourceRef` est opaque et le parseur Markdown reste distant.

### CM15. Publication et resynchronisation sont actives, sans garde de version visible

La bibliothèque appelle `publishSkill({workflowId,teamId})`, `resyncPublishedSkill({workflowId})`, `unpublishSkill({workflowId})`, puis relit les workflows du bot. La gateway applique son POST JSON générique `/api/<méthode>`. L’agent ID sert à la relecture locale; la mutation utilise le workflow ID. Ses arguments ne contiennent ni révision ni `expectedVersion`. Sources: [resource](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:52983), [arguments](../artifacts/formatted/dist/node-agent-coordinator/main.cjs:39204), [transport](../artifacts/formatted/dist/node-agent-coordinator/main.cjs:41371).

Limite: ce chemin établit publication et resynchronisation, pas un versionnement serveur complet. Historique, rollback, conflits et version installée restent inconnus.

### CM16. Synchronisation plugin, relecture bot et snapshot sont distincts

Le premier abonnement au statut lit `getPluginSyncStatus` et lance `syncPluginSkills`; après installation il relit les workflows regardés. `agents-workflow` apporte une liste qui remplace l’état. `agents-bot-skills` déclenche une relecture du listing de plugins du bot. Le contrat séparé `PublishGrokBotUserSkillsSnapshotRequest` n’envoie qu’un `fingerprint`; aucun appel direct à sa méthode ni à l’invalidation du cache n’est retrouvé dans le ledger. Sources: [sync](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:52858), [événements](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:76336), [snapshot](../artifacts/formatted/dist/node-agent-coordinator/main.cjs:30117).

Limite: aucun lien n’est établi entre fingerprint, teach et version de skill. Copie, choix et synchronisation réelle des ressources restent distants.

### CM17. Teach possède un état courant unique

`jke` a un snapshot `{state,agentId,startedAtMs,maxDurationMs}` et un état `armed`, sans map d’enregistrements par bot. Le start transmet l’origine `screen_hover`, `composer_menu` ou `fullscreen_title_bar`. L’état optimiste pose `maxDurationMs:600000`. Le timer actualise uniquement `nowMs` chaque seconde; il ne compare pas l’horloge à la durée maximale pour appeler stop dans ce contrôleur. Sources: [état](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:57469), [start](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:57524), [origines](../artifacts/formatted/dist/node-agent-coordinator/main.cjs:38401).

Limite: le serveur peut imposer 10 minutes. La valeur du snapshot ne prouve pas un arrêt local automatique ni une capture vidéo locale.

### CM18. Save, discard, reset et dispose ont des effets différents

Les contrôles lazy choisissent explicitement `stop(agentId,true)` ou `stop(agentId,false)`. Le start attend un stop en vol; le stop attend un start en vol. Les nettoyages sont chaînés et les générations protègent les retours tardifs. Pendant un stop, une mise à jour recording tardive ne réinstalle pas ce statut. Après erreur, une lecture status réconcilie l’état, sinon l’état antérieur revient. `reset` tente stop `save:false` pour le snapshot actif et les opérations en vol. `dispose` arrête le timer et invalide les continuations, sans ce stop. Sources: [resource](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:57570), [cleanup](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:57608), [save](../artifacts/formatted/dist/renderer/assets/chunk-teach-recording-controls-DRP1S1on.js:1480), [discard](../artifacts/formatted/dist/renderer/assets/chunk-teach-recording-controls-DRP1S1on.js:1525).

Limite: les erreurs de nettoyage sont ignorées. L’abandon n’est pas une garantie de livraison ou de suppression du média. Les commandes teach vont à la gateway; aucun skill n’est fabriqué par ce contrôleur.

### CM19. La consigne après démonstration est un marqueur d’affichage

Le renderer reconnaît une ligne teach seulement si le nonce commence par `teach-recording:` et si le contenu vaut exactement `The recording is finished. Learn the task from it.`. Il traduit alors l’affichage. Le catalogue distingue aussi `learn-from-demonstration`. Le resource stop applique un snapshot, sans recevoir de body de skill ni de workflow ID à cet endroit. Source: [marqueur](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:38167).

Limite: producteur du message, enregistrement associé, extraction de procédure et raccordement à un skill restent serveur. Le marqueur porte une intention d’apprentissage, pas une preuve d’apprentissage réalisé.

## Reconstruction proposée

Une reconstruction peut conserver quatre données distinctes: transcript complet, projection vocale de huit textes, faits durables privés/partagés, ressources de skill identifiées et sourcées. Le streaming écrit conserve le filtre auteur/session et l’identité d’événement; sa coupe de 2000 caractères ne modifie pas l’original. La sauvegarde teach devient une demande durable de traitement dont le résultat indique le skill créé ou l’échec. Ce sont des propositions, pas une implémentation récupérée du serveur Grok.

Il reste à définir remplacement d’un fait corrigé, oubli et ses effets sur copies/résumés, versions concurrentes de skills et priorité du prompt. Le client ne tranche pas ces règles.

## Checklist des trois sujets

| Sujet | answered-client-side | partially_answered | unresolved côté serveur |
|---|---|---|---|
| 1. Contexte et priorité | Plan vocal package, outils, projections/filtres, session.update, références de fichiers | Priorité vocale déclarée; setup; route backend par gate | Contexte écrit et backend, ordre mémoire/skills/outils, tokens, extraction fichiers |
| 2. Mémoire durable | Liste et promotion par agent; résultats partiels; état de carte; reset séparé | Scopes nommés et versions dans contrats | Extraction, correction, oubli, rétention, backfill, conflits, privé/équipe, injection |
| 8. Démonstration et skills | Lifecycle teach, save/discard, formats/identités, publication/resync, relectures/événements | Snapshot fingerprint compilé; fichiers interrompus à l’adapter | Capture réelle, recording→skill, validation, historique, rollback, sélection au tour |

Les trois sujets sont `partially_answered`. Les chemins client sont couverts; les moteurs de mémoire et d’apprentissage ne peuvent pas être déclarés reproduits à partir de ce package.
