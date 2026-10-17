# Compaction du contexte et gestion des runtimes

Dans Grok Bot 0.70.2, le client gère surtout l'identité, les processus locaux, le routage et la réplique des conversations. Les preuves livrées ne contiennent pas le moteur distant qui choisit les messages à compacter. On peut retracer la gestion des runtimes sans inventer cet algorithme.

Ce rapport approfondit la première analyse. Il repose sur les bundles sauvegardés, leur lecture statique et une recherche reproductible sur les 600 fichiers JavaScript de l'application. Aucun compte ni moteur de modèle n'a été exécuté.

La preuve applicative historique reste `ev_a8b822a62831e30bd01eb4c4488b36f0bd50cb8ccc41d83e2dcff4e00cf82a4d`. Son graphe est partiel. Les constatations ciblées ci-dessous ont des références de fichier et ne sont pas présentées comme de nouveaux résultats REA. [uncovered-search.json](../evidence/uncovered-search.json) conserve les recherches, les offsets UTF-16 originaux et les empreintes. [inspect-uncovered.py](../tools/inspect-uncovered.py) les reproduit.

## Ce que la compaction laisse voir

Compacter le contexte signifie remplacer une partie de l'historique fourni au modèle par une représentation plus courte. C'est différent de paginer l'historique affiché ou de conserver une mémoire durable du bot.

| Indice présent dans l'application | Ce qu'il établit | Ce qu'il n'établit pas |
|---|---|---|
| `client_speculative_summarization_config` | Un schéma de configuration de résumé spéculatif et ses valeurs de repli sont compilés | Son utilisation par le moteur Grok Bot, les valeurs effectives du compte et le prompt du résumé |
| `PreCompactRequestQuery` | Un contrat de hook expose l'usage de contexte et le nombre de messages concernés | Le déclenchement effectif du hook dans le daemon actif |
| `SummarizeAction` et `summarizedComposers` | Le SDK partagé peut représenter une action de résumé et des conversations résumées | Un appel Grok Bot à cette action ou une politique de conservation |
| `GrokBotTeamContextSummary` | Un résumé d'équipe possède version, date, prose et modèle | La compaction des messages du modèle et les règles de mémoire privée |
| `contextWindowOverflow`, `SAND-E0404` | Le catalogue classe le débordement de contexte comme non réessayable | Que ce catalogue réalise lui-même la détection distante ou la compaction |

### Une configuration de repli, pas un seuil effectif démontré

Le schéma accepte `tokenUsageThresholdPercentage`, `tolerancePercentage`, `inflightMaxAgeMinutes` et `speculativeStreamTimeoutMinutes`. Les valeurs de repli sont respectivement 70, 5, 5 et 5. Les deux occurrences de la clé de configuration dans les 600 fichiers sont sa déclaration et ses valeurs de repli. La même recherche trouve deux occurrences de `tokenUsageThresholdPercentage` dans ces déclarations.

Cela établit une configuration embarquée. Dire que Grok Bot compacte effectivement à 70 % serait trop fort. Un consommateur peut être absent du package, se trouver dans le serveur ou lire dynamiquement des valeurs. Les valeurs retournées par un service d'expérimentation restent inconnues. Sources : [schéma](../artifacts/formatted/dist/electron-main/main-app.cjs:57999), [valeurs de repli](../artifacts/formatted/dist/electron-main/main-app.cjs:59880) et [recherche complète des littéraux](../evidence/uncovered-search.json).

### Le hook connaît la quantité à compacter

`agent.v1.PreCompactRequestQuery` déclare `trigger`, `context_usage_percent`, `context_tokens`, `context_window_size`, `message_count`, `messages_to_compact` et `is_first_compaction`. Il peut aussi porter `conversation_id`, `generation_id`, `model`, `model_id` et `model_params`. La réponse déclare seulement `user_message` optionnel.

Le SDK relie `pre_compact` à `ExecuteHookRequest` et `ExecuteHookResponse`. Cependant, le constructeur actif du daemon n'enregistre que shell en flux, shell en arrière-plan, lecture et liste de fichiers. Le contrat de hook ne prouve donc pas un hook de compaction servi par ce daemon. Sources : [contrat du hook](../artifacts/formatted/dist/local-exec-daemon/main.cjs:95856), [union des hooks](../artifacts/formatted/dist/local-exec-daemon/main.cjs:96735) et [enregistrements actifs](../artifacts/formatted/dist/local-exec-daemon/main.cjs:128042).

### Trois opérations qu'il faut garder séparées

Le SDK contient `ConversationAction.summarize_action`, un message `SummarizeAction` vide et le champ `ConversationMessage.summarized_composers`. Ces types appartiennent au protocole partagé. Leur présence n'établit aucun appel Grok Bot à la compaction. Sources : [union d'actions](../artifacts/formatted/dist/electron-main/proto.cjs:26982), [action de résumé](../artifacts/formatted/dist/electron-main/proto.cjs:27826) et [messages](../artifacts/formatted/dist/electron-main/proto.cjs:28817).

Le résumé d'équipe est une donnée différente. `GetGrokBotTeamContextSummaryResponse` expose `summary`, `stale`, `skills`, `learned` et `memory_count`. `GrokBotTeamContextSummary` expose `memory_version`, `generated_at_ms`, `prose` et `summary_model`. Aucun champ ne suffit à établir quel morceau de conversation entre dans le contexte du prochain appel modèle. Source : [résumé d'équipe](../artifacts/formatted/dist/node-agent-coordinator/main.cjs:21483).

La réplique du transcript possède des snapshots, des deltas, une pagination et des corps chargés séparément. Ces opérations servent l'affichage et la reprise du client. Elles ne démontrent pas une réduction du contexte du modèle. `startFreshConversation` et `rollBackConversation` sont encore des commandes distinctes. Sources : [réplique et pagination](protocol.md:148), [commandes séparées](../artifacts/formatted/dist/node-agent-coordinator/main.cjs:39334).

Le chunk nommé `chunk-compact-C8-lyxgK.js` contient des données d'emojis. Son nom ne sélectionne pas un moteur de compaction. La provenance de sa copie dérivée est dans [detail-formatting.json](../evidence/detail-formatting.json).

### Ce qui manque encore pour reproduire la compaction

Le corpus ne donne pas le seuil effectif par modèle, le calcul exact des tokens, le prompt du résumé, les messages gardés textuellement, le traitement des résultats d'outils ni la reprise en cas de résumé incomplet. Il ne donne pas non plus la relation exacte entre résumé, mémoire durable et checkpoints du moteur distant.

Le catalogue décrit `SAND-E0404` comme un débordement de contexte dont un retry répéterait l'impasse. C'est une déclaration du catalogue d'erreurs compilé, pas une exécution observée. Source : [catalogue](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:62419).

## Les runtimes sont répartis entre plusieurs propriétaires

Un runtime est ici l'ensemble des ressources qui restent actives pendant une session : processus, ports, credentials, connexions et tâches. Le client ne gère pas tout cela dans une seule boucle.

| Ensemble | Ressources observées | Propriétaire ou frontière |
|---|---|---|
| Application desktop | Fenêtres, comptes, settings, notifications, services système | Main Electron |
| Interface | État React, source de transcript et port de données | Renderer et preload |
| Runtime de compte | Principal autorisé, génération, coordinator et ports révocables | `nKe` dans le main |
| Coordinator | Trois canaux IPC, gateway, tail serveur, supervision locale et providers | Utility process |
| Agent `box` | Commandes et événements via gateway `/api` et `/events` | Processus distant derrière la box, absent du package |
| Agent `temporal` | Commandes sélectionnées via main puis RPC backend | Runtime distant, absent du package |
| Ordinateur utilisateur | Shell, fichiers, Messages, credentials et supervision | Daemon local distinct |
| Services de credentials | Consentement WebAuthn, approbation d'origines cookies et livraisons | Providers distincts du coordinator |
| Ordinateur cloud | Gateway et affichage VNC main ou fork | Services distants et viewers locaux |

Les noms `box` et `temporal` sont les marqueurs réellement présents dans le routage. Ils ne permettent pas de reconstituer des workers Temporal, leur historique ni leur déploiement. Sources : [démarrage et processus](desktop.md:45), [composition du coordinator](../artifacts/formatted/dist/node-agent-coordinator/main.cjs:45588), [frontières ordinateur](computer-detail.md).

### Le changement de compte attend la sortie de l'ancien runtime

`nKe` possède les états `unstarted`, `inactive`, `active`, `blocked` et `disposed`. Chaque observation d'identité incrémente une version. Une autorisation terminée pour une ancienne version retourne `superseded` et ne crée pas le runtime demandé.

Quand le principal change, le client révoque d'abord la demande de port renderer. Il prépare le départ de compte, révoque le port main, appelle `dispose()` sur l'ancien coordinator et réinitialise l'état du compte. Le délai d'arrêt par défaut est de 10 secondes.

Si la sortie de l'ancien coordinator reste non confirmée, `nKe` passe à `blocked` et ne crée pas immédiatement un runtime pour l'autre compte. Une résolution tardive de la sortie peut déclencher la reprise `D()`. `requestRendererPort` ne livre un port que pour la version active. `whenIdle` attend que la chaîne de transitions soit stable. Sources : [machine d'états](../artifacts/formatted/dist/electron-main/main-app.cjs:96785), [attente de sortie et autorisation](../artifacts/formatted/dist/electron-main/main-app.cjs:96867), [transition et reprise](../artifacts/formatted/dist/electron-main/main-app.cjs:96946), [livraison de port](../artifacts/formatted/dist/electron-main/main-app.cjs:97069).

Cette gestion évite qu'une réponse tardive d'autorisation installe les ressources d'un ancien compte. Les lecteurs de fichiers et les settings possèdent aussi leur propre portée de compte. Source : [préparation du départ](../artifacts/formatted/dist/electron-main/main-app.cjs:81230) et [montage actif de `nKe`](../artifacts/formatted/dist/electron-main/main-app.cjs:127344).

### La relance d'un processus est différente de la reprise d'un run

`FYe` lance le coordinator, observe sa durée de vie et le relance après sortie avec backoff. Une vie inférieure à 30 secondes augmente le compteur des sorties rapides. Une nouvelle demande de port peut accélérer une relance ou remplacer le coordinator auquel un port a déjà été livré. `dispose` attend la sortie des lancements connus. Sources : [limite de vie courte](../artifacts/formatted/dist/electron-main/main-app.cjs:99745), [superviseur](../artifacts/formatted/dist/electron-main/main-app.cjs:100157).

Cette relance restaure les connexions du client. Elle ne prouve pas qu'un appel modèle reprend au token interrompu. La reprise durable d'un run distant reste absente du package.

### Le routage évite de mélanger les événements de deux moteurs

`kU` mémorise le type de moteur par agent. Une valeur absente conserve la valeur précédente ou choisit `box`. Une valeur explicite différente de `box` et `temporal` devient `unsupported`. Les agents `temporal` entrent dans `requiredAgents`.

La fonction `gatewayTranscript` refuse un événement de transcript gateway pour un agent `temporal` ou non pris en charge. Si plusieurs types de moteur coexistent, elle traite aussi les événements sans identité d'agent avec prudence. Source : [registre de routage](../artifacts/formatted/dist/node-agent-coordinator/main.cjs:40258).

`IJ.dispatchCommand` sélectionne les actions serveur disponibles. Une action nécessaire pour un agent `temporal` dont le performer retourne `unimplemented` produit une erreur explicite. Il existe des exceptions et des miroirs vers la gateway pour certaines familles. Il serait donc incorrect de dire que toutes les commandes vont toujours au même endpoint. Source : [routeur](../artifacts/formatted/dist/node-agent-coordinator/main.cjs:43616).

Le transcript serveur possède sa propre connexion `ND` et son propre état `lM`. Sa source peut devenir active, déclencher des resynchronisations de roster et publier les états de run. Le coordinator distingue la source serveur de la gateway et signale la parité quand il compare les deux. Sources : [tail serveur](../artifacts/formatted/dist/node-agent-coordinator/main.cjs:45937), [choix des lectures](../artifacts/formatted/dist/node-agent-coordinator/main.cjs:46019).

### Arrêt, annulation et supervision ont chacun leur portée

La fermeture `ze` est protégée par un booléen. Elle ferme la gateway, arrête le tail serveur et les observations d'activité, retire la présence d'équipe, dispose le superviseur local, ferme les trois ports puis quitte. Les erreurs non interceptées suivent une voie de rapport de crash avec délai de flush. Source : [fermeture du coordinator](../artifacts/formatted/dist/node-agent-coordinator/main.cjs:46196).

L'annulation d'un RPC local ferme son contrôleur et empêche sa réponse tardive. `interruptAgentRun` est une commande différente qui adresse un run distant. Une tâche shell mise en arrière-plan possède aussi un cycle différent du RPC foreground. Source : [annulation et reprise](execution.md:67), [shell](execution.md:135).

Le superviseur local se réveille toutes les 30 secondes. Il n'adopte un daemon occupé que si la clé, la génération et l'empreinte proxy correspondent. Le heartbeat et les leases d'update déterminent ensuite si le daemon peut rester actif. Source : [politique et adoption](../artifacts/formatted/dist/node-agent-coordinator/main.cjs:44654) et [possession du daemon](execution.md:95).

Les modes de pause ne sont pas équivalents. `client-pause` retire le daemon. `away-park` garde normalement le daemon mais retient le credential box et arrête le refresh. `present-park` garde le daemon et continue à décrire son état. Avec `parkRetiresLocalExec`, le mode absent ne le garde que pendant le travail actif. Source : [table de politiques](../artifacts/formatted/dist/node-agent-coordinator/main.cjs:44682).

Les providers WebAuthn et cookies ont leurs propres boucles, heartbeats, échéances et retries. Le code démarre explicitement ces providers dans le coordinator. Une pause de gateway les arrête ou les redémarre selon l'état. Sources : [composition des providers](../artifacts/formatted/dist/node-agent-coordinator/main.cjs:45819), [pause](../artifacts/formatted/dist/node-agent-coordinator/main.cjs:46177).

## Bilan des questions

| Question de cette passe | Statut | Limite restante |
|---|---|---|
| Où le client stocke-t-il et applique-t-il la compaction ? | Partiellement répondue | Contrats et configuration trouvés; algorithme et consommateur Grok non trouvés |
| Comment les runtimes locaux sont-ils créés, remplacés et fermés ? | Répondue statiquement pour les chemins cités | Aucun scénario exécuté avec authentification |
| Comment `box` et `temporal` coexistent-ils ? | Répondue statiquement côté client | Internes et garanties des moteurs distants inconnus |
| Comment un run modèle reprend-il après crash ? | Non résolue | Checkpoints et stockage serveur absents |
| Quel contexte le bot délègue-t-il à un autre bot ? | Partiellement répondue ailleurs | Voir [délégation et autres capacités](other-capabilities-detail.md); sélection interne du contexte inconnue |

Les inconnues portent sur le moteur distant, pas sur des instructions cachées qu'on pourrait établir en lisant le nom d'un champ.
