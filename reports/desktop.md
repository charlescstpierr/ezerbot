# Client desktop, frontières Electron et obligations Rust

Ce rapport couvre le client installé Grok Bot 0.70.2. Le comportement récupéré impose surtout une séparation entre interface, identité, services système et coordinator. Le passage à Rust doit conserver les transitions d'état et les limites de confiance. Remplacer Electron par une fenêtre native ne suffit pas.

## Nature et limites des preuves

Toutes les observations ci-dessous viennent de lectures statiques par shell des bundles extraits. Cette sous-analyse n'a consulté ni modifié aucun compte, magasin de secrets, session active, fenêtre Grok Bot ou service distant. Il n'y a donc aucune observation d'exécution dans ce rapport.

Les numéros de ligne désignent les copies dérivées et formatées dans `artifacts/formatted`. Les symboles courts comme `u6`, `Jj`, `NXe` ou `hg` sont les symboles réellement présents dans les bundles compilés. Ils ne sont pas les noms TypeScript originaux. Le bundle conserve parfois un nom de classe, une chaîne diagnostique ou une clé de registre. Ces noms sont cités seulement lorsqu'un chemin d'appel les relie au comportement décrit.

L'ASAR initial n'a pas de fichiers `.map`. L'inventaire commun déclare 673 fichiers et six contradictions d'intégrité dans les composants natifs. `sand-notification-settings.node`, utilisé dans ce périmètre, reste `observed-untrusted`. Sa présence et son appel par JavaScript sont observables. Son résultat natif effectif n'est pas établi.

La référence REA commune fournie par le responsable est `ev_a8b822a62831e30bd01eb4c4488b36f0bd50cb8ccc41d83e2dcff4e00cf82a4d`, avec résumé dans `evidence/rea-summary.json`. Elle a analysé syntaxiquement 600 fichiers JavaScript sans échec de parse. La couverture du graphe et de la sémantique reste `PARTIAL`. Les conclusions détaillées de ce rapport reposent sur les preuves shell ci-dessous et ne sont pas attribuées à des identifiants REA inventés.

La tentative d'instrumentation du responsable n'a pas fourni de preuve runtime authentifiée. Sa lecture statique du framework Electron, `evidence/electron-fuses.json`, observe `RunAsNode` et `EnableNodeCliInspectArguments` désactivés. Ces réglages expliquent une incompatibilité possible de l'instrumentation; ils ne prouvent pas à eux seuls la cause de tout échec runtime. Aucun byte de l'app ni signature n'a été modifié pour les contourner.

### Référentiel des fichiers

Chaque référence `D1`, `D2`, etc. désigne le chemin exact suivant. `Lx-Ly` indique les lignes de la copie formatée du même chemin. Le préfixe absolu est `/Users/cstpierrecharles/Desktop/grokbot-rust/`.

| Référence | Fichier extrait et copie formatée | SHA-256 du fichier extrait original |
| --- | --- | --- |
| D1 | `artifacts/client/dist/electron-main/main.cjs`; `artifacts/formatted/dist/electron-main/main.cjs` | `799f945a2ba26a4c2c1454172715bf0c4bad224e6ba26b2e0da064e516a32f37` |
| D2 | `artifacts/client/dist/electron-main/main-core.cjs`; `artifacts/formatted/dist/electron-main/main-core.cjs` | `e9eca6e83d4f7cae0efe7326b286ebcc2ebd94bc044e3e6029a091a7ecb9ef95` |
| D3 | `artifacts/client/dist/electron-main/main-app.cjs`; `artifacts/formatted/dist/electron-main/main-app.cjs` | `06756304c69a22e75aee48892fb0ac42eb7eb22feec239f1deee47e79c60131a` |
| D4 | `artifacts/client/dist/electron-preload/preload.cjs`; `artifacts/formatted/dist/electron-preload/preload.cjs` | `e05b76ff988aa5d6090d9d584db03f5e2c536f7d9497617aefb5cd9b929863b8` |
| D5 | `artifacts/client/dist/renderer/assets/index-D6ReTfRE.js`; `artifacts/formatted/dist/renderer/assets/index-D6ReTfRE.js` | `f3c242161efb08158161e259960b0ae91c150f6891983f6a70faf80c6f5ee697` |
| D6 | `artifacts/client/dist/renderer/assets/index.eager-app-BfVhIMYf.js`; `artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js` | `09f8a360a138c4680956c797c8a0fc25fbf619c9421327be75da0935a9aa9e80` |
| D7 | `artifacts/client/dist/renderer/assets/index.eager-platform-DUN5VkaE.js`; `artifacts/formatted/dist/renderer/assets/index.eager-platform-DUN5VkaE.js` | `560912e771f4989101d04ad06dca024cd1c0513cbdcad122222d9a6bd1df1183` |

Les termes « observation » et « inférence » sont employés littéralement. Les obligations expriment le comportement à reproduire. Les choix Rust de la dernière section sont des décisions proposées pour la reconstruction, pas des propriétés récupérées de Grok Bot.

## 1. Démarrage et répartition des processus

### Démarrage en deux étapes

**Observation statique.** Le point d'entrée `te({ runBundle })` exécute d'abord `main-core.cjs`, exige les exports `remainderDue` et `noteStartupFailed`, puis attend `remainderDue` avant d'exécuter `main-app.cjs`. Les deux bundles passent par `ee`, qui enveloppe le CommonJS dans `vm.Script`, tente un cache de code V8 et expose sa mesure à `reportMainCodeCache`. Le cache est un cache de compilation, pas une base de conversations. `Je` génère les données V8, les écrit atomiquement et `Ze` enlève les anciennes entrées et temporaires. Sources: D1 `te`, `ee`, `Je`, `Ze`, L398-L519.

**Observation statique.** Le noyau acquiert le verrou d'instance unique pour les builds packagés. Une instance secondaire quitte. Il installe les événements d'activation et de lien profond, construit le magasin `settings.json`, choisit accélération GPU et langue, puis attend `app.whenReady`. Ensuite il crée la fenêtre, installe le boot snapshot et les protocoles d'images, lance le document renderer, fournit le contexte au second bundle et résout `remainderDue` après `dF`, qui suit le chargement des ressources initiales. Le délai exact observé n'a pas été mesuré. Sources: D2 `$d`, `Mh`, `ks`, `$9`, `EDe`, L45381-L45651; `yD`, L21425-L21442.

**Observation statique.** La fenêtre initiale affiche un renderer avant que tous les services de `main-app` soient disponibles. Le preload lit synchroniquement `sand:boot-snapshot-sync`, conserve les valeurs de thème, langue et quelques options, puis `cs(ls(...))` suspend les invocations RPC tant que `mainEdge` n'est pas `served`. Lorsque les services et le menu sont assemblés, `ge.markMainEdgeServed()` envoie `sand:main-edge-served`. Sources: D4 `ls`, `cs`, `Cp`, L4981-L5074; D2 `V9`, L45416-L45424; D3 L127021-L127044.

**Obligation Rust.** Définir explicitement `BootPhase` et un snapshot initial disponible avant le service complet. L'interface peut peindre sa fenêtre et ses états de chargement sans commander des services non prêts. Les commandes doivent attendre la disponibilité ou retourner une erreur classifiée. Préserver le démarrage échoué et l'annulation lors de la fermeture.

### Frontières entre renderer, hôte système et coordinator

| Acteur observé | Responsabilité démontrée | Preuve shell statique |
| --- | --- | --- |
| Processus principal Electron | Fenêtres, menu, réglages, identité, stockage chiffré, notifications, updater, transport RPC `main`, création du coordinator | D2 `hg`, L32067-L32116; D3 `g8e`, L109320-L109335; montage des dépendances L126609-L126843 |
| Renderer | Vue React compilée, état client, restauration par compte, navigation et affichage; acquiert le port coordinator | D5 création de `On`, L139650-L139759; D6 `NXe`, L77830-L77934 |
| Preload sandboxé | Capacités étroites `desktop`, `coordinatorPort`, `desktopMount`, `sandRenderTrace`; aucun Node intégré dans la vue | D4 `Cp`, L5014-L5080; D2 `hg`, L32078-L32110 |
| Utility process `sand-node-agent-coordinator` | Contrôle, données renderer et données main sur trois canaux distincts | D3 `BYe`, `NYe`, L100083-L100140; `R5e`, L107352-L107376 |
| Guests webview | Pages externes, VNC et adaptateurs spécialisés; permissions différentes de celles du main renderer | D3 `Ogt`, L125736-L125906 |

**Observation statique.** `BYe` fait `utilityProcess.fork` via `R5e`, ouvre trois `MessageChannelMain` et transfère trois ports dans `bootstrap`. Le canal contrôle porte les exécuteurs et événements. Les deux autres portent respectivement les données renderer et main. `QYe` accepte la demande `sand:coordinator-port-request` seulement après `assertTrustedRequester`, mémorise une demande en attente tant que le runtime de compte n'est pas fourni, puis transfère le port vers `sender.postMessage`. Sources: D3 `BYe`, L100083-L100140; `QYe`, L100273-L100310; `zKe`, L98241-L98262; `R5e`, L107352-L107376.

**Observation statique.** Le lancement coordinator est renouvelé après sortie avec backoff. Une demande de nouveau port peut précipiter une relance ou remplacer un lancement dont le port a déjà été livré. La fermeture du processus ferme ses trois extrémités. Le chemin `dispose` demande d'abord la fin du contrôle puis tue le processus lorsque le contrôle est terminé. Sources: D3 cycle `FYe`, L100173-L100269; `BYe.dispose`, L100118-L100133.

**Inférence.** La séparation processus n'est pas seulement un choix de performance. Le service coordinator est créé pour un principal précis dans `R5e`, tandis que les capacités système restent dans le main. Un port est donc une capacité liée à un cycle de vie de compte. Une reconstruction qui réutilise sans invalidation un canal global entre tous les comptes perdrait cette propriété.

**Obligation Rust.** Garder un service de session distinct du thread de rendu, une identité de génération et des canaux révocables. Le coordinator peut devenir un processus Rust avec des canaux locaux ou des acteurs Rust si la frontière de crash et de sécurité est volontairement redéfinie. Il faut alors justifier ce changement et reproduire l'arrêt, la relance, le remplacement des abonnements et la perte de connexion.

## 2. Contrat IPC et confiance

### Contrat de commandes

**Observation statique.** La déclaration `os = Ce("main", ...)` du preload crée des méthodes avec schémas d'arguments. Les primitives savent valider chaînes, booléens, nombres finis, objets, tableaux, unions et `Uint8Array`. Des domaines spécialisés valident notamment des chemins relatifs, une timezone IANA, un entier de cents, un profil image limité à 2 MB et une clé de compte. La plupart des commandes système sont explicitement déclarées, par exemple `setThemePreference`, `setSpellCheckEnabled`, `getUpdateStatus`, `loginCursor`, `switchCursorAccount`, `readClientPersistence` et `stageAttachmentBytes`. Sources: D4 validateurs L354-L526; `os` L4267-L4855; profil `ap` L4215-L4231.

**Observation statique.** `Fe` fabrique les proxies et utilise les canaux `dune-rpc:{edge}:m:{method}`. Les événements utilisent `dune-rpc:{edge}:e:{event}`. Une réponse doit respecter `{ok: true, value}` ou `{ok: false, failure}`. Le proxy transforme une réponse mal formée, une méthode inconnue, une erreur de transport ou une erreur métier en `EdgeCallFailure`. Le probe `dune-rpc:{edge}:probe` distingue une méthode absente d'un transport perdu. Les listeners reçoivent la charge utile, puis rendent une fonction de désabonnement. Sources: D4 `Qs`, `Bt`, `ed`, `Fe`, L550-L637.

**Observation statique.** Côté main, `Jqt` vérifie la confiance avant les arguments et avant l'exécution. `Kqt` exécute le schéma Standard Schema et classe une validation échouée ou cassée. Les erreurs connues `It` conservent code et détail; les exceptions non prévues deviennent `edge/handler-failed`. Codes récupérés: `edge/unknown-method`, `edge/untrusted-sender`, `edge/handler-failed`, `edge/invalid-args`, `edge/validator-failed`. Sources: D3 `Kqt`, `Yqt`, `Jqt`, `qc`, L41855-L41997.

### Qui peut invoquer le main

**Observation statique.** `d8e` marque `isAppWindowTopFrame` uniquement si `sender` est le `webContents` de la fenêtre principale et si `senderFrame` est son `mainFrame`. `u8e().appWindow` refuse les autres appelants. `g8e` utilise ces faits pour toutes les méthodes montées par `p8e` avec `ir("appWindow", ...)`. Le canal coordinator utilise une vérification indépendante équivalente dans `btn` et `zKe`. Sources: D3 `d8e`, `u8e`, L107919-L107939; `g8e`, L109320-L109335; `p8e`, L108086-L108103; `btn`, `Ctn`, `zKe`, L98240-L98262.

**Observation statique.** La fenêtre est `contextIsolation: true`, `nodeIntegration: false`, `sandbox: true` et `webviewTag: true`. `Ag` refuse les nouvelles fenêtres et remet leurs URL à l'ouverture externe; il empêche une navigation principale vers une autre origine ou un autre chemin. Un clic de lien canvas utilise en plus un jeton de geste récent d'au plus 1 s dans `Tg`, venant d'une frame autorisée. Les schémas admis par cette voie sont `http`, `https`, `mailto`, `tel`. Sources: D2 `hg`, L32078-L32111; `Ag`, L32152-L32174; `Tg`, L32118-L32146.

**Observation statique.** `Ogt.hardenWebviewAttach` force les mêmes options de sandbox, choisit `preload-vnc.cjs` pour la partition VNC et `preload-webview.cjs` sinon, et retire un `src` hors `http`, `https` ou `about`. La partition VNC possède son RPC propre et une confiance basée sur la partition et sur l'URL de frame autorisée. Elle ne reçoit pas la confiance de la fenêtre principale. Le main lit les tokens de route VNC depuis l'URL et les transforme en en-têtes pour les assets et sockets de cette partition. Sources: D3 `Ogt`, L125736-L125906. Aucun token effectif n'a été lu.

**Obligation Rust.** Transformer le contrat en commandes et événements Rust typés, valider aux frontières, préserver les enveloppes d'erreur et l'ordre « confiance, arguments, exécution ». L'accès au coordinator, au presse-papiers, aux fichiers, aux secrets et aux fenêtres doit dépendre d'une capacité explicite. Avec une UI native Iced, cette limite reste nécessaire entre l'UI, les plugins, les documents externes et les processus auxiliaires, même si `senderFrame` disparaît.

**Inconnue.** Les méthodes IPC ne déclarent pas toutes un schéma complet de résultat. La lecture des seuls arguments n'établit donc pas un schéma Rust complet du produit. Une génération initiale peut extraire les déclarations; chaque résultat et événement doit ensuite être corroboré par son producteur et consommateur.

## 3. Authentification et comptes

### Connexion par navigateur et polling

**Observation statique.** L'action UI `uAe.login()` invoque `loginCursor`. Le handler main appelle `cursorAccount.login` via `Wl`, puis `MQe` délègue à `vy`, qui attend le règlement du runtime de compte avant de rendre le statut. `u6`, nom exporté `SandCursorAuthService`, ouvre le navigateur avec une URL de connexion, publie un vol `waiting-for-browser`, attend le résultat, stocke la paire puis publie `logged-in`. Sources: D6 `uAe`, L54356-L54533; D3 handlers L108798-L108810; `MQe`, L81990-L82028; `vy`, L81354-L81365; `u6.runLogin`, L36367-L36450.

**Observation statique.** Pour le flux non développement, `TRe` choisit `wv`. `t1t` crée 32 octets aléatoires, les encode en base64url sans padding comme verifier, applique SHA-256 pour le challenge et crée un UUID. L'URL `/loginDeepControl` porte `challenge`, `uuid`, `mode=login`, `redirectTarget=sand`, `supportsSelectedTeamLogin=true`. Le verifier reste dans les métadonnées de polling. Sources: D3 `t1t`, L32500-L32524; `wv.startLogin`, L32703-L32721; `TRe`, L32766-L32784.

**Observation statique.** `r1t` demande `/auth/poll?uuid=...&verifier=...`, jusqu'à 150 tentatives. Le retard croît à partir de 1 s avec facteur 1,2, plafonné à 10 s. Un 404 remet le compteur d'erreurs à zéro. Trois erreurs consécutives hors 404, un résultat 200 mal formé ou l'annulation rendent une absence de résultat. Un 403 portant le code de refus de politique produit `Mr`. Le service ajoute un délai global de 5 min à l'attente navigateur. Le choix développement `_Z` reproduit une autre implémentation de polling mais `TRe` le réserve à `usesDevAuthFlow`. Sources: D3 `r1t`, L32600-L32679; `TMe`, L35317-L35325; `u6.waitForBrowserLogin`, L35478-L35489; `_Z`, L32832-L32912.

**Inférence précise.** Le mécanisme challenge/verifier a la forme d'une preuve de possession de type PKCE. Cette forme est établie par le calcul. L'implémentation et la validation côté serveur ne sont pas récupérées ici. Le lien profond de l'application n'est pas le retour principal de ce flux de connexion: le client attend le polling backend.

### Statuts, rotation et courses

**Observation statique.** Le renderer sépare statut d'authentification et vol de connexion. `uAe` gère `logged-out`, `logging-in`, `logged-in`, le vol, `isPending`, `isLoaded`, l'erreur et l'avatar. Les publications portent une `freshness` monotone. Les résultats asynchrones trop anciens sont ignorés; des compteurs propres à l'auth et à l'avatar empêchent une réponse d'un compte précédent d'installer son profil sur le compte courant. Sources: D6 `Nk`, `Dk`, `uAe`, L54313-L54575.

**Observation statique.** `u6` possède `authOperationEpoch`, `pendingLoginEpoch`, un `AbortController`, `credentialMutationTail`, `refreshFlights` par refresh token et `switchesInFlight` par compte. Une connexion suivante annule la précédente. Chaque écriture de credentials revérifie l'epoch. Si l'epoch change entre écriture access et refresh token, `rollbackSupersededAuthentication` empêche de publier une paire obsolète. Sources: D3 champs et constructeur L35365-L35474; `advanceAuthOperationEpoch`, `mutateCredentials`, L35501-L35543; `runLoginAttempt`, L35969-L35999; `writeAuthentication`, L36455-L36496.

**Observation statique.** La rotation fait POST `/oauth/token` avec `client_id`, `grant_type=refresh_token`, `refresh_token`. Le service partage les rotations concurrentes d'un même token, vérifie l'epoch à plusieurs étapes, conserve une paire tournée même si l'opération est dépassée et peut récupérer une course perdue si un autre refresh token est déjà stocké. `shouldLogout` provoque un règlement de session ou un refus de politique. Les erreurs réseau, HTTP et de payload alimentent des compteurs et un backoff. Sources: D3 `refreshAccessToken`, `runRefreshAccessToken`, `rescueLostRotationRaceOrRevoke`, L36553-L36731.

**Observation statique.** L'expiration locale utilise le payload JWT parsé, avec une marge de 300 s. `Ii` décode le payload mais n'effectue aucune validation cryptographique de signature. `Mn` produit le SHA-256 du `sub` s'il existe, sinon du token. `lg` produit le hash du principal. Cette valeur est utilisée pour isoler les comptes, pas pour autoriser les opérations serveur. Sources: D2 `Ii`, `lg`, `Mn`, `Soe`, L28977-L29016.

**Obligation Rust.** Utiliser un acteur `IdentityService` propriétaire des credentials, de l'epoch et des opérations en vol. Ne pas exposer la paire au renderer. Garder les profils, statuts d'erreur et statuts du coffre distincts. Conserver la déduplication des refresh, la préservation des rotations en retard et l'annulation d'une connexion dépassée. Ne pas traiter le payload JWT décodé comme une authentification vérifiée.

### Ajout, bascule et suppression de compte

**Observation statique.** `runAddAccount` conserve le statut existant pendant l'attente, lance le même navigateur et publie un vol `add-account`. Lorsque la paire est stockée, il installe le nouveau compte. L'UI `uAe.addAccount` compare l'identité avant et après et retourne `added`, `cancelled`, `refused-by-policy` ou `failed`. Une deuxième demande très précoce est traitée comme annulée. Sources: D3 L36002-L36054; D6 L54493-L54511.

**Observation statique.** `runSwitchToSavedAccount` exige un compte sauvegardé, garde une copie logique de la paire du compte partant, annule la connexion active, avance l'epoch, installe la paire choisie et tente sa rotation. Lors d'un refus de politique ou de révocation du compte cible, il restaure le compte partant. Les opérations concurrentes vers la même cible partagent la même promesse. Sources: D3 `switchToSavedAccount`, `runSwitchToSavedAccount`, `restoreDepartingAccount`, L36055-L36136.

**Observation statique.** `MQe.switchAccount` relâche la présence client avant la bascule et la reprend en `finally`. `vy` attend `getAccountRuntime().whenIdle` puis, si le `authId` a changé, réinitialise le gestionnaire MCP et rafraîchit le host MCP. `removeSavedAccount` interdit de supprimer le compte actif, termine la session serveur au mieux et retire le compte sauvegardé. Sources: D3 `MQe`, L81995-L82020; `vy`, L81354-L81365; `removeSavedAccount`, L36201-L36226.

### Déconnexion et échec du coffre

**Observation statique.** `logout` avance l'epoch, annule la connexion, révoque l'usage local des credentials, vide les profils et tente les suppressions sérialisées. La requête POST `/auth/logout` est faite au mieux après une suppression locale réussie pour les causes concernées. Une suppression de coffre échouée produit `retained-after-failed-logout`, publie un statut et lève `AggregateError`; les credentials conservés ne redeviennent pas utilisables. Sources: D3 `endServerSessionBestEffort`, `revokeCredentials`, L36237-L36315; getters de révocation L35492-L35500.

**Obligation Rust.** La fermeture de session locale et la suppression physique du secret sont deux états distincts. Une erreur du coffre ne doit ni annoncer une déconnexion durable inexistante, ni continuer à envoyer des credentials conservés. La restauration du compte partant lors d'une cible refusée doit être une transition explicite, avec reprise des services et du cache associés.

### Départ et arrivée d'une identité

**Observation statique.** `O_e.prepareAccountTransition` ne change pas seulement les tokens. Il ferme les frames canvas, fait partir le scope de lecture des pièces jointes, informe les host settings et l'egress, dépasse la synchronisation d'équipe en cours, vide selectedTeam côté host, parque les réglages du compte lors d'une bascule ou les oublie lors d'une déconnexion, met le gateway en pause, conserve ou retire le daemon local selon politique, réinitialise MCP, efface les images d'attachments et remet à zéro notifications et badge. Sources: D3 `O_e`, L81190-L81286.

**Observation statique.** L'autorisation d'arrivée `y_e` vérifie le binding de compte et de descriptor, puis `scopeToAccount` remet les paramètres du compte cible. Le montage `nKe` révoque le port main, lie `createRuntime` à `R5e`, publie les statuts réglés et rend le nouveau runtime à `QYe`. Sources: D3 `y_e`, L81157-L81186; `nKe` montage L127344-L127376.

**Observation statique.** Dans l'UI, `beginIdentityChange` de `NXe` suspend la source coordinator, dispose le port actif et retient tout port reçu pendant la restauration. `completeIdentityChange` n'installe ce port que si l'identité restaurée est non nulle; `failIdentityChange` ferme le port en attente. La création `On` branche explicitement ces hooks. Sources: D6 `NXe`, L77830-L77934; D5 L139731-L139737.

**Obligation Rust.** L'identité doit passer par `Departing -> Restoring -> Authorized` ou `Refused/Failed`, avec une génération transport. Les anciennes réponses ne peuvent ni ouvrir une vue ni remplir un cache du nouveau compte. Mettre les données par compte dans une structure dédiée et remplacer cette structure lors d'une transition, plutôt que réinitialiser séparément des variables globales.

## 4. Liens profonds et navigation

### Parsing et livraison

**Observation statique.** La surface `rm` utilise les schémas `grokbot` et `sand`, l'autorité `app` et une longueur maximale 2048. Les chemins sont versionnés `/v1/...`. Des alias HTTPS correspondent à `https://cursor.com/grok-bot/link` et `/sand/link`. Le parser rejette caractères de contrôle, fragments, backslash, percent encoding mal formé, encodage de path, segments point, userinfo, port, autorité inconnue, paramètres inconnus ou dupliqués et valeurs hors schéma. Il produit un objet `version`, `route`, paramètres et une URL canonique. Sources: D2 `da`, L19897-L19906; `yP`, L19429-L19515; `rm`, L19976-L20035.

| Routes déclarées | Paramètres significatifs récupérés | Usage démontré |
| --- | --- | --- |
| `/v1/agent` | `id` | Recherche dans le roster puis focus; une sélection changée annule le résultat |
| `/v1/thread`, `/v1/task` | `id` de dix caractères hexadécimaux; `agent` optionnel | Ouvre le thread seulement si agent résolu et feature gate des tâches actif |
| `/v1/settings` | `id` parmi les ancres déclarées | Ouvre une section et focalise le réglage précis |
| `/v1/marketplace` | `tab=plugins|bots`, `id` optionnels | Ouvre une modale ou focalise un plugin/bot |
| `/v1/plugin/add`, `/v1/plugin/native` | identifiant numérique; ou `onepassword|email` | Focalise le plugin à ajouter |
| `/v1/bot-template` | share ID de 21 caractères | Ouvre la vue de partage/import de template |
| `/v1/create-team-bot` | aucun | Vérifie équipe et multiplayer avant création |
| `/v1/sidebar` | agent, tab, automation, target optionnels | Destination déclarée, mais chemin final complet non retracé ici |
| callbacks SCM et 1Password | `state`, `code`, `error` et valeurs spécifiques | Le renderer remet les callbacks SCM au service; le main intercepte le callback 1Password |

Sources du tableau: D2 `rm`, L19976-L20035; D5 `fZ`, `p8`, `CZe`, `NZe`, `pZ`, `TZe`, L125137-L125357; D3 dispatch main L126843-L126847. `open` est aussi déclaré mais son effet UI complet n'est pas établi dans ce rapport.

**Observation statique.** `IO` écoute `open-url`, `second-instance` et l'argv initial. `bO` parse puis déduplique par URL canonique et applique une fenêtre de 2 s. La capacité de file ou de rate limit est 16. Avant readiness renderer, il met en file; après readiness, il livre. Il focalise la fenêtre à l'admission initiale. `MX` rend le renderer non prêt pendant navigation ou crash, puis ne libère la file qu'après l'annonce et la fin du chargement. Sources: D2 `my`, `RO`, `bO`, `MX`, `IO`, L16864-L17008.

**Observation statique.** L'abonnement UI `iOe` installe les listeners avant d'appeler `markDeepLinksReady`. Le main appelle alors `bde.rendererAnnounced`. Cela évite de livrer le lien entre le boot et la pose du listener. Sources: D6 `iOe`, L78643-L78658; D3 shell `markDeepLinksReady`, L126734-L126736.

### Résultat UI

**Observation statique.** La navigation utilise deux surfaces `workspace` et `overlay`. Le catalogue lie des entrypoints, frontières de chargement/erreur et vues lazy. Les groupes de réglages sont `general`, `computer`, `vault`, `usage`, `updates`. La sélection d'une section crée un lien catalogue via `SX`. La réaction `open-settings` appelle `ingestOpenSettings`. Sources: D7 `h1`, L10086-L10089, routage des surfaces L14621-L14640 et rendu L15435-L15518; D5 `lc`, `L4`, `SX`, L115022-L115061; registre `qXe`, L123067-L123177; D6 events L76380-L76388.

**Observation statique.** Un lien settings devient `SX(rP(id), {focusSetting: {anchor: id, arrival}})`. Un lien agent passe par `findAgent`, vérifie que le compteur de sélection n'a pas changé, peut vérifier l'accès backend pour un agent manquant, demander l'ajout d'un bot caché puis focaliser. Un lien de tâche attend les experiments puis `sand_tasks` et ouvre le thread. Sources: D5 `fZ`, L125163-L125176; `p8`, L125225-L125258; `CZe`, `NZe`, L125260-L125281.

**Obligation Rust.** Parser des variantes de lien, puis produire une intention de navigation typée. Une arrivée tardive doit porter identité et génération de sélection. Conserver les deux niveaux de navigation, l'overlay et la destination courante, et reproduire les ancres de réglages. Les doublons, files pleines et liens sans accès doivent être des résultats explicites.

## 5. Premier lancement et restauration de l'interface

**Observation statique.** `G2e` sonde « onboarding vu » et « connecté » en parallèle. Si non connecté, il choisit sign-in ou onboarding selon le marqueur. Si connecté et onboarding vu, il choisit shell. Sinon il compte les agents: un roster non vide marque l'onboarding vu et ouvre shell; un roster vide choisit setup si la machine est déjà onboardée, sinon onboarding. Quelques échecs liés au host ouvrent un shell provisoire ou attendent la création de la box. Sources: D6 `G2e`, L53518-L53584.

**Observation statique.** Le reducer `$2e` possède `disconnected`, `checking`, `onboarding`, `shell`, `failed`, un `runId`, un `resolveSeq`, une arrivée de compte et le caractère provisoire du shell. Les actions signed-out, signed-in, account-switched, force/skip/complete-onboarding et reconnect ont des transitions différentes. Une bascule de compte réauditée n'est pas une continuation automatique de l'onboarding ancien. Sources: D6 `$2e` et reducers L53587-L53913.

**Observation statique.** La restauration client résout le compte actuel, lit l'enveloppe persistée `client-meta.account-slot`, vide les caches étrangers, invalide les slots corrompus et stage les données du compte avant de rendre les vues. Les étapes explicites comprennent état UI, options voix, infos cloud agents, sections sidebar, dernière sélection, dernier roster, replicas de transcript, drafts et journal d'envoi. Chaque étape vérifie la génération `We()`. Sources: D6 `jO`, L52270-L52291; restauration L76710-L76807; noms de slices et étapes L1898-L1938.

**Observation statique.** La vue d'attente navigateur `AZ` rend les actions relancer login et annuler `cancelLoginFlight`. Les actions sont liées aux méthodes du store auth, pas à une requête réseau du composant. Sources: D5 `AZ`, L127153-L127257; D6 `uAe`, L54490-L54527.

**Obligation Rust.** Modéliser le gate initial et les phases de restauration comme enums séparés de la présence d'une fenêtre. Un spinner global ne couvre pas le shell provisoire, la création de host, une restauration corrompue et une connexion navigateur annulée. Restaurer d'abord le scope et les données, ensuite accepter le transport, ensuite reprendre les commandes.

**Inconnue.** Les textes localisés, les étapes détaillées de choix de rôle métier, la permission notification d'onboarding et la totalité des micro-interactions ne sont pas retracés ici. Des feature gates existent, mais leur valeur dans le compte réel n'a pas été lue. Le catalogue empaqueté prouve les vues disponibles, pas leur activation à l'exécution.

## 6. Persistance, caches et paramètres

### Racines distinctes

**Observation statique.** `Me` choisit la racine durable de données: `SAND_DATA_ROOT` absolu si présent, sinon `SAND_USER_DATA_DIR/sand-data`, sinon `~/.grokbot` pour le produit packagé standard et `~/.cursor/sand-dev` ou `sand-lab` dans les autres modes. Le noyau crée `settings.json` sous cette racine. Le `userData` Electron est un autre chemin, modifiable par `--user-data-dir` ou l'environnement. Il y stocke fenêtre, secrets et persistence client. Sources: D2 `By`, `_l`, `Me`, L19246-L19291; `sU`, L31277-L31313; création `ks`, L45468-L45478.

**Obligation Rust.** Ne pas tout placer arbitrairement dans une nouvelle base globale. Distinguer données métier, configuration de host, état UI, caches et secrets. Toute importation de l'app existante doit être un outil dédié, avec opt-in concret, pas un accès silencieux aux fichiers actifs. Cette investigation n'a fait aucune importation.

### Réglages par appareil et par compte

**Observation statique.** Le schéma `wre` contient préférences de thème et langue, spell check, GPU, notifications sonores, timezone, egress, WebAuthn, Messages, update track, auto update idle, et données account-scoped. Valeurs par défaut démontrées: auto update idle `true`, egress `false`, WebAuthn proxy `true`, bot color `false`, Messages `false`; les getters peuvent affiner ces defaults selon préférence explicite. Le schéma conserve `accountScopes` et `activeAccountScope`. Sources: D2 `wre`, `TM`, L27403-L27485; getters L27850-L27909.

**Observation statique.** `scopeToAccount` sauve les paramètres du compte partant dans `accountScopes`, applique ceux du compte entrant et conserve le marqueur onboarding seulement s'il appartient au bon compte. `parkAccountScope` sauve avant clear; `forgetAccountScope` retire le compte indiqué. `loadForWrite` refuse d'écraser un magasin illisible, avec exceptions ciblées pour appliquer certaines valeurs restrictives. Sources: D2 `scopeToAccount`, `clearAccountScope`, `parkAccountScope`, `forgetAccountScope`, L27955-L28005; `eg`, `Ac`, L27445-L27465; usages `overwritingUnreadable`, L27850-L27882.

**Observation statique.** `ig` suit `system|light|dark`, réagit au thème système et publie seulement les changements. Le thème explicite est appliqué avec délai de 500 ms pour la source native. `rg` résout la langue à partir de préférence et langues système, active la locale puis publie. Le renderer recharge le document si la langue résolue diffère de celle chargée. Spell check est modifié immédiatement dans `defaultSession`; GPU conserve une distinction « paramètre voulu » et « actif au lancement ». Sources: D2 `ig`, L28589-L28647; `rg`, L28263-L28289; D3 paramètres L108318-L108340 et branchements L126644-L126667; D5 réaction langue L139782-L139791.

**Obligation Rust.** Définir des réglages appareil distincts des réglages compte. Prévoir un état `requested` et `effectiveAtLaunch` pour le GPU, une résolution système du thème et de la langue, et un résultat « enregistré mais host non synchronisé » lorsque la synchronisation distante échoue. Ne pas écraser des réglages illisibles avec des defaults ordinaires.

### Persistance UI durable

**Observation statique.** `Gpt` crée `Jj` sous `userData/sand-client-persistence`. Les méthodes du main `readClientPersistence`, `readManyClientPersistence`, `writeClientPersistence`, `removeClientPersistence` et migration délèguent à ce store. `UXe` les adapte côté renderer, et le point d'entrée crée la persistence avec `window.localStorage` comme source résiduelle de migration. Sources: D3 `Gpt`, L120469-L120522; `Jg`, L126045-L126056; handlers L109101-L109111; D6 `UXe`, L79248-L79262; D5 L139679-L139697.

**Observation statique.** `Jj` accepte seulement les clés `sand.`. Les clés courtes deviennent noms base32 `.blob`; celles dont le nom dépasserait 240 caractères deviennent SHA-256 base32 `.kblob`, avec la clé originale encadrée dans le contenu. `ledger` vérifie l'identité de ces fichiers et nettoie les `.tmp`. Chaque valeur est limitée à 8 MiB et le total à 256 MiB. Les mutations et lectures passent par une chaîne unique. Les écritures vont d'abord en `.tmp`, puis rename avec quelques reprises pour `EPERM|EBUSY|EACCES`. Une collision de digest est explicitement refusée. Sources: D3 `lpt`, `hpt`, `vxn`, L119758-L119826; `Jj`, L119865-L120108.

**Observation statique.** La migration localStorage ignore les clés déjà présentes, saute une entrée trop grande, écrit un marqueur `.migrated-from-local-storage` seulement à la fin et peut être rejouée après une erreur. Le renderer ne retire les anciennes clés `sand.` du localStorage que si la migration a réussi. Pendant une migration non réussie, `vOe` peut encore lire le résidu. `will-quit` attend le store avec un délai de 2 s puis relance le quit. Sources: D3 `Jj.migrateFromLocalStorage`, L120064-L120107; D6 `mOe`, `SOe`, `vOe`, L79148-L79237; D3 fin L127514-L127530.

**Inférence.** Un store Rust transactionnel peut remplacer les fichiers blob sans obligation de reproduire ce format, si la cible est une reconstruction indépendante. Si l'objectif inclut compatibilité des données existantes, les limites, encodages, marqueur et règles de reprise deviennent des obligations d'import/export. Dans les deux cas, la propriété essentielle est l'isolation des slices par compte, version de schéma et génération de restauration.

### Coffre de tokens et coffre de secrets utilisateur

**Observation statique.** `sand-secrets.json` contient des chaînes chiffrées par `safeStorage`, version de fichier 1. Le champ logique `cursor-accounts` contient `active` et un dictionnaire de comptes, avec access token, refresh token, selected team et profil chiffrés par compte. Les anciennes clés peuvent être migrées vers ce dictionnaire. Les accès utilisent un durable file avec règle no-follow et mode 0600 sur les plateformes concernées. Sources: D2 constantes L32648-L32673; `XF`, L32755-L32768; `_u`, `Eu`, `ZA`, `Bie`, L32822-L33076; `BF`, L32842-L32857.

**Observation statique.** Quand le chiffrement système est indisponible, le service conserve les tokens en mémoire pour la session et signale l'absence de persistence. Sur Linux, le wrapper peut activer explicitement le backend `basic_text` de `safeStorage`; ce comportement du wrapper n'établit pas un chiffrement robuste. Les anciennes entrées `plaintext:v1:` possèdent un chemin de migration. Sources: D2 `Iie`, `Lr`, `KF`, L32698-L32722; `Tu`, `Au`, L32993-L33019 et L33083-L33094.

**Observation statique.** `ez` représente un autre store, `user-secrets.json`, version 2 par compte. Il chiffre chaque valeur, conserve un cache mémoire, refuse la modification sans compte et exporte des snapshots pour synchronisation à la box. L'absence de coffre produit une session mémoire. Une valeur persistée indéchiffrable interdit le snapshot avec erreur spécifique, au lieu de l'omettre silencieusement. La migration version 1 vers compte arrive au premier scope pertinent. Sources: D3 `Dxn`, `ez`, L120317-L120467; `Gpt`, L120469-L120522.

**Obligation Rust.** Un coffre OS est un adaptateur dédié, avec état `Available`, `Unavailable`, `Unreadable`, et une stratégie mémoire explicite. Le design proposé ne reprend pas par défaut le fallback Linux `basic_text`. Ce serait une décision sécurité supplémentaire, distincte de la parité des états UI. Aucune donnée secrète effective ne doit entrer dans un rapport, un log ou le canal UI.

### Cache photo et géométrie fenêtre

**Observation statique.** `hD` hash l'URL avatar et sa version en un identifiant de 32 caractères. Il gère les images de roster sur disque, déduplique les remplissages, valide le type image et sert une ressource par protocole protégé avec token aléatoire et comparaison temporelle constante. Le boot snapshot fournit l'accès photo au preload. Sources: D2 `GQ`, `hD`, L21290-L21421; boot L21425-L21440; démarrage L45535-L45553.

**Observation statique.** `window-state.json` version 1 conserve `normalBounds` et `isMaximized`. Les mouvements et resize sont regroupés à 300 ms et flushés à la fermeture. La restauration choisit le display le plus recouvert, refuse une géométrie insuffisamment visible, et borne les dimensions à la work area. Minimum 512 × 520; défaut standard 1040 × 760, ou 1440 × 960 sur cloud agent VM. Sources: D2 `ko`, `LU`, `_g`, L31689-L31783; `zU`, `eF`, `Qae`, L31971-L32045.

**Obligation Rust.** Reproduire le placement multi-écran, la géométrie normale séparée du fullscreen/maximized et le flush avant fermeture. Le cache média doit dédupliquer les téléchargements et refuser les références hors scope. Une UI Rust native peut remplacer le token protocole par une capacité interne de ressource.

## 7. Fenêtres, événements OS et guests

**Observation statique.** `UU` configure une barre de titre macOS `hiddenInset` avec contrôles à x=16/y=15. Sur Windows la fenêtre est sans frame avec titlebar overlay; Linux est sans frame. Le main transmet fullscreen, maximize, focus, gestes de scroll et historique souris à l'UI. Une fermeture de toutes les fenêtres quitte hors macOS. Sur macOS, activate sans fenêtre demande la recréation. `bht` déduplique cette recréation et `Rht` reconnecte la VNC, la présence, les broadcasts et la surveillance de crash du renderer. Sources: D2 `UU`, L31790-L31802; `VU`, L31832-L31843; événements app L45653-L45657; D3 `bht`, `Uz`, L126022-L126042; `Rht`, L126426-L126477.

**Observation statique.** Le code configure des fenêtres auxiliaires avec responsabilités séparées. `wV` ouvre un panel de montage flottant always-on-top, visible dans tous les workspaces y compris fullscreen, avec son preload et un RPC de confiance différent pour main et panel. `yut` ouvre un OAuth 1Password 520 × 720 avec parent main, sandbox et partition `onepassword-oauth`, interceptant navigation/redirection pour retourner le callback. Sources: D3 `Mnn`, `wV`, `Onn`, L101025-L101095; `yut`, L118285-L118327.

**Obligation Rust.** Conserver une distinction entre fenêtre principale, panel flottant, popup d'authentification et contenu distant. Prévoir un adaptateur OS pour les comportements de focus, dock, fullscreen, all workspaces, presse-papiers et partition browser. Avec Iced, ces capacités ne deviennent pas automatiquement disponibles: il faut les traiter comme tâches d'intégration natives à prouver sur chaque OS.

**Inconnue.** Les fenêtres passkey et fullscreen files ont des producteurs présents, mais leur parcours complet n'a pas été analysé ici. Les contrôles spécifiques Windows et Linux sont observés dans un bundle distribué sur macOS, pas exécutés sur ces OS.

## 8. Notifications, présence et badge

### Décision de notification

**Observation statique.** Le flux agent alimente les managers OS et badge via `o5e`. `sl` conserve un baseline du roster; un nouveau roster n'entraîne pas une notification historique. `dce` produit `agent-needs-input` lorsque `awaitingReason` apparaît et `agent-done` après arrêt du travail ou nouveau message. Les transitions sont filtrées par agent masqué, préférence notify, focus et throttle de 5 s par agent/type. La déduplication sait garder un historique de 256 IDs de messages. Sources: D3 alimentation L106808-L106848; `dce`, `nRn`, `sl`, L116501-L116736.

**Observation statique.** `Uj` sépare des historiques pour feed box, feed serveur et team room. Il transfère l'historique quand une source change et supprime les graines de session afin de ne pas notifier le dernier message simplement parce qu'un flux serveur apparaît. Les events delta arrivés avant baseline sont mis en attente. Sources: D3 `Uj`, L117226-L117512.

**Observation statique.** `show` crée un toast avec titre/body formatés. Une demande d'input a urgence critical. Le clic restaure, montre et focalise la fenêtre puis émet `focus-agent`, éventuellement avec message/task. Le renderer focalise l'agent et ouvre le thread si `taskId` existe. Les sons personnalisés exigent feature gate et `playSound`; le main émet `notification-sound` et le renderer le joue. Sources: D3 `settleDecisions`, `playTransitionSound`, `show`, `focusAgent`, L117573-L117663; D6 focus L76390-L76396; D5 abonnement son L139663-L139667.

**Obligation Rust.** Extraire un moteur de décision pur avec événements baseline/delta, provenance de feed et historique borné. Le rendu OS et l'audio doivent être des adaptateurs séparés. Conserver l'effet du clic et l'absence de doublon quand box/serveur se relaient. Les caches de notification sont remis à zéro lors d'un changement d'identité.

### Permission OS et présence

**Observation statique.** `Ij` charge `sand-notification-settings`, vérifie l'export `notification_settings_async`, puis `kj` l'appelle. `ATn` mappe macOS authorized/provisional/ephemeral vers granted, denied vers denied, notDetermined vers undetermined; Windows enabled vers granted et les différents disabled vers denied. En cas d'absence native, plateforme inconnue ou échec de lecture, il produit `unknown` avec raison. Sources: D3 `Ij`, `ATn`, `kj`, L115494-L115583.

**Observation statique.** `Tdt` combine connecté, feature gate, verrouillage, idle, fenêtre, liveness du feed et possibilité de banners. Il rafraîchit la permission puis publie de la présence via `reportGrokBotDesktopActivity`, qui retourne une durée de lease. Sources: D3 `vdt`, L115484-L115491; `Tdt`, L115584-L115618. L'intention est vérifiable dans le code; l'effet backend de cette lease n'a pas été observé.

**Limite d'intégrité.** Le wrapper JS est analysable. Le module native qu'il appelle a une contradiction entre digest déclaré dans l'ASAR et digest observé. Il ne faut pas annoncer que les permissions ont été lues correctement sur la machine réelle.

### Badge

**Observation statique.** `Rdt` somme les unread counts d'agents non masqués et notify-enabled, en prenant au moins 1 pour un agent unread. `wj` tient compte de snapshot epoch/seq et du roster serveur pour les agents temporal. Une liste momentanément vide peut être ignorée si elle contredirait un roster existant. Il appelle `app.setBadgeCount` seulement si le total projeté change et le remet à zéro via `reset`. Sources: D3 `Rdt`, `wj`, L115625-L115754; montage L127174-L127177.

**Obligation Rust.** Le badge doit découler d'une projection du roster et de ses versions, pas d'un compteur incrémental sur chaque event. Tester les replays, les snapshots périmés et une source vide temporaire.

## 9. Mises à jour desktop

### Contrôle et feed

**Observation statique.** `kgt` instancie `Cz`, lit settings et gates, sélectionne Squirrel.Mac sur Darwin, un installateur Windows `Tz`, AppImage `Rj` ou delivery package manager. Le feed par défaut est `https://api2.cursor.sh/updates`. `G0` compose `/api/update/{platform}/{sand|sand-nightly|sand-dogfood}/{currentVersion}/{machineId}/stable`, avec `trackChanged=1` si nécessaire. Le client utilise cache-control no-cache; 204 signifie aucune mise à jour, non-OK et JSON mal formé sont classés. Sources: D3 `kgt`, L125573-L125675; `fetchUpdate`, L124882-L124903; D2 `U0`, `F0`, `B0`, `G0`, `V0`, L30741-L30821.

**Observation statique.** `Cz` possède idle/checking/available/downloading/staging/ready, disabled et move-required. Il refuse les checks concurrents et préserve une ancienne mise à jour déjà prête lorsqu'un check ultérieur échoue. Le premier check est planifié après 30 s, puis périodiquement avec base 1 h et jitter maximal 5 min. Le check a un délai de 30 s; le staging Squirrel peut attendre 10 min. Sources: D3 constantes L124090-L124117; `Cz.start`, `scheduleNextPeriodicCheck`, L124178-L124323; `checkForUpdates`, L124451-L124575; `startStagingTimeout`, L125083-L125118.

### Téléchargement et application

**Observation statique.** `cgt` télécharge dans un fichier partiel. Il reprend avec Range et If-Range si un validator existe, vérifie le Content-Range 206, nettoie une reprise incompatible, calcule SHA-256 et compare le digest fourni lorsqu'il existe. AppImage exige un digest. Il renomme seulement après intégrité. Sources: D3 `cgt`, `RPn`, `xPn`, `MPn`, L123869-L124003; `downloadAndStage`, L124919-L124980.

**Observation statique.** Pour macOS, `stageSquirrelZip` ouvre un feed local vers le zip, `setFeedURL` puis `checkForUpdates`; le retour update-downloaded donne le state ready. Des zip vérifiés peuvent être gardés et restagés au prochain lancement. L'installateur Windows vérifie explicitement sa signature avant de passer ready. Sources: D3 `stageSquirrelZip`, L124987-L125008; `stageParkedInstaller`, L125010-L125076; retained zip L123643-L123679; restage L124700-L124739; hook Squirrel L124140-L124157.

**Observation statique.** L'application explicite porte `expectedVersion`. `quitAndInstall` rend superseded si la version affichée n'est plus celle prête, not-ready si l'état ne permet pas l'application, et requested seulement après préparation. Le path macOS vérifie la présence ShipIt et peut restager le zip si le job a disparu. L'application automatique exige mise à jour prête, setting autoUpdateWhenIdle, écran verrouillé ou screensaver, système idle depuis au moins 600 s et host confirmé idle. Une sonde host indisponible ne vaut pas idle. Sources: D3 `quitAndInstall`, `restartSquirrel`, L124627-L124691; `IPn`, `kPn`, `Yft`, `Zft`, L123726-L123760; montage sonde host L125633-L125641.

**Limite.** Le parser Squirrel récupéré ne fournit pas de SHA-256 de serveur dans sa réponse. Le client calcule un digest et l'utilise pour conserver/restager le zip, mais cette lecture ne suffit pas à prouver une vérification indépendante d'authenticité de chaque zip macOS. Les garanties effectives du mécanisme Squirrel/ShipIt ne sont pas retracées ici. Aucun feed n'a été appelé, aucun fichier d'update téléchargé et aucune mise à jour appliquée.

**Obligation Rust.** Un updater doit être un service séparé de la vue avec états versionnés, reprise téléchargement, vérification d'intégrité et politique de signature choisie explicitement. Conserver l'identifiant attendu de l'offre, le staging durable, l'application seulement quand host et machine sont idle, et le canal de mise à jour géré par policy. Le protocole de mise à jour d'une reconstruction indépendante doit utiliser ses propres feeds et clés, sans dépendre des updates de l'app originale.

## 10. Architecture desktop Rust proposée

Cette section est un choix de conception. Elle ne prétend pas reconstituer les crates utilisées par Grok Bot, puisque le client récupéré est Electron/JavaScript.

### Décision UI

La cible principale proposée par le responsable est une UI native Iced dont le code écrit est en Rust. Le domain model et la logique de coordination doivent également être Rust. Le renderer React livré fournit un catalogue comportemental et une référence de layout; il n'est pas à intégrer comme implementation produit autorisée par défaut.

Le choix impose un travail spécifique pour le Markdown riche, les blocs de code, l'éditeur de document, les widgets HTML/MCP, PDF, sélection/copie, IME, accessibility, layout animé et guests browser. Le client livré utilise les capacités DOM/WebView dans les chemins observés. Une UI native Rust ne garantit pas cette parité par simple translation des composants. Les comportements concernés doivent être inscrits dans la matrice de reconstruction comme livrables autonomes, avec preuve visuelle et interaction réelle.

Une ADR doit comparer le rendu Iced avec une alternative WebView. Une WebView réduit le coût de layout et de contenu web, mais crée un bridge, des partitions et une frontière de contenu à reconstruire. Elle peut rester un adaptateur spécialisé pour documents externes ou guests même si la fenêtre principale demeure Iced. Ce choix ne doit pas être caché dans la dépendance d'une crate.

### Modules et propriétaires d'état

| Module Rust proposé | État qu'il possède | Contrat avec les autres modules | Obligation issue des preuves |
| --- | --- | --- | --- |
| `desktop_shell` | Fenêtres, focus, display work areas, fullscreen, OS events | Commandes typées vers OS, events vers app store | Géométrie persistée; recréation macOS; focus ordonné; panel séparé |
| `desktop_app` | Gate initial, navigation workspace/overlay, intents UI | Réducteur `AppEvent -> AppState`; effects explicites | Checking/onboarding/shell provisoire/failed; parcours deep links |
| `desktop_contract` | Schémas commandes, événements, erreurs et versions | Messages sérialisés seulement aux frontières process | Validation, confiance, erreurs classifiées; pas de méthode arbitraire |
| `identity_service` | Session, epoch, vols login/refresh/switch, profils | Snapshots publics sans tokens; capacité auth privée au réseau | Browser polling; rotation concurrente; logout non durable; fallback compte |
| `account_runtime` | Principal autorisé, génération, session coordinator, état departure/arrival | Dispose ancienne session puis crée nouvelle; port révocable | Purger ressources étrangères; attendre idle; fermer ancien transport |
| `settings_repository` | Réglages appareil et dictionnaires compte | Load/save versionnés, résultat unreadable, sync host explicite | Park/scope/forget; ne pas écraser un fichier illisible |
| `client_persistence` | Slices par compte, version de schéma, ordre des mutations | Transactions async; stage snapshot; flush deadline | Isolation/restauration; caps; migration rejouable; journal d'envoi |
| `secure_store` | Tokens et secrets chiffrés, état coffre, session memory | Access uniquement via identity/secret broker | Available/unavailable/unreadable; aucune valeur au renderer |
| `notification_engine` | Baselines, historique, throttle, sources | Décisions pures puis commande au `os_notifications` | Suppression focus/pref/replay; historique inter-feed |
| `presence_service` | Fenêtre visible/focused, lock/idle, agent affiché, lease | Publications annullables liées à génération de compte | Libérer avant switch; reprendre après arrivée; ne pas confondre host idle |
| `updater_service` | Offer/version, download, staging, apply marker, policy track | UI reçoit snapshot; applique une version attendue | Reprise + intégrité; délai; expectedVersion; idle confirmé |
| `media_cache` | Références compte, mémoire/disque, remplissages | Ressource locale par handle; aucun chemin fourni sans validation | Isolation attachments; cache avatar; bounded reads |
| `guest_browser` | Partition VNC/OAuth/document, host admis, capacités | Canal réduit par guest, aucune capacité main générale | Webview hardening; clipboard VNC borné; navigation interceptée |

### Structures qui rendent les règles explicites

- `AccountScope` représente l'identité stable du principal. `SessionGeneration` représente une exécution autorisée de ce compte. Une clé de cache inclut le scope; une réponse asynchrone inclut la génération. Le hash de token ne doit pas devenir le seul identifiant métier si un principal fiable est disponible.
- `AuthStatus` distingue connecté, déconnecté, connexion en cours, coffre non disponible et credentials retenus après échec de suppression. `LoginFlight` est séparé et porte flow, démarrage et deadline. `RefreshFlight` est unique par paire logique active.
- `AccountTransition` possède phases departing, authorizing, restoring, active et failed/refused. Les commandes qui nécessitent une identité attendent la phase active ou refusent explicitement. La transition sérialise les actions sur le même compte runtime, tandis que les caches de comptes différents restent séparés.
- `PersistedSlice<T>` possède version de schéma, scope ou marqueur device-global, corruption connue et résultat de migration. Les états absent, corrupt et unreadable restent distincts.
- `DeepLink` est une enum de routes et paramètres validés. `NavigationIntent` contient origine, compte et sélection attendus. Les intentions ne transportent pas directement un chemin fichier ou un nom de méthode IPC.
- `UpdateState` garde la version dans available/downloading/staging/ready. `ApplyRequest` indique explicitement la version attendue et le trigger explicit/idle. Un état ready ne vaut pas une autorisation de quitter si une nouvelle offre l'a dépassé.

Ces noms sont proposés. Il faudra choisir les bibliothèques Rust et vérifier leurs API au démarrage de l'implémentation; le rapport n'annonce pas de versions de crates ni de capacités OS non vérifiées.

### Flux Rust attendu pour une bascule de compte

1. L'UI émet `SwitchAccount(account_id, request_generation)` et affiche l'opération, sans effacer immédiatement le compte courant.
2. `IdentityService` suspend les opérations de login dépassées, installe la paire cible via le coffre et vérifie la session. Un refus définitif restaure la paire partante.
3. `AccountRuntime` libère présence et permissions liées au compte, ferme resources guest/canvas, interrompt les anciennes requêtes, parque les réglages et arrête l'ancien coordinator selon la politique local exec.
4. L'autorisation du principal entrant règle binding et données host. Elle crée une nouvelle session coordinator et une génération transport.
5. L'UI stage ses slices du compte entrant, restaure sélection, roster, drafts et replicas, puis accepte la nouvelle source coordinator. Les réponses d'une génération ancienne sont jetées.
6. Les notifications/badges partent d'un baseline, puis la présence et les commandes reprennent. Le résultat de la requête initiale est « arrivé », « cible refusée et ancien compte restauré » ou « échec de règlement ».

L'ordre des mécanismes récupérés est réparti entre `u6`, `O_e`, `nKe` et `NXe`; ce flux de conception fusionne leurs responsabilités dans un seul contrat lisible. Il faut valider l'ordre exact d'auth publication et d'arrêt coordinator contre les tests de contrat au moment de l'implémentation.

## 11. Vérifications futures et inconnues restantes

Les cas suivants sont des critères de reconstruction, pas des tests exécutés sur l'application originale:

| Critère | Résultat attendu | Base de preuve |
| --- | --- | --- |
| Boot lent puis click settings | La fenêtre s'affiche; aucune commande n'exécute un service non servi | D1 `te`; D4 `cs` |
| Deux logins successifs; ancien résultat arrive tard | Seul le vol courant peut installer une session; rotation tardive préservée sans publication erronée | D3 epochs et refresh |
| Bascule pendant loading transcript | Les lignes, images et drafts du compte partant ne rejoignent pas le compte entrant | D3 `O_e`; D6 restauration + `NXe` |
| Suppression coffre échouée au logout | Session inutilisable et erreur explicite de non-durabilité | D3 `revokeCredentials` |
| Lien profond au cold boot ou pendant reload | Livraison après listener prêt, une fois, avec validation stricte | D2 `bO`, `MX`; D6 `iOe` |
| Roster baseline après reconnect | Aucun toast historique; badge projeté selon séquences et préférences | D3 `sl`, `Uj`, `wj` |
| Migration UI interrompue | Reprise sans écraser une clé déjà durable; résidu disponible tant que migration non réussie | D3 `Jj`; D6 `vOe` |
| Fenêtre déplacée vers écran retiré | Géométrie visible, bornée; maximise selon état précédent | D2 `LU`, `eF` |
| Update ready V1 puis offre V2 | Apply V1 devient superseded; aucun installateur non prêt lancé | D3 `quitAndInstall` |
| Idle machine mais host busy/unavailable | Pas de relance automatique | D3 `Zft`, montage sonde |
| Guest externe tente main/clipboard | Refus du main; seules capacités de partition et origine admissibles passent | D3 `u8e`, `Ogt` |

Questions partiellement résolues ou ouvertes:

- Les protocoles backend, entitlement, binding de compte, exécution des agents et schemas complets du coordinator sont traités par les autres analyses. Ce rapport établit leur entrée/sortie desktop, pas leur implémentation distante.
- La parité visuelle du renderer n'a pas été observée à l'exécution. Les CSS, composants compilés et ressources sont récupérables, mais les snapshots de référence, la navigation réelle avec compte et l'accessibility effective manquent.
- Les menus système, tous les raccourcis, toutes les fenêtres auxiliaires et tous les événements du main n'ont pas été exhaustivement tracés. Les preuves données sont des parcours concrets et des contraintes principales.
- Les permissions notification, les bindings natifs de processus, le signer WebAuthn et le launcher ne peuvent pas être déclarés fiables tant que leur contradiction d'intégrité n'est pas expliquée par une vérification indépendante. Aucun composant natif n'a été exécuté ici.
- Le client chiffré révèle le format logique et les transitions des credentials, mais pas l'algorithme de coffre OS, l'autorisation serveur ou l'état réel des comptes.
- Les cheminements Windows, AppImage et package-manager sont présents dans le bundle macOS. Leurs résultats réels et leur UX nécessitent une vérification sur l'OS cible.
- Aucun package iOS ou Android n'est présent dans la cible autorisée de ce rapport. Des noms de gates et champs mobile dans les bundles ne suffisent pas à récupérer une application mobile, sa navigation, ses permissions ou ses contraintes background. Toute parité mobile reste inconnue et demanderait une cible livrée distincte.
- Les garanties de signature macOS via Squirrel/ShipIt, le détail de la permission notification d'onboarding et l'activation réelle des gates restent inconnus dans ce périmètre.
