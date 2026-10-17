# Ressources partagées et traçabilité — approfondissement

Le helper Mac possède un bail de contrôle attaché à une session. Le client sait corréler des étapes de transport et détecter certains écarts de transcript. Le partage du bureau cloud et l’audit des décisions du moteur texte restent partiellement inconnus.

Cible : Grok Bot macOS 0.70.2 sauvegardé. Analyse statique des bundles et de huit corps de fonctions x86_64 du helper, sans exécuter l’application. Les branches Swift sont interprétées avec prudence; aucune équivalence arm64 n’est revendiquée. Les propositions Rust des rapports antérieurs ne sont pas des preuves du produit.

## 7. Qui possède une ressource quand plusieurs acteurs travaillent ?

### R1. Un seul état de contrôle natif actif

**Interprétation statique.** Le démarrage du bail de contrôle prend un NSLock, consulte un unique état active, puis compare le requestID. Si le même identifiant est déjà actif, il rend la session et la tâche existantes; un identifiant différent rend le tuple optionnel vide. Un démarrage neuf fabrique un UUID et installe la session. Preuves : [A1](../evidence/native-lease-start_entry.asm.txt:5), [A2](../evidence/native-lease-start_locked.asm.txt:1), [A3](../evidence/native-lease-start_locked.asm.txt:70), [A4](../evidence/native-lease-start_locked.asm.txt:140).

**Limite.** Le refus décrit cette closure optimisée; le résultat public async et son texte d’erreur ne sont pas reconstruits intégralement. Ce bail appartient au helper Mac, pas au bureau cloud.

### R2. L’identité de session et l’activation conditionnent l’admission

**Interprétation statique.** La closure admit compare le sessionID aux identifiants conservés, vérifie la présence de l’état actif et un drapeau de disponibilité, puis ne rend le permit actif que pour la session correspondante. La closure de fin d’activation marque ce drapeau ou appelle revokeLocked sur la branche d’échec. Preuves : [A5](../evidence/native-lease-admit_locked.asm.txt:1), [A6](../evidence/native-lease-start_ready.asm.txt:8).

**Limite.** Les valeurs numériques de l’enum d’admission restent non nommées. Les branches sont visibles; aucun clic réel ni appel agent vers ce helper n’est observé.

### R3. Une libération vise sa session

**Interprétation statique.** release compare le sessionID à la session stoppée puis à la session active. L’appel à revokeLocked n’est présent que sur la branche où la session active correspond; les autres branches rendent des variantes distinctes. Preuves : [A7](../evidence/native-lease-release_locked.asm.txt:1), [A8](../evidence/native-lease-release_locked.asm.txt:72).

**Limite.** Les noms exacts des variantes de retour ne sont pas récupérés; cette comparaison n’établit aucune politique de partage du navigateur cloud.

### R4. La disparition du titulaire révoque le bail

**Interprétation statique.** watchHolderExit crée une Dispatch process source pour l’événement exit du PID fourni. Son callback prend le verrou, compare la session encore active à la session capturée, consigne un événement diagnostic, puis révoque seulement si elles correspondent. Preuves : [A9](../evidence/native-lease-watch_holder.asm.txt:41), [A10](../evidence/native-lease-holder_exit.asm.txt:12).

**Limite.** Le suivi est optionnel lorsque holderProcessID est absent. Le comportement du noyau et les courses réelles entre sortie et nouvelle session n’ont pas été testés.

### R5. La révocation nettoie aussi les entrées maintenues

**Interprétation statique.** revokeLocked annule la source de surveillance, vide l’état actif, appelle CURemoteControlPermit.revoke qui rend un Set<CUHeldInput>, puis transmet cet ensemble à la closure releaseHeldInput et appelle onStopped. Les noms des champs conservés dans le Mach-O correspondent aux offsets utilisés. Preuves : [A11](../evidence/native-lease-revoke_locked.asm.txt:44), [A12](../evidence/native-lease-revoke_locked.asm.txt:76), [A13](../evidence/native-lease-field-offsets.txt:1).

**Limite.** Cette séquence prouve le passage à un callback de nettoyage. L’émission effective des événements clavier ou souris par ce callback n’est pas reconstruite ici.

### R6. Le tunnel réseau négocie sa possession

**Observation statique.** Le tunnel egress démarre en mode if-free. claim peut demander une reprise explicite. La connexion transmet l’identité client et la génération d’attache à reconquérir; elle ne se déclare connectée qu’après un ping, pong ou message reçu. Si un autre ordinateur détient le tunnel, le client détruit ses flux et passe en attente avant de recontrôler. Preuves : [A14](../artifacts/formatted/dist/electron-main/main-app.cjs:93357), [A15](../artifacts/formatted/dist/electron-main/main-app.cjs:93434), [A16](../artifacts/formatted/dist/electron-main/main-app.cjs:93471), [A17](../artifacts/formatted/dist/electron-main/main-app.cjs:93529).

**Limite.** Le serveur qui décide le titulaire et incrémente la génération n’est pas livré. Ce verrou réseau ne démontre aucun verrou de fichier ou d’écran.

### R7. Les contrats de conflits fichiers ne garantissent pas leur activation

**Interprétation statique.** Le SDK contient AgentStoreConflict avec cursor, journal_epoch, seq, événements de conflits et gap. Mais le constructeur actif oYe n’enregistre que shell, shell en arrière-plan, lecture et listing. Le notice executor partagé Coe rend ack/released/noted avec count 0. Aucun verrou global de fichier n’est établi par ces seuls contrats. Preuves : [A18](../artifacts/formatted/dist/local-exec-daemon/main.cjs:95716), [A19](../artifacts/formatted/dist/local-exec-daemon/main.cjs:97692), [A20](../artifacts/formatted/dist/local-exec-daemon/main.cjs:128042).

**Limite.** Le périmètre est la construction locale oYe et ses types compilés. Les uploads, Messages, services distants et autres builds peuvent avoir des politiques différentes.

### R8. Les uploads locaux sérialisent l’écriture par chemin

**Observation statique.** L’upload actif P4 résout le chemin puis appelle R2. R2 prend une file P9t indexée par chemin absolu, attend son tour et la libère en finally. L’écriture passe par un fichier temporaire, fsync puis rename. Deux uploads du même chemin dans cette instance passent donc successivement par ce writer. Preuves : [A21](../artifacts/formatted/dist/local-exec-daemon/main.cjs:128370), [A22](../artifacts/formatted/dist/local-exec-daemon/main.cjs:78990), [A23](../artifacts/formatted/dist/local-exec-daemon/main.cjs:79047), [A24](../artifacts/formatted/dist/local-exec-daemon/main.cjs:79193).

**Limite.** Cette file est une Map en mémoire du processus; le shell arbitraire ne passe pas nécessairement par ce writer. Elle ne fournit pas une version attendue, un historique de conflits ou un verrou global entre machines.

## 9. Quelles traces permettent de comprendre un incident ?

### T1. Les opérations réseau héritent d’un identifiant de trace

**Observation statique.** La gateway valide un traceparent au format version 00, rejette les identifiants nuls et crée un span enfant aléatoire de 8 octets. request l’ajoute aux headers HTTP POST et rend compte du succès ou de l’erreur du command span. Preuves : [A25](../artifacts/formatted/dist/node-agent-coordinator/main.cjs:40318), [A26](../artifacts/formatted/dist/node-agent-coordinator/main.cjs:41341).

**Limite.** Une trace valide permet la corrélation. Elle ne contient pas à elle seule les instructions sélectionnées ni la raison sémantique d’une action.

### T2. Les étapes d’envoi sont bornées et terminées une fois

**Observation statique.** beginSend indexe les corrélations par accountSlot et clientNonce. Sans nonce, il renvoie un enregistreur inerte. beginStage utilise l’horloge monotone pour la durée et un drapeau V pour ne publier qu’une seule terminaison complete ou fail. Preuves : [A27](../artifacts/formatted/dist/node-agent-coordinator/main.cjs:45404), [A28](../artifacts/formatted/dist/node-agent-coordinator/main.cjs:45473).

**Limite.** L’idempotence de la terminaison concerne ce rapport de mesure, pas l’effet externe de l’action.

### T3. Une trace peut être perdue sous charge

**Observation statique.** Le collecteur laisse au plus 64 rapports simultanément en vol et 64 corrélations echo. Les corrélations anciennes sont nettoyées après 120 000 ms lors de l’ajout suivant; au plafond, la plus ancienne est évincée. Les abandons in_flight_cap, dispatch_failed et echo_evicted sont comptés et journalisés avec une fenêtre de 60 000 ms. Preuves : [A29](../artifacts/formatted/dist/node-agent-coordinator/main.cjs:45405).

**Limite.** Ces bornes sont celles de la télémétrie du coordinator. Elles ne constituent ni un budget de bot ni une garantie de conservation complète.

### T4. L’échantillonnage dépend de la trace parente

**Observation statique.** Le tracer sand-desktop-send est construit avec un AlwaysOffSampler racine enveloppé par un ParentBasedSampler. Les spans importent un contexte distant; le provider utilise BatchSpanProcessor et un exporteur muni d’un accès au token backend. Preuves : [A30](../artifacts/formatted/dist/electron-main/main-app.cjs:107150), [A31](../artifacts/formatted/dist/electron-main/main-app.cjs:107189).

**Limite.** L’export final dépend de l’échantillonnage, du token, de la connexion et du traitement backend. Le bundle n’établit pas leur rétention en production.

### T5. Accusé de réception, rendu et résultat client sont distincts

**Interprétation statique.** sand.send.ack consigne outcome, durée, fork, nombre de pièces jointes et identifiants. La télémétrie séparée renderTtfr signale le premier chunk affiché; turnClientOutcome inclut erreur, retryable, user_action, settle_basis et reconnected. Ces étapes évitent de confondre envoi accepté, interface mise à jour et résultat de travail. Preuves : [A32](../artifacts/formatted/dist/electron-main/main-app.cjs:107250), [A33](../artifacts/formatted/dist/electron-main/main-app.cjs:122650), [A34](../artifacts/formatted/dist/electron-main/main-app.cjs:122723).

**Limite.** L’issue du tour côté client n’est pas une vérification métier universelle de chaque action de bot.

### T6. Deux sources du transcript sont comparées

**Observation statique.** bJ compare les entrées gateway et serveur avec une représentation JSON aux clés ordonnées. Il mesure matched, mismatched, missing, extra et staleTail; les messages streaming et tool-call pending sont traités comme transitoires. Le call site Et envoie reportTranscriptParity et le main classe les écarts en avertissement. Preuves : [A35](../artifacts/formatted/dist/node-agent-coordinator/main.cjs:43900), [A36](../artifacts/formatted/dist/node-agent-coordinator/main.cjs:46020), [A37](../artifacts/formatted/dist/electron-main/main-app.cjs:99547).

**Limite.** La comparaison porte sur les pages disponibles. Elle ne prouve ni l’intégralité des conversations ni l’égalité du contexte réellement envoyé au modèle.

### T7. Un corps de transcript introuvable produit un signal explicite

**Observation statique.** Le lecteur cherche les bodies omis, relit les lignes et récupère les blobs disponibles. Après la limite de cinq tentatives, il retire le pending body et émet onRowLost avec agentId, generation, seq, bodySource et attempts. Le signal est dédupliqué selon updatedSeq puis transféré à reportTranscriptRowLoss. Preuves : [A38](../artifacts/formatted/dist/node-agent-coordinator/main.cjs:35603), [A39](../artifacts/formatted/dist/node-agent-coordinator/main.cjs:35955), [A40](../artifacts/formatted/dist/node-agent-coordinator/main.cjs:45971), [A41](../artifacts/formatted/dist/electron-main/main-app.cjs:99889).

**Limite.** Le signal décrit une perte constatée à la lecture; le client ne démontre pas une récupération automatique du contenu définitivement absent.

### T8. Le diagnostic de mise à jour possède un nettoyage spécifique

**Observation statique.** WPn lit une queue du log ShipIt limitée à 5 000 000 octets, la passe à Gft, puis joint uniquement ShipIt_stderr.sanitized.log au message diagnostic. Gft retire chemins personnels, credentials URL, en-têtes auth, clés privées et familles de tokens; il conserve le hash SHA-256 de la queue brute, sa taille et le nombre de remplacements. Preuves : [A42](../artifacts/formatted/dist/electron-main/main-app.cjs:123340), [A43](../artifacts/formatted/dist/electron-main/main-app.cjs:123414), [A44](../artifacts/formatted/dist/electron-main/main-app.cjs:125350).

**Limite.** Ce nettoyage concerne le diagnostic ShipIt retracé. Il n’est pas une preuve de nettoyage de tous les logs, des réponses outils ou des contextes modèle.

## Scénarios concrets

Si un contrôleur Mac demande deux fois le même requestID alors que son bail est actif, la closure de démarrage réutilise sa session. Une demande concurrente différente prend une autre branche. Si le titulaire disparaît, le callback vérifie l’identité de session avant de révoquer, ce qui évite de révoquer aveuglément une session remplacée.

Si un utilisateur cherche un message absent après reconnexion, une trace d’envoi ne suffit pas. Il faut relier clientNonce, résultat ack, affichage, génération du transcript et éventuel transcriptTailRowLost. Les plafonds et l’échantillonnage expliquent pourquoi une étape peut manquer même sans preuve d’échec de l’action.

## Vérification et inconnues

| Sujet | Couvert dans l’artefact | Encore absent |
|---|---|---|
| Ressources partagées | Bail helper, session, activation, décès du titulaire, nettoyage et tunnel egress | Arbitrage cloud, browser et fichiers entre bots |
| Traçabilité | Nonce/traceparent, étapes, pertes, comparaison de pages et log de mise à jour nettoyé | Décision modèle, contexte exact et rétention serveur |

Les corps natifs et leurs commandes sont conservés dans [native-lease-disassembly.json](../evidence/native-lease-disassembly.json). Le ledger [goal-resources-traceability.json](../evidence/goal-resources-traceability.json) relie chaque conclusion à un extrait hashé. Les scripts [inspect-native-lease.py](../tools/inspect-native-lease.py) et [build-goal-resource-evidence.py](../tools/build-goal-resource-evidence.py) permettent de reproduire ces preuves.

La vérification vérifie les empreintes et citations de cette investigation. Elle n’est pas un test du service Grok en production.
