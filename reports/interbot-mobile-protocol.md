# Messages entre bots et connexion iPhone↔Mac

Les échanges prévus entre bots sont des messages texte adressés et des tours de groupe corrélés. Le client sait les afficher et les relier. Pour iPhone↔Mac, les preuves côté Mac montrent surtout un compte, une liste d’appareils et des données servies par le backend. La partie native iOS n’est pas livrée dans la cible macOS sauvegardée.

Cible : Grok Bot 0.70.2 sauvegardé. L’utilisateur a précisé iPhone. Aucun accès à son téléphone, compte ou secret et aucune exécution de la cible n’ont été effectués. Les contrats ont été projetés depuis l’inventaire antérieur, dont la récupération évaluait un module protobuf contrôlé; cette passe ne le réévalue pas. Les littéraux sont recorroborés dans les copies LF et les originaux restent hashés.

Le parent REA portable a été réimporté sans nouvelle analyse globale. La trace ciblée [ev_c3c38ce2965b11faa9c65a2f220209937ce842f10376661ad1fc07d32c8650cd](../evidence/interbot-rea-trace.json) n’a pas de match sur son graphe partiel; elle n’est pas une preuve d’absence. Les observations ci-dessous suivent donc les artefacts et leurs consommateurs.

## Comment les bots se parlent

### W1. Un message direct est une enveloppe adressée

**Contrat compilé.** Le request SendGrokBotAgentMessage contient from_agent_id, to_agent_id, message_id, text, sent_at_ms et reply_to_id optionnel. Il déclare un appel Unary de GrokBotService. Ce request ne porte ni historique complet, ni fichier joint, ni session_id, ni prompt système. Sources : [A1](../artifacts/formatted/dist/electron-main/proto.cjs:56648), [A2](../artifacts/formatted/dist/electron-main/proto.cjs:64761).

**Limite.** Les champs absents de cette enveloppe peuvent être résolus ou enrichis côté serveur. Aucun initiateur client de cet RPC n’est retrouvé par la recherche littérale.

### W2. Livraison et réponse du bot sont deux résultats distincts

**Contrat compilé.** La réponse annonce delivery, identité et nom de la cible, avec workflow_id optionnel. Les issues nommées sont DELIVERED_TEMPORAL, DELIVERED_BOX, DUPLICATE, TARGET_NOT_FOUND, FORBIDDEN, BOX_UNREACHABLE, TEMPORAL_UNAVAILABLE et INVALID_TARGET. Aucun texte de réponse du destinataire n’est dans ce DTO; reply_to_id peut relier un autre message. Sources : [A3](../artifacts/formatted/dist/electron-main/proto.cjs:46221), [A4](../artifacts/formatted/dist/electron-main/proto.cjs:56677).

**Limite.** Le stockage de déduplication, la clé exacte de DUPLICATE, les ACL et les garanties d’ordre ne sont pas récupérés. Une livraison nommée ne prouve pas une tâche accomplie.

### W3. Un tour de groupe transporte un contexte explicite

**Contrat compilé.** Le tour désigne un membre par member_agent_id. Il reçoit room avec nom/description, peers avec ID/nom/description et new_messages. Chaque message décrit HUMAN ou AGENT, speaker_name, is_self, text et éventuellement une citation reply_to. La citation transporte à son tour type/nom du locuteur, is_self et quote. Le membre destinataire et les pairs possèdent des IDs; la projection de chaque locuteur porte surtout nom et rôle. Sources : [A5](../artifacts/formatted/dist/electron-main/proto.cjs:59072), [A6](../artifacts/formatted/dist/electron-main/proto.cjs:59166).

**Limite.** Ce paquet ne révèle pas la mémoire ajoutée ensuite au modèle, ni la validation des noms, ni quels nouveaux messages le serveur sélectionne.

### W4. Le groupe possède trois phases corrélées

**Contrat compilé.** Request demande un tour avec nonce, deadline_ms, is_winding_down, parent_request_id et root_parent_request_id. Dispatch distingue ACCEPTED/DUPLICATE et refus. Cancel vise nonce+membre avec raison et répond delivered. Deliver rend room+nonce+membre, outcome, messages texte, erreur et posts {text,message_json}; intake distingue ACCEPTED, UNKNOWN_NONCE et HOST_UNAVAILABLE. Les outcomes incluent SENT, PASS, SKIPPED, TIMEOUT, CANCELLED et ERROR. Sources : [A7](../artifacts/formatted/dist/electron-main/proto.cjs:46396), [A8](../artifacts/formatted/dist/electron-main/proto.cjs:59166), [A9](../artifacts/formatted/dist/electron-main/proto.cjs:59202), [A10](../artifacts/formatted/dist/electron-main/proto.cjs:59273).

**Limite.** Le protocole prévoit ces phases. Il ne montre pas l’ordonnanceur, l’arbitrage des tours, l’application de la deadline ni la propagation de cancel. Les quatre noms RPC n’ont aucun appel direct littéral dans les 600 JS.

## Comment le Mac et l’iPhone retrouvent les mêmes objets

### I1. Le Mac reconnaît les appareils iOS du même compte

**Observation statique.** Uf construit DashboardService avec accès au token, équipe et machine courante. Zu vérifie que le sujet/email du token reste celui du compte attendu. N5 lit ListSandMachines. NF conserve machineId, label, isCurrent, kind ios/android/desktop, connected et dates; le roster UI se lie à l’identité authentifiée et se réinitialise quand elle change. Sources : [A11](../artifacts/formatted/dist/electron-main/main-app.cjs:48621), [A12](../artifacts/formatted/dist/electron-main/main-app.cjs:48690), [A13](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:54150).

**Limite.** Cela établit une liste de machines côté compte, pas la connexion native de l’iPhone ni une paire physique entre un téléphone et ce Mac.

### I2. Enregistrement de machine et connexion du compte sont séparés

**Observation statique.** RegisterSandMachineRequest accepte label, local_tool_permission, messages_enabled et kind. La réponse prévoit credential, expires_at_ms et credential_held. Le caller Mac B5 fournit son identité machine au client puis envoie label et permission sous contrôle du compte. NF ne recopie pas mobile_local_tools alors que ce tableau est dans SandMachine. Sources : [A14](../artifacts/formatted/dist/electron-main/proto.cjs:36688), [A15](../artifacts/formatted/dist/electron-main/proto.cjs:36753), [A16](../artifacts/formatted/dist/electron-main/main-app.cjs:48639).

**Limite.** Le caller Mac n’implémente pas l’enregistrement iOS. Ni l’obtention native du kind ios, ni le renouvellement de son credential ne sont visibles ici. Les outils mobiles ne peuvent pas être déduits de cette seule projection du roster.

### I3. Les messages partagent des identités de conversation

**Observation statique.** Le request utilisateur transporte agent_id, message_id, session_id, reply_to_id, task_id et machine_id optionnel. source distingue desktop/mobile; send_source distingue CHAT, PUSH, APP_INTENT et ACTION_BUTTON_DICTATION. Le performer z8e convertit ces champs en appel backend et traite ACCEPTED_BOX, ACCEPTED_TEMPORAL ou DUPLICATE comme une acceptation. Sources : [A17](../artifacts/formatted/dist/electron-main/proto.cjs:55724), [A18](../artifacts/formatted/dist/electron-main/main-app.cjs:110278), [A19](../artifacts/formatted/dist/electron-main/main-app.cjs:110633).

**Limite.** Ce chemin actif est dans le Mac. Les variantes mobiles sont supportées par le contrat; le code iOS qui les produit n’a pas été inspecté. machine_id n’est pas une preuve de connexion directe au téléphone.

### I4. La synchronisation Mac lit un flux authentifié du serveur

**Observation statique.** resolveServerTranscriptTail refuse l’accès lorsque le compte n’est pas logged-in puis fournit baseUrl et headers. ND construit le transport depuis cette URL et ajoute les headers actualisés à chaque demande. Son watch appelle WatchGrokBotTranscripts. Le curseur porte agent_id, session_id, generation et after_updated_seq. Sources : [A20](../artifacts/formatted/dist/electron-main/main-app.cjs:107309), [A21](../artifacts/formatted/dist/node-agent-coordinator/main.cjs:35004), [A22](../artifacts/formatted/dist/node-agent-coordinator/main.cjs:35043), [A23](../artifacts/formatted/dist/electron-main/proto.cjs:48147).

**Limite.** Ce flux établit le chemin serveur→Mac. Son usage identique par l’application iPhone n’est pas démontré sans artefact iOS ou capture passive autorisée. Les agents box possèdent aussi leurs chemins gateway.

### I5. La reprise utilise lignes, générations et relecture

**Observation statique.** Le consommateur reçoit rows, cleared, cursorTooOld, agentState et rosterChanged. Il normalise la session, protège les retours contre une génération locale remplacée et réhydrate après cursorTooOld. Les curseurs de reconnexion repartent de streamAckUpdatedSeq, ou de zéro si l’état demande une réhydratation; les bodies en attente peuvent reculer ce curseur. Sources : [A24](../artifacts/formatted/dist/node-agent-coordinator/main.cjs:36336).

**Limite.** Ces mécanismes synchronisent une projection de transcript côté Mac. Ils ne copient pas nécessairement tous les fichiers, réglages locaux, cookies ou états natifs entre appareils.

### I6. La carte de permission mobile reste informative sur le Mac

**Observation statique.** Le loader relie mobile-tool-permission au chunk DhxNwitm. Ce composant affiche titre, texte et une note otherDeviceNote ou otherDeviceGrantedNote selon permission.status. Il ne contient aucun handler local pour accorder cette permission; l’état granted modifie seulement sa note. Sources : [A25](../artifacts/formatted/dist/renderer/assets/index-D6ReTfRE.js:123388), [A26](../artifacts/formatted/dist/renderer/assets/chunk-view-DhxNwitm.js:35).

**Limite.** La permission OS sur iPhone et l’outil qui la demande restent côté application mobile. Une carte reçue sur Mac ne constitue pas une autorisation iOS exécutée.

### I7. Notification, présence et transcript ont des circuits distincts

**Observation statique.** Le protocole de notification porte plateforme/device_id/permission OS et connaît IOS. L’implémentation Hdt du Mac sélectionne uniquement MACOS/WINDOWS/LINUX, déduplique les rapports pendant 24 heures par compte/appareil/empreinte et appelle ReportGrokBotNotificationPermission. Tdt publie séparément l’activité desktop selon connexion, gate, verrouillage, idle, fenêtre, feed et bannières; le serveur retourne une durée de lease. Sources : [A27](../artifacts/formatted/dist/electron-main/proto.cjs:45683), [A28](../artifacts/formatted/dist/electron-main/main-app.cjs:116216), [A29](../artifacts/formatted/dist/electron-main/main-app.cjs:115384), [A30](../artifacts/formatted/dist/electron-main/main-app.cjs:115484), [A31](../artifacts/formatted/dist/electron-main/main-app.cjs:115581).

**Limite.** Aucun motif d’enregistrement APNs spécifique n’est retrouvé par la recherche retenue. La décision serveur de notifier l’iPhone et la priorité entre appareils restent inconnues.

### I8. Le Mac exécute les demandes qui lui sont adressées par le backend

**Contrat compilé.** Le contrat user-computer utilise machine_id+credential pour watch/poll, ack_ids pour les livraisons et frames pour les résultats. Un appel d’outil sur le Mac suit donc un autre circuit que la simple lecture de conversation. Ces contrats sont raccordés au provider local décrit dans le rapport d’exécution précédent. Sources : [A32](../artifacts/formatted/dist/electron-main/proto.cjs:58338), [A33](../artifacts/formatted/dist/electron-main/proto.cjs:58448), [A34](../artifacts/formatted/dist/electron-main/proto.cjs:58519).

**Limite.** Cela ne prouve pas qu’un iPhone donné peut lancer n’importe quelle action sur ce Mac. L’ordonnanceur, les droits et le choix de la machine distante restent serveur.

### I9. Les termes pairing et Bluetooth trouvés ont d’autres consommateurs

**Recherche bornée.** Les recherches de 600 JS donnent zéro appel direct des RPC interbot ciblés, zéro motif d’inscription APNs spécifique et zéro appel mobile bridge selon les motifs retenus. Les hits pairing renvoient surtout à 1Password ou au code de recherche; Bluetooth est une liste de routes audio; qrcode se trouve dans le worker PDF. Aucun de ces hits n’établit un appairage iPhone↔Mac. Sources : [A35](../artifacts/formatted/dist/electron-main/main-app.cjs:40832), [A36](../artifacts/formatted/dist/electron-main/main-app.cjs:117756).

**Limite.** C’est une recherche littérale bornée avec faux positifs examinés. Un appel calculé, une bibliothèque native, un autre artefact mobile ou un service distant peut porter le mécanisme manquant.

## Exemple de dialogue

Exemple inventé à partir des champs récupérés, sans appel réel : le bot Analyste envoie « Vérifie les trois chiffres du rapport » au bot Relecteur, avec un messageId. Relecteur peut envoyer un autre message vers Analyste avec replyToId égal au premier ID. Le retour de livraison du premier message ne contient pas cette réponse du Relecteur. Dans un groupe, le serveur demande séparément un tour au membre choisi et attend un résultat corrélé par nonce.

Les paquets illustratifs et leurs types sont dans [interbot-message-contracts.json](../evidence/interbot-message-contracts.json). Ils ne sont ni une capture réseau ni un moteur serveur reconstruit.

## Lecture pratique iPhone↔Mac

L’hypothèse la mieux soutenue est une continuité par les services du compte : chaque client retrouve agents et sessions côté serveur, puis maintient sa projection locale. Le Mac n’a pas besoin de recevoir directement le texte depuis le téléphone pour afficher une modification serveur. La connexion propre de l’iPhone, son transport de fond et son enregistrement push demandent encore l’artefact iOS ou une observation passive ciblée.

Un ordre donné depuis un appareil peut aussi déclencher un outil sur une autre machine, mais ce serait une chaîne distincte : conversation → décision serveur → demande user-computer adressée → exécuteur Mac → résultat. Les permissions, la disponibilité et le choix du Mac interviennent dans cette chaîne; la synchronisation du texte ne les remplace pas.

## Checklist et preuves manquantes

| Question | Établi | Encore manquant |
|---|---|---|
| Messages directs | DTO, livraison, réponses liées et consommateurs d’affichage | Initiateur backend, ACL, ordre et déduplication |
| Groupes | Membres, contexte de tour, corrélations et résultats | Ordonnanceur et contexte modèle enrichi |
| iPhone↔Mac | Roster du compte, source mobile, sessions et flux Mac | App native iOS, authentification, APNs et éventuel transport supplémentaire |

Les trois sujets sont `partially_answered`. Pour résoudre les frontières restantes, il faut les corps du service distant ou des captures passives de scénarios explicitement délimités; changer de décompilateur ne peut pas recréer des corps serveur absents de l’app.

Voir aussi [consommateurs interbot](interbot-client-paths.md), [contexte restant](server-context-residual.md), [orchestration restante](server-orchestration-residual.md), le [ledger de cette lecture](../evidence/interbot-mobile-protocol.json) et les [recherches sur 600 scripts](../evidence/interbot-mobile-search.json).
