# Délégation, sessions et reprise du client

Cette passe précise neuf mécanismes de Grok Bot 0.70.2. Le client conserve une activité de parent pendant le travail des enfants, distingue les tours de room et restaure les lectures des agents cloud. Les sources ne révèlent toujours pas comment le moteur distant choisit le contexte délégué ni comment une interruption se propage aux enfants.

Les preuves précédentes sont réutilisées depuis `other-capabilities-detail.json` et `compaction-runtime-detail.json`. Aucun inventaire global, appel REA, lancement de l'application, accès au compte ou appel réseau n'a été répété. Les lectures et recherches de cette passe ciblent cinq bundles : `P = artifacts/formatted/dist/electron-main/proto.cjs`, `M = artifacts/formatted/dist/electron-main/main-app.cjs`, `A = artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js`, `C = artifacts/formatted/dist/node-agent-coordinator/main.cjs`, `U = artifacts/formatted/dist/renderer/assets/index-D6ReTfRE.js`. Les empreintes, extraits et offsets UTF-16 des originaux sont dans [le ledger](../evidence/delegation-lifecycle-deep.json). Les lignes citées concernent les copies formatées.

## 1. Parent de session, parent de requête et parent de worker ont des rôles distincts

| Relation | Forme observée | Usage établi |
|---|---|---|
| Fil de tâche vers conversation mère | `task:<taskId>:<parentSessionId>` | Clé de transcript et rattachement de l'activité affichée |
| Tour de membre vers room | `group:<roomId>` dans les états de session | Présence du membre dans une room particulière |
| Requête de tour vers appels parents | `parent_request_id`, `root_parent_request_id` | Champs optionnels du request de tour de room |
| Worker vers manager cloud | `workerBcId`, `managerBcId` | Regroupement des PR et lecture du nombre de workers actifs |
| Message entre bots | `from_agent_id`, `to_agent_id`, `message_id`, `reply_to_id` | Expéditeur, destinataire, déduplication/corrélation de réponse dans le contrat |

Le parseur de fil accepte un task ID de dix caractères hexadécimaux. Sa session mère peut être vide ou `dm:<entier positif sûr>`. Il ne traite pas `group:<roomId>` comme une session mère de tâche valide. Le coordinator construit ces noms pour la présence; le renderer les interprète pour le fil. Sources DL-S01 et DL-S02, A:10660–10704, C:5150–5164.

Les IDs parent/root de tour ne sont pas des IDs de session. Le request contient aussi la room, les pairs et les nouveaux messages, mais aucun champ qui établisse une copie complète du contexte modèle. Le transport backend sait ajouter `x-parent-request-id`, `x-root-parent-request-id` et un ID d'appel d'outil parent depuis `options.lineage`; le constructeur `te` installe ce transport. Aucun constructeur de `lineage` propre à Grok Bot n'a été retrouvé par la recherche ciblée de cette passe. Cette capacité du transport ne démontre donc pas l'héritage du contexte d'un enfant. Sources DL-S03 et DL-S04, P:59192–59223,56672–56705; M:16080–16093,16615–16636,16723–16742.

Certitude élevée sur les noms et leurs consommateurs. La relation entre ces différents identifiants reste partiellement inconnue côté serveur; ils ne sont pas interchangeables.

## 2. Un envoi dans une tâche conserve la session mère et possède une lecture de fil distincte

Dans `fIe`, `k(input)` conserve la session déclarée dans la commande. `C(input)` construit une session de transcript de tâche si `taskId` existe. `retain`, confirmation d'écho et rafraîchissement du transcript utilisent cette seconde clé. Le journal conserve ainsi le fil visé pendant qu'une livraison reste en attente.

L'envoi `m2` transmet pourtant `taskId` et la session mère séparément. Les recherches d'acceptation dans `Be` utilisent également cette session mère, avec agent et nonce, puis rafraîchissent le transcript sous la clé de tâche après confirmation. Le RPC `SendGrokBotUserMessage` transmet les deux champs; `GetGrokBotSendStatus` ne reçoit que agent, message ID et session. Sources DL-S05 et DL-S06, A:35869–35912,71176–71222,71429–71504; M:110631–110689.

Exemple dérivé de ces fonctions, sans exécution : une commande pour la tâche `0123456789` dans `dm:42` conserve `sessionId=dm:42` et `taskId=0123456789` à l'envoi et consulte le nonce dans `dm:42`. Son affichage et son écho sont rattachés à `task:0123456789:dm:42`. Ce découpage n'établit ni création d'un sous-agent ni fork du contexte modèle.

## 3. Un enfant peut maintenir le parent actif sans faire croire que le parent compose

`b0` calcule `isRunning = wire.isRunning || wire.hasRunningSubagents`. `isRunningTurn` reste égal au seul `wire.isRunning`. Les flags composing et retrying sont conditionnés par le tour du parent; `activeGroupMemberId` l'est aussi. L'activité thinking/tool peut rester présente quand seul un enfant travaille. Source DL-S07, C:5391–5428.

| État wire | État client dérivé |
|---|---|
| Parent arrêté, enfant actif | running vrai; runningTurn faux; composing et retrying faux |
| Parent actif, enfant absent | running vrai; runningTurn vrai; composing/retrying suivent le parent |
| Parent et enfants arrêtés | running faux; activité courante retirée |

L'agrégateur `rv` parcourt les sessions triées lexicalement. Il garde les IDs de sessions en cours, crée leur clé de présence avec `surfaceTaskId` si disponible, et choisit le premier état actif comme base de l'activité affichée. `isRunningTurn` global est le OR des tours de ces sessions. Sources DL-S08, C:5349–5389.

Dans le renderer, `_Ke` ne montre composing/retrying/activity que si le premier ID de session active correspond à la session affichée ou à la session mère du fil. `A4e` fait la même vérification pour l'activité d'une tâche. Une activité du bot dans une autre conversation ne doit donc pas apparaître comme la frappe dans le fil actuellement ouvert. Sources DL-S01 et DL-S09, A:10660–10674,46738–46772. Il s'agit d'un choix de présentation client, pas d'une règle d'exclusivité serveur.

## 4. Le tour de room possède sa propre présence

`rv` reconnaît les sessions `group:<roomId>`, les ajoute à `groupTurns` et passe à la session suivante. Ces sessions ne rejoignent ni `runningSessionIds` ni `presenceSessionIds` des conversations personnelles. Un membre peut donc présenter un tour actif de room sans être présenté comme exécutant un tour dans son chat personnel. Source DL-S08.

`bKe` parcourt les membres d'une room et cherche, pour chacun, le `groupTurns` dont `roomId` correspond exactement. `ume` distingue typing, working et reading. `vKe` conserve un ensemble de membres déjà engagés, pour qu'un retour de phase reading ne fasse pas immédiatement disparaître leur état working. Quand un seul membre fournit une activité, la room peut l'afficher; plusieurs activités ne sont pas fusionnées en une activité arbitraire. Source DL-S09, A:46668–46747.

Ces fonctions ne distribuent pas les tours. Les contrats Request/Cancel/Deliver décrits dans la première passe restent la frontière du moteur de room. La reprise ou l'annulation d'un tour ne peut pas être déduite d'une simple disparition de `groupTurns`.

## 5. Une présence périmée est retirée sans annuler le travail distant

`ov` remplace l'état d'une session et arme un timer quand son état client dérivé est actif. Il choisit `staleAfterMs` s'il est positif, sinon **90 000 ms**. Il soustrait l'âge de `updatedAtMs`; un état déjà trop ancien expire immédiatement. Un état où seul `hasRunningSubagents` est vrai suit donc aussi cette échéance. Source DL-S10, C:5319,5490–5569.

À l'échéance, la fonction `d` remplace la présence par l'état inactif `nv`, supprime `surfaceTaskId` et conserve l'attente utilisateur ainsi que le handoff éventuel. Elle ne fait aucun RPC d'interruption. Le contrôle d'identité de l'objet de session empêche un ancien timer d'effacer un état plus récent. Un snapshot serveur réinitialise auparavant les sessions et timers; une source devenue inactive ou un arrêt du consommateur possède encore son propre nettoyage.

Un voyant qui s'éteint après une perte de connexion ne prouve donc pas que l'enfant s'est arrêté. L'application cesse de considérer cette observation comme fraîche.

## 6. Interrupt, start fresh et rollback ciblent un agent et une session

Le renderer traduit ces trois intentions en `id` et `sessionId`. Le validator du coordinator conserve uniquement ces champs. Le router décide ensuite entre performer serveur et gateway selon le harness; une commande exigée pour un bot temporal ne bascule pas silencieusement vers la gateway si le serveur la refuse comme non implémentée. Sources DL-S11 et DL-S12, A:64000–64003,67119–67122; C:42916–42927,43649–43679,43850–43861.

| Commande | Request serveur client | Réponse exposée |
|---|---|---|
| Interrupt | agent, `user_interrupt`, session optionnelle | `hadActiveRun` |
| Start fresh | agent, session optionnelle | Pas de résultat de contexte dans l'adaptateur |
| Rollback | agent, session optionnelle | Pas de checkpoint/version dans l'adaptateur |

L'adaptateur `z8e` n'énumère aucun enfant et ne transmet ni sous-agent ID, ni nonce de tour de room, ni manager/worker ID. Aucune propagation d'annulation ou de reset n'est prouvée par ces trois branches. Le booléen `hadActiveRun` ne constitue pas un reçu d'arrêt de tous les enfants. Source DL-S13, M:110691–110738, et DTO correspondants réutilisés.

Le bouton rollback possède un `fromSeq`, mais l'utilise localement pour récupérer du texte du transcript avant l'appel. `oXe` récupère des messages utilisateur, exclut ceux avec channel ou fromAgent, puis les assemble. Après succès du RPC, la UI propose ce texte à l'édition seulement si aucun draft non envoyé n'existe. **`fromSeq` n'est pas transmis au RPC de rollback.** Sources DL-S14 et DL-S15, U:135078–135121, A:51671–51701,80638. Ce champ UI ne révèle donc pas quel checkpoint serveur est choisi.

## 7. Le redémarrage du client restaure des métadonnées cloud, avec des bornes

`cloud-agents.infos` est un slice `host-durable`, version 1, sensible au compte. Le cache garde au maximum 200 entrées et filtre celles âgées de plus de sept jours. Une entrée dont la sérialisation dépasse 8 192 unités de longueur JavaScript est omise. Le client retire les fichiers et artefacts avant de conserver les métadonnées; ce cache n'est pas un checkpoint d'exécution du worker. Les écritures sont regroupées sur deux secondes. Source DL-S16, A:56009–56214.

Le démarrage de compte appelle réellement `Ce.restore(accountSlot)`. Le cache valide le schéma, l'identité `bcId`, les valeurs de statut et les dates; il rejette les enveloppes corrompues. Un reset incrémente sa génération, vide la map et retire l'accountSlot, ce qui invalide les restaurations tardives. Le module cloud injecte ce cache dans `Cke`. Sources DL-S16 et DL-S17, A:57258–57307,76761–76785.

Une carte restaurée peut ainsi fournir nom, prompt, branche, PR et compteurs de modification avant une lecture réseau réussie. Le cache ne conserve pas l'état complet des enfants, leurs prompts de continuation ou leur pile d'outils.

## 8. La reprise active des workers est une relecture, avec arbitrage entre push et réponse

`Cke` charge l'état par `getCloudAgentInfo({bcId})` et la conversation par `getCloudAgentConversation({bcId})`. Une carte attachée avec un seed cache déclenche une relecture. Les lectures simultanées de conversation sont regroupées; reconnexion et retour à une fenêtre visible relancent le polling. Les politiques embarquées sont de 5 secondes pour live et 60 secondes pour recovery; le choix dépend du statut, des lecteurs attachés, des échecs et du comptage des workers. Sources DL-S18 et DL-S19, A:56522–56756,56762–56839,57098–57179.

`fke` refuse une mise à jour push dont le timestamp n'est pas strictement plus récent. Une lecture plus ancienne peut conserver temporairement le statut du push. Le hold est abandonné après douze réponses retardataires ou en cas de réponse vide. Un reset augmente `generation`, remet les ressources à zéro et retire les polls. Les lectures de première réponse et les prefetchs vérifient leur génération avant certaines mises à jour; cela ne prouve pas une transaction unique pour toutes les couches. Source DL-S20, A:56476–56522,56594–56618,56923–56948,57148–57178.

`Cke.stop` possède une commande distincte `stopCloudAgent({bcId})`, puis retire le hold, relit l'état et choisit le prochain poll. Son contrat figure parmi les commandes génériques du coordinator, pas parmi les actions serveur du router `Yh`. Les lectures de manager utilisent séparément `ListWorkersForManager({managerBcId})`. Sources DL-S19, DL-S21 et DL-S22, C:39086–39093,39474–39534,41355–41470; M:111820–111842.

Aucun appel de redémarrage, réadoption ou réattachement de worker n'a été retrouvé dans cette frontière client. Une reprise de lectures ne prouve pas qu'un worker recommence son dernier appel modèle, ni que stop attend ses enfants.

## 9. Les indices de réattachement distant restent des déclarations ou des données affichées

Le schéma `cloud_agent_stream_reattach` et ses fallback values annoncent enabled vrai et `rehydrateAfterMs=120000`. Le flag `glass_projects_restart_offline_workers` a un défaut faux. Dans les cinq bundles ciblés, les deux occurrences de la configuration sont sa déclaration et ses valeurs de repli; le flag n'a qu'une déclaration. Aucun consommateur actif de ces noms n'a été retrouvé. Source DL-S23, M:57045,59327–59330,60661–60668.

La présentation connaît aussi `reattached_after_host_restart` et `durable_pending_wake_ledger`. `dGe` les traduit en texte affiché. Le deuxième peut inclure un `subagentType`. Cela montre que le client sait présenter ces motifs reçus, pas qu'il possède le ledger durable ou réalise lui-même le réattachement. Source DL-S24, A:32851–32865.

La recherche ciblée n'a trouvé aucun littéral `restartCloudAgent`, `resumeCloudAgent`, `restartWorker`, `resumeWorker`, `reattachWorker` ou `adoptWorker` dans les cinq bundles. Cette absence est bornée à ces noms et fichiers. Un service distant, un nom calculé ou un autre artefact restent possibles.

Les neuf constats précisent donc le **cycle client** : identités distinctes, présence du parent pendant le travail de l'enfant, affichage scoped des tasks et rooms, péremption des observations, commandes d'arrêt adressées et restauration des lectures cloud. Le cycle d'exécution distant, l'héritage du contexte et les garanties de propagation aux enfants restent inconnus.
