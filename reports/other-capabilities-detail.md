# Autres capacités et délégation de Grok Bot 0.70.2

Le client confirme surtout la **gestion et le suivi des délégations**. Il crée des rooms, choisit leurs membres, lit les sous-agents en cours et rattache les tâches à leurs sessions. Les contrats décrivent aussi des messages entre bots et des tours de room corrélés par nonce. Leur moteur, les règles qui choisissent un destinataire et la propagation des interruptions ne sont pas présents dans les chemins client retrouvés.

La comparaison avec les rapports existants apporte quatre précisions de produit qui étaient absentes ou seulement nommées : comptes financiers connectés, génération d'avatars par IA, workers d'agents de code cloud, édition et partage de documents/canvas. Les autres résultats approfondissent des familles déjà inscrites dans la matrice.

## Périmètre et méthode

La cible sélectionnée est `inputs/Grok Bot.app`, version 0.70.2. Le SHA-256 de son `Contents/Resources/app.asar` est `9fe94a66942f8511281f6d7c80a42432d749284c46e1e26f5b6d2f99b87652b0`, identique au manifeste historique. Aucun lancement de l'application, appel réseau, compte ou secret n'a été utilisé. Aucun code Rust n'a été modifié. Cron/routines et accès ordinateur/shell/egress sont hors périmètre, sauf une lecture du registre local nécessaire pour vérifier si un exécuteur de sous-agents est réellement enregistré.

Le parent REA `ev_a8b822a62831e30bd01eb4c4488b36f0bd50cb8ccc41d83e2dcff4e00cf82a4d` et `evidence/rea-bundle.json` ont été réutilisés. La référence n'était pas retenue dans la connexion courante; l'import de données a rétabli les trois records, sans relancer l'analyse globale. Une trace ciblée de l'API `listGrokBotSubagents` produit `ev_e34978642eb68191368b90d330a4d1e14781bd67c65f24684a41e0310fbf390c`, avec `no-match` sur un graphe `partial`. Ce résultat ne prouve aucune absence. Les chemins ci-dessous viennent donc de lectures ciblées des bytes expédiés et des copies formatées.

L'inventaire existant comprend 7 services, 484 entrées RPC, 429 triplets RPC/types uniques, 2 313 messages et 287 enums. Cette passe compare les 484 entrées avec `README.md`, `reports/parity.md`, `reports/protocol.md`, `reports/desktop.md` et `reports/execution.md`. Elle relit aussi `evidence/investigation.json` et les 27 traces de `protocol-flows.json`. Les recherches textuelles ciblées parcourent les **600 fichiers JavaScript** de `artifacts/client`, copies de dépendances comprises. Chaque fichier de ce périmètre reçoit une empreinte dans le ledger.

| Service | Entrées RPC | Noms avec appel de propriété candidat | Noms cités littéralement dans les cinq textes de référence |
|---|---:|---:|---:|
| AgentStoreService | 24 | 6 | 0 |
| AiService | 8 | 5 | 3 |
| BackgroundComposerService | 7 | 7 | 0 |
| DashboardService | 73 | 71 | 1 |
| AnalyticsService | 8 | 3 | 0 |
| GrokBotService | 309 | 152 | 54 |
| SandBoxService | 55 | 21 | 1 |
| Total | 484 | 265 | 59 |

La dernière colonne est un repérage lexical sensible à la casse, par présence du nom RPC ou de son nom local. Les **425 entrées sans citation littérale ne sont pas 425 fonctions nouvelles**. Une famille peut être décrite sans nommer chaque méthode; un SDK compilé peut déclarer une fonction jamais appelée. La colonne des candidats vient de l'index existant et ne prouve pas l'identité du récepteur. Le ledger conserve chaque entrée, les références trouvées et les appels candidats. Les conclusions suivantes reposent sur la lecture du récepteur, du routeur ou de l'état client.

Les alias de sources sont `P = artifacts/formatted/dist/electron-main/proto.cjs`, `M = artifacts/formatted/dist/electron-main/main-app.cjs`, `A = artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js`, `C = artifacts/formatted/dist/node-agent-coordinator/main.cjs`, `D = artifacts/formatted/dist/local-exec-daemon/main.cjs`. Les lignes portent sur les copies formatées. Les SHA-256 et offsets originaux en unités UTF-16 figurent dans `evidence/other-capabilities-detail.json`; ils ne sont pas assimilés aux lignes ou aux offsets bytes.

## Délégation entre bots, sous-agents et rooms

### Le suivi des sous-agents a un chemin client concret

`p2e` construit une ressource par agent autour de `source.getSubagents({id})`. Un événement `subagents` remplace sa liste à partir de `parentAgentId`, puis la ressource gère reconnexion, reset et dispose. Le coordinator émet ces événements après une demande de rafraîchissement de la famille `subagents`. Il regroupe les rafraîchissements par agent/famille et demande une nouvelle lecture lorsqu'un changement arrive pendant la première. Sources OTH-S01 et OTH-S02, A:53030–53043 et C:42436–42510.

Pour un bot temporal, `getSubagents` appartient aux commandes routées vers le serveur. `z8e` appelle réellement `GrokBotService.listGrokBotSubagents({agentId})` et transforme chaque résultat en `subagentId`, `subagentType`, `title`, `startedAtMs`, avec **`status:"running"` ajouté par le client**. Ce résultat ne contient pas un cycle complet terminé/échoué/annulé. Sources OTH-S03 et OTH-S04, M:111028–111040 et C:39474–39491,39549–39584,43850–43861.

`getAsyncTasks` possède une autre ressource, alimentée par `parentAgentId` et `tasks`. Il ne figure pas dans les commandes protobuf du routeur `Yh` et suit la voie de commandes génériques vers la gateway. Il serait incorrect de le présenter comme un alias de `ListGrokBotTasks`. Sources OTH-S05 et OTH-S06, A:52292–52306 et C:39474–39534,41355–41470,43616–43679.

### Une tâche a un fil et une session, sans être automatiquement un sous-agent

La famille effectivement utilisée est `GrokBotAgentTask`, avec ID, nom, description, statut texte, mise à jour et entrée non lue. Le client appelle `ListGrokBotAgentTasks` avec `includeRecentlyDone:true`, puis `MarkGrokBotAgentTaskDone` et `MarkGrokBotAgentTaskRead`. Les réponses remplacent la liste. Le modèle plus ancien `GrokBotTask`, avec l'enum WORKING/NEEDS_INPUT/DONE, est déclaré mais ses méthodes `KeepGrokBotTaskList`, `ListGrokBotTasks`, `DismissGrokBotTask`, `MarkGrokBotTasksSeen` n'ont pas de candidats d'appel dans l'index. Sources OTH-S07 et les contrats réutilisés.

`NU` conserve les tâches ouvertes et celles modifiées depuis trois jours. Il maintient séparément les fils non lus et le fil ouvert. Marquer une tâche terminée ferme le fil et applique un état optimiste; une erreur relance la lecture et peut rouvrir le fil. Marquer l'entrée lue ne supprime l'indicateur que si l'ID de l'entrée correspond encore à celle affichée. Source OTH-S08, A:12595–12704.

La session de fil est `task:<taskId>` ou `task:<taskId>:<parentSessionId>`. Le parseur n'accepte qu'un ID de dix caractères hexadécimaux et une session mère `dm:<entier positif sûr>`. Le journal d'envoi transporte séparément `taskId`, `sessionId`, `replyToId` et le nonce. Le RPC d'envoi utilisateur transmet `task_id` et `session_id`. Ces champs identifient la conversation visée; ils ne prouvent pas qu'un nouvel agent est créé pour chaque tâche. Sources OTH-S09 et OTH-S10, A:10680–10704,35869–35912; M:110631–110683.

Le coordinator agrège les états par agent et session. `hasRunningSubagents` peut maintenir un bot actif même si `isRunning` est faux. `surfaceTaskId` construit les `presenceSessionIds` utilisées pour afficher le fil en cours. Les sessions `group:<roomId>` alimentent une liste séparée `groupTurns`. Les timestamps et `staleAfterMs` retirent les états périmés. C'est du suivi de présence client, pas un ordonnanceur de délégation. Sources OTH-S11 et OTH-S12, C:5150–5164,5349–5418,5528–5569.

### Le contrat d'envoi entre bots expose la corrélation, sans appel client retrouvé

`SendGrokBotAgentMessageRequest` contient `from_agent_id`, `to_agent_id`, `message_id`, `text`, `sent_at_ms` et `reply_to_id` optionnel. La réponse nomme le destinataire et peut fournir un `workflow_id`. L'enum de livraison distingue temporal, box, duplicate, target not found, forbidden, box unreachable, temporal unavailable et invalid target. Il n'y a pas de `session_id` dans ce request. Sources OTH-S13, P:56672–56705 et enum réutilisé dans le ledger.

Dans les 600 fichiers parcourus, le nom local `sendGrokBotAgentMessage` apparaît **trois fois**, une fois dans chaque descripteur de `proto.cjs`, du coordinator et du daemon. Aucun appel direct ni implémentation d'envoi sous ce nom n'a été retrouvé. La présentation connaît pourtant les aperçus `sent_agent_message` et `received_agent_message`, avec destinataire ou expéditeur et texte. Un aperçu reçu du serveur ne révèle pas la politique qui a déclenché l'envoi. Source OTH-S14, A:29363–29377 et M:116811–116826.

Conclusion de certitude élevée sur l'artefact : le contrat est compilé et le client sait afficher son résultat. Conclusion de certitude moyenne : l'initiative de la délégation appartient aux services distants, car aucun initiateur client concret n'a été retrouvé. Le choix du bot, les budgets, cycles, retries, attentes et héritages de contexte restent inconnus. Les appels calculés dynamiquement ou un autre artefact ne sont pas exclus par cette recherche.

### Les rooms ont une gestion active, les tours ont surtout un contrat

`qYe.createGroup` exige une route temporal et un nonce, fixe le scope compte/backend, appelle `CreateGrokBotRoom` avec les IDs de bots et d'humains, puis vérifie l'identité et le kind ROOM de l'agent retourné. `setGroupMembers` appelle `SetGrokBotRoomMembers` et refuse une réponse qui désigne une autre room. Des bots non temporal peuvent emprunter la gateway pour une room sans humains; une room avec humains exige une route serveur appropriée. Sources OTH-S15 et OTH-S16, M:100678–100764 et C:43680–43744.

Le contrat de tour transporte un `nonce`, la room, `member_agent_id`, les pairs, les nouveaux messages avec speaker/reply target, `is_winding_down`, une deadline, `parent_request_id` et `root_parent_request_id`. La réponse de départ distingue accepted/duplicate et plusieurs refus. L'annulation vise `nonce` et membre, avec raison. Le retour apporte room, nonce, membre, outcome, messages, erreur et posts structurés; l'intake distingue accepted, unknown nonce et host unavailable. Sources OTH-S17, P:59072–59381 et enums détaillés dans le ledger.

Les outcomes incluent SENT, PASS, SKIPPED, TIMEOUT, CANCELLED et ERROR. Ce protocole prévoit donc un membre qui passe son tour, un délai et un résultat corrélé. Pour chacun des trois noms Request/Cancel/Deliver, les recherches retrouvent seulement les trois descripteurs, sans appel client actif. Le serveur qui distribue les tours, les règles de quorum, l'ordre et la propagation d'une annulation ne sont pas reconstruits.

L'interruption utilisateur active appelle `InterruptGrokBotAgentRun` avec agent, raison `user_interrupt` et session optionnelle. Elle ne fournit ni sous-agent cible ni nonce de tour de room. Il n'est pas établi qu'elle annule tous les enfants. Source OTH-S18, M:110695–110711.

### Les adaptateurs SDK du daemon ne prouvent pas une délégation locale

Le daemon compile `SubagentArgs`, avec modèle, prompt, readonly, resume/fork, background, interrupt, parent/root parent, environnement et credentials. Il compile aussi des ressources `subagent-executor`, `subagent-await-executor` et `force-background-subagent-executor`. Leur constructeur `rt` retourne seulement un objet décrivant une implémentation distante et une fonction d'enregistrement; il ne les enregistre pas. Les symboles `oPt`, `aPt` et `cPt` n'ont qu'une occurrence chacun dans D. Le registre concret `oYe`, appelé par `P4`, n'enregistre aucune de ces trois ressources. Sources OTH-S19, OTH-S20 et OTH-S21, D:93411–93457,97638–97646,98545–98575,128042–128099.

Cette passe ne décrit donc pas le SDK comme un moteur local de sous-agents actif. Une implémentation distante ou un autre binaire peut l'utiliser; cette disponibilité n'est pas établie ici.

### Les workers de code cloud constituent une autre forme de délégation

`G7e` construit réellement un client `BackgroundComposerService`. `F7e` lit l'agent cloud par `bcId`, ses branches et PR, son statut, ses fichiers et éventuellement ses artefacts. Quand le composer possède des `projectMetadata`, `Pmn` appelle `ListWorkersForManager({managerBcId})` et compte les memberships CREATING ou RUNNING. Les PR liées à un `workerBcId` sont regroupées comme fils de workers. Sources OTH-S22 et OTH-S23, M:111794–111841,112033–112091.

La lecture des diffs utilise `GetOptimizedDiffDetails`; les résumés et artefacts utilisent `GetTurnSummaryBackgroundComposer` puis `ListBackgroundComposerArtifacts`. Cette preuve confirme une vue manager/workers distincte de `ListGrokBotSubagents`, qui renvoie des IDs de sous-agents Grok Bot. Elle ne retrouve pas ici le code qui crée, délègue ou arrête un worker. Sources OTH-S24, M:111957–112032.

## Capacités nouvelles ou mieux établies

| Capacité | État antérieur | Apport de cette passe | Limite |
|---|---|---|---|
| Comptes financiers connectés | Absents des rapports de référence | Appels Dashboard actifs, lecture comptes/connections, suppression, linking hébergé et réparation | Aucun mécanisme bancaire serveur ni accès effectif observé |
| Génération d'avatars IA | Profil/création de bots traités globalement | Générateur actif, modèle par défaut, prompt spécialisé et ratio 1:1 | Pas une preuve d'un outil général de génération d'images |
| Manager/workers de code cloud | Sous-agents seulement au niveau contrat | Lecture active des workers, PR, diffs, résumés et artefacts | Création et ordonnanceur distants inconnus |
| Documents et canvas partagés | Lecture nommée dans fichiers | Écriture markdown, visibilité, renommage canvas et adresse publique | Permissions serveur et génération de contenu inconnues |
| Meetings | Contrat partiel | Listing/leave/record actifs, gate, cache et structure notes/transcript | Capture locale et moteur de réunion non retrouvés |
| Teach recording | Trace client déjà inscrite | UI lazy save/discard, limite 10 min, séquencement et nettoyage | Capture et fabrication de skill derrière la gateway |
| Apps hébergées | Trace client déjà inscrite | Suivi des propositions, états off/not_owner, polling et relecture | Publication distante et transaction provider inconnues |
| Dépenses/cartes | Trace client déjà inscrite | Résultats needs_auth/passkey/enrollment et ceremony status | Pas de débit ni de ledger bancaire observé |
| Credentials | 1Password et formulaires déjà traités | Nom générique provider relié concrètement à 1Password; vault indépendant | Pas un catalogue de plusieurs providers prouvé |
| Email, Slack, Teams, administration | Familles déjà nommées | Frontière appels actifs/descripteurs clarifiée | Compte, droits et fonctions serveur inconnus |

### Finance et avatars

`w8e` lit en parallèle `ListFinancialConnections` et `ListFinancialAccounts`, regroupe les comptes par connection et conserve les comptes dont la connection n'est pas dans la première réponse. Les catégories sont cash, investment, liability ou unknown. Le client conserve masque, sous-type, solde texte et monnaie. `DeleteFinancialConnection` supprime une connexion. `StartFinancialLink` accepte `repairLinkedBank` et un retour vers `https://cursor.com/grok-bot/link/v1/finance`, puis fournit une URL hébergée. Le handler vérifie cette URL avec son parseur autorisé, l'ouvre et retourne `completionUnconfirmed:true`. Le retour "started" ne vaut donc pas liaison bancaire réussie. Sources OTH-S25 et OTH-S26, M:109524–109561,113460–113503. Aucun lien n'a été ouvert pendant l'enquête.

`createAvatarImageGenerator` installe `AiService.RunGenerateImage` avec `SAND_DEFAULT_MODEL_ID` et ratio `1:1`. Le handler ajoute un prompt précis pour un avatar recadré en cercle, sans texte/logo, puis reçoit les données base64 et les fait normaliser par son décodeur. Le contrat sait accepter des références et une option maxMode, mais ce chemin d'avatar ne les utilise pas. Sources OTH-S27, OTH-S28 et OTH-S29, M:109342–109428,113824–113833,114386–114418.

### Documents, canvas et bibliothèque

`qWe.saveDocument` appelle `WriteAgentStoreDocument` avec store/document et source markdown. Le changement de visibilité appelle `SetAgentStoreDocumentVisibility` avec private ou team-view; le client exige un echo reconnu et traduit certains refus en lecture seule. La lecture de room utilise `ReadGrokBotGroupChatDocument`, avec des URL d'assets expirantes gardées dans un cache borné. Sources OTH-S30 et OTH-S31, M:94619–94723,94742–94784.

Le client canvas appelle `GetCanvasPayload`, `ReadGrokBotGroupChatCanvas`, `SetCanvasVisibility`, `SetPageAddress` et `RenameCanvas`. Il vérifie les identités store/canvas et la réponse de visibilité; il exige une page URL non vide après attribution d'un slug. `ListGrokBotLibraryDocs` apporte une bibliothèque de documents touchés ou créés par bot, et `ReadGrokBotMcpAppDocument` relit du HTML et metadata par agent/session/entry. Sources OTH-S32, OTH-S33, OTH-S34 et OTH-S35, M:94217–94254,94287–94360,95484–95520,112355–112386. Ces opérations précisent les familles documents/MCP existantes; elles ne prouvent pas un moteur distant complet de génération de documents.

### Meetings, teach et publication

`N_` appelle effectivement list/leave/get record avec agent et meeting, sous timeout 10 s. Un record contient meeting status, machine, nom, failure, channelClosed, notes avec version temporelle/segment couvert, fenêtres de résumé et transcript texte horodaté. `Ej.desktop` installe ce client derrière `sand_pitman`, initialement désactivé, et retourne une liste en cache en cas d'échec de lecture. Le `sinceMs` est fixé à la création du client. Sources OTH-S36 et OTH-S37, M:103675–103762,114874–114931. `PullGrokBotMeetings` et `ReportGrokBotMeetings` restent sans appel direct retrouvé. Les noms ne suffisent pas à prouver une capture audio locale.

La ressource `jke` de teach utilise une limite de **600 000 ms**, actualise son horloge chaque seconde, sérialise start/stop, réconcilie après erreur/reconnexion et tente un stop `save:false` au reset. La UI lazy contient des actions distinctes `stop(P,true)` et `stop(P,false)`. Les trois commandes de teach figurent dans le contrat de gateway et suivent son POST générique. Aucun handler local de capture ni algorithme qui transforme la démonstration en skill n'est retrouvé. Sources OTH-S38, OTH-S39 et OTH-S40, A:57469–57664, chunk `chunk-teach-recording-controls-DRP1S1on.js` aux offsets enregistrés, C:39240–39242,41355–41470.

La ressource de publication appelle list apps/get proposals/confirm/dismiss/update. Elle distingue off et not_owner, ingère le résultat d'une décision, relit les propositions et les apps, et suspend les lectures périodiques quand la fenêtre est invisible. Les commandes sont déclarées dans C:39094–39117 mais ne figurent pas dans le routeur protobuf `Yh`; leur implémentation de déploiement reste derrière la gateway. Source OTH-S41, A:56270–56477. La trace UI présente dans la matrice ne valide donc ni un provider d'hébergement ni une transaction de déploiement.

### Dépenses et credentials

Le chemin carte appelle `ResolveGrokBotVirtualCardApproval`, `BeginGrokBotSpendApproval` et `ListGrokBotStripeLinkPaymentMethods`. Il transmet un challenge ID dans l'attestation et exige un spendRequestId pour interpréter une réponse APPROVED comme approuvée. NEEDS_AUTH, NEEDS_PASSKEY, absence de credential inscrit et FAILED donnent des états différents. Le client gère l'inscription de passkey et lit `GetPasskeyCeremonyStatus` pour pending/completed/failed/expired. `FinishGrokBotSpendApprovalAssertion` reste déclaré sans candidat direct. Sources OTH-S42 et OTH-S43, M:110449–110502,110867–110905,118345–118370. La finalisation dans une page hébergée reste hors artefact analysé.

Les noms `syncCredentialProvider`, `approveCredentialRequest` et `connectCredentialProviderOAuth` correspondent concrètement à l'implémentation 1Password. `put` construit un client GrokBotService et appelle start/store key/complete OAuth, begin/complete connection, get/sync/delete et approve/deny OnePassword. Le catalogue transporte génération, expiration, renewBy/reminder, état lifecycle, révision du catalogue et règles de cible host/port ou domaine enregistrable. Ce détail manquait, mais ne démontre pas plusieurs fournisseurs interchangeables. Source OTH-S44, M:117728–117934.

Un autre chemin `yXe` gère le vault Grok Bot par list/put/delete. La lecture de ses métadonnées n'inclut pas de mot de passe; le request de put peut en transmettre un selon le contrat. Il ne faut pas confondre ce vault avec le catalogue 1Password. Source OTH-S45, M:104521–104548. Aucun contenu du vault n'a été lu.

### Email, intégrations et administration

Le handler email `b9e` liste et crée effectivement des inboxes. Les opérations de send/search/read/delete d'email sont présentes dans les descripteurs mais sans candidats directs dans l'index. Une création de boîte n'établit pas le fonctionnement complet du serveur email. Source OTH-S46, M:102793–102807.

Dans GrokBotService, 5 des 6 méthodes dont le nom contient Slack ont des candidats, contre **0 des 6 méthodes Teams**. Pour les opérations d'opérateur, les **41 noms commençant par Admin** et les **29 noms finissant par Internal** ont tous zéro candidat direct. Leur présence est une couverture de contrat, pas un accès admin actif ou la preuve d'un client Teams complet. Ces comptes portent uniquement sur les 309 entrées GrokBotService, sans ajouter les doublons de SandBoxService. La gestion ACL, le token de concurrence, les templates et la mémoire d'équipe étaient déjà expliqués dans `protocol.md`; ils ne sont pas recomptés comme nouveautés.

## Ce qui reste inconnu

La sélection des bots délégués, les prompts transmis, l'héritage et la compaction de leur contexte, les budgets, l'anti-cycle, les garanties de livraison et la propagation de l'annulation restent à établir côté serveur. Les contrats de nonce et de deadline permettent de décrire des attentes, pas leur mise en œuvre. La création/lifecycle des workers cloud et la finalisation de leur résultat restent également distants.

La capture réelle de teach ou de réunion, les algorithmes de notes et de skill, la transaction d'hébergement, l'émission et le débit des cartes, la synchronisation bancaire, les scopes effectifs et les droits d'administration ne sont pas prouvés par cette passe statique. Des branches calculées dynamiquement peuvent échapper aux recherches par nom.

Cette enquête n'affirme aucune exhaustivité du backend. Les additions concrètes sont les quatre mécanismes de produit détaillés plus haut; teach, meetings, hosting, spending, credentials, rooms et tasks complètent des familles déjà traitées. La nouveauté la plus utile pour la délégation est la séparation explicite entre **contrats bot-vers-bot, suivi de sous-agents Grok Bot, fils de tâches, tours de room et workers de code cloud**.
