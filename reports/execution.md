# Exécution, ordinateurs, outils et frontières natives

Ce rapport décrit ce que le client Grok Bot 0.70.2 permet d'établir sur l'exécution. Le coordinateur local transporte et route les demandes. Le daemon local exécute un ensemble restreint d'actions sur la machine de l'utilisateur. Le calcul des agents, l'exécution MCP de la box et l'attribution des écrans distants dépendent de services absents de cet ASAR.

La distinction la plus importante pour une reconstruction est celle des capacités réellement installées. Le daemon embarque un gros SDK avec des descripteurs pour MCP, computer-use, capture vidéo, édition et sous-agents. Son constructeur actif `oYe` n'enregistre que quatre ressources: shell avec sortie en flux, lancement de shell en arrière-plan, lecture de fichier et liste de dossier. `P4` ajoute les transferts de fichier, les opérations Messages et la découverte de configuration d'outils IA. La présence d'un descripteur générique dans le bundle ne prouve pas que l'action est servie par ce daemon.

Autre distinction: `desktop-mount` ouvre un panneau flottant avec un bot et son composeur. Ce nom ne désigne pas un montage de système de fichiers.

## Statut des preuves

Toutes les conclusions de ce rapport sont des observations statiques, sauf les sections explicitement nommées « inférence » ou « choix Rust ». Aucun outil de la cible, helper natif, daemon ou commande shell de la cible n'a été exécuté pour cette analyse. Aucun secret vivant, compte ou session utilisateur n'a été lu. Les extraits sont des bundles compilés sans sourcemaps, avec des symboles souvent raccourcis.

Le runtime instrumenté est indisponible, selon [runtime-status.json](/Users/cstpierrecharles/Desktop/grokbot-rust/evidence/runtime-status.json). Les conclusions de fonctionnement ne sont donc pas des observations d'une session vivante.

La preuve REA commune est `ev_a8b822a62831e30bd01eb4c4488b36f0bd50cb8ccc41d83e2dcff4e00cf82a4d`, exportée dans [rea-summary.json](/Users/cstpierrecharles/Desktop/grokbot-rust/evidence/rea-summary.json). Elle annonce 600 fichiers JavaScript analysés sans échec de parsing. Le graphe et l'analyse sémantique sont partiels pour des raisons de capacité. Les lectures ciblées ci-dessous complètent cette vue sans prétendre rétablir les sources originales.

Les chemins `D`, `C`, `A`, `K`, `O`, `P` dans les références suivantes signifient:

| Alias | Bundle extrait d'origine | Copie lisible utilisée pour les lignes |
|---|---|---|
| D | `artifacts/client/dist/local-exec-daemon/main.cjs` | `artifacts/formatted/dist/local-exec-daemon/main.cjs` |
| C | `artifacts/client/dist/node-agent-coordinator/main.cjs` | `artifacts/formatted/dist/node-agent-coordinator/main.cjs` |
| A | `artifacts/client/dist/electron-main/main-app.cjs` | `artifacts/formatted/dist/electron-main/main-app.cjs` |
| K | `artifacts/client/dist/electron-main/main-core.cjs` | `artifacts/formatted/dist/electron-main/main-core.cjs` |
| O | `artifacts/client/dist/electron-main/onepassword-connection-service.cjs` | `artifacts/formatted/dist/electron-main/onepassword-connection-service.cjs` |
| P | `artifacts/client/dist/electron-preload/` | `artifacts/formatted/dist/electron-preload/` |

L'identité et les hashes des fichiers sont dans [manifest.json](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/manifest.json). Les compagnons natifs concernés restent `observed-untrusted`, même si leur signature Apple est valide. La contradiction entre leurs octets et la métadonnée d'intégrité de l'ASAR n'est pas effacée par une validation codesign. Voir [native-boundaries.json](/Users/cstpierrecharles/Desktop/grokbot-rust/evidence/native-boundaries.json).

## Carte des responsabilités observées

| Élément | Entrée et effet observés | Limite |
|---|---|---|
| Electron principal | `BYe` lance un utility process, crée trois canaux, transmet `bootstrap.processConfig` et les ports | Ne fait pas l'inférence d'un agent dans cette chaîne |
| Coordinateur | `yz` adopte le bootstrap, compose transports, routage, transcript, supervision du daemon et fournisseurs WebAuthn/cookies | Les endpoints backend et box ne sont pas contenus ici |
| Daemon local | `Tyr` reçoit clé de fichier et machine ID puis compose `rQe` | Exécuteur local avec racine par défaut au dossier personnel |
| Box distante | URL de gateway, token, VNC principal et VNC fork retournés par le broker | Provisioning, sandbox distante, desktop leases et moteurs d'agent non reconstruits par preuve locale |
| Session webview VNC | `Ogt` fixe partition, preloads, isolation et headers d'accès | Affiche un écran distant; ne transforme pas le bureau local en serveur VNC |
| Fournisseur WebAuthn | SSE reverse-channel, consentement local, processus signer, réponse au gateway | Algorithme et contrôles internes du signer natif non prouvés |
| 1Password | Bridge de provisioning CLI, token remis à un backend avec ticket/génération | Le stockage backend et le remplissage dans le navigateur distant ne sont pas présents |
| Egress | WebSocket multiplexé venant de la box, sockets TCP ouverts par le bureau | Mécanisme distinct du proxy des requêtes Node du client |

Preuves: A `BYe`, lignes 100085–100139; C `yz`, 45568–45599 et 45760–45895; D `Tyr`, 163968–164014; A `g7`/`Nze`, 92369–92400; A `Ogt`, 125730–125909; C `rO`/`cO`, 3706–4146; O `Re`, 4729–5035; A `zG.openStream`, 93621–93708.

## Coordinateur: du renderer à la box ou au backend

### Bootstrap et contrat des ports

`BYe` appelle `fork(artifactPath, { serviceName: "sand-node-agent-coordinator" })`. Il crée trois `MessageChannel`: un canal contrôle avec les exécuteurs du processus principal, un canal données destiné au renderer et un autre destiné au processus principal. Il passe les trois ports avec le bootstrap. Les sorties du processus ferment les trois ports et règlent `processExited`.

`preload.cjs` expose `desktop`, `coordinatorPort`, `desktopMount` via `contextBridge`. `coordinatorPort` demande le port à l'IPC `sand:coordinator-port-request`, puis adopte celui remis par `sand:coordinator-port`. `QYe` vérifie `assertTrustedRequester` avant le transfert au `sender` Electron. Le chemin contrôle est distinct du chemin renderer; un renderer ne reçoit pas directement la table de fonctions natives du processus principal.

Le protocole `nE`/`Lx` a une version 3. Sa machine d'état est `awaiting-hello → serving → settled`. Les trames possibles sont `lifecycle`, `request`, `cancel`, `reply`, `event`. Avant `hello`, toute demande est une violation. Un `hello` répété, un ID de demande déjà actif, une réponse envoyée dans la mauvaise direction ou une version différente ferme la session avec une raison détaillée. `wx` trouve la méthode dans une table puis valide les arguments avant `dispatchCommand`. Les fautes normales produisent un résultat typé `failed`; une promesse de dispatch qui rejette au lieu de régler son résultat provoque une violation du protocole.

Sources: [A:BYe](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/main-app.cjs:100085), [A:QYe](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/main-app.cjs:100273), [P:exposition des bridges](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-preload/preload.cjs:5014), [C:nE](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/node-agent-coordinator/main.cjs:39807), [C:wx](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/node-agent-coordinator/main.cjs:42378), [C:Lx](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/node-agent-coordinator/main.cjs:45251).

### Transport gateway et routage d'agent

`dE.request` résout la connexion puis POSTe JSON vers `${baseUrl}/api/${method}`, avec headers de connexion et Authorization si le token existe. Il valide la réponse par `QU`. `dE.streamEvents` ouvre `${baseUrl}/events` en SSE, crée un compteur de génération et publie `transport-connected`. Les blocs passent dans `Or` puis `dispatchEventBlock`; les échos de test sont séparés des événements fonctionnels.

`IJ.dispatchCommand` choisit une route en fonction de la méthode, du harness de l'agent et de certaines identités de session. Pour un agent `temporal`, les actions prises en charge peuvent passer par `control.performServerAgentAction`, au lieu de la gateway box. Les créations peuvent résoudre une `creationRoute` `box` ou `temporal`. Les dev/helper agents imposent une création médiée par l'hôte. Une route serveur requise mais non implémentée produit une erreur explicite, pas une création box silencieuse.

Les approbations, authentification MCP, ressources d'apps MCP et données de roster entrent dans ce routage. Certains résultats serveur sont ensuite miroirés vers la gateway; les échecs de miroir sont journalisés séparément. Cette coexistence explique pourquoi « le coordinateur » n'est pas un moteur local de conversation unique.

Sources: [C:dE.request](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/node-agent-coordinator/main.cjs:41355), constantes `/api` et `/events` à C:40313; [C:streamEvents](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/node-agent-coordinator/main.cjs:41844); [C:IJ.dispatchCommand](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/node-agent-coordinator/main.cjs:43616), création 43669–43825, sélection serveur 43830–43952; [C:composition des canaux données](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/node-agent-coordinator/main.cjs:46102).

### Annulation et reprise du coordinateur

`Lx` associe chaque demande à un `AbortController`. `cancel` enlève l'ID de la map, annule le signal et renvoie `failed/cancelled`. Une réponse tardive ne sera pas envoyée, car son contrôleur n'est plus la valeur active. Cette annulation concerne le RPC local et les transports qui honorent le signal. Arrêter une exécution d'agent distante est une action distincte, `interruptAgentRun`, avec ID d'agent et éventuellement ID de session.

Le comportement `sendPrompt` mérite un contrat dédié. Il enregistre l'URL et le fait que le POST a commencé. Après une faute de transport, il ne rejoue qu'une fois, avec `clientNonce` non vide, si le POST n'a pas commencé ou si la gateway a déjà prouvé la déduplication. Après un POST commencé, le retry exige la même URL. Une réponse `accepted: true` prouve la déduplication pour cette URL. Une erreur fonctionnelle `Re` ne suit pas ce retry.

`createAgentWithRetry` crée lui aussi un nonce stable, distingue les fautes avant dispatch des fautes après dispatch et exige une preuve de déduplication avant de rejouer une mutation au résultat incertain. Le client utilise un header de réponse pour établir cette preuve. Les lectures de roster et transcript ont des reprises bornées séparées.

Le flux d'événements est repris avec backoff. Après retour réseau, `liveStreamAnswers` envoie un écho et attend sa présence dans le flux avant de détruire une connexion qui pourrait encore fonctionner. Un watchdog de silence déclenche le même test. Ce n'est donc pas une simple boucle de reconnexion à chaque notification réseau.

`FYe` relance le utility process après sortie, avec un backoff déterminé par la durée de la vie précédente. Une nouvelle demande de port renderer peut remplacer le coordinateur courant. `mnn` filtre les événements de transport par génération, pour éviter qu'une ancienne instance réécrive l'état d'une nouvelle. `yz.ze` ferme transports, souscriptions, ports et contrôles puis quitte avec un code distinct pour arrêt propre, faute ou bootstrap refusé.

Sources: C:Lx 45249–45352; `interruptAgentRun` à C:39082 et 39499; [C:sendPrompt](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/node-agent-coordinator/main.cjs:41669); C:createAgentWithRetry 41550–41606; C:liveStreamAnswers 41223–41260 et streamEvents 41884–41920; [A:FYe](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/main-app.cjs:100157); A:mnn 100145–100158; C:ze 46196–46264.

## Daemon local: bootstrap, détenteur et transport

### Bootstrap et fichiers d'état

`Tyr` exige un `computerId` reçu dans le handoff. `xyr` accepte le canal parent d'un utility process ou un canal IPC enfant, avec une échéance de 5 secondes. Il récupère la clé de chiffrement des fichiers; son absence est permise, contrairement à l'absence du machine ID. `Eyr` retire de `process.env` les variables privées du handoff, des FD hérités et du proxy après lecture. `rYe` choisit `SAND_LOCAL_EXEC_ROOT`, sinon `SAND_AGENT_PROJECT_DIR`, sinon `homedir()`.

Le processus principal `y1` obtient clé et machine ID, vérifie le marqueur de retraite de compte, incrémente/charge la génération et lance soit `ise` par utility process, soit `sse` par processus détaché avec IPC. Le handoff contient la clé et l'ID machine. La clé ne passe normalement pas par un argument CLI. `sse` ignore stdin, redirige stdout/stderr vers le journal et utilise `unref()` sur le processus et le canal.

`Ug/qie` cherche une clé de fichier dans le secret store. Si le stockage chiffré est indisponible ou échoue, la fonction renvoie `null`. `FKe.sealLocalExecFile` renvoie alors le JSON en clair avec `keyId: null`. Avec une clé, `BA` utilise AES-256-GCM, une clé de 32 octets, un nonce aléatoire de 12 octets et un tag de 16 octets. Le contenu est un JSON `sandSealedFile: 1`; le key ID est un préfixe de 16 caractères du SHA-256 de la clé. C'est une observation du fallback, pas une recommandation de persister des identifiants en clair.

Le protocole sur disque utilise notamment `local-exec-daemon.json`, `local-exec-daemon-connection.json`, `local-exec-daemon-credential.json`, `local-exec-supervisor.json`, `local-exec-update-lease.json`, `local-exec-account-retire.json`, `local-exec-account-generation.json` et `local-exec-backend-return.json`. L'enregistrement de découverte contient PID, début, dernier `servingAt`, nombre de requêtes actives, key ID, génération, comptes rejetés et fingerprint du proxy.

Sources: [D:Tyr](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/local-exec-daemon/main.cjs:163870); D:rYe 127980–127983; [K:y1 et lancement](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/main-core.cjs:33475), 33556–33600; K:Ug 33242–33274; [K:BA/CF](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/main-core.cjs:32343); A:FKe.sealLocalExecFile 97675–97690; C:fichiers et découverte 37516–37584.

### Supervision et retraite

`yG` dans le coordinateur supervise le daemon. Sa décision `oz` adopte un daemon uniquement lorsqu'il a du travail actif et que key ID, génération et fingerprint de proxy correspondent. Sinon il lance ou remplace. Un PID seul n'est pas suffisant pour l'adoption utile.

La supervision renouvelle une connexion et des credentials, publie le heartbeat et lit l'état toutes les 30 secondes. Elle rafraîchit les credentials avant leur expiration, en prenant le minimum des expirations et des dates de retry. Elle distingue état inconnaissable, absent, mort, vivant mais stale et vivant actif. Un daemon stale avec travail reçoit une période de grâce; les limites de guérison sont de dix remplacements/restarts avant un signalement `respawn_limit` ou `files_key_replace_limit`.

`Cwe` côté daemon contrôle la possession. Sans heartbeat récent, sans lease de mise à jour et sans grâce liée aux requêtes en cours, il ordonne la sortie. Fenêtre supervisor par défaut 90 secondes, grâce requêtes 120 secondes, contrôle toutes les 10 secondes. S'il est remplacé par un autre PID vivant nommé dans la découverte, il termine après sa grâce. Une longue interruption de scheduling reçoit une tolérance temporaire; cela évite de traiter immédiatement une reprise machine comme une perte de propriétaire.

Les pauses ont des politiques différentes. `client-pause` retire le daemon; `away-park` le conserve par défaut mais retire le credential box; `present-park` le conserve et continue à décrire. Avec `parkRetiresLocalExec`, `away-park` ne le garde que tant qu'il est occupé. `vI` retire le credential box et conserve les canaux `userComputer` backend; la pause box ne signifie donc pas nécessairement arrêter tout travail local backend.

`A1` dans le processus principal sérialise la retraite de compte: augmente la génération, écrit le marqueur, termine le PID de découverte et les processus connus, supprime découverte/credentials/connexion. `Mi` vérifie une identité de daemon, envoie SIGTERM, attend jusqu'à 4 secondes et peut envoyer SIGKILL. Les sondes utilisent `/proc`, sinon `ps` ou PowerShell; ces sondes décrivent l'application, elles n'ont pas été exécutées dans l'analyse.

Sources: [C:oz/yG](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/node-agent-coordinator/main.cjs:44669), états 45030–45247; C:politiques de pause 44690–44724; C:vI 37586–37591; [D:Cwe](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/local-exec-daemon/main.cjs:78942); D:timings 78392–78399 et boucle possession 156315–156344; [K:Mi](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/main-core.cjs:33455), [K:A1](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/main-core.cjs:33612).

### Deux transports actifs alternatifs

`rQe` installe un provider `JM` destiné à la box et une map de providers `mj`, un par `accountScope`, pour le backend. Une channel backend utilisable rend la connexion box idle: `JM.resolveConnection` refuse alors explicitement ce dial. Une channel serveur rejetée est inscrite dans `rejectedAccountScopes`; le daemon peut reprendre le chemin box. Quand le backend accepte à nouveau le compte, le scope est retiré et la box est redialée pour appliquer la préférence.

Le chemin box `JM` GETe `/local-exec/requests` en SSE, parse les blocs `data:`, valide leur forme, puis transmet à `P4.handle`. Il reçoit un `providerId` par `welcome`; il POSTe les réponses vers `/local-exec/responses` avec ce provider ID. `hello` annonce racine, dossier terminals, machine, label, supervision, variante et capacités Messages. Un ping toutes les 10 secondes annonce aussi la supervision. Connexion: échéance 15 secondes, watchdog de silence 35 secondes, backoff 1–10 secondes. Trois échecs consécutifs du flux déclenchent une tentative de renouvellement de connexion. Les réponses ont trois essais, espacement 200 ms; après échec, le lot est abandonné et journalisé.

Le chemin backend `mj` crée un client Connect sur `backendUrl`. Il appelle `watchGrokBotUserComputerRequests` avec machine ID, credential et hello. `connected`, `notify` et heartbeat avec travail déclenchent `pollGrokBotUserComputerRequests`. Un polling de sûreté fonctionne toutes les 30 secondes. Le poll transmet les `ackIds`, déduplique les identités dans `seen`, valide chaque frame avec `Qir`, puis démarre les actions. Un refus `Unauthenticated` vide la preuve de connexion et force une nouvelle résolution.

`mj` borne à huit opérations actives, une backlog de 64 et des polls de 64 éléments. Une annulation retire une demande encore en backlog ou annule l'action en cours. Les trames de réponse passent par `P3e` et sont envoyées en lots jusqu'à 256 trames ou 8 MiB. Les réponses fichier se découpent en blocs de 4 MiB. Une seule trame peut dépasser le budget de lot si elle est la première. Les limites ne doivent donc pas être assimilées à un maximum universel de message.

La reprise ne garantit pas un résultat durable. Les réponses sont une outbox mémoire. Après trois échecs de submit elles peuvent être abandonnées. Sur un refus de credential, le lot est remis en tête de queue. À la fermeture, `mj` abandonne la backlog, annule les opérations actives, attend leur drain avec échéance et tente d'expédier les réponses restantes. `JM.close` ferme le flux mais ne suit pas ce protocole complet de drain. Il n'a pas la même file bornée que `mj` dans la chaîne inspectée.

Sources: [D:rQe](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/local-exec-daemon/main.cjs:156063), préférence transport 156134–156275; [D:JM](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/local-exec-daemon/main.cjs:128504); D:endpoints et limites 81896–81924; [D:mj](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/local-exec-daemon/main.cjs:155226), hello 155376–155404, watch 155500–155569, polling 155609–155715, submit 155745–155789, conversion/chunking 155825–156011.

## Exécuteur local et autorisation

### Actions réellement servies

`P4.handle` dispatch les requêtes vers `handleExecRequest`, `handleMessagesOp`, `upload`, `download` et la découverte `sPe`. Les actions `retire-approval` et `cancel` n'entrent pas dans un nouvel exécuteur. `withInflight` refuse de lancer un ID déjà actif et publie le nouveau compteur à chaque changement.

Pour `exec`, `handleExecRequest` parse un `ServerMessage` avec tolérance aux champs inconnus, calcule une description d'action, vérifie l'autorisation, affecte un ID d'exécution interne monotone, crée un contexte annulable puis lit le flux `execManager.handle`. Les messages `Kf` deviennent réponses `client`; heartbeat, fin et throw deviennent réponses `control`. Sur un throw shell, la réponse peut inclure `cwdState` `exists`, `missing` ou `unset`.

`oYe` construit `gg`, y enregistre `Sg` avec `shellStreamArgs`, `ES` avec `backgroundShellSpawnArgs`, `wP` avec `readArgs`, `yP` avec `lsArgs`, puis produit `SS.fromResources`. `SS` parcourt ces handlers, gère l'annulation par exec ID, émet un heartbeat pendant l'opération, termine par `streamClose` ou `throw`. Un type sans handler renvoie une erreur suivie d'une fermeture. Les contrôles génériques `writeShellStdinArgs` et `forceBackgroundShellArgs` existent dans le SDK mais ne sont pas enregistrés ici; leur nom dans `Cbe` ne les rend pas actifs.

Sources: [D:P4](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/local-exec-daemon/main.cjs:128117), handleExecRequest 128309–128363; [D:oYe](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/local-exec-daemon/main.cjs:128042); descripteurs ES 97718, yP 98172, wP 98318, Sg 98418; [D:SS](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/local-exec-daemon/main.cjs:97291).

### La racine de fichiers n'est pas un sandbox shell

`LE` résout les chemins absolus ou relatifs à la racine, refuse ceux hors racine et compare aussi le chemin réel de la racine avec le chemin résolu à travers les symlinks. Les uploads, downloads et wrappers read/ls appellent ce garde. La taille par défaut d'un fichier lu ou transféré est 100 MiB. Un upload utilise `R2`, la famille d'écriture temporaire/rename; download lit le fichier après vérification de taille. La vérification de taille préalable ne remplace pas un lecteur borné contre un fichier qui grossit entre le stat et la lecture.

En revanche, `XKe` vérifie seulement que le working directory shell demandé existe. Il accepte donc un chemin absolu existant hors racine. Si le chemin manque, il choisit la racine et émet un stderr qui explique le changement. `DE`, le service de permissions injecté dans les exécuteurs, retourne allow et `insecure_none` par défaut. `R4` conserve des contrôles de denylist et de sandbox demandée; `Wg` choisit une politique non sandboxée lorsque le caller n'en fournit pas. Il serait faux de présenter la racine local-exec comme un confinement système des commandes shell.

`Wg.executorFor` clone un exécuteur par `conversationId`, préserve son cwd et injecte des variables d'identité de requête/conversation. Les exécutants shell Zsh/Bash/PowerShell sélectionnés par `Pae` gèrent leur état. `Wg` borne la sortie stdout et stderr renvoyée à environ 1 MiB chacune et peut écrire une sortie volumineuse dans un fichier d'artifact. `R4` sépare abort utilisateur, timeout, hard timeout et passage en arrière-plan. Une demande qui a été adoptée comme tâche background peut survivre au scope foreground; l'arrêt du RPC n'est pas une promesse de destruction universelle de tous les processus descendants.

Sources: [D:LE et XKe](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/local-exec-daemon/main.cjs:128006), LE 128017–128037; P4:upload/download 128378–128400; [D:DE](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/local-exec-daemon/main.cjs:127906); [D:R4](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/local-exec-daemon/main.cjs:127116); [D:Wg](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/local-exec-daemon/main.cjs:107215); D:Pae/Qke 40715–40755.

### Décision locale et décision serveur

`fQ` retourne `allow`, `ask` ou `refused`. `Bbe` convertit ce verdict en erreur de blocage pour les actions ordinaires. Les types d'action sont explicitement reliés à des catégories de permission: shell, stdin, read, ls, write et découverte IA utilisent `global`; Messages read utilise `setting`; envoi iMessage utilise `subject`; phone utilise `call`. Cette taxonomie ne prouve pas que tous ces types sont servis par ce daemon.

Dans le mode local/box, `ceiling: never` refuse. `permission: always` autorise la catégorie global; `permission: never` refuse les catégories non global. Messages read dépend du réglage `messagesEnabled`. Messages send peut utiliser un grant d'action ou de destinataire lié à `accountScope`. Sinon, une approval ID doit correspondre à un enregistrement non retiré. `Pbe` compare action+cible ou chemin de ressource attaché; pour run-command, le dossier terminals peut être ce chemin partagé. La formule compare aussi machine ID quand il est présent dans les descriptions. Il faut reproduire explicitement les champs effectivement fournis, pas supposer que cette comparaison constitue à elle seule une vérification complète de machine.

Dans le mode `serverAuthoritative`, `authorization: approval` autorise immédiatement et `authorization: standing` autorise seulement global. Ces cas passent avant le plafond local. Le daemon fait donc confiance à un contexte de backend authentifié pour une décision déjà prise. Si ce marquage ne suffit pas, l'approbation persistée doit appartenir au même `accountScope`. `K6` dérive l'autorisation de deux booléens de la frame protobuf. Le chemin SSE box `znr` les remplace par `none`.

`recordLocalToolApproval` côté Electron exige une action connue, une ID et une cible. Si machine ID est fourni, il doit être celui de ce bureau. L'enregistrement ajoute l'accountScope actif. Les retraites ne suppriment pas simplement une ligne: `Dbe` écrit des tombstones `retiredIds`, sous sérialisation par chemin, puis `pxt` enlève ces IDs de la map effective. La lecture fautive d'un fichier d'approbations retourne une map vide et signale l'erreur, ce qui ne produit pas une approbation implicite.

Sources: [D:fQ/Bbe](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/local-exec-daemon/main.cjs:79892); D:K6 79769–79777; D:Pbe/Cbe 79710–79758; D:fichiers approvals/retraites 79794–79855; D:taxonomie 79441–79481; [A:recordLocalToolApproval](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/main-app.cjs:108180); D:znr 128833–128850.

### Auto-review: frontière prouvée et moteur inconnu

Le client expose et normalise des instructions `isEnabled`, `allowInstructions`, `blockInstructions`. `pQ/Fbe` limite les instructions à vingt par liste, mille caractères chacune, les nettoie et les déduplique. `q5e` charge la version backend, la miroirise seulement si compte et write generation sont toujours les mêmes, puis synchronise la box. Lors d'une modification, il écrit le réglage backend; si l'écriture échoue, il rétablit la valeur locale précédente.

`resolveAutoReviewApproval` transmet agent, request, resolution `approved/denied/always`, plateforme, éventuelle commande approuvée, session et consentement de réunion. Le routage distingue les sessions serveur des sessions box. Aucun moteur complet d'évaluation auto-review n'est installé dans `P4/oYe`. L'ordre des règles, les prompts de review, le modèle, les garanties d'injection et la réécriture de commande sont inconnus dans ce périmètre. Le texte UI « Auto-review still checks everything first » est une promesse de produit; la chaîne locale prouve le transport de la décision, pas l'exécution du reviewer.

Sources: D:pQ/Fbe 79951–79970; [A:q5e](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/main-app.cjs:107725); [A:résolution backend](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/main-app.cjs:110906); C:routage 43835–43952.

### Messages et idempotence limitée

`P4.handleMessagesOp` annonce `messages-accepted` avant la décision lorsque send n'a pas d'approval ID et que `decideLocalUse` existe. Un verdict ask/refused produit une trame de consentement. Après décision, le flag d'annulation est vérifié avant `messages.run`.

Pour send, `withSendKey` sérialise par clé, `replayedSend` réutilise le résultat si l'opération JSON correspond et `rememberSend` conserve jusqu'à 64 résultats par request ID et 64 succès par send key. Les erreurs et consentements sont mémorisés par request ID, seuls les succès sont mémorisés par send key. Ces maps sont en mémoire. Un redémarrage perd cette protection, et l'annulation pendant l'opération native ne transmet pas ici de signal à `messages.run`.

`Pnr` relie les opérations Messages à `Ere/HRe`. `HRe` cherche le sidecar, lit un `service.json` dans le dossier Application Support du helper, envoie une demande JSON terminée par newline sur `rpcSocketPath` et lit la première réponse JSON. Si le service manque ou refuse la connexion, il installe/copiera le helper puis le lance via `/usr/bin/open -g`, avec une attente de démarrage bornée. Le wrapper exige version et verbes Messages attendus. Les droits Full Disk Access, Automation et Contacts sont traités comme états retournés par le helper. Les effets natifs précis et l'authentification du socket restent à vérifier séparément.

Sources: D:P4 Messages 128217–128308; [D:HRe et Ere](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/local-exec-daemon/main.cjs:84065), 84065–84191. Le sidecar `Contents/Helpers/Grok Bot Computer Use.app/Contents/MacOS/CUGrokBotService` est hors ASAR. Il ne fait pas partie des six contradictions d'intégrité ASAR recensées. Son comportement interne demeure inconnu; une signature valide ne prouve pas ses contrôles de socket, ses permissions ou ses opérations Messages.

## Ordinateur distant, écrans et navigateur

`g7` reçoit une box du broker et forme une connexion avec `gatewayUrl`, `gatewayToken`, `networkToken`, `vncUrl` principal et `forkVncBaseUrl`. La structure confirme deux catégories d'URL VNC. Elle ne suffit pas pour déduire le nombre d'écrans, leur stockage ou la stratégie d'isolation par bot. L'architecture « ordinateur persistant par compte et lease d'écran par bot » est un choix de reconstruction à corroborer avec le rapport UI/agents et à implémenter indépendamment du backend fermé.

`Ogt` fixe une partition `persist:sand-forever-box`. Lors de `will-attach-webview`, il impose `nodeIntegration=false`, `contextIsolation=true`, `sandbox=true` et remplace le preload par celui du viewer VNC ou celui du webview ordinaire. Les protocoles de src autorisés sont HTTP, HTTPS et about. Il collecte les tokens de port/réseau présents dans l'URL du viewer et les applique comme headers aux requêtes de la partition correspondante. Le bridge presse-papiers `box-vnc` exige à la fois la bonne partition, une frame URL VNC et une origine enregistrée/loopback. Ce garde est spécifique au viewer et ne donne pas ce bridge à tout site.

Le preload VNC instrumente noVNC/RFB pour compter touches, clics, frames et octets, reporte session et visibilité, applique mapping clavier Mac, champ IME, qualité adaptive, maintien d'image et miroir presse-papiers. Ce sont des mécanismes du viewer local d'un framebuffer distant. `preload-webview` wrappe `navigator.credentials.create/get` avec détection d'échéance, rapporte les stalls au host et remplace les dialogues JS par des réponses prédéfinies. Cette couche ne réalise pas elle-même une signature WebAuthn distante.

Le manager MCP du bureau installe `boxMcpExec.executeTool` qui renvoie explicitement une erreur disant que les tools MCP s'exécutent sur l'ordinateur de Grok Bot. Il peut afficher définitions/catalogue, déclencher authentification et lire des ressources autorisées; cela ne prouve pas une exécution MCP locale générale.

Le panneau `desktop-mount` accepte jusqu'à 64 bots proposés. Il est disponible sur macOS/Linux, suit une position écran, crée un petit BrowserWindow, vérifie que les commandes mount viennent de la fenêtre principale et que les commandes panel viennent du panneau, puis émet `prompt-submitted {agentId,prompt}`. À fermeture ou navigation de la fenêtre principale, il se démonte. Aucun path de dossier, montage kernel ou protocole FUSE ne figure dans ce contrat.

Sources: [A:g7/Nze](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/main-app.cjs:92369); [A:Ogt et trust VNC](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/main-app.cjs:125694); [P:preload-vnc](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-preload/preload-vnc.cjs:801), instrumentation 830–844 et installation 2132–2179; [P:preload-webview](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-preload/preload-webview.cjs:240); [A:MCP box runtime](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/main-app.cjs:113598); [A:wJe desktop mount](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/main-app.cjs:101127).

Recherche bornée: dans D, C, A et K, les termes `sshfs`, `rclone`, `localMount`, `mountPath`, `mountPoint` et variantes ne révèlent pas une implémentation active de montage de dossier utilisateur. Les flags `agent_store_principal_local_mounts`/`agent_store_local_user_mount` ne prouvent aucune chaîne d'effet. Un mécanisme serveur de montages locaux reste inconnu. La lecture locale par daemon, le transfert d'attachement et le panneau flottant sont trois comportements distincts.

## WebAuthn et 1Password

### WebAuthn reverse-channel

`rO` GETe `/webauthn/requests`. Il traite `welcome`, `ceremony {kind:get/create, origin, optionsJson}` et `cancel`. Il n'admet qu'une cérémonie active; une autre reçoit un refus `busy`. Un refus utilisateur mute temporairement l'origine. Le consentement via `GKe` affiche origin et rpId, ferme le prompt précédent et retourne une ID de prompt plus le handle de fenêtre lorsqu'il est approuvé. Une fermeture du prompt équivaut à un refus. `QKe` refuse un PIN si son prompt ID ne correspond plus.

Après consentement, le provider émet une stage `grant:ok`, appelle `signer.sign` puis renvoie `stage sign` et credential JSON ou DOMException. Une annulation serveur abort le contrôleur associé. Rupture du flux abort toutes les cérémonies actives. Résultats et erreurs passent par une politique de livraison avec jusqu'à cinq tentatives. Le heartbeat est de dix secondes; trois fautes consécutives le ferment.

`lO` choisit le helper packaged `app.asar.unpacked/dist/native/sand-webauthn-signer`, un override explicite existant ou un binaire de développement. Ce choix fait un test d'existence. Dans cette chaîne JS, il n'y a pas de vérification codesign avant spawn. `cO` lance le binaire sans argument, avec stdin/stdout/stderr pipes. La première ligne stdin est la cérémonie JSON, éventuellement avec `windowHandle`. Les événements stderr `[signer-event]` signalent présence, sélection de device, UV et PIN. Les requêtes PIN sont sérialisées; les réponses stdin sont `{kind:pin,pin}` ou `{kind:cancel}`. Sur abort, le wrapper envoie SIGTERM et règle une erreur `cancelled_or_timeout`.

La vérification rpId/origin, l'encodage des credentials, le type de device et l'exclusion des passkeys logiciels ne peuvent pas être conclus de ce wrapper. Le helper natif reste `observed-untrusted`.

Sources: [C:rO](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/node-agent-coordinator/main.cjs:3706); [C:lO/cO](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/node-agent-coordinator/main.cjs:3982); [A:GKe/QKe](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/main-app.cjs:98061); C:composition consent/signer 45812–45853.

### Provisioning 1Password

`OnePasswordConnectionService` est exporté sous ce nom réel via la classe minifiée `Re`. `startMint` sérialise un flow actif et distingue saisie d'un token de service existant et provisioning par CLI. Le flow CLI trouve ou crée un vault, demande au backend un ticket et une génération attendue, puis appelle `mintAndDeliver`. Le backend peut imposer une expiration. Le résultat n'est accepté que si la connexion/vault correspond et si la génération a augmenté exactement de un.

`OnePasswordDesktopProvisioningBridge`, classe `Je`, valide identifiants, noms, listes et sorties JSON. Il lance les commandes CLI à travers `sand-op-launcher`. `mintAndDeliver` crée un service account avec un droit `${vaultId}:read_items` et une expiration éventuelle, lit son token brut, le valide comme `ops_...`, puis l'envoie directement au sink actif. Dans le service, ce sink appelle `backend.complete` avec `mintTicket`. Le retour public contient une référence de connexion et des métadonnées, pas le token. Le backend demeure le consommateur du secret. Si sa remise devient incertaine, `deliveryIndeterminate` interdit une nouvelle mutation dans cette instance jusqu'à réconciliation. La persistance de cet état au redémarrage n'est pas établie.

`SandOnePasswordCliRuntime`, classe `$e`, accepte uniquement macOS. Il privilégie un CLI système vérifié, sinon installe sa version managed 2.35.0. Les URLs de download sont fixées sur `cache.agilebits.com`, HTTPS, sans credentials/query/hash, redirects refusés. Archive et fichier op sont bornés à 64 MiB; l'archive doit avoir exactement `op` et `op.sig` sans symlinks. Le JS extrait op mais ne vérifie pas le contenu de `op.sig`; sa validation effective d'op passe par codesign et lecture de version via le launcher. Le launcher a une exigence d'identité et de Team ID différente du CLI 1Password. L'installation est atomique par fichier temporaire, fsync, vérification, rename puis vérification finale.

`Ae` supprime les variables `OP_*` héritées et ajoute `OP_CACHE=false`, `OP_BIOMETRIC_UNLOCK_ENABLED=true`, `OP_LOAD_DESKTOP_APP_SETTINGS=false`. Il ne supprime pas toutes les autres variables d'environnement. La restriction d'accès au stockage 1Password promise par le launcher exige une analyse native séparée. La signature valide du launcher ne suffit pas à lever son statut `observed-untrusted`.

La classe `Re.approve/deny` appelle des méthodes backend. Le bundle de provisioning ne contient pas la chaîne complète « approbation d'un login → lecture du secret → remplissage du navigateur distant ». Il serait donc excessif d'affirmer que l'agent ne voit jamais les valeurs d'un login à partir de cette seule preuve. On observe surtout que le token de service a le droit de lire les items du vault et est remis au backend.

Sources: [O:Re](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/onepassword-connection-service.cjs:4729); O:connectWithToken/connect 4892–5034; [O:Je.mintAndDeliver](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/onepassword-connection-service.cjs:3334), 3398–3527; O:runOp 3555–3579; [O:$e runtime](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/onepassword-connection-service.cjs:2713); [O:download/archive/env](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/onepassword-connection-service.cjs:2551); O:approve/deny 4780–4787.

## Routage réseau et egress

Le proxy client Node et le tunnel egress sont deux contrats.

`NM` et `y_e` lisent `SAND_NODE_PROXY_ENV` sous forme JSON contenant `HTTP_PROXY`, `HTTPS_PROXY`, `NO_PROXY`, retirent cette variable de l'environnement puis appellent `node:http.setGlobalProxyFromEnv`. Un JSON incorrect ou une API refusée produit un log « staying direct ». Un fingerprint stable des trois valeurs entre dans le critère d'adoption du daemon. Le proxy affecte la connectivité du client/daemon; il ne constitue pas le tunnel TCP de la box.

Pour ce tunnel, `nqe` transforme l'URL gateway en WebSocket port 8790, ou remplace le suffixe du port dans le hostname proxy. Il exige un token, transmet headers et interdit par défaut les cibles privées. `zG` envoie Authorization et des headers `x-sand-egress-attach`, `client`, `if-match`; modes `if-free` et `claim` rendent la possession par un ordinateur explicite. Le serveur retourne une attach generation. Une connexion WebSocket ouverte reste `connecting` jusqu'à ping/pong/message du serveur; seulement alors l'attache est confirmée. Conflit HTTP 409 ou close 4002 mène à standby; remplacement close 4001 fait aussi céder la possession. Une claim récente peut reprendre immédiatement. Le watchdog box est de 45 secondes.

Le protocole binaire a un type sur un octet et un stream ID u32 big-endian. `open` ajoute un port u16 et un hostname UTF-8, `data` contient des octets, `close` termine le flux. `openStream` vérifie la politique réseau sur hostname/port, résout DNS sur le bureau, refuse par défaut les IP privées/réservées/loopback/multicast et réévalue les règles d'IP lorsque nécessaire. Puis il ouvre un `net.createConnection` sur l'IP choisie. Les données box sont écrites dans cette socket, ses retours sont multiplexés dans le WebSocket. Au-delà de 1 MiB de buffer WebSocket, la socket TCP est mise en pause, puis reprise après drain. À la fermeture du tunnel, les sockets actives sont détruites.

`qG` combine préférence utilisateur, permission admin, connexion de box et politique reçue du backend. Il n'ouvre pas le client avant de disposer d'une politique si le provider est configuré. Les politiques sont rafraîchies toutes les cinq minutes, avec retries bornés. Après trois échecs de cycles et présence d'une politique cache, il la supprime et détache. Une évolution qui desserre les règles n'applique que la partie restrictive jusqu'à reconnexion de box. Le bureau est donc une sortie réseau pour la box; il n'est pas un proxy ouvert sans décision de compte/politique dans cette chaîne.

Sources: [C:NM](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/node-agent-coordinator/main.cjs:37119), D:y_e 78687–78722; [A:nqe](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/main-app.cjs:93137); A:frames et DNS 93178–93316; [A:zG.connect](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/main-app.cjs:93434); [A:openStream](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/main-app.cjs:93641); [A:qG](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/main-app.cjs:93740).

## Architecture Rust proposée

Les interfaces ci-dessous sont des choix de conception pour un système indépendant. Elles ne prétendent pas parler aux services privés de Grok Bot. Elles définissent les propriétaires et les garanties qu'une implémentation devra démontrer.

### Processus et propriétaires

| Propriétaire Rust | État possédé exclusivement | Interface conceptuelle |
|---|---|---|
| `DesktopHost` | Fenêtres, OS dialogs, identité machine, secret store, demandes d'approbation humaine | `DesktopControl` reçoit seulement des demandes typées et validées |
| `Coordinator` | Route d'agent, génération des connexions, souscriptions, requêtes RPC actives | `dispatch(Command, RequestContext) -> CommandOutcome`, flux `RunEvent` |
| `LocalExecutionDaemon` | Registry d'outils effectivement disponibles, processus locaux, backlog, outbox, root policy | `execute(AuthorizedAction) -> ExecutionEventStream`, `cancel(ExecutionId)` |
| `CloudComputerManager` | Ordinateur persistant par compte, version/snapshot, ressources, cycle de vie | `ensure(AccountId) -> ComputerHandle`, `lease_screen(BotId) -> ScreenLease` |
| `ToolRouter` | Binding outil → route backend/box/local, capabilities versionnées | `resolve(ToolCall, AgentContext) -> ToolRoute` |
| `ApprovalService` | Décisions persistées, grants, plafond admin, version des règles | `evaluate(ActionDescription, Actor, Context) -> ApprovalDecision` |
| `RunStore` | Journal durable, nonces, réponses terminales, livraisons/acks | append conditionnel et reprise par séquence |
| `NativeBroker` | Processus/sockets OS, prompts WebAuthn, bridge Messages, provisioning 1Password | Commandes bornées séparées par capacité et jamais une shell arbitraire |
| `EgressController` | Lease d'attache, politique, DNS autorisé, map de flux, buffers | `attach(EgressLease)` et messages Open/Data/Close |

L'ordinateur cloud doit posséder les fichiers, le navigateur, les apps et les sessions machine du compte. Chaque bot reçoit une lease d'écran et de navigateur avec une identité distincte. Le bot ne possède pas l'ordinateur global et ne peut pas redémarrer celui des autres implicitement. L'écran partagé principal et l'écran de bot sont des variantes de viewer sur des leases distinctes. Leur contenu peut partager le même stockage selon la politique du compte, mais les entrées clavier/pointeur doivent toujours porter une lease valide. C'est un choix à tester en concurrence, pas un effet prouvé du champ `forkVncBaseUrl`.

### Types et frontières

Définir des types distincts pour `AccountId`, `MachineId`, `BotId`, `RunId`, `ExecutionId`, `RequestId`, `ApprovalId`, `ScreenLeaseId` et `ConnectionGeneration`. Les conserver distincts dans les structures et signatures. Les booléens `authorizedByApproval` et `authorizedByStanding` de la cible deviennent une enum d'autorité explicite avec issuer, compte, action canonique, expiration, policy version et preuve. Un token backend ne doit pas permettre à un renderer non privilégié d'inventer cette autorité.

Une demande entrante traverse la séquence suivante:

1. Parse et validation de version/capabilities à la frontière du transport.
2. Déduplication durable sur scope de compte + RequestId ou nonce de mutation.
3. Résolution de route et calcul canonique de `ActionDescription`.
4. Décision de policy, puis approbation humaine si nécessaire.
5. Binding de la décision à la commande finale, aux fichiers, à la machine et aux leases.
6. Exécution par l'outil sélectionné avec cancellation token et budget.
7. Journal durable de stdout/stderr/result, puis résultat final livré avec ack/replay.

L'auto-review appartient à `ApprovalService`. Séparer le verdict du reviewer, la règle admin, la règle utilisateur, le résultat humain et l'action finalement exécutée. Si une commande est réécrite après approbation, elle repasse par une nouvelle évaluation. Ne pas transporter une permission non liée à la commande en simple booléen.

### Annulation et reprise

Définir au moins `Queued`, `AwaitingApproval`, `Running`, `Background`, `Cancelling`, `Succeeded`, `Failed`, `Cancelled`, `OutcomeUnknown`. Une cancellation de RPC signifie « ne plus attendre/retourner à ce caller »; une cancellation d'exécution arrête l'outil; une interruption de run empêche de nouvelles actions et annule celles dont la politique autorise l'arrêt. Ces opérations portent des identifiants distincts.

Le daemon conserve le résultat final avant de l'envoyer. Le backend ack ce résultat. Une nouvelle connexion rejoue les frames non ackées sans réexécuter l'action. Une requête cancel ne peut pas attendre derrière les huit exécutions bloquées. Les outboxes, queues et buffers sont bornés par compte et par machine. Un transport qui refuse un compte ne transfère jamais les réponses de ce compte vers un autre compte.

Pour une mutation externe comme iMessage ou mint 1Password, une faute après effet potentiel produit `OutcomeUnknown`. La reprise consulte un registre ou un statut distant avant de refaire l'action. Le cache mémoire de 64 sends et le flag mémoire `deliveryIndeterminate` observés sont des signaux utiles, mais insuffisants pour une garantie à travers redémarrage.

Superviser par identité de processus plus génération et preuve de début. Garder un seul writer par fichier d'état ou remplacer les fichiers partagés par IPC et une base locale transactionnelle. Une instance ancienne ne peut pas supprimer le fichier de découverte d'une nouvelle. Les générations de compte, clés et proxy restent des données de migration explicites.

### Fichiers et shell

`LocalFileService` exige un root handle, des chemins relatifs et une résolution symlink contrôlée. Les transferts lisent en flux avec quota réel, hash, offset, numéro de chunk et finalisation atomique. Une lecture de stat suivie d'une lecture illimitée ne suffit pas. Les sorties shell portent taille maximale, budget disque pour spill, ordre stdout/stderr et compteur de séquence.

`ShellService` reçoit une politique OS explicite et une description d'accès fichiers/réseau. Sa racine de fichiers ne remplace pas cette politique. Le working directory doit faire partie de l'action approuvée. Les tâches background sont détenues par un `BackgroundJobRegistry` avec durée de vie, politique d'arrêt, processus descendants et reprise d'affichage. Les variables de requête sensibles et les tokens privés ne sont pas exportés à la shell.

Le contrat de montage de fichiers local/cloud, s'il est retenu dans le produit indépendant, doit être un service spécifique avec roots, modes lecture/écriture, quotas, cohérence, ACL de bot et compte. Il ne doit pas être déduit du panneau `desktop-mount`.

### NativeBroker et egress

WebAuthn reste une opération avec consentement par cérémonie, origin/rpId/challenge fixés, une seule cérémonie active et PIN lié au prompt ID. Vérifier l'authenticité et l'intégrité du helper avant lancement, et borner ses stdout/stderr. Les PIN et credentials privés ne sont ni journalisés ni persistés. Le signer doit refuser les incohérences origin/rpId et les challenges expirés; cette obligation n'est pas prouvée par les JS inspectés.

1Password reste un provisioning à commandes fixées, avec token remis à un secret sink. Le frontend voit une référence opaque. Persister la génération et le résultat indéterminé avant de proposer une nouvelle émission. Le launcher natif doit avoir son propre contrat audité; ne pas réutiliser les octets `observed-untrusted` comme base de confiance.

L'egress applique une lease de propriétaire, contrôle hostname et toutes les IP résolues selon la policy, conserve le binding IP jusqu'au connect et refuse les règles inconnues. Prévoir les quotas de flux, buffers et idle timeouts des sockets. Les assouplissements de policy demandent une nouvelle version/lease; les restrictions s'appliquent immédiatement aux nouvelles connexions et selon une règle définie aux flux ouverts. Le proxy client corporate reste une configuration séparée, avec une politique explicite sur l'échec de configuration au lieu d'un fallback direct implicite.

## Obligations de vérification de la reconstruction

| Cas à provoquer | Assertion requise |
|---|---|
| Deux requests avec le même ID en vol | Une seule action; protocole refuse ou rejoue un résultat durable selon contrat |
| Changement de compte pendant mint, approval ou sync | Aucune décision/token/réponse ne rejoint le nouveau compte |
| Port renderer remplacé | Ancienne génération ne modifie ni connexion ni transcript courant |
| Coupure après POST accepté | Rejeu avec même nonce sur route compatible; aucun doublon d'effet |
| Coupure après iMessage envoyé | État inconnu/reconciliation ou déduplication durable, jamais retry aveugle |
| Restart daemon pendant outbox non ackée | Résultats repris sans réexécution |
| Cancel en backlog et cancel en cours | Queue retirée; processus/outil arrêté selon politique; résultat tardif ignoré sans perte de résultat durable |
| Passage foreground → background | Tâche garde un propriétaire et un budget; cancellation de caller ne la laisse pas orpheline |
| Rotation de clé/génération/proxy | Ancien daemon ne reste pas titulaire par PID seul |
| Retraite de compte concurrente à spawn | Aucun nouveau daemon ne démarre avec la génération retirée |
| Root symlink, traversal, fichier qui grossit | Aucun accès hors root ni dépassement de quota réel |
| Shell cwd hors root | Décision OS/approval explicite; l'API fichier ne fait pas office de sandbox |
| Reviewer approuve puis commande modifiée | Autorisation invalidée et nouvelle décision |
| WebAuthn cancel, prompt remplacé, PIN tardif | Aucune signature/PIN n'atteint une autre cérémonie |
| 1Password complete timeout après mint | Mutation suivante bloquée jusqu'à reconciliation persistée |
| Egress DNS public vers privé, policy qui change | Refus correct avant connect et comportement défini des flux déjà ouverts |
| Deux bureaux réclament l'egress | Une lease active; l'ancien titulaire détruit ses flux |
| Deux bots utilisent un écran | Entrées attachées à leurs leases; absence de contamination du focus/input |
| Viewers VNC et sites ordinaires | Presse-papiers/bridge privilégié disponible seulement sur le viewer autorisé |
| Capability déclarée sans handler | Refus clair; aucune exécution découverte par nom de descripteur |

## Inconnues et couverture restante

La chaîne client d'exécution est largement établie statiquement. Les limites suivantes empêchent une équivalence totale avec le produit fermé:

- Implémentation de l'agent distant, de la box gateway et du backend Connect, dont policy auto-review, appels modèles et dispatch computer-use.
- Allocation réelle des displays/forks, règles de partage du stockage et des sessions navigateur entre bots.
- Implémentation active des montages de dossiers locaux côté serveur, si elle existe.
- Durabilité/déduplication serveur réelle, pruning des requêtes et garanties transactionnelles des endpoints. Le client sait reconnaître une preuve de dedupe; il ne démontre pas son implémentation.
- Authentification et validation interne des sockets sidecar, intégrité des opérations natives Messages et confinement réel de `sand-op-launcher`.
- Validation WebAuthn du signer, gestion de présence/UV et sélection des devices. Les événements stderr ne prouvent pas leurs contrôles cryptographiques.
- Chemin complet de lecture/remplissage des secrets 1Password dans la box et observation éventuelle de ces valeurs par un agent.
- Résistance aux crashs au milieu du rename des fichiers, et éventuels TOCTOU de `LE` avant read/ls. Des fonctions no-follow existent, mais chaque caller n'utilise pas nécessairement leur mode root-handle.
- Paramètres réellement activés, permissions OS et conformité des services en fonctionnement. Aucun runtime de daemon ou helper n'a été lancé ici.

La reconstruction Rust devra définir ces contrats manquants et les prouver. Copier les types protobuf et la forme des fenêtres ne suffirait pas à reconstruire leur comportement serveur.
