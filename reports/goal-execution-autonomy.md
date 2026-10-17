# Actions incertaines, boucle d’outils et limites d’autonomie

Cette passe répond aux sujets 3, 4 et 6 de Grok Bot 0.70.2. Le client possède des reprises spécialisées pour les messages, une protection Messages limitée à la vie du daemon et une boucle vocale retracée jusqu’à ses consommateurs. Les sources ne révèlent pas un moteur texte local qui replanifie jusqu’à réussite avec un budget de tokens.

La cible est la copie sauvegardée dans `inputs/Grok Bot.app`. Les originaux sont sous `artifacts/client`; les lignes citées appartiennent aux copies LF sous `artifacts/formatted`. Le [ledger](../evidence/goal-execution-autonomy.json) garde les SHA-256 des originaux, les empreintes des copies et les extraits. Le parent REA réutilisé est `ev_a8b822a62831e30bd01eb4c4488b36f0bd50cb8ccc41d83e2dcff4e00cf82a4d`. Son graphe reste partiel. Les nouvelles lectures shell ne sont pas de nouveaux Evidence IDs REA. Aucun lancement cible, outil cible, compte, secret vivant ou appel réseau n’a été effectué.

Les [contrats](protocol.md), [exécuteurs](execution.md), [cycles de délégation](delegation-lifecycle-deep.md) et [runtimes](compaction-runtimes.md) restent les acquis. Les constats 1, 2 et 5 précisent des mécanismes déjà connus. Les nouveautés principales sont la perte possible des résultats après effet, le rôle des ACKs de livraison, les deadlines MCP actives, le lien exact de la boucle vocale, l’arbitrage de parole et la concurrence locale. Les valeurs de repli restent des déclarations. Les recherches négatives couvrent les 600 fichiers JavaScript originaux, avec chemins, SHA-256, offsets UTF-16 et motifs dans le ledger.

Chaque constat suit un consommateur concret ou nomme la frontière absente. Observation signifie lecture statique d’un appel installé, jamais exécution observée. Contrat et configuration ne prouvent aucun enregistrement actif.

## 1. L’acceptation perdue est recherchée avant un renvoi

Le performer backend rejoue `sendPrompt` après une erreur de transport seulement si `GetGrokBotSendStatus` retourne `not-found`. Les statuts inconnus deviennent `unknown-durability` et ne satisfont pas ce test. Le journal renderer recherche aussi les nonces précédents. Un nonce trouvé sans rejet passe en `accepted-awaiting-echo`; un écho connu confirme l’affichage et rafraîchit le transcript. L’acceptation de la commande reste distincte de son effet métier.

Preuve d’observation statique. Sources [A1 main-app.cjs:111086](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/main-app.cjs:111086), [A2 main-app.cjs:110329](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/main-app.cjs:110329), [A3 index.eager-app-BfVhIMYf.js:71426](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:71426).

Limite. La durabilité réelle du status et la déduplication serveur ne sont pas inspectables dans le client. Le journal ne prouve pas un effet unique des outils exécutés ensuite.

## 2. La gateway limite le replay d’un POST déjà commencé

La gateway garde `postStarted` et l’URL tentée. Avec un nonce non vide, elle autorise au plus un retry de sendPrompt si le POST n’a pas commencé ou si l’URL figure dans `sendDedupeProvenBaseUrls`. Une réponse `accepted:true` ajoute l’URL à cet ensemble. Après dispatch, un changement d’URL bloque le retry. La création d’agent possède un autre nonce et une autre preuve, issue du header `x-sand-mint-dedupe`.

Preuve d’observation statique. Sources [A4 main.cjs:41665](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/node-agent-coordinator/main.cjs:41665), [A5 main.cjs:41718](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/node-agent-coordinator/main.cjs:41718), [A6 main.cjs:41543](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/node-agent-coordinator/main.cjs:41543), [A7 main.cjs:41408](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/node-agent-coordinator/main.cjs:41408).

Limite. Ces ensembles sont en mémoire et définissent une règle client par URL. Ils ne prouvent ni le stockage serveur des nonces ni la transaction des outils ultérieurs.

## 3. Un ACK de livraison ne confirme pas l’effet

Le provider serveur ajoute l’ID de l’enveloppe à `seen` et `pendingAcks` avant de décoder la demande. Après un poll réussi avec ces ACKs, il retire les IDs de `seen`. Le `requestId` de l’action vient du frame, tandis que le `sendKey` Messages est un champ séparé. ID de livraison, ID de demande et clé d’effet ont ainsi des consommateurs distincts.

Preuve d’observation statique. Sources [A8 main.cjs:155637](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/local-exec-daemon/main.cjs:155637), [A9 main.cjs:155858](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/local-exec-daemon/main.cjs:155858).

Limite. seen protège une livraison encore non acquittée dans cette instance. Il n’enregistre pas les effets terminés, et son retrait ne confirme pas la remise du résultat au demandeur.

## 4. Un shell ou un upload terminé peut recommencer

Dans `P4`, `withInflight` ignore seulement un requestId déjà actif et nettoie sa map en finally. Le shell reçoit ensuite un nouvel ID interne monotone avant `execManager.handle`. L’upload appelle `R2` puis produit son résultat sans consulter les caches Messages. R2 sérialise l’écriture par chemin; cette sérialisation ne mémorise pas les demandes terminées.

Preuve d’inférence statique. Sources [A10 main.cjs:128142](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/local-exec-daemon/main.cjs:128142), [A11 main.cjs:128309](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/local-exec-daemon/main.cjs:128309), [A12 main.cjs:128368](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/local-exec-daemon/main.cjs:128368), [A13 main.cjs:78997](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/local-exec-daemon/main.cjs:78997).

Limite. Inférence bornée à P4. Après nettoyage de l’ID actif, ces branches peuvent recommencer si le transport remet la demande. Réécrire les mêmes octets peut converger au même contenu; un shell arbitraire n’est pas rendu idempotent.

## 5. Messages réutilise certains résultats pendant la vie du daemon

Pour send, `withSendKey` sérialise une même clé. `replayedSend` exige que JSON.stringify(op) corresponde. `finishedSends` garde jusqu’à 64 résultats par request ID; `sentByKey` garde jusqu’à 64 succès par send key. Les erreurs et consentements restent dans le premier cache. `messages-accepted` peut partir avant la décision de permission, donc cette trame ne prouve pas l’envoi natif.

Preuve d’observation statique. Sources [A14 main.cjs:128217](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/local-exec-daemon/main.cjs:128217), [A15 main.cjs:128101](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/local-exec-daemon/main.cjs:128101), [A10 main.cjs:128142](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/local-exec-daemon/main.cjs:128142), [A16 main.cjs:128348](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/local-exec-daemon/main.cjs:128348).

Limite. Les caches disparaissent au redémarrage et leur éviction réduit la fenêtre de replay. Cette branche ne transmet pas le signal d’annulation à messages.run. Les garanties du helper natif restent hors preuve.

## 6. La remise du résultat peut échouer après l’effet

Le provider retire un batch de l’outbox puis réessaie `SubmitGrokBotUserComputerResponses` jusqu’à trois fois. Chaque tentative utilise la deadline de données de 120 secondes. Après épuisement, il journalise l’abandon du batch. Une credential refusée replace le batch en tête et force sa réauthentification. Ces essais soumettent les mêmes frames de réponse; ils ne rappellent pas l’exécuteur.

Preuve d’observation statique. Sources [A17 main.cjs:155733](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/local-exec-daemon/main.cjs:155733), [A18 main.cjs:155180](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/local-exec-daemon/main.cjs:155180), [A19 main.cjs:155240](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/local-exec-daemon/main.cjs:155240), [A20 main.cjs:81909](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/local-exec-daemon/main.cjs:81909).

Limite. Les files sont en mémoire. Aucun rollback d’effet n’est rattaché à cet échec de submit. Le serveur peut avoir reçu un essai dont la réponse s’est perdue; la signification transactionnelle de acceptedCount reste distante.

## 7. Le timeout MCP conserve l’incertitude d’effet

Le manager desktop construit la discovery et expose executeToolCall. La discovery route les outils HTTP vers `backendMcpExec.executeTool`. Le handler appelle `ExecuteSandMcpTool` avec toolCallId, agentId, turnId et eventSequence. Sa deadline normale est de 3 minutes. `notion-create-pages` et `create_anyship_deployment` figurent dans la liste qui porte la deadline à 45 minutes, si le nom normalisé du handler correspond. Sur DeadlineExceeded, il retourne une erreur disant que le connector a peut-être déjà appliqué l’action. Aucun replay ne figure dans ce catch. L’entrée directe fabrique un nouvel UUID à chaque appel.

Preuve d’observation statique. Sources [A75 main-app.cjs:113550](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/main-app.cjs:113550), [A22 main-app.cjs:78912](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/main-app.cjs:78912), [A21 main-app.cjs:64555](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/main-app.cjs:64555), [A24 main-app.cjs:42512](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/main-app.cjs:42512), [A25 main-app.cjs:64810](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/main-app.cjs:64810), [A23 main-app.cjs:79105](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/main-app.cjs:79105).

Limite. Ces IDs prouvent une corrélation, sans démontrer une clé d’idempotence durable. Le choix du modèle de rappeler l’outil et les effets distants restent inconnus.

## 8. La publication relit les propositions après décision

Le consommateur appelle `confirmCloudAgentPublishProposal`, `dismissCloudAgentPublishProposal` et `updateCloudAgentPublishApp`, puis ingère la décision et relit les propositions. La confirmation transporte agentId, bcId, proposalId, slug et préférences de confirmation, sans nonce d’opération dans ce validator. Ces commandes passent par la voie gateway générique, sans politique dédiée de replay comme sendPrompt. Le polling observe les propositions et suspend ses lectures lorsque la fenêtre est invisible.

Preuve d’observation statique. Sources [A26 index.eager-app-BfVhIMYf.js:56432](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:56432), [A28 main.cjs:39094](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/node-agent-coordinator/main.cjs:39094), [A29 main.cjs:41355](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/node-agent-coordinator/main.cjs:41355), [A27 index.eager-app-BfVhIMYf.js:56335](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:56335).

Limite. proposalId peut servir à dédupliquer côté serveur, mais aucune garantie n’est prouvée. L’absence de nonce est bornée au validator cité. Le provider d’hébergement et la transaction de déploiement sont distants.

## 9. Un spendRequestId approuvé n’est pas un reçu de débit

Le performer transmet requestId et paymentMethodId à `BeginGrokBotSpendApproval`. La réponse devient un challenge si challengeId et ceremonyToken existent. `ResolveGrokBotVirtualCardApproval` transmet l’ID du challenge attesté. Le résultat APPROVED n’est exposé comme approuvé que si spendRequestId existe. Le wrapper backend du constat 1 limite son replay à sendPrompt et ne rejoue pas ces branches dans ce chemin.

Preuve d’observation statique. Sources [A30 main-app.cjs:110867](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/main-app.cjs:110867), [A31 main-app.cjs:110457](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/main-app.cjs:110457), [A32 main-app.cjs:110496](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/main-app.cjs:110496), [A1 main-app.cjs:111086](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/main-app.cjs:111086).

Limite. Les sources prouvent une décision et un challenge. Le débit, la consommation unique de l’attestation et le traitement d’une résolution répétée restent au backend ou à la page hébergée.

## 10. La voix installe une vraie table d’outils

Le bridge desktop construit `DesktopVoiceCallHarness` avec gateway et backend. Le choix peut employer le harness backend ou celui du package. Le package installe send_task, recall_text_messages, end_the_call et, selon le gate, stay_silent. Pour un package temporal, send_task va au backend. `Kv.callTool` trouve le handler par nom, valide l’entrée et parse la sortie. La session prend les descripteurs de cette table et transmet ses tools à `Kn.sessionUpdateFrame`.

Preuve d’observation statique. Sources [A34 main-app.cjs:81855](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/main-app.cjs:81855), [A33 main-app.cjs:48465](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/main-app.cjs:48465), [A35 index.eager-app-BfVhIMYf.js:75644](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:75644), [A36 main-app.cjs:38116](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/main-app.cjs:38116), [A37 main-app.cjs:40994](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/main-app.cjs:40994), [A38 index.eager-app-BfVhIMYf.js:42002](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:42002), [A39 index.eager-app-BfVhIMYf.js:80418](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:80418).

Limite. Les gates effectifs et les outils renvoyés par le backend ne sont pas observés. Cette table sert la voix; elle ne constitue pas le moteur texte Grok distant.

## 11. La boucle vocale relie appel, résultat et tour suivant

Le parseur reçoit `response.function_call_arguments.done` avec callId, nom et arguments. Le router appelle onToolCall. Le store appelle le harness et conserve le résultat sous l’ID de l’outil. Si la session est encore courante, `sendToolResult` écrit un function_call_output avec ce callId. Une sortie spoken demande ensuite une réponse; une sortie silent ne l’invite pas. `sendResponseCreate` émet finalement response.create.

Preuve d’observation statique. Sources [A40 index.eager-app-BfVhIMYf.js:79622](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:79622), [A41 index.eager-app-BfVhIMYf.js:79837](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:79837), [A42 index.eager-app-BfVhIMYf.js:41259](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:41259), [A43 index.eager-app-BfVhIMYf.js:41154](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:41154), [A44 index.eager-app-BfVhIMYf.js:79985](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:79985), [A45 index.eager-app-BfVhIMYf.js:80059](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:80059).

Limite. Le client exécute le choix reçu du modèle distant; il ne calcule pas ce choix. Le contrôle de session évite l’envoi sur une ancienne session, sans annuler un effet du harness déjà effectué.

## 12. send_task sépare receipt, acceptation et résultat ultérieur

Le store rejette une request vide ou dépassant 2 000 unités de longueur JavaScript après trim. Il renvoie le receipt d’une demande valide à la voix avant d’attendre `harness.callTool(send_task)`. Seul relay:accepted ajoute la request à acceptedRequests. Un résultat ultérieur arrive par ingestInboundNudge ou par le transcript d’un teammate, devient un nudge outcome et alimente la voix. Les frames work_landed servent aussi aux progrès; landed ne signifie pas à lui seul que toute la tâche est terminée.

Preuve d’observation statique. Sources [A46 index.eager-platform-DUN5VkaE.js:10935](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index.eager-platform-DUN5VkaE.js:10935), [A47 index.eager-app-BfVhIMYf.js:41349](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:41349), [A48 index.eager-app-BfVhIMYf.js:41180](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:41180), [A49 index.eager-app-BfVhIMYf.js:41754](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:41754), [A50 index.eager-platform-DUN5VkaE.js:11117](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index.eager-platform-DUN5VkaE.js:11117).

Limite. Le rapprochement des résultats et l’errand sont distants. Ce receipt ne prouve pas l’acceptation, et aucun replay durable de send_task n’est établi ici.

## 13. L’arbitrage local replanifie la parole

`N1` distingue une ligne libre, occupée, tenue par le caller ou fermée. Une demande de réponse attend quand la ligne est occupée. Les demandes en attente peuvent être fusionnées. La fin de la réponse ou de la parole relance la demande restante. Une réponse spontanée du serveur peut porter le résultat déjà attendu. Le handler actif stay_silent choisit entre silence et instructions de parole selon theyJustTalked, newsOwed, receiptOwed et alreadySpokeThisTurn.

Preuve d’observation statique. Sources [A51 index.eager-platform-DUN5VkaE.js:12211](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index.eager-platform-DUN5VkaE.js:12211), [A52 index.eager-platform-DUN5VkaE.js:12324](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index.eager-platform-DUN5VkaE.js:12324), [A53 main-app.cjs:40200](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/main-app.cjs:40200).

Limite. Ces consommateurs ordonnent des tours parlés. Ils ne choisissent pas un plan métier, ne comptent pas les tours de l’agent texte et ne vérifient pas l’objectif utilisateur.

## 14. Réponse finie, appel fini et tâche finie sont distincts

`response.done` arrête l’état agent-speech et appelle onAssistantResponseEnded. end_the_call attend la fin de l’audio parlé, puis raccroche. Le chemin de fin d’appel temporal peut encore transmettre un message de clôture et une request restante via send_task. Les DTO de tâches distinguent WORKING, NEEDS_INPUT et DONE. `KeepGrokBotTaskListRequest` expose séparément is_awaiting_user, has_background_work, turn_end et finished_work. Ses fins de tour sont FINISHED, STOPPED et SUPERSEDED.

Preuve d’observation statique. Sources [A54 index.eager-app-BfVhIMYf.js:79738](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:79738), [A55 index.eager-app-BfVhIMYf.js:41318](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:41318), [A56 index.eager-app-BfVhIMYf.js:41675](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:41675), [A57 proto.cjs:45721](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/proto.cjs:45721), [A58 proto.cjs:50443](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/proto.cjs:50443).

Limite. Ces statuts ne révèlent pas le prédicat distant qui décide DONE. Aucun appel direct keepGrokBotTaskList n’a été trouvé dans le corpus. La fin de l’appel ne prouve pas l’arrêt du travail relayé.

## 15. La deadline shell peut terminer le foreground ou laisser un background

Le handler shell installé atteint `R4.execute`. Quand timeout vaut zéro sans background ni hardTimeout positif, vhe choisit 30 secondes. Le foreground arme un AbortController à cette échéance. Le mode BACKGROUND emploie un autre signal et peut adopter l’itérateur dans backgroundShellManager. Un hardTimeout positif arme un second timer et peut annuler le shell adopté. Deadline du RPC et deadline d’exécution ont donc des portées distinctes.

Preuve d’observation statique. Sources [A62 main.cjs:128042](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/local-exec-daemon/main.cjs:128042), [A59 main.cjs:126959](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/local-exec-daemon/main.cjs:126959), [A60 main.cjs:127277](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/local-exec-daemon/main.cjs:127277), [A61 main.cjs:127414](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/local-exec-daemon/main.cjs:127414).

Limite. Les valeurs non nulles viennent de la demande, pas d’un budget global d’autonomie. L’arrêt de tous les descendants et l’application native des signaux ne sont pas observés.

## 16. Les limites de sorties actives ne sont pas des budgets de tokens

Le `Wg` installé borne stdout et stderr émis à 1 MiB chacun. Un chunk dépassant la borne produit un marqueur trimmed; le processus peut continuer. P4 choisit 100 MiB comme maxFileBytes par défaut. L’upload vérifie les bytes, le download vérifie le stat et le read enregistré vérifie le fichier avant son exécuteur. Ces contrôles ont des consommateurs actifs identifiés.

Preuve d’observation statique. Sources [A62 main.cjs:128042](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/local-exec-daemon/main.cjs:128042), [A63 main.cjs:107329](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/local-exec-daemon/main.cjs:107329), [A64 main.cjs:128117](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/local-exec-daemon/main.cjs:128117), [A12 main.cjs:128368](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/local-exec-daemon/main.cjs:128368), [A65 main.cjs:128078](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/local-exec-daemon/main.cjs:128078).

Limite. Le défaut peut être remplacé dans P4. Un stat préalable ne borne pas une lecture contre un fichier qui grossit ensuite. Aucune conversion de ces tailles en tokens modèle n’est prouvée.

## 17. Huit requêtes locales actives et 64 en attente bornent un provider

Dans le provider serveur `mj`, exec, upload, download, Messages et discovery comptent comme du travail. À huit requêtes actives, le poll place les suivantes en backlog. À 64 entrées en backlog, il émet busy. Les cancel restent traités sans prendre de slot de travail. La fin d’une demande démarre les éléments suivants tant que la borne de huit le permet.

Preuve d’observation statique. Sources [A18 main.cjs:155180](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/local-exec-daemon/main.cjs:155180), [A66 main.cjs:155673](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/local-exec-daemon/main.cjs:155673).

Limite. Cette borne appartient à la voie serveur du provider. Elle ne borne ni les enfants Grok distants, ni tous les transports alternatifs, ni les shell déjà adoptés en background.

## 18. Les valeurs de repli ne prouvent pas une limite d’autonomie

Les schémas toolCallIdleTimeoutMs et toolCallMaxTotalTimeoutMs ont des replis de 120 secondes et 60 minutes, avec boundedTimeout activé. La recherche complète retrouve ces clés dans leurs schémas et replis, sans consommateur actif retracé. `cloud_agent_stream_conversation_turn_window` a maxTurns:null en repli. Son nom vise une fenêtre de conversation de stream. `ReadAgentTranscriptArgs.max_turns` appartient aussi à une lecture de transcript.

Preuve de configuration embarquée. Sources [A67 main-app.cjs:57739](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/main-app.cjs:57739), [A68 main-app.cjs:60390](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/main-app.cjs:60390), [A69 main-app.cjs:59327](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/main-app.cjs:59327), [A70 main-app.cjs:60661](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/main-app.cjs:60661), [A71 proto.cjs:23213](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/proto.cjs:23213).

Limite. Les valeurs effectives et les consommateurs dynamiques ou distants restent inconnus. maxTurns null ne démontre pas une autonomie illimitée. Une borne de lecture de transcript ne constitue pas une limite de tours exécutés.

## 19. Le SDK représente un goal sans révéler son moteur ni son budget

`agent.v1.GoalStatus` expose ACTIVE, PAUSED, COMPLETE et CLEARED. CreateGoalArgs porte seulement objective; UpdateGoalArgs porte seulement status. GoalContinuationAction est vide. Aucun appel direct createGoal ou updateGoal n’a été trouvé dans le corpus. Le constructeur actif oYe enregistre shell foreground, shell background, read et ls, sans installer ces handlers goal. Aucun littéral tokenBudget ou token_budget n’a été trouvé dans les fichiers JavaScript examinés.

Preuve de contrat du SDK. Sources [A72 proto.cjs:22738](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/proto.cjs:22738), [A73 proto.cjs:22851](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/proto.cjs:22851), [A74 proto.cjs:27532](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/proto.cjs:27532), [A62 main.cjs:128042](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/local-exec-daemon/main.cjs:128042).

Limite. Ce sont des contrats du SDK partagé. Ils ne prouvent pas un goal Grok actif, une continuation automatique, un compteur de tokens, un maximum de tours ou un critère de réussite. Le moteur texte derrière box ou temporal reste la frontière absente.

## Scénarios dérivés statiquement

Ces scénarios suivent les branches citées. Aucun n’a été exécuté.

| Situation | Conséquence prouvée dans le client | Ce qui reste inconnu |
|---|---|---|
| Le backend accepte un prompt, puis la réponse se perd. Le status devient ACCEPTED. | Le wrapper ne rejoue pas le send. Le journal attend l’écho ou rafraîchit le transcript. | La durabilité du status et les effets d’outils ultérieurs. |
| Le status du même prompt est UNKNOWN_DURABILITY. | Cette valeur ne satisfait pas le test not-found du replay backend. | Le sort réel du message; le client garde l’incertitude. |
| Messages termine un send; la même clé et le même JSON reviennent avant éviction. | Le daemon reprend son résultat en mémoire sans rappeler messages.run. | Après redémarrage ou éviction, cette protection n’est plus démontrée. |
| Un upload termine; la remise du résultat échoue trois fois. | Le provider abandonne le batch de réponse. P4 ne garde pas de résultat terminé pour cet upload. | Le serveur peut avoir reçu un essai. Une nouvelle livraison de l’action peut réécrire le fichier. |
| Un appel dont le nom normalisé est create_anyship_deployment dépasse 45 minutes. | Le handler retourne une erreur, sans replay local. | Le déploiement peut être déjà appliqué ou toujours en cours. |
| send_task vocal reçoit son receipt, puis le relay devient unreachable. | La voix reçoit déjà le receipt; le store signale ensuite l’issue du relay. | Aucun succès d’errand ne suit de ce receipt. |
| Un résultat vocal arrive pendant que l’assistant parle. | Le scheduler diffère ou fusionne la parole et reprend la demande après la réponse ou l’audio. | Aucune nouvelle planification métier n’est prouvée. |
| Huit demandes locales sont actives et le backlog contient 64 demandes. | Une nouvelle demande de travail reçoit busy dans cette voie provider. | Cette borne ne compte pas les enfants distants ou tous les transports. |

Une reconstruction qui veut garantir un effet unique devra choisir son propre journal durable d’effets et ses règles de résolution d’incertitude. C’est une proposition de reconstruction, pas une capacité Grok prouvée.

## Checklist des trois sujets

| Sujet | Statut | Frontière restante |
|---|---|---|
| 3. Reprise d’actions au résultat incertain et idempotence | partially_answered | Reprises client, Messages, fichiers, MCP, hosting et spend retracés. Transactions d’effets distants et reprise du moteur après crash inconnues. |
| 4. Boucle tools, choix, replanification et critère de fin | partially_answered | Boucle vocale et outils answered-client-side. Choix et replanification du moteur texte, critère de réussite métier unresolved. |
| 6. Budgets et limites d’autonomie | partially_answered | Deadlines, sorties et concurrence locales answered-client-side. Budget effectif de tours, tokens, durée globale et enfants distants unresolved. |
