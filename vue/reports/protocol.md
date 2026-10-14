# Contrats, conversations et services de Grok Bot 0.70.2

Cette analyse décrit le client expédié et propose les limites d'un équivalent Rust. Les contrats de transport sont récupérables. Le moteur d'agent distant, son stockage interne, ses prompts, ses fournisseurs de modèles et les garanties transactionnelles du serveur ne figurent pas dans l'application. Un client Rust peut reproduire ces fonctions en s'appuyant sur ses propres services. L'analyse ne promet pas une connexion compatible au backend fermé.

## Sources et couverture

Les comportements décrits proviennent de traces statiques. L'inventaire des DTO utilise une évaluation contrôlée du JavaScript expédié `proto.cjs` dans Node VM, distincte du parsing inerte de REA et de l'exécution de l'application. Cette évaluation initialise les classes et descripteurs et lit leurs métadonnées de champs. Elle ne lance ni Electron, ni coordinator, ni l'interface utilisateur. Le contexte ne fournit ni `require`, ni `process`, ni API réseau ou minuterie; Node VM reste un environnement borné d'évaluation et ne garantit pas une isolation de sécurité. Aucun compte, secret ou service distant n'a été lu.

Les références ci-dessous indiquent des lignes des copies formatées. Chaque copie correspond au même chemin relatif sous `artifacts/client`; les offsets originaux comptent les unités UTF-16 du JavaScript décodé, et non les bytes du fichier.

| Alias | Copie formatée |
|---|---|
| P | `artifacts/formatted/dist/electron-main/proto.cjs` |
| M | `artifacts/formatted/dist/electron-main/main-app.cjs` |
| A | `artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js` |
| U | `artifacts/formatted/dist/renderer/assets/index-D6ReTfRE.js` |
| F | `artifacts/formatted/dist/renderer/assets/index.eager-platform-DUN5VkaE.js` |
| C | `artifacts/formatted/dist/node-agent-coordinator/main.cjs` |

Le parent REA commun est `ev_a8b822a62831e30bd01eb4c4488b36f0bd50cb8ccc41d83e2dcff4e00cf82a4d`. Son graphe applicatif et sémantique reste `PARTIAL`. L'absence de source maps et les six contradictions d'intégrité natives restent des limites communes. Ce rapport n'attribue aucun comportement aux modules natifs `observed-untrusted`.

Le générateur `tools/protocol-index.cjs` produit `evidence/protocol-contracts.json`. Il conserve les noms RPC, les types d'entrée et de sortie, les numéros de champs, les types scalaires, les cardinalités, les `oneof`, les enums et les points d'appel candidats. Exécution reproductible:

```sh
node tools/protocol-index.cjs
```

L'inventaire couvre **7 descripteurs complets**, **484 entrées de méthodes**, **429 triplets uniques nom RPC/type d'entrée/type de sortie**, **2 313 messages**, **287 enums** et **599 autres fichiers JavaScript examinés pour les appels**. Les doublons entre `GrokBotService` et `SandBoxService` expliquent une partie de l'écart entre 484 et 429. Ce chiffre ne compte pas tous les protocoles IPC et JSON du client.

| Descripteur complet | Symbole dans P | Ligne | Méthodes | Noms ayant un appel de propriété candidat dans l'artefact |
|---|---|---:|---:|---:|
| `aiserver.v1.AgentStoreService` | `ur` | 4827 | 24 | 6 |
| `aiserver.v1.AiService` | `Tt` | 31514 | 8 | 5 |
| `aiserver.v1.BackgroundComposerService` | `tr` | 40843 | 7 | 7 |
| `aiserver.v1.DashboardService` | `I` | 40890 | 73 | 71 |
| `aiserver.v1.AnalyticsService` | `gL` | 41757 | 8 | 3 |
| `aiserver.v1.GrokBotService` | `n6` | 63662 | 309 | 152 |
| `aiserver.v1.SandBoxService` | `o6` | 65494 | 55 | 21 |

Les valeurs de la dernière colonne sont des recherches syntaxiques `.nomDeMéthode(`. Elles peuvent viser un adaptateur IPC ou une méthode homonyme. Elles omettent les appels dynamiques par index et ne prouvent ni l'identité du récepteur, ni l'exécution de la branche, ni l'existence du serveur. Les traces manuelles ci-dessous apportent cette distinction.

L'export `AgentStoreService` ne contient que 6 des 24 méthodes du descripteur `ur`. Les exports d'autres services sont aussi construits explicitement à partir des descripteurs. Il serait faux d'affirmer que l'application utilise toutes les méthodes présentes dans le bundle. Preuve: P `U7`, lignes 65812 à 65825, et inventaire `exportedServiceAliases`.

## Encodage et responsabilité des contrats

`f.typeName` et `f.fields` reconstruisent les métadonnées des messages à partir de la méthode statique `$()`. `D6` interprète chaque segment de forme `numéro nom type suffixe`: `?` signifie présence optionnelle, `*` signifie répétition, un quatrième identifiant signifie un groupe `oneof`, et une virgule indique une map. `qF` résout un indice `#n` en message ou enum. Preuve: P `qF`, `D6`, `f`, lignes 2562 à 2621. Cette reconstruction permet de conserver les véritables numéros de champs même si les noms JavaScript sont minifiés. Dans l'inventaire JSON, `optional` indique le flag explicite `opt` du descripteur. Un champ message possède aussi une présence protobuf intrinsèque; `optional:false` ne signifie pas qu'il existe obligatoirement dans chaque réponse.

`te` construit un client Connect à partir d'un descripteur et de `backend.backendUrl`. `_Mt` utilise les messages binaires par défaut, HTTP 1.1 par défaut dans `a_e`, des intercepteurs et les codecs de compression acceptés. `SMt` distingue les flux `single-shot` des requêtes `replayable`. Preuve: M `_Mt`, `a_e`, `te`, `SMt`, lignes 16704 à 16774. Les chaînes `application/connect+proto` et les encodeurs Connect existent aussi dans M:14377 et 14820 à 14935. Il s'agit de code client observé; aucun endpoint n'a été interrogé.

Les messages `google.protobuf.Struct`, les champs `record_json`, `mcp_config_json`, `variables_json`, et les corps JSON des transcripts constituent des contrats supplémentaires. Le descripteur protobuf ne décrit pas le schéma complet de ces documents.

Le générateur inclut aussi 22 messages à descripteurs explicites, dont les well-known types protobuf, en plus des 2 291 messages au format compact. La validation croisée vérifie les références de tous les champs et RPC. Preuve: `evidence/protocol-verification.json`.

La séparation suivante est nécessaire dans l'équivalent:

1. DTO de transport, avec présence optionnelle, `oneof`, valeurs numériques d'enum et `i64` ou `u64` exacts.
2. Documents JSON versionnés, avec conservation des champs inconnus utiles à l'affichage.
3. Modèle de domaine Rust, qui traduit les variantes de transport en états valides.
4. Présentation Iced, qui observe le domaine et envoie des intentions.

Les services de l'équivalent sont indépendants. Les numéros et noms Grok Bot servent de spécification de parité et de preuves, pas d'autorisation à appeler des services internes.

## Identité des bots, sessions, rooms et état utilisateur

`GrokBotAgent` n'est pas un historique de conversation. Il contient une identité serveur `id`, `legacy_agent_id`, l'identité agent `agent_id`, les noms et avatars, `harness`, `role`, `visibility`, `team_id`, les capacités de gestion du lecteur, `viewer_session_id`, `kind`, les membres d'une room, les personnes, et les préférences de voix. Preuve: P `W.$()`, ligne 51706; champs détaillés dans `protocol-contracts.json`.

La distinction entre `id`, `legacy_agent_id` et `agent_id` doit survivre dans la reconstruction. M `Ew`, lignes 103922 à 103951, transforme la réponse serveur pour les lignes de l'interface et conserve les décisions relatives au lecteur. Les chaînes `harness` servent au choix de source et d'opération. Elles ne prouvent pas que le serveur distant utilise une dépendance Temporal précise.

| Entité | Contrat observé | Source |
|---|---|---|
| Bot | `GrokBotAgent`, champs 1 à 29 avec lacunes de numérotation conservées | P:51706 |
| Session | `GrokBotAgentSession`: `agent_id`, `session_id`, `kind`, création, mise à jour, dernière activité optionnelle | P:48675 |
| Exécution courante | `GrokBotAgentLiveState`: running, composing, retrying, activity, awaiting, expiration d'état, handoff, session, sous-agents et tâche | P:48406 |
| État personnel de lecture | `GrokBotAgentClientState`: dernière lecture, unread_count, dernière activité, aperçu, notifications et visibilité dans la sidebar | P:48437 |
| Room | `CreateGrokBotRoomRequest`: identité, nom, description, IDs bots et IDs humains | P:51763 |
| Capacités de compte | `GrokBotRuntimeCapabilities`: durable identity, writes, temporal creation, agent messaging, server rooms | P:60178 |
| Réglages d'utilisateur | `GrokBotUserRuntimeSettings`: bots épinglés, sections, onboarding, bot principal, termes, règles auto-review, fournisseur email, permissions | P:51036 |

A `UTe`, lignes 70435 à 70523, ouvre le transcript en passant par `openAgentTail`, hydrate le domaine, utilise le `viewerSessionIdFor` et limite la sélection de session selon les droits et la disponibilité de multiplayer. A `OTe`, lignes 69053 à 69108, crée une réplique distincte pour chaque couple agent/session. Il faut donc indexer les conversations par compte, bot et session. Une seule liste par bot mélangerait les conversations personnelles d'un bot d'équipe.

Les rooms exposent aussi `SetGrokBotRoomMembers`, `AddGrokBotRoomPeople`, `RemoveGrokBotRoomPeople`, `RequestGrokBotRoomMemberTurn`, `CancelGrokBotRoomMemberTurn` et `DeliverGrokBotRoomMemberTurnResult`. Preuve: P `n6.methods`, lignes 64939 à 64988. Les schémas attestent le protocole. Ils ne permettent pas de reconstituer le planificateur de tours de la room ni l'arbitrage du serveur.

## Envoi utilisateur, accusé, renvoi et annulation

### Point d'entrée et préparation locale

U, lignes 124505 à 124547, construit une commande `sendPrompt` avec texte, rich text, pièces jointes et heure d'entrée. Son callback `onJournaled` arme les suivis `sendAck`, `turnOutcome` et `renderStream`. A `W4e`, lignes 12103 à 12162, expose `sendPrompt`, `resendFailed`, `deleteFailed` et `cancelQueued`. `vE`, A:6834, réclame le tunnel puis attend `sendJournal.sendPrompt`, propose un titre de conversation, efface les approvals locales et note l'usage.

Le journal `fIe`, A:71204 à 72308, possède les vrais états:

```text
prepared -> dispatching -> accepted-awaiting-echo -> suppression après écho
queued -> préparation des attachments -> dispatching
échec -> failedAtMs conservé sur un état existant
```

`N3`, A:70899, définit `send-journal`, schéma version 3, stockage `client-persisted`, et sensibilité au compte. `dIe`, A:71198, impose `crypto.randomUUID` car le nonce arrive à l'hôte. `QTe`, A:70958, contient exactement les quatre phases ci-dessus. Un échec est un attribut supplémentaire, pas une cinquième valeur de phase.

`sendPrompt`, A:72055 à 72253, refuse un texte vide sans attachment, crée le nonce, ajoute l'affichage optimiste, commit les fichiers si possible, puis persiste le record avant la commande distante. La phase `dispatching` est elle aussi persistée avant l'appel. En cas d'échec de persistance, le client n'envoie pas aveuglément la commande: il retire ou restaure l'état approprié. Les drafts sont supprimés seulement si l'identité du draft consommé est encore courante.

L'affichage optimiste précède parfois le commit des pièces jointes et la persistance. Il n'est donc pas une preuve de livraison. Les états affichés comprennent `queued`, `pending`, `failed`, la confirmation et l'attente de décision hôte. Les records contiennent les chemins staged et committed séparément, l'ancien draft, la tâche, la session, la provenance des routines et les nonces antérieurs.

### Adaptation au RPC

Le domaine injecte sa source dans le journal à A:72386. C `IJ.dispatchCommand`, lignes 43616 à 43880, valide et route les commandes selon le harness, les capacités et les exceptions par opération. Pour un bot temporal, une commande prise en charge par le serveur passe par C `ur`, lignes 43294 à 43310, puis `control.performServerAgentAction`. M `fun`, lignes 107298 à 107307, installe ce performer depuis `desktopMcp.createServerAgentActionPerformer`. La voie gateway reste distincte pour les autres commandes. Un serveur `unimplemented` ne produit pas automatiquement un fallback pour une opération exigée par un bot temporal. Le router refuse dans ce cas; des opérations explicitement compatibles peuvent utiliser la gateway. Cette décision appartient au router, pas à l'écran ou au journal.

M `z8e`, lignes 110631 à 110683, traduit `sendPrompt` vers `SendGrokBotUserMessage`. `clientNonce` devient `message_id`. `prompt` devient `text`. L'adaptateur transmet rich text, reply target, fork, noms et chemins des fichiers, traceparent, heures de composition, source, config MCP, session, machine, présence de 1Password, initiator, task et send source.

Le request exact est P `sB.$()`, ligne 55726:

```text
1 agent_id:string; 2 message_id:string; 3 text:string; 4 sent_at_ms:int64;
5 rich_text:optional string; 6 reply_to_id:optional string; 7 is_fork:bool;
8 attachment_paths:repeated string; 9 attachment_names:repeated string;
10 traceparent:optional string; 11 enter_epoch_ms:optional int64;
12 composed_at_ms:optional int64; 13 source:enum;
14 mcp_config_json:optional string; 15 session_id:string;
16 machine_id:optional string; 17 one_password_app_installed:optional bool;
18 initiator:enum; 19 task_id:optional string; 21 send_source:enum.
```

`SendGrokBotUserMessageResponse`, P:55778, comprend `dispatched`, `mode`, `workflow_id`, `delivery`, et `refusal`. Le client de M décide d'après `delivery`: `ACCEPTED_BOX`, `ACCEPTED_TEMPORAL`, et `DUPLICATE` donnent `{accepted:true}`; `REFUSED` donne un refus structuré; `UNSPECIFIED` donne un refus `unknown-delivery`. L'interface ne doit pas interpréter `dispatched` seul comme succès.

Après succès de l'appel, `fIe.x`, A:71404, passe en `accepted-awaiting-echo`. Le record reste sur disque. La confirmation serveur de la commande et l'écho dans le transcript sont deux événements différents.

### Incertitude et reconnexion

`GetGrokBotSendStatusRequest`, P:55803, recherche le nonce par agent et session. `GetGrokBotSendStatusResponse`, P:55952, retourne `NOT_FOUND`, `ACCEPTED`, `REJECTED`, `PENDING`, ou `UNKNOWN_DURABILITY`, avec ID d'écho, code de rejet et instant d'acceptation optionnels. M `q8e`, lignes 110329 à 110368, traduit les valeurs inconnues ou non spécifiées en `unknown-durability`.

M `W8e`, lignes 111086 à 111111, ne répète un send après erreur transport qu'après une recherche d'acceptation et uniquement si le résultat est `not-found`. A `fIe.Be`, lignes 71426 à 71545, interroge d'abord les nonces antérieurs puis le nonce courant. `unknown-durability` conserve l'incertitude. Un état trouvé sans rejet conserve l'attente d'écho; un `echoEntryId` connu confirme l'affichage optimiste et rafraîchit le transcript. A `noteReconnect`, ligne 72272, relance la réconciliation. Les changements de compte ou de génération de travail invalident les continuations asynchrones.

A `resendFailed`, lignes 71670 à 71763, crée un nonce neuf lié au nonce logique original et conserve `priorNonces`. A `noteEchoReconciled`, lignes 72258 à 72271, retire le journal et les staged files après l'écho correspondant. L'identité logique empêche que le même texte apparaisse deux fois après un renvoi tardif.

Les retries du journal ont cinq essais, un délai initial de 1 seconde, un maximum de 30 secondes, et du jitter equal. Preuve: A `O3`, lignes 70835 à 70842. Ces paramètres sont observés pour cette couche; ils ne définissent pas les retries de tous les transports.

### Trois annulations distinctes

`cancelQueued` dans A `fIe.de`, lignes 71899 à 71935, remet un message queued dans le draft. Pour un message déjà parti, cette opération vérifie `promptAcceptanceStatus` et n'annule localement que si l'hôte répond `not-found`. Supprimer un failed record retire un élément local; cela ne prouve pas l'annulation d'un agent distant.

`interruptAgentRun` passe par `InterruptGrokBotAgentRun`, avec raison `user_interrupt` et session optionnelle, puis retourne `had_active_run`. Preuve: M `z8e`, lignes 110691 à 110709; P:55977 à 56004. Le request ne promet pas l'annulation de toute opération externe déjà effectuée.

`startFreshConversation` et `rollBackConversation` sont des RPC distincts et ne reçoivent pas le nonce queued. Preuve: M:110711 à 110739; P `n6.methods`, lignes 64722 à 64733. La reconstruction doit garder ces intentions séparées dans l'UI et le domaine.

### Erreurs du domaine

M `Zl`, lignes 110561 à 110627, distingue `unimplemented`, `refused` avec failure code, et `unavailable` avec catégories network, timeout, access_denied et http_5xx. Il lit aussi `x-grok-bot-refusal` et `x-grok-bot-local-tool-permission-fallback` pour des refus de capacité précis. Une erreur réseau ne constitue pas un refus utilisateur et un refus de permission ne doit pas déclencher automatiquement un renvoi.

## Transcript, ordering, pagination et corps distants

### Contrat de stockage et de flux

| Message protobuf | Champs de contrôle | Source |
|---|---|---|
| `GrokBotTranscriptEntry` | `seq:u64`, `entry_kind`, `body:optional bytes`, `blob_hash`, `updated_seq:u64`, `entry_id`, `body_omitted` | P:48116 |
| `GrokBotTranscriptEntryDelete` | suppression identifiée par seq et updated_seq, entrée optionnelle | Inventaire JSON |
| `GrokBotTranscriptCursor` | agent, generation:u32, after_updated_seq:u64, session | P:48168 |
| `WatchGrokBotTranscriptsRequest` | repeated cursors, include_unlisted_agents optionnel, inline_body_max_bytes | P:48193 |
| `GrokBotTranscriptWatchRows` | agent, generation, entries, deletes, replay, session | P:48249 |
| `ListGrokBotTranscriptEntriesRequest` | agent, generation optionnelle, before_seq optionnel, limit, session, frontière unread optionnelle | P:51610 |
| `ListGrokBotTranscriptEntriesResponse` | entries, generation, IDs peers, présence du résultat peers, unread anchor | P:51664 |
| `CommitGrokBotTranscriptEntriesRequest` | agent, generation, entries, deletes, session | P:51530 |

Le `oneof frame` de `GrokBotTranscriptWatchFrame` comprend connected, rows, cleared, cursor_too_old, heartbeat, agent_state, computer_actions, agent_state_changed, turn_failed, roster_changed, box_state et page_title_changed. Preuve: P:51490. La présence d'une famille dans le `oneof` n'atteste pas sa fréquence d'émission.

`CommitGrokBotTranscriptEntries` n'a aucun appel direct de propriété dans les 599 fichiers parcourus. La méthode est dans le descripteur. On ne peut donc pas conclure que le renderer écrit les transcripts par ce RPC.

### Du flux protobuf à l'événement de domaine

C `ND`, lignes 35002 à 35123, construit le client backend après résolution des droits et des headers. `watch` appelle effectivement `watchGrokBotTranscripts`; `list` appelle `listGrokBotTranscriptEntries`; `readBlobs` presigne des chemins `blobs/<hash>` et récupère les bytes. L'accès est mis en cache pendant 5 secondes. La présence de la voie serveur et de la voie gateway est explicite.

C `lM`, lignes 35679 à 36800, possède un état par agent/session. `o9`, C:35669, fusionne les entrées et suppressions et les trie par `updatedSeq`. Ce tri utilise l'ordre des mutations, alors que `seq` identifie la position de la ligne. `R` et `M`, C:36031 à 36092, ignorent une version déjà appliquée, traduisent une ligne en appended ou updated, et une suppression en removed. Le remplacement d'un ID à une position peut produire removed puis appended/updated. Une arrivée historique hors fenêtre peut provoquer une réhydratation.

`fe`, C:35949 à 35998, choisit le corps inline, relit les lignes `bodyOmitted`, puis lit les blobs. L'absence de corps ne devient pas silencieusement une ligne vide. `ze`, C:36008 à 36025, programme les retries; après cinq essais, `_e` signale une ligne perdue. `_n`, C:36428 à 36452, abaisse le curseur avant tout `updatedSeq` de corps encore manquant. Cela permet une relecture lors de la reconnexion.

La traduction des deux ordres est démontrée par C `Ae`, `Ne`, `xt` et `At`, lignes 35808 à 35888:

```text
ordre serveur = generation + updated_seq de mutation, seq de position
ordre émis localement = replicaKey + epoch + compteur sequence

epoch = bootId:generation[:recovery:n][:epochBump]
sequence = compteur local incrementé pour chaque événement émis
```

Le compteur UI n'est donc pas la valeur `updated_seq` reçue du serveur. Confondre ces valeurs provoquerait des trous artificiels ou des données mélangées après changement de source.

`zs`, C:36360 à 36425, traite `cleared` par changement de génération, `cursorTooOld` par changement de génération suivi de réhydratation, et transmet les changements d'état aux abonnés. `Rn`, C:36524 à 36599, demande les agents non listés et 256 KiB de corps inline, attend la frame connected, arme un watchdog de silence de 60 secondes, puis reconnecte après fin de flux. Les cursors sont bornés à 256 états récemment touchés. Les backoffs de flux et des corps sont distincts.

Le fallback des bots BOX vers la gateway apparaît dans C `j`, lignes 35824 à 35851. Il peut être récupérable ou durer toute la vie du coordinator. Les agents requis ont un traitement distinct. Cela impose un concept de source de transcript dans le domaine; le simple booléen connected ne décrit pas la fraîcheur des données.

### Réplique UI et données restaurées

F `$C`, lignes 1519 à 1692, est l'algorithme de réplique. Il valide `replicaKey`, conserve l'epoch et le dernier sequence accepté, bufferise les événements d'une autre epoch, ignore les sequences déjà appliquées, applique seulement une suite contiguë, et lance une resynchronisation sur trou. Un snapshot doit avoir le coverage attendu, un attemptId valide quand nécessaire, et un throughSequence suffisamment récent. Un dépassement de buffer efface les événements stockés et relance une tentative. La fin de resynchronisation dépend du baseline et des trous restants.

A `OTe`, lignes 68974 à 70287, instancie cette réplique par agent/session avec buffer 512 et coverage `transcript-live-range`. Il sépare la fenêtre live, les pages historiques, l'affichage optimiste et le seed restauré. Un snapshot ne peut pas écraser une ouverture plus récente ou une conversation cleared entre-temps. Les détails sont dans `fetchAttempt`, `installGeneration`, `pageGeneration` et `clearedGeneration`.

A `yTe`, lignes 68591 à 68839, persiste un transcript réduit avec `epochHint`, `acceptedSequenceHint` et `persistedAt`. Ces champs sont des indices. A `hydrateFromOpen`, lignes 70031 à 70112, peut afficher le seed local ou le server-tail quand la source courante échoue, puis installe les données autoritatives. Un transcript restauré doit être montré comme restauré ou en resynchronisation.

A `Oe`, lignes 69841 à 69887, charge l'historique avant `beforeSeq`, puis ignore un résultat si la page ou la resynchronisation a changé. Les limites observées sont 500 lignes pour la vue sélectionnée, 200 pour certains bots hors vue et 100 pour une page historique. La somme des lignes hors vue est limitée à 1 000. Preuve: A constantes `RTe`, `ITe`, `Pg`, `sw`, lignes 68923 à 68930, et `rt`, lignes 69706 à 69720.

M `Tft`, lignes 123098 à 123217, constitue une autre lecture du transcript stocké: list RPC, fetch des blobs avec concurrence 8, JSON.parse du corps, normalisation, conversion de seq, reverse de la page. Une ligne invalide peut être comptée et omise. Ce chemin ne remplace pas les garanties de la réplique live.

### Corps JSON et variants inconnus

A `aT`, lignes 16470 à 16497, normalise les entrées `message`, `send-message`, `user-attachment`, `tool-call`, `mcp-app`, `notice`, `voice-call`, `event` et `feedback`. Il accepte aussi certaines valeurs `kind` inconnues. Des migrations locales réparent les anciens targets de secret, les compteurs de voix et l'adresse from des drafts email. Preuve équivalente dans C `Oh`, lignes 35153 à 35256.

A `Nae`, lignes 35618 à 35698, révèle les sous-types de `send-message` réellement exploités pour les aperçus: text, attachment, widget, cursor-agent, secret-request, credential-request, user-form, email-draft, slack-draft, meeting-chat-draft, calendar-event, permission-request, auto-review-approval, cookie-origin-approval, mobile-tool-permission, local-tool-permission, connector-grant, virtual-card-approval, publish-approval, connector, connectors, listener-connect, team-access, slack-connect, scm-connect, onepassword-connect et bot-template-share. Cette liste représente cet appel de présentation; elle n'est pas un enum exhaustif garanti par le serveur.

Un modèle Rust doit donc préserver une variante `Unknown` pour l'entrée et le contenu, avec les données brutes et un état de présentation compatible. Une désérialisation fermée qui échoue sur tout nouveau `kind` casserait la lecture d'une conversation entière.

## Routines, horaires, tâches et mémoire

### Routines

Le protocole protobuf représente une routine comme `GrokBotAgentAutomation { automation_id, record_json }`, P:49934. `ListGrokBotAgentAutomations`, `ListGrokBotAccountAutomations`, `SetGrokBotAgentAutomationEnabled` et `DeleteGrokBotAgentAutomation` sont présents dans `n6`. L'activation transmet un time zone optionnel. Preuve: P:50651 et méthodes 64123 à 64202.

Le domaine client expose davantage d'opérations: `getAgentAutomations`, `listAllAutomations`, `createAgentAutomation`, `updateAgentAutomation`, `deleteAgentAutomation`, `runAgentAutomationNow`, activation, et lecture de la credential webhook. Preuve: A `nS.automations`, lignes 5079 à 5092. M, lignes 112638 à 112676, route seulement une partie des commandes vers le backend serveur dans `iet`. Il serait faux de compter une méthode de domaine comme un RPC protobuf supplémentaire.

A `d2e`, lignes 52349 à 52803, lit les routines, tient l'état ready/empty/loading/failed/unavailable, applique les listes retournées après les mutations et revalide après reconnexion. La création cherche une nouvelle routine parmi les IDs absents avant l'appel; le nom seul n'est pas une identité. Le run manuel empêche un deuxième départ local pour le même agent/routine pendant awaiting, running ou cooldown. Il surveille les run IDs et leur statut, puis réarme après 3 secondes. Les credentials webhook sont relus toutes les 2 secondes jusqu'à 15 essais. Ces clés ont été décrites statiquement, jamais lues sur le disque.

A `i0`, `yne`, `SR`, `bR` et `vR`, lignes 30603 à 31101, analyse les expressions cron, leurs plages calendaires et les zones de temps. Les triggers `cron` sont explicitement distingués dans A:31636 à 31652. Cette logique prouve le calcul et l'affichage côté client, pas l'exécution d'un scheduler distant. M `Upn`, `Ure` et `u1`, lignes 111209 à 111240, vérifient les champs JSON id, name, prompt, trigger, schedule, triggerDescription, isEnabled, createdAt, filePath et runs. Les listes sont triées par nextRunAt puis createdAt. M `o7e`, lignes 111244 à 111376, fait effectivement les appels serveur de lecture, activation et suppression; il vérifie le trigger webhook avant de demander une API key via DashboardService. Les règles de retard, DST, rattrapage, concurrence de runs ou livraison de webhook restent des contrats à stabiliser pour l'équivalent.

### Tâches et sous-agents

`GrokBotTask`, P:50391, comprend titre, état, note, timestamps done et seen. `KeepGrokBotTaskListRequest`, P:50467, reçoit les messages, awaiting_user, background_work, turn_end et finished_work. `ListGrokBotAgentTasks`, `MarkGrokBotAgentTaskDone`, `MarkGrokBotAgentTaskRead`, `ListGrokBotSubagents`, `ListGrokBotAgentTodos`, `ListGrokBotTasks`, `DismissGrokBotTask` et `MarkGrokBotTasksSeen` existent dans le service. Les deux familles de tâches déclarées indiquent que l'interface ne doit pas assimiler toute activité en arrière-plan à une simple routine.

### Mémoire

Deux représentations sont visibles. `GrokBotMemoryFolder`, P:46805, possède un profil et une map de logs. Les shards des définitions admin contiennent scope, scope_key, version, box_backfilled, updated_at et folder, P:61325. `GrokBotMemoryFact`, P:54717, contient fact_id, texte et learned_at.

L'écran utilisateur ne dispose pas d'un accès arbitraire à tous les shards. A `GU`, lignes 13067 à 13087, appelle `listGrokBotUserBotMemories` puis `promoteGrokBotMemoriesToTeam` avec une liste de fact IDs. M `x9e`, lignes 103051 à 103069, transforme le résultat en created, alreadyInTeam et keptPrivateCount. Les faits peuvent donc rester privés et une promotion peut être partielle.

`GrokBotTeamContextSummary`, P:54642, contient memory_version, generated_at, prose et summary_model. Ces champs attestent un résumé versionné. Ils ne révèlent ni le prompt qui extrait les faits, ni la règle qui les garde privés, ni l'algorithme de consolidation. La reconstruction Rust doit posséder son extraction, ses scopes et son mécanisme de promotion.

## Skills, plugins, MCP et marketplace

### Trois couches distinctes

`GrokBotAgentSkill`, P:50764, contient id, name, description, body et source. Les opérations de gestion de skills dans le plugin listing utilisent aussi des compétences avec sourcePath, content et environments. M `R9e`, lignes 102907 à 102916, construit cette seconde forme. `GrokBotAgentPluginSkill`, P:54201, déclare précisément cette seconde forme. `AgentSkill`, P:24248, représente encore un autre contexte avec full_path, parse_error, environments, disabled_environments, origine Git, globs, scopes et flags de désactivation d'invocation modèle ou utilisateur. La présence de ce contexte partagé ne prouve pas que chaque champ est utilisé par le desktop. Une skill est une ressource de travail avec contenu et provenance; une connexion OAuth a un cycle distinct.

`GrokBotAgentPlugin`, P:54172, comprend un plugin_id int64, son nom, display_name, description, logo, owned_by_marketplace, install_mode, variable_names, dépréciation, MCP JSON optionnel, skills, noms des serveurs MCP, is_bot_skills, connector_type, variables JSON et has_configured_variables. M `asn` et `bne`, lignes 102924 à 102953, convertissent l'ID vers une chaîne et refusent les IDs non entiers positifs. Rust doit conserver cet ID exact, sans le convertir en `f64`.

`GrokBotAgentPluginEntry` est un `oneof` qui accepte une référence plugin, un config update ou un plugin inline. M `ssn`, lignes 102956 à 102984, choisit réellement ces variantes. `setPlugins` remplace la sélection; `setPluginVariables` a une option `mergeWithStored`. L'absence de cette option n'a pas la même signification qu'une fusion explicite.

Les credentials de connecteur et les permissions sont une troisième couche. `GrokBotUserMcpSettings`, P:50861, contient serveurs, instructions par nom, zone horaire et auto-review. `GrokBotToolPermissions`, P:51202, est une map par scope. Le catalogue Dashboard comprend découverte des serveurs, config MCP, OAuth, liste d'accounts, validation de tokens, auth managed skills, install/update/uninstall de plugins et permissions de tools. L'inventaire JSON conserve chaque schéma.

### Chaîne de gestion utilisée

A `VU`, lignes 13089 à 13141, lit le listing par agent, enregistre plugins et variables, ajoute/modifie/supprime une skill et remplace le cache avec la réponse. M `x9e`, lignes 102986 à 103072, implémente effectivement ces appels GrokBotService. Sa lecture commence avec `includeMcpConfig:true`; un `PermissionDenied` provoque une deuxième lecture sans cette option. Il ne traite pas ce cas comme une permission d'écriture.

`GetGrokBotAgentPlugins`, `SetGrokBotAgentPlugins`, `SetGrokBotAgentPluginVariables`, `AddGrokBotAgentSkill`, `UpdateGrokBotAgentSkill`, `RemoveGrokBotAgentSkill`, `GetGrokBotAgentPluginFile`, `PublishGrokBotUserSkillsSnapshot` et `InvalidateGrokBotUserSkillsCache` sont des descripteurs. Les deux dernières méthodes n'ont aucun appel direct de propriété dans l'inventaire parcouru. Leur présence ne suffit pas à prouver leur déclenchement par le desktop.

Les cartes `connector-grant`, `local-tool-permission` et `auto-review-approval` ont leur état et leur résolution propres. `ResolveGrokBotConnectorGrant` expose allowed, skipped, always_this_bot, always_all_team_bots; `clearGrokBotConnectorAlwaysAllow` est séparé. Preuve: M `Epn`, lignes 110416 à 110423; méthodes dans P:64798 à 64816. L'existence d'une connexion ne vaut pas autorisation pour toute action.

### Templates de bots et publication marketplace

`GrokBotTemplate`, P:55181, contient share_id, source_agent_id, nom, description, avatar, clé blob, published, active_version, visibility et owner_type. Une listing marketplace ajoute slug, statut, creator, catégories et version épinglée. Preuve: `GrokBotMarketplaceListing`, P:47358.

M `L$e.getPublicTemplate`, lignes 87467 à 87511, appelle `GetGrokBotTemplateImportDetails`, vérifie la présence d'une recette et d'une activeVersion, télécharge le blob et parse la recette avec `TS`. La valeur expected_active_version protège l'import contre une version différente. `listMarketplace`, M:87513 à 87542, demande des pages de 100 entrées jusqu'à nextPageToken vide et conserve featured, listings et ordre de catégories.

M `net`, lignes 112461 à 112530, appelle `CreateGrokBotAgentFromTemplate` avec expectedActiveVersion, setupDelegationSupported et contexte optionnel. Il exige que l'agent retourné ait l'ID demandé et un harness temporal. `setupHandledByServer`, `conversationalSetupEnabled`, `gettingStartedTrusted` et `updateStateSplit` dirigent ensuite le setup. Une importation de template n'est donc pas uniquement une copie de nom et de description.

Les méthodes `*Internal` de création, modification et suppression des listings, creators, catégories et templates sont présentes dans `n6`, P:65045 à 65238. Leurs noms attestent des capacités d'opérateur dans le contrat. Ils ne prouvent pas les droits du compte installé ni une API publique réutilisable.

## Équipes, accès et administration

`GetMeResponse`, P:32770, inclut user_id, team_id, organization_id et `is_team_admin`, mais plusieurs valeurs sont optionnelles. `TeamRole` distingue OWNER, MEMBER, FREE_OWNER et REMOVED. Une décision d'accès ne doit pas se déduire uniquement du texte d'un rôle affiché.

`GrokBotAgentAccessState`, P:31818, possède principals, team_row_on, available_roles, expected_state_token, groups_enabled et team_row_off_allowed. `GrokBotAgentAccessUpdate`, P:31845, renvoie principals et expected_state_token. M `updateAccess`, lignes 104162 à 104175, transmet réellement ce token. C'est une preuve de contrôle de concurrence sur l'état d'accès. Le client reconnaît un conflit quand Connect retourne `Aborted` avec `ErrorDetails.UNSPECIFIED` et `isRetryable:true`, puis retourne `{kind:"changed"}`. Preuve: M `Msn`, lignes 103874 à 103886, et `updateAccess`, lignes 104177 à 104180. Le contrôle transactionnel exact du serveur reste inconnu.

Les principals ont un type et un ID. Le service expose lecture et mise à jour de l'accès, managers, accès par lien, visibilité des meetings, promotion ou clone vers équipe, publication et dépublication, secrets, vault et service accounts. Les méthodes de création et retrait de bots d'équipe, bulk operations des boxes, listes des membres, groupes et manifests font partie de l'inventaire. L'équivalent doit distinguer utilisateur, groupe, équipe, manager, propriétaire et bot cible.

Les descripteurs admin couvrent les boxes et leur stockage, agents d'équipe, définition d'agent et shared state, suppressions, email et quotas, meetings/transcripts, consentement de meetings, avatars, versions de résumé et migrations de harness. Sources: P `n6.methods`, lignes 63688 à 63831 et 65257 à 65468; `AdminGetGrokBotAgentDefinitionResponse` et autres messages dans l'inventaire. La présence d'une méthode admin compilée n'est pas une preuve de sa joignabilité.

Une implémentation indépendante doit appliquer l'autorisation côté service pour chaque mutation. Les champs UI `viewer_is_owner` et `viewer_can_manage` expliquent l'affichage et ne remplacent pas la validation de la session et du principal serveur.

## Fichiers et pièces jointes

Deux familles existent. Les fichiers persistants d'un bot utilisent `GrokBotAgentFile`, P:54881, avec name, path, taille, MIME, timestamp et etag. M `I9e`, lignes 102839 à 102882, calcule le SHA-256 des bytes, demande `BeginGrokBotAgentFileUpload`, réalise le PUT avec headers retournés, puis `FinishGrokBotAgentFileUpload`. Le listing indique totalBytes et canEdit. Le delete porte agent_id et name. Le PUT réussi n'est pas la confirmation finale du fichier.

Les pièces jointes d'un message passent par staging et commit. Les méthodes IPC de A `Gm`, lignes 16970 à 17014, prennent bytes ou chemin source, filenames, session, routage store et clé de progression. Le journal garde les chemins staged et committed séparés.

Le backend négocie `BeginGrokBotAttachmentUploadRequest` avec supports_store_attachment_refs, supports_multipart, taille et hash. Le response choisit USE_BOX, SINGLE_PUT, MULTIPART ou REFUSED, avec token, expiration, limite, instruction PUT et paramètres de parts. Preuve: P:49259, 49290 et enum dans `protocol-contracts.json`. Les RPC de presign parts, complete et abort sont distincts. La reconstruction doit négocier les capacités, finaliser ou abandonner un upload, et ne publier le message qu'avec des références acceptées.

Les méthodes de lecture comprennent chunk d'attachment, résolution group chat, document, canvas et MCP app. Les grants d'accès et les scopes de bot/session doivent se transmettre jusque dans la lecture. Une URL presignée n'est pas une identité de fichier stable dans le domaine.

## Voix, dictée, mémos et meetings

### Appel temps réel

Le credential de voix vient de `MintSandVoiceCallSecretResponse`: client_secret, expiration Unix, model et websocket_url. Preuve: P:45439. M `jYt` et `zYt`, lignes 48551 à 48588, demandent cette credential via GrokBotService avec timeout 15 secondes et transforment les refus en un fault. La credential n'a pas été obtenue pendant cette analyse.

A `xu`, lignes 79379 à 79386, limite l'adresse dialable à la chaîne préfixe `wss://api.x.ai/`. A `Kn.realtimeProtocols`, `realtimeUrl` et `sessionUpdateFrame`, lignes 80401 à 80453, montrent un autre contrat que Connect: WebSocket JSON avec sous-protocole `xai-client-secret.<secret>`, paramètre model, conversation_id pour reprise, session.update, instructions, tools, server_vad, resumption, audio PCM 24 kHz, vitesse et langue. La conversion float vers PCM16 apparaît à A:80466 à 80474. La longueur de texte et les tableaux de temps utilisent les unités UTF-16 de `.length` côté JavaScript. Rust ne doit pas confondre cette longueur avec le nombre de bytes de `String::len`.

A `Kn` traite response.created, session.updated, événements de speech et transcription, conversation items, audio et tool calls. Preuve: A:79574 à 79835. A `voiceCallHarnessSession` et `voiceCallHarnessTool`, lignes 75662 à 75663, délèguent les instructions et outils au backend. Les request/response de P:63423 à 63618 comprennent tools, greeting, receipt, ligne de dialogue et output Struct. M:41314 à 41327 appelle effectivement les RPC voiceCallHarnessSession et voiceCallHarnessTool avec ces DTO. Les prompts et l'exécution de ces tools côté serveur restent inconnus.

A `Gt`, lignes 40720 à 40760, définit idle, connecting, live, reconnecting et ending comme phases d'interface. La reconnexion exige un feature gate, un compteur de génération et un plan existant. `conversationId` et la suppression de la salutation de connexion évitent de recommencer le dialogue. Preuve: A:41569 à 41650. Les paramètres de reconnexion sont propres à la voix et ne reprennent pas ceux du transcript.

### Dictée et synthèse de mémo

M `SandTranscriptionManager` alias `L5`, lignes 48928 à 48979, refuse les bytes vides, réduit le MIME, normalise une langue connue, et appelle `AiService.TranscribeAudio` sous deadline 60 secondes. La réponse contient text et transcription_time_ms. Preuve des DTO: P:31437 et 31460. `StreamStt` bidirectionnel est présent dans `AiService` mais aucun appel direct n'est trouvé par le générateur. Il ne faut pas attribuer ce flux à la dictée du desktop sans trace supplémentaire.

M `SandSpeechSynthesisManager` alias `U5`, lignes 49043 à 49092, limite le texte à 1 500 caractères après trim, demande textToSpeech avec withTimestamps, et valide que les temps de début/fin correspondent à la longueur du texte. La permission ou le refus d'entitlement devient une erreur de domaine distincte. Deadline observée de 30 secondes. Les DTO sont P:31481 et 31509.

Les meetings ont une autre famille de RPC: pull/report local meetings, list/leave server meeting, get record, et outils admin de consentement et transcript. Le sous-type meeting-chat-draft transmet meeting_id, titre optionnel, body et mark_internal. Preuve: M `Lpn`, lignes 111042 à 111085, et P `n6.methods`, lignes 64881 à 64909. La collecte audio et la présence dans un meeting appartiennent aux contrats runtime spécialisés; le serveur de réunions n'est pas reconstitué dans le client.

## Proposition de types et interfaces Rust

Les déclarations suivantes sont un schéma d'architecture. Elles ne sont pas du code produit ni un engagement de compatibilité avec les services Grok Bot.

Le caller Iced envoie une intention unique, puis observe un état de conversation. Exemple de séquence:

```text
conversation.send(SendIntent { text, rich_text, attachments, reply, task })
conversation.observe() -> lignes, envois, fraîcheur, activité
conversation.cancel_queued(send_id)
conversation.interrupt(run_id)
```

Le core gère les étapes de persistence, upload et transmission. La UI ne construit pas les tuples de cursors, les headers d'auth ou les phases du journal.

```rust
struct AccountId(String);
struct BotId(String);
struct SessionId(String);
struct SendId(String);
struct EntryId(String);
struct RoutineId(String);
struct FactId(String);
struct PluginId(i64);
struct Generation(u32);
struct RowSeq(u64);
struct UpdatedSeq(u64);
struct ReplicaSequence(u64);
struct ReplicaEpoch(String);

struct ConversationKey {
	account: AccountId,
	bot: BotId,
	session: SessionId,
}

struct AttachmentRef {
	id: String,
	name: String,
	mime: String,
	size: u64,
	content_hash: Option<[u8; 32]>,
}

enum SendPhase {
	Prepared,
	Queued,
	Dispatching,
	AcceptedAwaitingEcho,
}

enum Acceptance {
	NotFound,
	Accepted { echo: Option<EntryId>, at_ms: Option<i64> },
	Pending { at_ms: Option<i64> },
	Rejected { code: Option<String> },
	UnknownDurability,
}

struct PendingSend {
	key: ConversationKey,
	logical_id: SendId,
	attempt_id: SendId,
	prior_attempts: Vec<SendId>,
	phase: SendPhase,
	input: SendIntent,
	staged: Vec<StagedAttachment>,
	committed: Vec<AttachmentRef>,
	created_at_ms: i64,
	failed_at_ms: Option<i64>,
	awaiting_host_decision: bool,
	draft_recovery: DraftSnapshot,
}

enum TranscriptDelta {
	Appended(TranscriptEntry),
	Updated(TranscriptEntry),
	Removed { id: EntryId, row: Option<RowSeq> },
	Cleared,
}

struct OrderedDelta {
	key: ConversationKey,
	epoch: ReplicaEpoch,
	sequence: ReplicaSequence,
	delta: TranscriptDelta,
}

enum TranscriptEntry {
	User(UserMessage),
	Assistant(AssistantMessage),
	UserAttachment(UserAttachment),
	ToolCall(ToolCall),
	McpApp(McpAppDocument),
	Notice(Notice),
	VoiceCall(VoiceCallRecord),
	Event(DomainEvent),
	Feedback(FeedbackRequest),
	Unknown { id: EntryId, kind: String, raw: JsonValue },
}

enum MemoryScope {
	UserBot { account: AccountId, bot: BotId },
	TeamBot { team: TeamId, bot: BotId },
}

struct MemoryFact {
	id: FactId,
	scope: MemoryScope,
	text: String,
	learned_at_ms: i64,
}

enum RoutineTrigger {
	Cron { expression: String, time_zone: IanaTimeZone },
	Interval { every_ms: u64, phase_ms: Option<i64> },
	Webhook { public_id: String },
	Unknown(JsonValue),
}
```

`RoutineTrigger` est une proposition du nouvel équivalent. Le client montre des règles cron et interval; cette déclaration ne prétend pas que son `record_json` possède exactement ces champs. `AttachmentRef` est également une identité indépendante, pas la copie d'une URL presignée.

Les services et propriétaires proposés sont les suivants:

| Module Rust | Responsabilité unique | Contrat principal |
|---|---|---|
| `domain::conversation` | envoi, draft et intentions d'annulation | `send`, `resend`, `cancel_queued`, `interrupt`, `fresh`, `rollback` |
| `domain::replica` | epoch, sequence, coverage, buffer et baseline | `ingest_event`, `ingest_snapshot`, `reconnect` |
| `storage::journal` | outbox durable par compte et transaction de phases | append/persist/retire de PendingSend |
| `storage::transcript_cache` | seeds bornés, historique et état de restauration | lire/écrire par ConversationKey |
| `service::conversation` | acceptation durable et lecture du transcript | send/status/list/watch |
| `service::bot` | identité, sessions, rooms et managers | create/list/update/delete/access |
| `service::memory` | scopes de facts, versions et promotion | list/promote/team_summary |
| `service::routine` | plans, runs, time zone et webhook | create/update/enable/delete/run_now |
| `service::plugin` | sélection, variables, skills et grants | listing/set/variables/skill/grant |
| `service::catalog` | templates versionnés et marketplace | list/details/import/publish |
| `service::file` | upload, hash, finalisation et lecture autorisée | begin/parts/finish/abort/resolve |
| `service::voice` | credential éphémère, instructions et outils d'appel | session/tool/mint |
| `service::team` | droits, groupes, tokens d'accès et policies | membership/access_update/policy |
| `service::admin` | opérations réservées avec scope d'opérateur | gestion boxes, agents, emails, meetings |
| `adapter::connect_observed` | DTO observés et fixture hors ligne | encoder/décoder sans logique métier |
| `adapter::realtime_voice` | messages JSON et audio du fournisseur choisi | session/update/audio/tool/close |
| `ui_iced` | composition, présentation et commandes utilisateur | observe domaine, dispatch intentions |

Le service indépendant et son worker possèdent les mutations persistantes et l'autorisation. Le client possède le journal des intentions locales et l'affichage optimiste. Un acteur de réplique par ConversationKey possède l'ordre de l'UI. Un seul composant alloue un sequence dans une epoch donnée; aucun composant de présentation ne l'invente.

```rust
trait ConversationService {
	async fn send(&self, request: SendRequest) -> Result<SendReceipt, ServiceError>;
	async fn acceptance(&self, key: &ConversationKey, id: &SendId)
		-> Result<Acceptance, ServiceError>;
	async fn tail(&self, request: TailRequest) -> Result<TailSnapshot, ServiceError>;
	fn watch(&self, cursors: Vec<Cursor>) -> EventStream;
	async fn interrupt(&self, request: InterruptRequest)
		-> Result<InterruptReceipt, ServiceError>;
}

trait MemoryService {
	async fn facts(&self, scope: MemoryScope) -> Result<Vec<MemoryFact>, ServiceError>;
	async fn promote(&self, request: PromotionRequest)
		-> Result<PromotionReceipt, ServiceError>;
}

trait RoutineService {
	async fn mutate(&self, command: RoutineCommand)
		-> Result<RoutineSnapshot, ServiceError>;
	async fn run_now(&self, request: RunNowRequest)
		-> Result<RunReceipt, ServiceError>;
}
```

Ces signatures laissent les types de futur et de stream au choix d'implémentation. Aucun framework réseau n'est imposé par les preuves.

### Deux formes comparées

Une copie directe des 309 RPC GrokBot dans le core Rust exposerait la migration BOX/temporal, les ID legacy et les choix de transport à chaque écran. Elle faciliterait le rapprochement avec les descripteurs, mais rendrait la logique de reprise et d'accès diffuse. Elle dépendrait aussi de services fermés.

La forme retenue garde les DTO observés à une frontière et donne des responsabilités de domaine aux services indépendants ci-dessus. L'UI utilise ConversationKey et SendIntent. Le worker prend en charge une seule sémantique d'acceptation durable. Cette forme couvre les comportements observés et permet de changer un fournisseur de modèles ou un transport de transcript.

La taille et la disposition physique de ces services peuvent commencer dans un même backend Rust. Les modules ne nécessitent pas un déploiement par domaine. Séparer les processus n'est utile que lorsqu'une autorisation, une durée de travail ou une ressource le justifie.

## Obligations de parité et vérifications à prévoir

La table décrit les vérifications nécessaires à une future reconstruction, pas des tests exécutés sur un produit Rust inexistant.

| Situation issue du client | Résultat exigé |
|---|---|
| App fermé après acceptation avant écho | journal restauré, status interrogé, une ligne logique affichée |
| Réseau coupé avant réponse du send | conserver l'incertitude, status avant répétition |
| Réseau coupé avant dispatch | record queued conservé et draft consommé selon identité |
| Renvoi après échec, ancien écho tardif | alias de nonce réconcilié sans doublon |
| Échec de disque avant dispatch | aucun send sans record durable |
| Attachment PUT réussi, finish refusé | message non publié comme ayant une pièce jointe valide |
| Suppression et update dans une même frame | ordre de updated_seq conservé |
| Événement reçu avant baseline | buffer puis baseline approprié |
| Trou de sequence ou nouvelle epoch | resynchronisation, stale snapshot refusé |
| Corps blob indisponible | retry, cursor rabaissé, perte explicitement signalée après limite |
| CursorTooOld ou clear pendant fetch | ancienne réponse de page ignorée |
| Bot d'équipe, deux sessions utilisateur | états séparés par ConversationKey et contrôle d'accès |
| Nouvelle variante JSON inconnue | transcript lisible avec carte compatible et raw préservé |
| Promotion de mémoires partielle | created, alreadyInTeam et keptPrivateCount conservés |
| Activation de routine dans une autre zone | zone IANA explicite, calcul et exécution cohérents |
| Deux clics run-now rapprochés | un run local actif et état awaiting/running/cooldown |
| Mise à jour des droits avec état ancien | token de concurrence présenté et conflit traité |
| Reconnexion d'appel vocal | reprise de conversation, salutation non dupliquée |
| Dictée vide ou synthèse trop longue | erreur locale avant réseau |
| Compte changé pendant requête | continuation incapable d'écrire dans le nouveau compte |

La future suite de tests doit simuler ces séquences avec horloge et réseau contrôlés, puis exercer l'UI Iced avec le backend indépendant. Les deux scripts d'inventaire et de vérification sont les seuls programmes ajoutés dans ce périmètre. Ils sont relançables contre l'artefact original. Ils ne constituent pas une implémentation de Grok Bot.

## Inconnues qui limitent la reconstruction

Les méthodes et types protobuf donnent les entrées et sorties déclarées. Ils ne donnent pas la validation complète, la configuration de déploiement, les migrations SQL, les prompts, l'ordre transactionnel du serveur ou les droits réels du compte installé.

Les principaux trous restent:

- Orchestrateur distant, interruptions réellement propagées, reprise d'un run après crash, outils de modèle et prompts de planification.
- Implémentation de l'acceptation durable, fenêtre de déduplication, rétention de status et sens exact d'UNKNOWN_DURABILITY.
- Scheduler des routines, DST, rattrapage après indisponibilité, déduplication des webhooks, limites de concurrence et droits de routine par session.
- Extraction des mémoires, décision privé/équipe, classement des shards et consolidation des résumés.
- Sémantique complète des corps JSON de transcript, formats legacy et variantes présentes uniquement dans d'autres versions.
- Accès admin effectif et politiques serveur pour team bots, secrets et appels commerciaux.
- Mécanismes de validation et de révocation des credentials de voix, OAuth, plugins et upload presign.
- Disponibilité des méthodes compilées sans point d'appel observé et branches conditionnées par feature gates.

Ces fonctions doivent être conçues dans les services Rust indépendants. Elles ne peuvent pas être proclamées reproduites sur la seule base de ce bundle.
