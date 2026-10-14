# Ce qui manque pour la prochaine session

Le dossier contient une analyse statique du client et une architecture de reconstruction. Il manque encore les observations après connexion, certains internes, les choix de fournisseurs, les intégrations Rust et leur validation complète. Un socle local est déjà implémenté; [consolidation-rust.md](reports/consolidation-rust.md) décrit sa portée. La liste couvre le périmètre connu des 45 familles; elle ne prétend pas découvrir toutes les fonctions cachées d’un serveur fermé.

## État acquis

La cible est Grok Bot 0.70.2 pour Mac Intel. Les 673 fichiers extraits sont conservés. REA a parsé 600 fichiers JavaScript sans erreur de parsing; son graphe sémantique et applicatif est partiel. L’inventaire des descripteurs compte 7 services, 484 entrées de méthodes, 429 triplets RPC/types uniques, 2 313 messages et 287 enums. Ces nombres ne comptent pas uniquement des endpoints réellement utilisés.

L’architecture définit les composants, les identités, les états, les reprises, la persistance et les approbations. La matrice contient 45 familles : 25 avec un chemin client retracé, 15 avec un contrat partiel, une avec un comportement documenté et quatre avec une implémentation inconnue. Ces statuts qualifient les preuves; aucun ne signifie qu’une fonction Rust est terminée.

## Observations et connaissances manquantes

| Manque | Travail à faire | Résultat attendu |
|---|---|---|
| Parcours après authentification | Observer création de bot, envoi, reconnect, changement de compte, outil, approbation, ordinateur, routine et voix | Captures et événements datés, avec état initial, action et état final |
| Activation des fonctions | Vérifier les plans, gates, rôles et capacités réellement activées | Matrice fonction/compte/permission et refus observés |
| Réponses réseau réelles | Observer les corps JSON, événements, erreurs, curseurs et limites dans des parcours de test | Corpus anonymisé et relié aux contrats récupérés |
| Exhaustivité sémantique REA | Réduire les cibles et tracer les chemins encore décisifs | Preuve ciblée pour chaque conclusion encore partielle |
| Moteur d’agent serveur | Concevoir contexte, boucle de modèle, outils, checkpoints, budgets et interruption | Moteur indépendant et corpus d’évaluation |
| Prompts et modèles privés | Choisir un fournisseur et écrire nos instructions; les prompts et poids privés ne sont pas livrés par le client | Configuration versionnée et résultats évalués |
| Mémoire serveur | Établir sélection, remplacement, scopes, provenance, budget et oubli | Contrats, stockage et tests de mémoire |
| Coordination entre bots | Établir corrélation, attentes, budget, cycles et propriété des runs | Scénarios à plusieurs bots et reprises après panne |
| Garanties serveur | Établir transaction d’acceptation, ordre d’événements, outbox, rétention et déduplication | Invariants et essais de panne; aucune promesse arbitraire d’exécution exactement une fois |
| Ordinateur cloud | Choisir provisioning, isolation par compte, volumes, snapshots, migration, leases et reprise humaine | Infrastructure reproductible et tests de concurrence |
| Helpers natifs | Analyser ou reconstruire les internes du signer WebAuthn, launcher, notification settings, proclist, bindings tree-sitter et Computer Use | Contrats OS et validation sur machine réelle |
| Six contradictions d’intégrité ASAR | Expliquer leur origine ou les conserver comme limites | Provenance vérifiée; statut `observed-untrusted` tant que la contradiction reste |
| 1Password et secrets | Établir la livraison des credentials, les vault scopes, les statuts ambigus et le chemin d’autofill | Broker Rust et réconciliation; les wrappers ne prouvent pas tous les internes |
| Intégration Messages | Établir permission OS, lecture/écriture et réconciliation après un envoi au résultat incertain | Tests dédiés avec environnement de test |
| Démonstrations | Établir capture, événements, génération de draft de skill et décision save/discard | Session de démonstration et validation de ses artefacts |
| Routines et listeners | Établir livraison fournisseur, filtres, regroupements, fuseaux, rattrapage et doublons | Occurrences durables et tests de triggers |
| Voix et meetings | Établir prompts serveur, endpoints, reprise, capture, interruption et rétention | Sessions audio et corpus de scénarios |
| Documents | Établir lecture, conversion, génération, rendu et limites de PDF/Office/slides/diagrammes | Corpus de formats et erreurs contrôlées |
| Dépenses et cartes virtuelles | Choisir fournisseur, émission, règlement, challenges et réconciliation | Ledger et tests sandbox; aucune transaction réelle pour la simple analyse |
| Publication d’apps | Choisir hébergement, versionnement, DNS, secrets, coûts, droits et garanties d’update | Contrat de publication et scénarios de panne |
| Équipes et administration | Établir ACL serveur, SSO, membres, invitations, règles verrouillées et audit | Tests d’isolation et de révocation |
| Facturation et quotas | Choisir entitlements, mesure d’usage, plafonds et refus | Comptabilisation reproductible et refus explicites |
| Mises à jour | Établir distribution, manifests signés, staging, rollback et migration client/daemon/machine | Parcours d’update testés |
| Autres plateformes et mobiles | Obtenir leurs artefacts et observer leurs parcours si la parité les inclut | Spécifications propres à chaque plateforme |

## Choix et accès à préparer selon la phase

| Élément | État actuel | Quand il devient nécessaire |
|---|---|---|
| Rust, Cargo et cible de compilation | Toolchain installé dans `~/.cargo/bin`; absent du PATH de certains shells | Dès le début du code Rust |
| Dépôt, workspace et CI | Workspace de huit crates, dépôt Git local propre au projet et tests locaux; aucune CI distante | Phase 1 de construction |
| UI native | Wrapper Cocoa/WKWebView local construit; Iced candidat pour une UI entièrement Rust; éditeur riche, IME, accessibilité et vidéo non validés | Avant de figer le client |
| PostgreSQL, SQLite et stockage d’objets | Architecture proposée; SQLite locale et migrations du socle créées; PostgreSQL et stockage serveur à construire | Persistance locale/serveur et artefacts |
| API d’identité | Fournisseur, OAuth, SSO et coffre OS à choisir | Login du produit Rust |
| Modèle, transcription et synthèse | Fournisseurs, modèles, prompts, budgets et clés non configurés | Runs et voix réels |
| Machines cloud et volumes | Fournisseur, région, images, sauvegardes et coût non choisis | Ordinateur persistant et workers |
| Compte Grok de test | Aucun parcours authentifié observé; accès à revalider avec l’utilisateur | Tests de l’original |
| Connecteurs | Comptes de test, OAuth, apps et endpoints webhook à préparer par service | Validation de chaque connecteur |
| Matériel et permissions Mac | Micro/capture écran/notifications/Messages/WebAuthn à vérifier selon la fonction | Tests OS concernés |
| Cartes et hébergement | Providers et environnements sandbox non définis | Tests des familles dépenses et publication |
| Distribution signée | Identité de signature, notarisation, canal d’update et plateformes à choisir | Livraison d’un produit installable |
| REA runtime | Connexion MCP échouée; lancement CLI instrumenté échoué | Nouvelles observations runtime |
| Fenêtre de présentation | HTML fonctionnel; affichage à l’écran non confirmé par l’utilisateur | Présentation interactive; aucun blocage des vérifications statiques |

Les clés et tokens se configurent dans un coffre ou l’environnement de la future application. Ce dossier conserve les contrats et les besoins d’accès, sans demander de coller des secrets dans les documents. Plusieurs accès ne sont pas nécessaires pour commencer : le domaine, le journal et la réplique peuvent être construits avec des transports et un modèle simulés.

## Implémentation et preuves encore à produire

Le socle `domain`, `wire`, `replica`, `submission-journal`, `store` et `session` est implémenté localement, avec la démo `try`. Les modules suivants restent à écrire : `desktop`, `desktop-host`, `control-plane`, `run-engine`, `context`, `policy`, `tools`, `computer`, `worker`, `local-exec`, `connectors`, `scheduler`, `demonstrations`, `spending`, `hosting`, `voice`, `native-broker`, `egress` et `observability`.

Les newtypes, enums d’état, DTO versionnés, migrations SQLite, journal et replays de snapshots et deltas existent. Leur orchestration autour d’un transport simulé existe. Le transport réseau réel, les migrations PostgreSQL et les garanties serveur restent à construire. Un wrapper Mac local du socle existe, avec conversations, thèmes et transport simulé; voir [wrapper-local.md](reports/wrapper-local.md). Viennent ensuite le moteur, l’ordinateur et les intégrations selon les phases de [reports/parity.md](reports/parity.md).

La compilation, les tests de domaine, certaines pannes et reprises, l’isolation SQLite par compte et les écritures concurrentes de journal et de réplique sont vérifiés localement. Les preuves manquantes comprennent la concurrence de bout en bout, le réseau, l’OS réel, les intégrations sandbox, un client fermé pendant un run cloud, les upgrades et les migrations serveur. La matrice conserve les obligations complètes de chaque famille; les tests du socle n’établissent pas leur parité complète.

Une équivalence avec un système à modèle se mesure par les états, les effets, les résultats des tâches et les reprises. L’égalité de toutes les phrases ne peut pas être déduite des contrats du client.

## Liste des 45 familles

Les familles et obligations du tableau proviennent de `evidence/parity.json`. La colonne « État Rust » reflète la consolidation et le wrapper local; leurs sous-ensembles ne couvrent pas les obligations complètes. Chaque famille reste à compléter et à valider en Rust. Les mentions « Socle partiel » renvoient uniquement aux bibliothèques et scénarios locaux. La colonne « Preuve actuelle » précise seulement ce qui est établi sur l’original.

| Famille | Preuve actuelle | État Rust | Travail prévu | Validation à produire |
|---|---|---|---|---|
| Démarrage et processus | Chemin client retracé | Socle partiel | DesktopHost et Coordinator séparés; hello versionné et confiance du caller | Remplacer le renderer sans accepter les réponses d'une ancienne génération |
| Interface, composeur et documents | Chemin client retracé | Socle partiel | Iced, éditeur riche, viewers et adaptateur OS | Mentions, IME, collage, clavier, lecteurs d'écran et grandes conversations |
| Connexion et rotation | Chemin client retracé | Non implémenté | AuthService avec epoch, single-flight et coffre OS | Refresh tardif après logout; retour navigateur périmé |
| Multi-compte | Chemin client retracé | Socle partiel | AccountRuntime révocable et caches scoped | Changer de compte pendant send, sync, approval et accès local |
| Deep links | Chemin client retracé | Non implémenté | Grammaire, canonicalisation, queue et déduplication | Lien avant boot, après reload et vers un bot inaccessible |
| Profils et création de bots | Chemin client retracé | Non implémenté | BotRegistry, versions et route de run indépendante | Création répétée avec même nonce; changement de rôle et visibilité |
| Sessions et rooms | Contrat partiel | Socle partiel | ConversationId scoped par compte, bot et session; membership | Deux humains avec un bot d'équipe ne partagent pas accidentellement une session privée |
| Bot principal et roster | Contrat partiel | Non implémenté | Préférences de roster et coordination durable | Bot masqué, épinglé, supprimé et bot principal remplacé |
| Journal d'envoi | Chemin client retracé | Socle partiel | Journal durable avec nonce, reçu et attente d'écho | Panne avant POST, après acceptation et avant écho; aucun doublon logique |
| Snapshots et deltas | Chemin client retracé | Socle partiel | Génération serveur distincte de ReplicaEpoch et ReplicaSequence | Trou, duplicate, cursorTooOld, corps manquant et changement de source |
| Annulation et nouvelle conversation | Chemin client retracé | Socle partiel | CancelQueued, cancel RPC, interrupt run et reset conversation distincts | Réponse tardive d'outil après interruption; message accepté conservé |
| Moteur d'agent et prompts | Implémentation inconnue | Non implémenté | RunEngine, ModelGateway, checkpoints, budgets et outils | Évaluations de tâches et résultats; pas d'égalité supposée des réponses du modèle |
| Mémoire | Contrat partiel | Non implémenté | Mémoire avec portée, version, source et budget de contexte | Fact obsolète remplacé; duplication de bot sans mémoire privée copiée |
| Sous-agents et coordination | Contrat partiel | Non implémenté | Messages durables avec corrélation, budgets et prévention des cycles | Deux bots actifs, échec du receveur et cycle de délégation |
| Ordinateur cloud partagé | Documentation | Non implémenté | ComputerManager par compte et volume persistant | Client fermé, update machine et restauration de snapshot |
| Écrans de bot et reprise humaine | Contrat partiel | Non implémenté | ScreenLease et ownership des entrées | Deux bots en parallèle; contrôle humain; clic avec lease périmée |
| Registre d'outils | Chemin client retracé | Non implémenté | Capabilities annoncées seulement pour handlers actifs | Descripteur présent mais handler absent produit un refus clair |
| Daemon local et supervision | Chemin client retracé | Non implémenté | Supervisor, générations, outbox et transports bornés | Ancien PID, changement de compte, credential expiré et reprise de résultat |
| Shell et jobs background | Chemin client retracé | Non implémenté | ShellService avec politique OS explicite et groupe de processus | Cwd hors scope; kill descendants; sortie volumineuse; budget |
| Fichiers et transferts | Chemin client retracé | Non implémenté | Root handles, uploads staged/committed, hashes et quotas | Symlink remplacé, chunk rejoué, fichier grossissant et abandon d'upload |
| Panneau desktop-mount | Chemin client retracé | Non implémenté | Panneau flottant du bot; pas un montage de dossiers | Ouverture, déplacement, fermeture et scope du composeur |
| Approvals et auto-review | Contrat partiel | Non implémenté | PolicyService, grant lié au digest et règles admin prioritaires | Commande modifiée, refus, AlwaysAllow, expiration et réponse tardive |
| Formulaires et secrets | Contrat partiel | Non implémenté | Secret broker et canal hors transcript | Secret absent des logs, mémoire, modèles et export de conversation |
| Clé matérielle WebAuthn | Contrat partiel | Non implémenté | Cérémonie bornée et consentement par challenge | Origin/rpId erronés, PIN tardif, abort et device retiré |
| 1Password | Contrat partiel | Non implémenté | Provisioning à commandes fixes et secret sink | Timeout après mint; statut inconnu persisté; scope vault |
| Messages du Mac | Chemin client retracé | Non implémenté | Native broker et registre durable d'effet | Panne après envoi; reprise sans envoi aveugle |
| MCP et connecteurs | Chemin client retracé | Non implémenté | Manifeste, registry, OAuth broker et révocation | Connexion expirée, tool inconnu, refus scope et déconnexion |
| Skills et plugins | Chemin client retracé | Non implémenté | Bibliothèque versionnée et permissions explicites | Mise à jour de skill, private/team scope et instructions externes |
| Apprentissage par démonstration | Chemin client retracé | Non implémenté | DemonstrationSession et draft de skill, save/discard explicites | Start doublé, stop save, stop discard, limite durée, changement de compte et erreur de capture |
| Routines, horaires et triggers | Contrat partiel | Non implémenté | Prompt autonome ou skill optionnelle; triggers provider et groupes; occurrences durables | Routine sans skill, groupe de listeners, filtres provider, DST, retry après crash et webhook doublé |
| Templates et marketplace | Contrat partiel | Non implémenté | Versions de modèle, provenance et import sans sessions privées | Import public/team, update de modèle, scopes et fichiers exclus |
| Voix, dictée et mémos | Chemin client retracé | Non implémenté | VoiceSession, audio streaming, transcription et TTS | Reconnect, interruption vocale, epoch audio et fin de mémo |
| Meetings et capture | Contrat partiel | Non implémenté | Session de capture consentie et artefacts distincts | Arrêt capture, permission refusée, limites durée et rétention |
| PDF, Office, diagrammes et slides | Contrat partiel | Non implémenté | Lecteurs/converters isolés et renderer de document | Corpus de formats, fichiers invalides et quotas; génération serveur à concevoir |
| Bots d'équipe, membres et rôles | Contrat partiel | Non implémenté | Membership, BotAcl, invitations et catalogue scoped | Membre retiré, rôle modifié, session privée et règle verrouillée |
| Abonnements et usage | Contrat partiel | Non implémenté | EntitlementService, budget et refus explicite | Plan expiré, quota atteint et usage compté sans détour local |
| Dépenses et cartes virtuelles | Chemin client retracé | Non implémenté | SpendingService, challenge, grant scoped, ledger et adapter fournisseur | Attestation périmée, montant modifié, plafond concurrent et charge au résultat incertain |
| Publication et update d'apps hébergées | Chemin client retracé | Non implémenté | HostingProposal, confirmation/dismissal/update et provider isolé | Version inattendue, confirmation doublée, update interrompue et droit de propriétaire retiré |
| Egress via le bureau et proxy | Chemin client retracé | Non implémenté | Lease de tunnel, contrôle DNS/IP et quotas séparés du proxy | Deux bureaux titulaires, DNS privé, policy indisponible et flux saturé |
| Notifications et présence | Chemin client retracé | Non implémenté | NotificationEngine et adaptateur OS | Snapshot rejoué, conversation visible, focus perdu et notification cliquée |
| Caches et paramètres | Chemin client retracé | Socle partiel | SQLite scoped, migration de schéma et quotas | Cache incompatible, écriture échouée, purge et flush de fermeture |
| Updates client et ordinateur | Chemin client retracé | Non implémenté | Manifeste signé, staging, expectedVersion et leases de maintenance | Download interrompu, version inattendue, host occupé et rollback |
| Clients mobiles | Implémentation inconnue | Non implémenté | Réutiliser domain/wire/replica, adapters UI à spécifier | Captures et parité sur vrais artefacts mobiles avant revendication de support |
| Internes des helpers natifs | Implémentation inconnue | Non implémenté | Adapters OS audités et helpers reconstruits avec contrats propres | FIDO, permissions notifications, launcher et Messages sur OS réel |
| Fleet, modèles et stockage privés | Implémentation inconnue | Non implémenté | Control-plane/worker indépendants et garanties documentées | Tests de panne, charge, restauration et évaluations de tâches dans notre système |
