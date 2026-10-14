# Architecture Rust pour un système équivalent à Grok Bot

Le système proposé conserve les comportements essentiels de Grok Bot: des bots persistants, des conversations qui survivent aux coupures, un ordinateur cloud partagé par compte, des outils, des approbations, une mémoire, des routines et un client de bureau. Tous les composants applicatifs que nous écrivons sont en Rust. Chromium, les modèles, les services d'identité, les codecs et la base de données restent des dépendances externes.

Cette architecture est celle d'un produit indépendant. Elle ne prétend pas reconstituer le code des serveurs Cursor ou SpaceXAI, les prompts privés, les poids des modèles, ni garantir une compatibilité avec leurs API privées. Les observations sur le client sont dans [desktop.md](desktop.md), [protocol.md](protocol.md) et [execution.md](execution.md). Les décisions qui suivent sont des choix de conception, pas des faits récupérés dans l'ASAR.

Le socle local implémenté est décrit dans [consolidation-rust.md](consolidation-rust.md). Les signatures ci-dessous restent la conception du produit complet.

## Ce que l'utilisateur doit pouvoir faire

Le parcours principal est court. L'utilisateur crée un bot avec un nom, un rôle et des limites, lui donne une tâche, observe son travail et répond aux demandes d'approbation. Fermer le client n'arrête pas les tâches cloud. Une reconnexion retrouve le même travail et les mêmes fichiers.

Le parcours secondaire permet de transformer une tâche réussie en skill puis en routine, d'ajouter un connecteur, de partager un modèle de bot ou de publier un bot d'équipe. La voix et la reprise manuelle d'un écran portent le même contexte de tâche que le chat.

Un message accepté n'est pas nécessairement exécuté. Un résultat d'outil reçu n'est pas nécessairement affiché. Ces étapes doivent rester visibles et distinctes dans le modèle.

## Décision sur les trois formes possibles

| Forme | Comportement concret | Coût ou limite | Décision |
|---|---|---|---|
| Client natif Rust et services cloud Rust | Les bots continuent sans le Mac; interface et moteur indépendants | Éditeur riche, documents, accessibilité et vidéo exigent des composants natifs dédiés | Proposition retenue |
| Client Rust avec UI WebView | Le noyau est Rust; une interface web rapproche le rendu Electron | Une UI TypeScript ne satisfait pas une lecture stricte de « tout en Rust ». Une UI générée depuis Rust conserve un runtime web | Alternative si la parité visuelle domine |
| Application entièrement locale | Tous les travaux passent par le Mac et ses processus | Le travail s'arrête lorsque le Mac est éteint; pas d'ordinateur cloud indépendant | Ne couvre pas le comportement demandé |

Iced est le candidat pour le client natif. Il fournit un modèle explicite état, messages, vue, tâches et abonnements. L'éditeur avec mentions, l'IME, les lecteurs d'écran, les documents et le viewer distant sont des validations à réaliser avant de figer ce choix. La documentation présente Iced comme expérimental. Le risque doit être traité par une preuve sur ces composants, pas par une promesse de parité automatique. [Documentation Iced](https://docs.rs/iced/latest/iced/).

La WebView reste admissible pour afficher un document HTML non privilégié dans un processus séparé. Elle n'obtient ni le coffre d'identifiants ni un bridge universel vers les outils. Le viewer de bureau distant natif reçoit des pixels et une lease d'entrée. Il ne charge pas une page distante ayant les privilèges du client.

## Déploiement et carte des composants

Trois programmes sont livrés: `desktop`, `control-plane` et `worker`. Un quatrième, `local-exec`, est installé seulement pour les capacités locales autorisées. Le client reste utilisable comme observateur lorsque `local-exec` est absent.

| Module Rust proposé | Responsabilité et état possédé | Dépendances autorisées |
|---|---|---|
| `domain` | Identités, profils, commandes, événements, états et règles pures | Bibliothèque standard et types partagés |
| `wire` | DTO versionnés et schémas de notre protocole | Protobuf ou Serde, sans comportement métier |
| `replica` | Snapshots, deltas, corps manquants, curseurs, projections | `domain`, interfaces de stockage |
| `submission-journal` | Livraison durable d'un message jusqu'à son écho | `domain`, transactions locales, adapter réseau |
| `desktop` | Iced, fenêtres, clavier, composeur, historique, viewer | API du domaine et adaptateur OS |
| `desktop-host` | OAuth, coffre OS, notifications, deep links, mises à jour | Adaptateurs OS et contrats bornés |
| `control-plane` | Identité, bots, conversations, commandes, autorisations et routes | Domaine, DB, objets, transports |
| `run-engine` | Un acteur propriétaire de chaque run; checkpoints et reprise | Store, modèle, policy, outils |
| `context` | Assemblage du contexte et mémoire avec provenance | Transcript, documents, mémoire, tokenizer |
| `policy` | Règles admin/utilisateur, auto-review, consentement humain | Domaine et store d'approbations |
| `tools` | Registre des capacités effectivement installées | Contrats d'exécution et adaptateurs |
| `computer` | Provisioning, ordinateur du compte, snapshots, leases d'écran | Infrastructure de machines et volumes |
| `worker` | Exécution navigateur, fichiers, shell et connecteurs | Outils, machine du compte, budgets |
| `local-exec` | Outils du Mac explicitement autorisés, jobs et outbox | API locale authentifiée et politiques OS |
| `connectors` | OAuth, MCP, webhooks, révocation et catalogue | Secret broker et adaptateurs de service |
| `scheduler` | Calendrier, événements, occurrences, reprises et historique | Store, outbox, API des runs |
| `demonstrations` | Capture consentie, statut recording, sauvegarde ou abandon et draft de skill | ScreenLease, stockage média, API des skills |
| `spending` | Intention de dépense, challenge, approbation, budget et références de moyen de paiement | Policy, ledger, adapter du fournisseur de paiement |
| `hosting` | Proposition de publication d'une app, confirmation, abandon et update | Policy, artefacts, adapter d'hébergement |
| `voice` | Session audio, transcription, synthèse et interruption | Fournisseurs audio/modèle et API de run |
| `native-broker` | Clé matérielle, Messages et provisioning 1Password | Capacités OS distinctes et consentement |
| `egress` | Propriétaire du tunnel, policy DNS/IP, flux et quotas | Machine, transports, configuration réseau |
| `observability` | Traces, métriques, diagnostics et audit sans secrets | Événements de tous les composants |

Ces limites de modules peuvent cohabiter dans les mêmes programmes. Le premier déploiement regroupe les responsabilités du control-plane dans un programme et les adaptateurs d'exécution dans le worker. Les travailleurs et machines restent séparés du serveur API. Une responsabilité ne devient un service distinct que si sa durée de vie, ses privilèges ou sa charge le justifie.

`domain` ne connaît ni HTTP, ni Iced, ni SQLx. Un adaptateur traduit les DTO ou lignes de base vers les types du domaine. Le moteur ne peut pas appeler directement le coffre du bureau ou piloter une fenêtre.

## Interfaces qui cachent la coordination interne

Ces signatures sont des esquisses de contrat. Elles ne sont pas du code compilé.

```rust
async fn submit_message(
	actor: AuthenticatedActor,
	command: SubmitMessage,
) -> Result<DeliveryReceipt, SubmitError>;

async fn subscribe_conversation(
	actor: AuthenticatedActor,
	conversation: ConversationId,
	cursor: ResumeCursor,
) -> Result<ConversationStream, SubscribeError>;

async fn resolve_approval(
	actor: AuthenticatedActor,
	command: ResolveApproval,
) -> Result<ApprovalResolution, ApprovalError>;

async fn interrupt_run(
	actor: AuthenticatedActor,
	run: RunId,
	request: InterruptionRequest,
) -> Result<InterruptionReceipt, RunError>;

async fn execute_tool(
	action: AuthorizedAction,
	budget: ExecutionBudget,
	cancel: CancellationToken,
) -> Result<ToolEventStream, ExecutionError>;
```

L'appelant de `submit_message` ne choisit pas lui-même une machine, un transport interne ou un worker. Le serveur résout la propriété du bot et le scope de conversation. Le reçu donne une acceptation ou un refus, avec une identité permettant la réconciliation.

Un flux arrêté par le client ne vaut pas interruption du run. Les deux opérations ont des identités et des effets séparés, comme dans le client observé.

## Types et états

`AccountId`, `TeamId`, `BotId`, `ConversationId`, `SessionId`, `RunId`, `MessageId`, `ToolCallId`, `ApprovalId`, `ComputerId` et `ScreenLeaseId` sont des newtypes distincts. Notre système peut générer ses identifiants sous forme d'UUID. Un adaptateur de corpus conserve les identifiants opaques de Grok Bot sans supposer qu'ils sont tous des UUID.

Les générations et séquences ne sont pas interchangeables. La cible expose une génération de transcript serveur, des séquences d'entrées et de mises à jour, puis un epoch et un compteur locaux dans le coordinateur. La reconstruction garde cette séparation.

```rust
struct ServerGeneration(u64);
struct EntrySequence(u64);
struct UpdateSequence(u64);
struct ReplicaSequence(u64);

struct ReplicaEpoch {
	boot_id: BootId,
	server_generation: ServerGeneration,
	recovery: u32,
}

enum SubmissionState {
	Prepared,
	Queued,
	Dispatching { attempt: AttemptId },
	AcceptedAwaitingEcho { receipt: DeliveryReceipt },
	Echoed { entry: TranscriptEntryId },
	Refused { reason: Refusal },
	OutcomeUnknown { reconciliation: ReconciliationId },
}

enum RunState {
	Queued,
	Running { checkpoint: CheckpointId },
	AwaitingApproval { approval: ApprovalId },
	AwaitingHumanInput { request: HumanInputId },
	Cancelling { reason: InterruptionReason },
	Succeeded { result: ArtifactSet },
	Failed { error: RunFailure },
	Cancelled { checkpoint: CheckpointId },
}

enum TranscriptBody {
	Inline { content: EntryBody },
	Blob { reference: BlobRef, digest: ContentDigest },
	Pending { expected_digest: ContentDigest },
	Unavailable { reason: BodyUnavailable },
}

enum ApprovalDecision {
	Allow { grant: ActionGrant },
	Ask { request: ApprovalRequest },
	Deny { reason: PolicyReason },
}
```

`AuthorizedAction` ne peut être construit que par le service de policy après validation. Il contient l'action canonique, son digest, le compte, la cible, la version des règles, l'expiration et une identité de grant consommable. Modifier les arguments d'une commande invalide l'autorisation.

Les enums du protocole gardent une variante explicite pour une valeur inconnue. Une action ou une permission inconnue est refusée. Un contenu d'affichage inconnu donne une carte compatible avec son identité et ses pièces jointes; il n'est pas perdu silencieusement.

## Livraison d'un message et reprise

Le journal local attribue un `MessageId` stable avant l'envoi. Il écrit le brouillon complet, ses références de fichiers et son état dans une transaction. Le composeur peut afficher le message en attente dès cette écriture.

Le serveur traite la clé `(account_id, conversation_id, message_id)` comme unique. Dans une même transaction, il enregistre la commande, l'événement d'acceptation et l'outbox qui demandera le run. Une répétition renvoie le reçu existant. La confirmation ne dépend pas de la disponibilité immédiate du worker.

Après une coupure au milieu d'un envoi, le client consulte le statut du même message avant de décider de rejouer. L'état `accepted-awaiting-echo` disparaît seulement lorsque le transcript reflète le message. Une échéance ou un refus reste une donnée visible, pas une disparition de l'UI.

L'outbox du serveur publie les jobs avec leur identité. Le worker utilise une lease et un fencing token avant de commencer. Une ancienne instance ne peut pas enregistrer la fin d'un run repris par une autre.

L'idempotence d'un message n'assure pas celle d'une mutation externe. Si un outil peut avoir envoyé un email avant la panne, il produit `OutcomeUnknown` et consulte son fournisseur ou demande une décision humaine. Le worker ne promet pas « exactement une fois » lorsque le service externe ne peut pas prouver l'effet.

## Identité du bureau et changement de compte

`AccountRuntime` possède un compte, un scope d'équipe validé, une génération révocable, les transports, les subscriptions et les projections de ce scope. Les continuations asynchrones portent cette génération. Une réponse issue d'une identité retirée ne peut pas modifier la nouvelle session.

La bascule suit un contrat de départ puis d'arrivée. Le départ bloque les nouvelles intentions, révoque les continuations, ferme la session média et les canaux, suspend les services locaux concernés, sauvegarde le journal et parque les caches du compte. L'arrivée ouvre le coffre du nouveau compte, restaure ses projections avec leur schéma, réinitialise les notifications, crée un transport et demande une synchronisation. La validation de scope précède les effets locaux et distants.

Une rotation de credentials est sérialisée. Deux requêtes de refresh partagent la même opération active. Une réponse tardive ne peut pas réinstaller un credential révoqué. Lorsque le coffre refuse une suppression à la déconnexion, le bureau marque immédiatement les credentials comme inutilisables dans la session, affiche l'échec et garde une opération de nettoyage à réconcilier.

Les deep links suivent une grammaire autorisée et une canonicalisation. Ils sont mis en attente avant la restauration du compte et la disponibilité de leur destinataire, puis dédupliqués. Un lien de bot, tâche ou réglage ne porte pas un droit d'accès. Il déclenche une résolution authentifiée du scope avant navigation.

Les notifications ont une baseline par feed, une déduplication par entrée et une policy liée aux préférences et au focus. Rejouer un historique après reconnexion ne produit pas une rafale de notifications. Un clic de notification résout l'identité courante avant d'ouvrir sa conversation. Les permissions OS et le badge ne constituent pas des événements de lecture du transcript.

Les limites de cache observées, dont 8 MiB par clé et 256 MiB au total, sont des références à tester. Notre SQLite peut adopter des quotas équivalents avec des politiques explicites de purge et un flush borné. Les projections restaurées ne sont annoncées fiables qu'après validation de version et de scope.

## Réplication et persistance

Les snapshots et deltas transportent une identité de conversation, une génération, une couverture et des mises à jour ordonnées. Les suppressions et modifications suivent la même séquence de mise à jour. Un trou, un epoch étranger ou un corps manquant déclenche une récupération ciblée ou un snapshot complet.

Un corps manquant est une entrée incomplète conservée dans l'état. La projection ne peut pas annoncer une couverture complète simplement parce qu'elle possède l'en-tête. Le curseur de reprise ne franchit pas un élément dont le corps nécessaire reste absent.

Le buffer de réordonnancement est borné. La cible utilise notamment une limite de 512 pour la réplique transcript; c'est une valeur de référence à garder dans les fixtures, pas une performance garantie. Au dépassement, le client resynchronise au lieu d'accumuler indéfiniment.

| Stockage proposé | Données | Règles |
|---|---|---|
| SQLite du bureau | Journal d'envoi, projections, positions UI, références de blobs | Scope explicite de compte; transactions; secrets hors de la DB ordinaire |
| PostgreSQL | Bots, conversations, commandes, runs, événements, jobs, approvals, routines, ACL | Index sur les scopes; unicité de nonces; append conditionnel et outbox transactionnelle |
| Stockage d'objets | Corps volumineux, pièces jointes, captures et artefacts | Digest, taille, type, ACL, version et rétention; URLs d'accès à durée limitée |
| Volume persistant du compte | `/workspace`, profils navigateur et travail partagé | Séparé de l'image système; snapshots et restauration explicitement versionnés |
| Coffre de secrets | Tokens connecteurs, credentials du worker, clés locales | Références opaques; accès par capacité; pas de secret dans les événements |

Les tables minimales sont `accounts`, `memberships`, `bots`, `bot_versions`, `conversations`, `message_submissions`, `transcript_entries`, `entry_updates`, `runs`, `run_events`, `tool_calls`, `approval_requests`, `approval_grants`, `computers`, `screen_leases`, `connector_installations`, `skills`, `skill_versions`, `routines`, `routine_occurrences`, `artifacts`, `memory_items`, `outbox` et `consumer_checkpoints`.

Les invariants critiques sont l'unicité d'un message par scope, d'une occurrence de routine, d'un résultat terminal d'outil, et d'un grant consommé. Une transaction de fin d'outil écrit résultat et événements ensemble. Une suppression conserve le tombstone et la séquence nécessaire à la réplication.

SQLx est un candidat pour les adaptateurs PostgreSQL et SQLite. Le choix ne rend pas automatiquement correctes les transactions, migrations ou règles de propriété. [Documentation SQLx](https://docs.rs/sqlx/latest/sqlx/).

## Moteur d'agent, mémoire et coordination entre bots

Un acteur de run possède son état, son budget et son checkpoint. Sa boucle prépare le contexte, interroge le modèle, valide les tool calls, demande la policy, exécute les actions autorisées et enregistre leurs résultats. Un checkpoint durable existe avant et après une frontière d'effet externe.

Le `ModelGateway` présente un contrat stable pour les deltas de texte, les appels d'outil, les raisons d'arrêt et l'usage. Le fournisseur et le modèle sont une configuration explicite du bot et du run. Les commandes visibles dans l'ASAR ne révèlent pas le prompt système complet ou la logique d'inférence serveur. La sélection des modèles et les prompts devront donc être conçus et évalués dans notre produit.

La mémoire est séparée en profil du bot, instructions durables, faits avec provenance, résumés de conversation, état du projet partagé et résultats de recherche temporaires. Un fait garde sa source, sa portée, sa date et son statut de remplacement. Les résumés ne détruisent pas le transcript d'origine.

`ContextAssembler` applique un budget par catégorie et conserve les sources utiles. Un secret ou une réponse destinée à un formulaire sécurisé ne devient pas une mémoire de conversation. Un contenu de site, document ou plugin demeure une donnée externe; il ne peut pas modifier les permissions de la policy.

Une demande entre bots est un message durable adressé avec un ID de corrélation. Le bot receveur possède son run. Le bot émetteur attend un résultat, une demande humaine ou une erreur. Limites de profondeur, budget commun et graphe des dépendances empêchent les cycles de délégation. Le comportement exact de coordination du serveur original reste inconnu; ces règles rendent notre choix testable.

## Ordinateur cloud et concurrence

L'ordinateur persistant appartient au compte. Les bots partagent les fichiers et certaines sessions navigateur; un écran n'est pas une frontière de confidentialité. Cette relation est documentée par le produit. [Ordinateur et applications](https://docs.x.ai/grok-bot/computer-and-apps).

Notre `ComputerManager` alloue un volume durable et une machine isolée pour ce compte. Le worker tourne dans cette machine avec des capacités limitées. Redémarrer ou mettre à jour l'image conserve le volume selon le contrat. Une restauration peut perdre des écritures postérieures au snapshot, ce qui doit être affiché avant l'opération.

Une lease d'écran identifie le bot, l'écran, le propriétaire actuel des entrées et sa génération. Le navigateur et le bureau de cet écran ne peuvent recevoir des clics provenant d'une lease périmée. La prise en main humaine suspend les actions d'entrée du bot; rendre le contrôle réactive la même tâche après vérification.

Chaque run a un dossier de travail propre. Publier un résultat dans le workspace partagé utilise une opération atomique ou une coordination sur le fichier concerné. Les commandes concurrentes n'écrivent pas arbitrairement le même fichier. Une migration ou un reset de l'ordinateur utilise une lease de maintenance distincte.

Le viewer expose soit le bureau principal, soit l'écran d'un bot, avec les mêmes règles de consentement. Le champ `forkVncBaseUrl` de la cible ne suffit pas à déterminer l'implémentation de ces écrans. Dans le client, `desktop-mount` est un panneau flottant; il ne constitue pas la preuve d'un montage de dossiers.

## Outils et accès locaux

Le registre d'outils annonce seulement les handlers effectivement installés. Chaque handler déclare un schéma d'entrée, une catégorie d'effet, ses besoins en capacités, sa route et son mode d'annulation. Une famille de descripteurs présente dans un SDK n'est pas une capacité disponible.

Le navigateur est piloté par un adaptateur CDP Rust tel que chromiumoxide, avec un handler d'événements et des délais explicites. Une exécution de bureau native utilise un adaptateur distinct pour captures et entrées. Chromium demeure un moteur externe; nous ne réécrivons pas un navigateur. [Documentation chromiumoxide](https://docs.rs/chromiumoxide/latest/chromiumoxide/).

Le service shell possède le groupe de processus, les descendants, les flux stdout et stderr, les fichiers de débordement et le budget. Son annulation n'est pas celle d'un simple appel RPC. Le service fichiers applique des handles de racine, des opérations relatives, des quotas réels et une finalisation atomique. Une vérification de chemin effectuée avant un accès ne remplace pas une résolution résistante aux changements de symlink.

Les différences avec la cible sont délibérées: le daemon observé peut accepter un working directory shell hors de sa racine fichiers et possède des voies `insecure_none`. Notre défaut exige une politique OS explicite. Le serveur produit un diagnostic si elle est indisponible. La compatibilité fonctionnelle se teste en déclarant les capacités voulues; elle ne repose pas sur un fallback silencieux.

Le daemon local traite les demandes signées pour un compte et une machine, avec backlog et outbox bornées. Une annulation a un canal prioritaire. Il conserve les résultats non acquittés après reconnexion; il ne réexécute pas les outils pour reconstruire leur sortie.

## Approbations et secrets

Une demande d'approbation décrit la cible, les arguments finaux, l'effet, la portée et l'expiration. Les règles admin constituent un plafond. Les règles utilisateur peuvent le rendre plus strict. Une règle « demander » l'emporte sur une règle « autoriser ». L'auto-review peut ajouter une raison de suspendre; elle ne peut pas élargir un plafond administratif. Ces comportements sont annoncés par le produit, sans prouver l'algorithme privé du reviewer. [Approbations et confidentialité](https://docs.x.ai/grok-bot/approvals-security-and-privacy).

`AllowOnce` consomme un grant atomiquement. `AlwaysAllow` écrit une règle étroite et versionnée. `Deny`, `Expired` et `Cancelled` sont des fins explicites. Une réponse tardive ou appartenant à un autre compte ne réactive pas l'action. Les approbations issues d'une routine ont une durée de vie définie; la référence publique indique environ dix minutes, à tester lors des captures futures.

La saisie d'un secret passe par un canal séparé du transcript. Le modèle reçoit le statut de la résolution, jamais la valeur. Le stockage et les logs des adaptateurs vérifient cette exclusion. Il s'agit d'une garantie à construire; le wrapper 1Password du client ne démontre pas à lui seul ce que voit chaque composant serveur.

Une cérémonie WebAuthn garde un origin, un rpId, un challenge, une expiration et un consentement. Un PIN est lié à une demande active précise. Les validations cryptographiques du helper original n'ont pas été décompilées. Le broker Rust devra les vérifier avec une bibliothèque et des tests dédiés, sans réutiliser une signature valide comme preuve de ces règles.

## Skills, routines, connecteurs et fichiers

Un skill est un document versionné avec description, déclencheurs, entrées, méthode, limites et validation. Une routine contient son propre prompt et un bot propriétaire. Elle peut aussi appeler une version de skill. La création et l'édition d'une routine contenant seulement un prompt doivent rester possibles. Le produit distingue les skills réutilisables et les routines déclenchées à une date ou par un événement. [Skills et routines](https://docs.x.ai/grok-bot/skills-routines-and-automations).

`RoutineBody` distingue `InlinePrompt` et `SkillInvocation`, cette dernière portant une version et ses entrées. `RoutineTrigger` conserve les variants récupérés: cron exact ou flexible avec fenêtre, Slack, GitHub, origin, Microsoft Teams, Linear, Sentry, PagerDuty, email et webhook. Un group de listeners contient au moins deux triggers. Les filtres de chacun gardent leur schéma et leur scope; la normalisation et livraison des événements côté serveur restent à concevoir. Preuve de contrat: `artifacts/formatted/dist/node-agent-coordinator/main.cjs:38806` à `38883`.

Le scheduler crée une occurrence durable identifiée par `(routine_id, schedule_version, occurrence)`. L'occurrence garde sa date prévue, son fuseau, son contexte et son run. Le changement de fuseau ou d'heure d'été suit une règle explicite: une heure inexistante est déplacée au premier instant valide; une heure répétée ne produit qu'une occurrence. Les webhooks utilisent une identité d'événement fournisseur pour la déduplication. Une panne du scheduler ne doit ni supprimer un run prévu ni le doubler.

Le bot, les skills et les routines ont des versions et des politiques de duplication distinctes. Copier un bot reprend sa configuration et ses règles selon le contrat, sans copier aveuglément mémoire, historique, fichiers ou connexions. Le catalogue distingue modèle de bot, skill et connecteur. Chaque import montre la provenance, la version et les permissions demandées.

Un connecteur possède un manifeste, des outils versionnés, un scope de compte ou d'équipe, une connexion et un état de révocation. Le broker OAuth garde les credentials. Un serveur MCP externe s'exécute dans une frontière de processus et de capacités; il ne reçoit pas toutes les variables du worker. Une réponse de connecteur ne peut pas remplacer les règles de la policy.

Les fichiers sont chargés avec un digest, une taille, une détection de type et un propriétaire. Prévisualisation, conversion et export tournent avec un budget dans des processus séparés. PDF, Word, tableurs, diagrammes et slides sont des capacités distinctes du rendu chat. Les paquets tiers trouvés dans l'ASAR donnent des indices sur leurs lecteurs, pas les algorithmes serveur de génération.

## Démonstrations, dépenses et publication d'apps

Le client contient une autre voie d'apprentissage: `getTeachRecordingStatus`, `startTeachRecording` et `stopTeachRecording` avec une décision `save`. Le domaine garde l'agent, le début, une durée maximale, les opérations en vol et les générations de continuation. Sources: `artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:57502`, `57552` et `57588`. La capture et la génération de la skill serveur ne sont pas établies par ces seuls appels.

`DemonstrationSession` distingue `Idle`, `Recording`, `Stopping`, `DraftReady`, `Saved`, `Discarded` et `Failed`. Une prise en main consentie démarre l'enregistrement de l'écran concerné, avec une limite et un état visible. Arrêter avec save produit un draft de skill à relire; arrêter sans save abandonne la capture selon sa rétention. Une capture meeting ou une dictée ne vaut pas consentement à enregistrer une démonstration. Les champs sensibles et les séquences de connexion exigent une pause ou une exclusion explicite.

Le client expose également `BeginGrokBotSpendApproval`, `ResolveGrokBotVirtualCardApproval` et `ListGrokBotStripeLinkPaymentMethods`. La résolution peut porter un `paymentMethodId` et une attestation avec `challengeId`. Source: `artifacts/formatted/dist/electron-main/main-app.cjs:110867` à `110904`. Cette famille est distincte de la facturation de l'abonnement et de l'approbation d'un shell.

`SpendingService` possède les intentions et leurs décisions. `SpendIntent` porte un compte, un bot, une cible, une devise et un montant en unités mineures, un plafond, une expiration et une identité de requête. Les variantes sont `Proposed`, `ChallengeRequired`, `Approved`, `Denied`, `Expired`, `Executing`, `Settled` et `OutcomeUnknown`. Un challenge est lié à l'intention et à sa version. Le ledger réserve le budget dans une transaction et conserve les références du fournisseur. Un retour ambigu après émission de carte ou charge ne relance pas l'effet sans réconciliation. Le contrôle des moyens de paiement, l'émission et le règlement sont des mécanismes fournisseur inconnus dans le client; notre adapter doit les établir indépendamment.

La publication d'apps a son propre domaine. Le client lit `getCloudAgentPublishProposals`, confirme ou refuse une proposition et appelle `updateCloudAgentPublishApp`. Sources: `artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:56305` et `56433` à `56443`. Elle ne se confond pas avec la publication d'un template de bot dans la marketplace.

`HostingProposal` décrit les artefacts par digest, la version attendue, la visibilité, les permissions, l'app et l'effet proposé. `HostingService` reçoit une confirmation, une dismissal ou une update avec identifiant stable. Son adapter conserve les états `Proposed`, `Deploying`, `Published`, `UpdateProposed`, `Dismissed`, `Failed` et `OutcomeUnknown`. Les fichiers générés sont inspectés et servis dans un scope séparé des credentials du compte. Les coûts, DNS, secrets de déploiement, routing, durabilité et garanties d'update du service original restent inconnus; chaque provider retenu devra avoir son contrat de publication et de reprise.

## Voix, équipes, facturation et clients supplémentaires

La voix possède une `VoiceSessionId` attachée à un bot et un run. Les événements de transcription, audio et interruption portent un epoch. Le texte final rejoint le transcript; les chunks audio temporaires ont une rétention séparée. Une interruption vocale annule la synthèse en cours et transmet une demande typée au moteur. Les confirmations sensibles restent visibles dans le chat ou dans une interaction de consentement explicite.

La voie temps réel observée utilise un WebSocket WSS avec JSON, audio PCM16 à 24 kHz, détection de voix côté serveur et identité de conversation pour reprise. La reconstruction fidèle conserve un adapter de ce transport, avec credential éphémère, expiration et validation de l'endpoint. Le backend de voix fournit les instructions et les outils; leurs prompts restent inconnus. La dictée unary et la synthèse de mémo sont des adapters distincts. Les positions de texte et timestamps définis en unités UTF-16 ne sont pas remplacés par des offsets de bytes Rust.

WebRTC est une alternative de transport si un autre fournisseur l'exige, pas une déduction sur le protocole du client livré. Une bibliothèque Rust WebRTC ne fournit pas le modèle de transcription, de voix ou les prompts de contrôle. [Documentation WebRTC Rust](https://docs.rs/webrtc/latest/webrtc/).

Les équipes ajoutent membership, rôles, bots publiés, catalogue partagé, règles verrouillées, SSO et audit. Toute commande est évaluée avec un acteur authentifié et le scope du bot. Le choix d'un `TeamId` dans une requête ne donne pas accès à l'équipe. La facturation porte des entitlements et des budgets; un refus explicite ne doit pas transformer une tâche en exécution locale gratuite non demandée.

Les clients mobiles réutilisent `domain`, `wire`, `replica` et le journal de soumission. Leur adaptateur UI reste un chantier distinct: l'ASAR Mac n'établit pas les détails des applications mobiles. Ils figurent dans la couverture du produit à construire, sans être présentés comme rétro-ingénierés ici.

## Transport, réseau et mises à jour

Notre contrat canonique est défini dans des schémas Protobuf versionnés, avec un adapter HTTP pour opérations et événements de reprise. Les communications worker utilisent gRPC via tonic; le flux vers le bureau peut utiliser gRPC streaming. Une façade SSE ou WebSocket doit porter les mêmes identités et curseurs, sans devenir une seconde définition métier. Tonic prend en charge les flux unary et streaming, y compris bidirectionnels. Cela ne garantit pas une compatibilité filaire Connect avec la cible. [Documentation tonic](https://docs.rs/tonic/latest/tonic/server/struct.Grpc.html).

Axum est un candidat pour le serveur HTTP, les redirections OAuth, les uploads, les webhooks et la façade d'événements. Il reste dans l'adaptateur réseau. [Documentation Axum](https://docs.rs/axum/latest/axum/).

Le proxy corporate du client et l'egress de l'ordinateur cloud sont distincts. L'egress a une lease de bureau propriétaire, une politique hostname et IP, des vérifications DNS, des quotas et des timeouts. Une évolution restrictive s'applique sans attendre un assouplissement. Une perte de propriétaire coupe ses flux. Les requêtes ne continuent pas silencieusement en connexion directe lorsque la configuration imposée échoue.

Les mises à jour vérifient le manifeste signé, l'architecture, le digest et la version. La mise à jour du client, celle du daemon et celle de l'image de l'ordinateur ont des états séparés. Les migrations de données sont transactionnelles et l'ancienne instance ne peut pas supprimer l'état d'une génération nouvelle.

## Différences assumées et limites d'équivalence

La proposition reprend les contrats observables et crée les mécanismes absents du client. Le moteur de modèle, les prompts, le stockage serveur, la fleet cloud, les garanties transactionnelles originales, les helpers natifs internes et les clients mobiles restent inconnus ou partiellement établis.

Le journal durable d'effets externes, les grants liés au digest d'action, le chiffrement sans fallback en clair et la politique shell explicite sont des garanties de notre architecture. Certains comportements observés dans le client sont plus permissifs ou ne prouvent pas une durabilité après crash. Ces différences sont enregistrées dans la matrice de parité.

La fidélité d'un système à modèle ne se mesure pas à l'égalité de toutes les phrases produites. Elle se mesure aux scénarios, résultats, effets d'outil, reprises, décisions d'approbation et états visibles. Utiliser le même fournisseur ne recrée pas automatiquement un prompt et un moteur privés.

La [matrice de parité](parity.md) décrit les preuves restantes et les scénarios qui permettraient de valider une future implémentation. Aucun programme Rust produit n'est livré dans cette phase d'analyse.
