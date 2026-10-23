# Ce qui manque pour la prochaine session

## Capacités locales livrées et suites

Le wrapper a maintenant sept capacités implémentées : skills versionnées/import texte par bot, mémoire privée/équipe avec correction/oubli, moteur de run et contexte/compaction, formulaires/approbations/secrets, routines, délégation et images avec prédescription. [Wrapper local](reports/wrapper-local.md#les-sept-capacités) décrit leurs parcours, défauts et références REA. Cela ne ferme pas les inconnues du **serveur original** décrites plus bas. Les quinze crates, l’adaptateur ChatGPT et les panneaux locaux ne sont ni une fleet cloud ni la parité des 45 familles.

Les valeurs par défaut retenues sont locales : coffre fichier `0600`, mémoire écrite par outils, équipe = tous les bots locaux, délégation 4 actifs/3 par racine/profondeur 3/file 16, un rattrapage de routine dans 60 minutes quand ni l’app ni l’agent ne devaient tourner, budgets de run 12 appels modèle/24 outils/10 minutes, compaction à 70 % de 100 000 tokens estimés. Exception au défaut initial de conception : **une routine proposée par le modèle est activée après confirmation utilisateur dans la conversation principale, sans gate d’approbation séparée**, comme CR19. Le routage des demandes d’approbation vers une conversation de `grokbot`, pour décision par l’utilisateur, est en place et testé. Les outils locaux protégés (`requires_approval: true` dans `crates/wrapper/src/features/local.rs`) passent par une carte d’approbation; les permissions « Toujours autoriser » persistées et le reviewer en langage naturel sont évalués automatiquement.

| Capacité livrée | Suite ou limite encore ouverte |
|---|---|
| Skills | Import nommé par frontmatter pouvant mettre à jour une skill existante : choix local, anciennes versions conservées. Le modèle ne peut proposer une skill que par une carte validée d’un clic humain (`skills/proposals.rs`); démonstration locale (`demonstrations.rs`); pas de marketplace. `GET /api/skills` charge encore tous les corps. |
| Mémoire | Livrés : rappel ciblé (`recall_memory`), apprentissage automatique qui compare les 20 faits proches du message (`MAX_KNOWN`), carte « Partager à l’équipe ? », profil versionné, recherche dans le panneau; voir [docs/memory-scale.md](docs/memory-scale.md). Restent : oublier ne supprime ni les messages utilisateur, ni toutes les copies déléguées/cartes, ni les sauvegardes. Expurgation sous verrou coûteuse; refus 409 si une autre réponse concernée tourne. Tombstones UI limitées à la fenêtre courante. « Équipe » = les bots de l’espace local, pas une équipe multi-comptes. Aucun essai avec un vrai compte ChatGPT. |
| Runs/contexte | Reprise et outils internes durables, pas garantie d’unicité d’effets externes. Fin de tour ≠ fin de tâche; pas de continuation infinie après budget. Évaluer les tâches et les fenêtres réelles avec un vrai fournisseur. |
| Formulaires/approbations/secrets | Reçu de reprise et règles Toujours persistées fonctionnent. Le reviewer en langage naturel est livré (`crates/forms/src/reviewer.rs`, `GET`/`POST /api/approvals/reviewer`, réglages dans `app/ui/panels/forms.js`); son verdict est strict, seule une décision complète et explicite peut autoriser, tout le reste redemande. Manquent les règles administrateur et le niveau de vigilance réglable. Suppression du fil de décision annule les demandes au lieu de les rerouter. Coffre local non chiffré par le trousseau; pas de consommateur de secret. |
| Routines | Un launch agent `local.ezerbot.routines` réveille chaque minute une sonde en lecture seule; le moteur n’est ouvert que pour le travail dû (`routines_background.rs`). Déclencheurs livrés : horaires, webhook, GitHub, courriel, calendrier, groupe composé. L’historique est paginé par curseur. Restent : pause pendant l’absence, fenêtre approximative, Linear/Sentry/PagerDuty, Slack/Teams; reprise manuelle d’affichage parfois nécessaire après coupure; concurrence de la reprise depuis le panneau et retours d’erreur de confirmation/historique à renforcer. |
| Délégation | Contexte choisi et contrôlé, pas snapshot immuable des skills. Les images choisies (`selected_images`) et celles des entrées sélectionnées passent à l’enfant avec leurs légendes et documents. `POST /api/delegation/send` et `/api/delegation/cancel` existent; le fil d’une tâche accepte un envoi et chaque enfant s’annule individuellement. Une tâche terminée refuse un suivi (409). |
| Images et bibliothèque | Staging, légendes et repli vision testés avec faux OpenAI, pas un vrai compte. Certains rejets non reconnus échouent sans repli; une reprise tardive peut retenter une requête sans vision. Pas de purge automatique générale du stockage d’images. Génération d’images avec références, galerie et bibliothèque de documents par bot sont livrées (`images.rs`, `library.rs`); la bibliothèque lit au plus 100 000 caractères et 50 pages par fichier (`crates/attachments/src/documents.rs`) et n’extrait pas par passage ni par page (état de `main` au 23 octobre 2026). |

Le bilan de parité du 23 octobre 2026 (`gb-parite-grokbot/report.md`, hors dépôt, §2 pour la mémoire et §3 pour les lots) est la source des lignes corrigées ci-dessus; la colonne « État Rust » du tableau des 45 familles a été relue contre les fichiers de `crates/wrapper/src/features/`.

Résultats d’une passe antérieure (non rejoués depuis) dans un clone sans les archives ignorées par Git : **791 tests passants**, doctests compris, fmt, clippy strict, scénarios SQLite, build `--locked` et **60 contrôles HTTP/SQLite passants**. **`verify-rust.py` doit être lancé dans le dossier principal contenant les archives REA** : sans les 1 011 chemins archivés ni l’ASAR, `saved evidence` et `static dossier` ne peuvent pas être validés dans cette copie isolée. Cette absence d’archives ne constitue pas un défaut du code Rust, sans valider pour autant le dossier complet. Les liens du rapport local sont vérifiés séparément selon les règles de chemins du contrôle `static dossier`; aucun contrôle n’est contourné. Les JSON [Rust](evidence/rust-verification.json) et [HTTP](evidence/wrapper-verification.json) restent à leur version de `main`, sans rapport d’échec environnemental ajouté au diff livré; ils ne représentent pas les nouveaux comptes de la passe isolée. Les preuves natives antérieures ne certifient pas cette livraison. Les imports de skills et une routine en mode simulé ont été contrôlés dans le navigateur; **contrôle natif dans l’app à faire par le capitaine** (imports fichiers/dossiers, images, cartes, export). Les essais ChatGPT réels, l’accessibilité/IME et l’évaluation métier restent à produire.

## Frontières de l’investigation originale

L’[audit de clôture de l’investigation](reports/goal-evidence-audit.md) distingue les preuves acquises des mécanismes encore inconnus. Il fournit pour chaque sujet l’artefact exact qui permettrait d’aller plus loin. Cette livraison de recherche n’explique pas le moteur fermé et ne termine ni les intégrations ni la parité de notre app; notre moteur local est désormais implémenté séparément.

Les [horaires testés](reports/cron-candidate-replay.md) sont des instants candidats du client. La [jointure d’un run de routine à sa conversation et sa sortie](reports/routine-run-conversation.md) reste non établie. Les [cartes mémoire](reports/memory-widget-consumer.md) distinguent désormais fait traité, fait privé, promotion et reçu ; l’oubli et l’injection au modèle restent inconnus. La [reprise Messages](reports/messages-worker-recovery.md) et son [chemin natif](reports/native-messages-recovery.md) ne livrent pas de garantie durable d’unicité après effet incertain.

Objectif produit ajouté : rendre notre app capable du même travail que Grok Bot. Le [découpage](reports/app-parity-delivery.md) distingue les parcours déjà présents des lots à construire. Le premier lot [groupes/mentions/tours](docs/superpowers/specs/2026-10-17-multibot-design.md) est une proposition à relire. Le wrapper courant conserve un seul `bot_id` par conversation; un adaptateur ChatGPT existe déjà, avec mode simulé sans connexion. Ne pas présenter les nouveaux rapports d’analyse comme du code produit livré.

La [suite multi-bot](reports/multibot-membership-paths.md) établit les opérations client, et les [tours côté hôte](reports/multibot-turn-paths.md) précisent transport, invalidation et interruption. Restent le choix effectif d’un répondant après mention, les ACL serveur, le contexte partagé, l’anti-boucle et les effets d’un retrait sur un tour déjà engagé. La [passe REA](reports/multibot-rea-followup.md) conserve sa couverture sémantique inconnue; son résultat nul de seed n’est pas une absence fonctionnelle. Le design de notre app reste non approuvé après la demande utilisateur de poursuivre avec REA.

La suite native établit la [composition graphique .71](reports/local-cua-path-0712.md), les [grants et leur révocation](reports/local-cua-consent-0712.md), et le [raccord MCP→companion](reports/native-0712-mcp.md). La révocation d’un grant n’est pas reliée à l’arrêt rétroactif des appels admis. Neuf corps [arm64 .70](reports/native-arm64-residual.md) confirment six propriétés ciblées. Restent les effets observés, les courses entre conversations et la couverture native complète.

La [nouvelle recherche de preuves](reports/remaining-evidence-progress.md) réduit plusieurs manques : démarrage réel accessible, app iPhone identifiée, connexion Cursor distincte de la liaison Grok, et callback natif de relâchement reconstruit. Les déclarations publiques sur compaction, mémoire, messages et démonstrations complètent le comportement produit ; elles ne fournissent pas leurs corps serveur. Restent particulièrement la session de test authentifiée, le paquet natif iOS et les implémentations du moteur distant. L’échec de lancement ancien reste historique, sans invalider la nouvelle capture CDP.

La [passe communications/iPhone](reports/communications-investigation.md) ajoute les consommateurs des échanges interbot et les interfaces communes des appareils. Restent à obtenir : initiateur/handler des messages et tours de room, contexte modèle délégué, app native iOS pour login/push/synchronisation de fond, politique effective de notification entre appareils et jobs Teach→skill. Le Mac lit un roster et un transcript authentifiés; cela ne prouve pas un appairage direct avec l’iPhone. Les nouveaux chemins mémoire en lots et import de skills ne ferment pas ces frontières.

La [passe des neuf sujets](reports/goal-nine-topics.md) établit les projections vocales, mutations de mémoire/skills, limites d'outils, décisions de permission, circuits de secrets, contrôle natif de session et traces de transport. Elle précise les manques : contexte texte réel et priorité, correction/oubli durables, unicité des effets après panne, critère métier de fin, reviewer distant et visibilité modèle des secrets, budgets globaux, arbitrage cloud, démonstration→skill/versions et audit causal complet. Huit corps natifs supplémentaires sont désassemblés; cela ne clôt pas les internes de tous les helpers.

La [nouvelle enquête](reports/investigation-continuation.md) précise les manques suivants : compaction effective et prompt de résumé, contexte hérité lors des délégations, ordonnanceur des tours de room, checkpoints des moteurs distants, garanties du scheduler, isolation des écrans et volumes, corps natifs restants et paramètres effectifs des politiques d'accès. Huit fonctions du helper ont désormais été suivies au désassemblage. Les traces nouvelles couvrent aussi banques connectées, avatars IA, workers de code cloud et édition de documents/canvas. La matrice historique de 45 familles reste une base de travail, pas un inventaire exhaustif du serveur.

Le dossier contient une analyse statique du client, une architecture et une reconstruction locale partielle. Il manque encore les observations après connexion à Grok, certains internes, les fournisseurs des futures intégrations et leur validation complète. Un socle local est déjà implémenté; [consolidation-rust.md](reports/consolidation-rust.md) décrit sa portée. La liste couvre le périmètre connu des 45 familles; elle ne prétend pas découvrir toutes les fonctions cachées d’un serveur fermé.

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
| Dépôt, workspace et CI | Workspace de quinze crates, dépôt Git local et vérifications locales; aucune CI distante | Phase 1 de construction |
| UI native | Wrapper Cocoa/WKWebView local construit; Iced candidat pour une UI entièrement Rust; éditeur riche, IME, accessibilité et vidéo non validés | Avant de figer le client |
| PostgreSQL, SQLite et stockage d’objets | Architecture proposée; SQLite locale et migrations du socle créées; PostgreSQL et stockage serveur à construire | Persistance locale/serveur et artefacts |
| API d’identité | Sign in with ChatGPT implémenté/testé avec faux serveur; identité Grok/produit, SSO et coffre OS restent à construire | Login du produit Rust |
| Modèle, transcription et synthèse | Adaptateur Responses et catalogue ChatGPT, mode simulé et budgets locaux implémentés; compte réel non validé, transcription/TTS absents | Runs et voix réels |
| Machines cloud et volumes | Fournisseur, région, images, sauvegardes et coût non choisis | Ordinateur persistant et workers |
| Compte Grok de test | Aucun parcours authentifié observé; accès à revalider avec l’utilisateur | Tests de l’original |
| Connecteurs | Comptes de test, OAuth, apps et endpoints webhook à préparer par service | Validation de chaque connecteur |
| Matériel et permissions Mac | Micro/capture écran/notifications/Messages/WebAuthn à vérifier selon la fonction | Tests OS concernés |
| Cartes et hébergement | Providers et environnements sandbox non définis | Tests des familles dépenses et publication |
| Distribution signée | Identité de signature, notarisation, canal d’update et plateformes à choisir | Livraison d’un produit installable |
| REA runtime | Démarrage natif/CDP réussi, profil neuf, écran de connexion et 45 scripts vérifiés ; ancien lancement instrumenté échoué | Scénarios authentifiés et effets corrélés |
| Fenêtre de présentation | HTML fonctionnel; affichage à l’écran non confirmé par l’utilisateur | Présentation interactive; aucun blocage des vérifications statiques |

Les clés et tokens se configurent dans un coffre ou l’environnement de la future application. Ce dossier conserve les contrats et les besoins d’accès, sans demander de coller des secrets dans les documents. Plusieurs accès ne sont pas nécessaires pour commencer : le domaine, le journal et la réplique peuvent être construits avec des transports et un modèle simulés.

## Implémentation et preuves encore à produire

Le socle `domain`, `wire`, `replica`, `submission-journal`, `store` et `session` est implémenté localement, avec la démo `try`. Le wrapper et les crates `skills`, `memory`, `engine`, `forms`, `scheduler`, `delegation`, `attachments` ajoutent les sept capacités locales. Restent les services cloud `control-plane`/`worker`/`computer`, l’exécution locale externe, les connecteurs, démonstrations, dépenses, hébergement, voix, broker natif, egress et observabilité complète, ainsi qu’un DesktopHost Rust distinct de la coque Cocoa.

Les newtypes, enums d’état, DTO versionnés, migrations SQLite, journal et replays de snapshots et deltas existent. Leur orchestration autour d’un transport simulé existe. Le transport de conversation distant Grok, les migrations PostgreSQL et les garanties serveur restent à construire. Le wrapper Mac ajoute conversations, thèmes, passerelle ChatGPT ou simulée et outils internes; voir [wrapper-local.md](reports/wrapper-local.md). Viennent ensuite l’ordinateur, les intégrations et les validations restantes de [reports/parity.md](reports/parity.md), matrice historique non mise à jour par cette passe.

La compilation, les tests de domaine, certaines pannes et reprises, l’isolation SQLite par compte et les écritures concurrentes de journal et de réplique sont vérifiés localement. Les preuves manquantes comprennent la concurrence de bout en bout, le réseau, l’OS réel, les intégrations sandbox, un client fermé pendant un run cloud, les upgrades et les migrations serveur. La matrice conserve les obligations complètes de chaque famille; les tests du socle n’établissent pas leur parité complète.

Une équivalence avec un système à modèle se mesure par les états, les effets, les résultats des tâches et les reprises. L’égalité de toutes les phrases ne peut pas être déduite des contrats du client.

## Liste des 45 familles

Les familles et obligations du tableau proviennent de `evidence/parity.json`. La colonne « État Rust » distingue maintenant les sous-ensembles locaux livrés, sans réécrire la matrice REA historique; ces sous-ensembles ne couvrent pas les obligations complètes. Chaque famille reste à compléter et à valider en Rust. Les mentions « Socle partiel » renvoient uniquement aux bibliothèques et scénarios locaux. La colonne « Preuve actuelle » précise seulement ce qui est établi sur l’original.

| Famille | Preuve actuelle | État Rust | Travail prévu | Validation à produire |
|---|---|---|---|---|
| Démarrage et processus | Chemin client retracé | Socle partiel | DesktopHost et Coordinator séparés; hello versionné et confiance du caller | Remplacer le renderer sans accepter les réponses d'une ancienne génération |
| Interface, composeur et documents | Chemin client retracé | Socle partiel | Iced, éditeur riche, viewers et adaptateur OS | Mentions, IME, collage, clavier, lecteurs d'écran et grandes conversations |
| Connexion et rotation | Chemin client retracé | Adaptateur ChatGPT local | Auth Grok/produit, SSO et coffre OS | Vrai compte, refresh tardif après logout et retour périmé |
| Multi-compte | Chemin client retracé | Socle partiel | AccountRuntime révocable et caches scoped | Changer de compte pendant send, sync, approval et accès local |
| Deep links | Chemin client retracé | Lien vers un réglage livré (`deep_links.rs`) | Grammaire, canonicalisation, queue et déduplication | Lien avant boot, après reload et vers un bot inaccessible |
| Profils et création de bots | Chemin client retracé | Profils locaux livrés | Étendre vers rôles, ACL et templates distants | Visibilité et isolation multi-utilisateur |
| Sessions et rooms | Contrat partiel | Socle partiel | ConversationId scoped par compte, bot et session; membership | Deux humains avec un bot d'équipe ne partagent pas accidentellement une session privée |
| Bot principal et roster | Contrat partiel | Non implémenté | Préférences de roster et coordination durable | Bot masqué, épinglé, supprimé et bot principal remplacé |
| Journal d'envoi | Chemin client retracé | Socle partiel | Journal durable avec nonce, reçu et attente d'écho | Panne avant POST, après acceptation et avant écho; aucun doublon logique |
| Snapshots et deltas | Chemin client retracé | Socle partiel | Génération serveur distincte de ReplicaEpoch et ReplicaSequence | Trou, duplicate, cursorTooOld, corps manquant et changement de source |
| Annulation et nouvelle conversation | Chemin client retracé | Socle partiel | CancelQueued, cancel RPC, interrupt run et reset conversation distincts | Réponse tardive d'outil après interruption; message accepté conservé |
| Moteur d'agent et prompts | Implémentation inconnue | Moteur local livré | Évaluer contexte, compaction, budgets et reprises avec vrai fournisseur | Résultats métier et coût; pas d’égalité supposée des réponses |
| Mémoire | Contrat partiel | Faits, rappel ciblé et apprentissage automatique locaux livrés | Compléter purge des copies, tombstones UI et essai avec un vrai compte | Oubli vs messages utilisateur, délégation et concurrence à grande échelle |
| Sous-agents et coordination | Contrat partiel | Délégation locale livrée, avec envoi et annulation par tâche | Rooms et échanges interbot généraux | Contexte modifié/révoqué et reprises réelles |
| Ordinateur cloud partagé | Documentation | Non implémenté | ComputerManager par compte et volume persistant | Client fermé, update machine et restauration de snapshot |
| Écrans de bot et reprise humaine | Contrat partiel | Bail de conversation pour le contrôle local du Mac livré (`screen.rs`); pas d’écran cloud | ScreenLease et ownership des entrées | Deux bots en parallèle; contrôle humain; clic avec lease périmée |
| Registre d'outils | Chemin client retracé | Outils internes livrés | Ajouter outils externes avec politique et journal d’effets | Refus explicites, effet incertain après panne et permissions |
| Daemon local et supervision | Chemin client retracé | Non implémenté | Supervisor, générations, outbox et transports bornés | Ancien PID, changement de compte, credential expiré et reprise de résultat |
| Shell et jobs background | Chemin client retracé | Shell local et tâches de fond livrés | ShellService avec politique OS explicite et groupe de processus | Cwd hors scope; kill descendants; sortie volumineuse; budget |
| Fichiers et transferts | Chemin client retracé | Images locales et bibliothèque de documents par bot livrées | Fichiers généraux, root handles, transferts et quotas | Vision réelle, purge, symlinks et transferts interrompus |
| Panneau desktop-mount | Chemin client retracé | Panneau flottant livré (`floating.rs`) | Panneau flottant du bot; pas un montage de dossiers | Ouverture, déplacement, fermeture et scope du composeur |
| Approvals et auto-review | Contrat partiel | Gate, règles et reviewer langage naturel locaux livrés | Règles admin; outils externes à protéger | Décision utilisateur dans le chat principal, digest modifié et révocation |
| Formulaires et secrets | Contrat partiel | Cartes/reçus/coffre local livrés | Consommateurs de secrets, coffre OS et résilience des demandes orphelines | Compte réel, OS et exclusion des valeurs sur nouveaux connecteurs |
| Clé matérielle WebAuthn | Contrat partiel | Non implémenté | Cérémonie bornée et consentement par challenge | Origin/rpId erronés, PIN tardif, abort et device retiré |
| 1Password | Contrat partiel | Non implémenté | Provisioning à commandes fixes et secret sink | Timeout après mint; statut inconnu persisté; scope vault |
| Messages du Mac | Chemin client retracé | Non implémenté | Native broker et registre durable d'effet | Panne après envoi; reprise sans envoi aveugle |
| MCP et connecteurs | Chemin client retracé | Catalogue local de huit services (Gmail, Calendar, Drive, Slack, Notion, GitHub, Linear, Microsoft 365) et MCP HTTPS par URL livrés (`mcp/catalogue.rs`) | Manifeste, registry, OAuth broker et révocation | Connexion expirée, tool inconnu, refus scope et déconnexion |
| Skills et plugins | Chemin client retracé | Bibliothèque/import locaux et propositions de skill avec clic humain livrés | Plugins et publication d’équipe | Imports natifs/dossiers, réimport nommé et contexte hostile |
| Apprentissage par démonstration | Chemin client retracé | Démonstration locale livrée (`demonstrations.rs`); capture native jamais jouée | DemonstrationSession et draft de skill, save/discard explicites | Start doublé, stop save, stop discard, limite durée, changement de compte et erreur de capture |
| Routines, horaires et triggers | Contrat partiel | Horaires, occurrences, launch agent, webhook, GitHub, courriel et calendrier livrés | Pause d’absence, fenêtre approximative, Linear/Sentry/PagerDuty et Slack/Teams | Confirmation dans le chat principal, vrai fournisseur et webhook doublé |
| Templates et marketplace | Contrat partiel | Modèles de bots et export/import par fichier ou code livrés (`templates.rs`); pas de marketplace | Versions de modèle, provenance et import sans sessions privées | Import public/team, update de modèle, scopes et fichiers exclus |
| Voix, dictée et mémos | Chemin client retracé | Dictée et mémos vocaux locaux livrés (`voice.rs`); l’appel vocal (`voice_call.rs`) n’a pas de fournisseur en production | VoiceSession, audio streaming, transcription et TTS | Reconnect, interruption vocale, epoch audio et fin de mémo |
| Meetings et capture | Contrat partiel | Bot Recall invité dans Meet, Zoom ou Teams (`recall.rs`) et capture locale au micro (`meetings.rs`) livrés | Session de capture consentie et artefacts distincts | Arrêt capture, permission refusée, limites durée et rétention |
| PDF, Office, diagrammes et slides | Contrat partiel | Non implémenté | Lecteurs/converters isolés et renderer de document | Corpus de formats, fichiers invalides et quotas; génération serveur à concevoir |
| Bots d'équipe, membres et rôles | Contrat partiel | Non implémenté | Membership, BotAcl, invitations et catalogue scoped | Membre retiré, rôle modifié, session privée et règle verrouillée |
| Abonnements et usage | Contrat partiel | Quota du fournisseur et télémétrie locale livrés (`usage.rs`); pas d’entitlements ni de facturation | EntitlementService, budget et refus explicite | Plan expiré, quota atteint et usage compté sans détour local |
| Dépenses et cartes virtuelles | Chemin client retracé | Non implémenté | SpendingService, challenge, grant scoped, ledger et adapter fournisseur | Attestation périmée, montant modifié, plafond concurrent et charge au résultat incertain |
| Publication et update d'apps hébergées | Chemin client retracé | Non implémenté | HostingProposal, confirmation/dismissal/update et provider isolé | Version inattendue, confirmation doublée, update interrompue et droit de propriétaire retiré |
| Egress via le bureau et proxy | Chemin client retracé | Non implémenté | Lease de tunnel, contrôle DNS/IP et quotas séparés du proxy | Deux bureaux titulaires, DNS privé, policy indisponible et flux saturé |
| Notifications et présence | Chemin client retracé | Boîte d’envoi locale durable livrée (`notifications.rs`) | NotificationEngine et adaptateur OS | Snapshot rejoué, conversation visible, focus perdu et notification cliquée |
| Caches et paramètres | Chemin client retracé | Socle partiel | SQLite scoped, migration de schéma et quotas | Cache incompatible, écriture échouée, purge et flush de fermeture |
| Updates client et ordinateur | Chemin client retracé | Mise à jour locale depuis `main` livrée (`updates.rs`); ni manifeste signé ni notarisation | Manifeste signé, staging, expectedVersion et leases de maintenance | Download interrompu, version inattendue, host occupé et rollback |
| Clients mobiles | Implémentation inconnue | Non implémenté | Réutiliser domain/wire/replica, adapters UI à spécifier | Captures et parité sur vrais artefacts mobiles avant revendication de support |
| Internes des helpers natifs | Implémentation inconnue | Non implémenté | Adapters OS audités et helpers reconstruits avec contrats propres | FIDO, permissions notifications, launcher et Messages sur OS réel |
| Fleet, modèles et stockage privés | Implémentation inconnue | Non implémenté | Control-plane/worker indépendants et garanties documentées | Tests de panne, charge, restauration et évaluations de tâches dans notre système |
