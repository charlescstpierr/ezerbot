# Parité et vérification de la reconstruction Rust

La matrice couvre 45 familles. 25 ont un chemin client retracé, 15 un contrat partiel, 1 un comportement documenté, et 4 une implémentation inconnue.

Ces statuts qualifient la preuve disponible. Ils ne mesurent pas un pourcentage de produit terminé. Aucun code Rust produit ni scénario authentifié exécuté ne valide la reconstruction à ce stade.

| Fonction | Preuve disponible | Composant Rust proposé | Scénario de validation future |
|---|---|---|---|
| Démarrage et processus | Trace client; desktop.md, sections 1 et 2 | DesktopHost et Coordinator séparés; hello versionné et confiance du caller | Remplacer le renderer sans accepter les réponses d'une ancienne génération |
| Interface, composeur et documents | Trace client; desktop.md, sections 5, 7 et 10; CSS extrait | Iced, éditeur riche, viewers et adaptateur OS | Mentions, IME, collage, clavier, lecteurs d'écran et grandes conversations |
| Connexion et rotation | Trace client; desktop.md, section 3 | AuthService avec epoch, single-flight et coffre OS | Refresh tardif après logout; retour navigateur périmé |
| Multi-compte | Trace client; desktop.md, départ et arrivée d'une identité | AccountRuntime révocable et caches scoped | Changer de compte pendant send, sync, approval et accès local |
| Deep links | Trace client; desktop.md, section 4 | Grammaire, canonicalisation, queue et déduplication | Lien avant boot, après reload et vers un bot inaccessible |
| Profils et création de bots | Trace client; protocol.md, identité; execution.md, routage de création | BotRegistry, versions et route de run indépendante | Création répétée avec même nonce; changement de rôle et visibilité |
| Sessions et rooms | Contrat partiel; protocol.md, GrokBotAgentSession et RPC room | ConversationId scoped par compte, bot et session; membership | Deux humains avec un bot d'équipe ne partagent pas accidentellement une session privée |
| Bot principal et roster | Contrat partiel; protocol.md, GrokBotUserRuntimeSettings | Préférences de roster et coordination durable | Bot masqué, épinglé, supprimé et bot principal remplacé |
| Journal d'envoi | Trace client; protocol.md, fIe et z8e | Journal durable avec nonce, reçu et attente d'écho | Panne avant POST, après acceptation et avant écho; aucun doublon logique |
| Snapshots et deltas | Trace client; protocol.md, ND, lM et $C | Génération serveur distincte de ReplicaEpoch et ReplicaSequence | Trou, duplicate, cursorTooOld, corps manquant et changement de source |
| Annulation et nouvelle conversation | Trace client; protocol.md, trois annulations; execution.md, Lx | CancelQueued, cancel RPC, interrupt run et reset conversation distincts | Réponse tardive d'outil après interruption; message accepté conservé |
| Moteur d'agent et prompts | Inconnu; protocol.md, corps JSON et inconnues serveur | RunEngine, ModelGateway, checkpoints, budgets et outils | Évaluations de tâches et résultats; pas d'égalité supposée des réponses du modèle |
| Mémoire | Contrat partiel; protocol.md, mémoire | Mémoire avec portée, version, source et budget de contexte | Fact obsolète remplacé; duplication de bot sans mémoire privée copiée |
| Sous-agents et coordination | Contrat partiel; protocol.md, tâches/sous-agents et tours de room | Messages durables avec corrélation, budgets et prévention des cycles | Deux bots actifs, échec du receveur et cycle de délégation |
| Ordinateur cloud partagé | Documentation; execution.md, gateway et VNC; documentation computer-and-apps | ComputerManager par compte et volume persistant | Client fermé, update machine et restauration de snapshot |
| Écrans de bot et reprise humaine | Contrat partiel; execution.md, main/fork VNC; documentation | ScreenLease et ownership des entrées | Deux bots en parallèle; contrôle humain; clic avec lease périmée |
| Registre d'outils | Trace client; execution.md, oYe et P4 | Capabilities annoncées seulement pour handlers actifs | Descripteur présent mais handler absent produit un refus clair |
| Daemon local et supervision | Trace client; execution.md, yG, Cwe, JM et mj | Supervisor, générations, outbox et transports bornés | Ancien PID, changement de compte, credential expiré et reprise de résultat |
| Shell et jobs background | Trace client; execution.md, racine fichiers versus XKe/insecure_none | ShellService avec politique OS explicite et groupe de processus | Cwd hors scope; kill descendants; sortie volumineuse; budget |
| Fichiers et transferts | Trace client; execution.md, LE/P4; protocol.md, attachments | Root handles, uploads staged/committed, hashes et quotas | Symlink remplacé, chunk rejoué, fichier grossissant et abandon d'upload |
| Panneau desktop-mount | Trace client; execution.md, wJe | Panneau flottant du bot; pas un montage de dossiers | Ouverture, déplacement, fermeture et scope du composeur |
| Approvals et auto-review | Contrat partiel; execution.md, autorisation/auto-review; documentation sécurité | PolicyService, grant lié au digest et règles admin prioritaires | Commande modifiée, refus, AlwaysAllow, expiration et réponse tardive |
| Formulaires et secrets | Contrat partiel; protocol.md, RespondWidget/SubmitSecret; execution.md | Secret broker et canal hors transcript | Secret absent des logs, mémoire, modèles et export de conversation |
| Clé matérielle WebAuthn | Contrat partiel; execution.md, rO/lO/cO et GKe/QKe | Cérémonie bornée et consentement par challenge | Origin/rpId erronés, PIN tardif, abort et device retiré |
| 1Password | Contrat partiel; execution.md, Re/Je/$e et launcher | Provisioning à commandes fixes et secret sink | Timeout après mint; statut inconnu persisté; scope vault |
| Messages du Mac | Trace client; execution.md, intégration Messages et cache de nonce | Native broker et registre durable d'effet | Panne après envoi; reprise sans envoi aveugle |
| MCP et connecteurs | Trace client; protocol.md, trois couches et chaîne de gestion | Manifeste, registry, OAuth broker et révocation | Connexion expirée, tool inconnu, refus scope et déconnexion |
| Skills et plugins | Trace client; protocol.md, gestion skills/plugins | Bibliothèque versionnée et permissions explicites | Mise à jour de skill, private/team scope et instructions externes |
| Apprentissage par démonstration | Trace client; index.eager-app-BfVhIMYf.js:57502,57552,57588; get/start/stopTeachRecording | DemonstrationSession et draft de skill, save/discard explicites | Start doublé, stop save, stop discard, limite durée, changement de compte et erreur de capture |
| Routines, horaires et triggers | Contrat partiel; protocol.md, routines; coordinator main.cjs:38806–38883 | Prompt autonome ou skill optionnelle; triggers provider et groupes; occurrences durables | Routine sans skill, groupe de listeners, filtres provider, DST, retry après crash et webhook doublé |
| Templates et marketplace | Contrat partiel; protocol.md, templates et publication | Versions de modèle, provenance et import sans sessions privées | Import public/team, update de modèle, scopes et fichiers exclus |
| Voix, dictée et mémos | Trace client; protocol.md, trois voies audio | VoiceSession, audio streaming, transcription et TTS | Reconnect, interruption vocale, epoch audio et fin de mémo |
| Meetings et capture | Contrat partiel; protocol.md, famille de RPC meetings de GrokBotService et capacités média | Session de capture consentie et artefacts distincts | Arrêt capture, permission refusée, limites durée et rétention |
| PDF, Office, diagrammes et slides | Contrat partiel; package.json et protocol.md, fichiers | Lecteurs/converters isolés et renderer de document | Corpus de formats, fichiers invalides et quotas; génération serveur à concevoir |
| Bots d'équipe, membres et rôles | Contrat partiel; protocol.md, administration et ACL | Membership, BotAcl, invitations et catalogue scoped | Membre retiré, rôle modifié, session privée et règle verrouillée |
| Abonnements et usage | Contrat partiel; protocol.md, RuntimeCapabilities et DashboardService | EntitlementService, budget et refus explicite | Plan expiré, quota atteint et usage compté sans détour local |
| Dépenses et cartes virtuelles | Trace client; main-app.cjs:110867–110904; BeginSpendApproval/ResolveVirtualCardApproval | SpendingService, challenge, grant scoped, ledger et adapter fournisseur | Attestation périmée, montant modifié, plafond concurrent et charge au résultat incertain |
| Publication et update d'apps hébergées | Trace client; index.eager-app-BfVhIMYf.js:56305–56311,56433–56443 | HostingProposal, confirmation/dismissal/update et provider isolé | Version inattendue, confirmation doublée, update interrompue et droit de propriétaire retiré |
| Egress via le bureau et proxy | Trace client; execution.md, zG/qG et proxy Node | Lease de tunnel, contrôle DNS/IP et quotas séparés du proxy | Deux bureaux titulaires, DNS privé, policy indisponible et flux saturé |
| Notifications et présence | Trace client; desktop.md, baseline/feed/focus/badge | NotificationEngine et adaptateur OS | Snapshot rejoué, conversation visible, focus perdu et notification cliquée |
| Caches et paramètres | Trace client; desktop.md, Jj et scopes | SQLite scoped, migration de schéma et quotas | Cache incompatible, écriture échouée, purge et flush de fermeture |
| Updates client et ordinateur | Trace client; desktop.md, updater; execution.md, machine distante | Manifeste signé, staging, expectedVersion et leases de maintenance | Download interrompu, version inattendue, host occupé et rollback |
| Clients mobiles | Inconnu; Aucun artefact mobile sélectionné ou analysé | Réutiliser domain/wire/replica, adapters UI à spécifier | Captures et parité sur vrais artefacts mobiles avant revendication de support |
| Internes des helpers natifs | Inconnu; native-boundaries.json; wrappers JS; six contradictions d'intégrité | Adapters OS audités et helpers reconstruits avec contrats propres | FIDO, permissions notifications, launcher et Messages sur OS réel |
| Fleet, modèles et stockage privés | Inconnu; Frontières réseau du client; aucune source serveur | Control-plane/worker indépendants et garanties documentées | Tests de panne, charge, restauration et évaluations de tâches dans notre système |

## Phases de construction et critères de sortie

| Phase | Périmètre | Preuve nécessaire avant la suivante |
|---|---|---|
| 1 | Identités, protocole, account runtime, journal et répliques | Replays et pannes déterministes; aucune contamination de compte; nonce stable |
| 2 | Client natif et services de bots/conversations | Composeur riche, IME, accessibilité, profils, rooms, notifications et reprises |
| 3 | Runs, modèle, ordinateur cloud, outils et approvals | Client fermé, panne worker, résultat externe incertain, leases d’écran et consentement |
| 4 | Mémoire, connecteurs, skills, démonstrations et routines | Routines prompt-only, triggers provider, DST, révocation OAuth, save/discard |
| 5 | Voix, meetings, fichiers, dépenses et hosted apps | Corpus de formats, audio repris, challenge de dépense, publication/version attendue |
| 6 | Équipes, administration, egress, updates et autres clients | ACL serveur, budgets, tunnel à propriétaire unique, migrations et OS réels |

Cette séquence répartit la livraison de toutes les familles. Les délais et coûts demandent des preuves d’implémentation et une capacité d’infrastructure réelle.

## Ce qui reste à observer dans le produit original

Les parcours après connexion et l’activation réelle des feature gates restent à capturer. Les internes du backend, de la fleet, des modèles, des helpers natifs et des clients mobiles restent absents ou non établis. La déduplication du client ne démontre pas une transaction serveur.

## Référence commune de preuve

`ev_a8b822a62831e30bd01eb4c4488b36f0bd50cb8ccc41d83e2dcff4e00cf82a4d`

Le bundle REA portable est `evidence/rea-bundle.json`. La couverture sémantique est partielle; le rapport conserve cette limite. Les tests ci-dessus sont des obligations futures, pas des tests passés.
