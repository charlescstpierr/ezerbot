# Analyse de Grok Bot et architecture de reconstruction en Rust

L’[audit de l’objectif REA](reports/goal-evidence-audit.md) relie les neuf sujets aux preuves acquises et précise les artefacts encore nécessaires. Il distingue la livraison de l’investigation de la connaissance complète du serveur et de la parité produit. Vérification : `python3 tools/verify-goal-audit.py`.

Le [calcul candidat des horaires](reports/cron-candidate-replay.md) a été exécuté sur 16 cas datés, dont les deux changements d’heure. Le [suivi des runs](reports/routine-run-conversation.md) sépare modifications de routine et exécutions. Les [cartes mémoire](reports/memory-widget-consumer.md) séparent promotion de faits, reçu au bot et état traité. Les garanties serveur restent explicites dans ces rapports. Vérification hors ligne : `python3 tools/verify-memory-routines-followup.py`.

La [reprise Messages .71](reports/messages-worker-recovery.md), le [chemin natif jusqu’à osascript](reports/native-messages-recovery.md) et les [scénarios REA avec fixtures](reports/messages-worker-slice-replay.md) précisent les pertes de résultat après effet. Le cache reste volatil et aucune garantie de déduplication durable n’est établie. Vérification hors ligne : `python3 tools/verify-messages-recovery-followup.py`.

L’utilisateur vise désormais la parité fonctionnelle de son app avec Grok Bot. Le [découpage de livraison](reports/app-parity-delivery.md) part de l’app actuelle et la [proposition du premier lot](docs/superpowers/specs/2026-10-17-multibot-design.md) couvre groupes, mentions et délégation dans le fil. Cette proposition est à relire; groupes, mentions et tours de room ne sont pas encore implémentés. La délégation locale en fils de tâche est livrée séparément ci-dessous.

La [nouvelle analyse des membres](reports/multibot-membership-paths.md), les [tours de room](reports/multibot-turn-paths.md) et le [complément REA](reports/multibot-rea-followup.md) approfondissent le `@` dans une conversation existante et les groupes. Les sources produit actuelles sont [conservées séparément](reports/multibot-public-usage.md). `python3 tools/verify-multibot-investigation.py` vérifie la suite sans exécuter la cible.

Le [contrôle local .71](reports/local-cua-path-0712.md), son [consentement par app](reports/local-cua-consent-0712.md) et sa [façade native MCP](reports/native-0712-mcp.md) sont maintenant reliés. L’original .70 possède aussi une [vérification arm64 ciblée](reports/native-arm64-residual.md). Vérifier cette suite avec `python3 tools/verify-local-control-followup.py`.

La [nouvelle passe de preuves](reports/remaining-evidence-progress.md) ajoute un [démarrage original observé](reports/original-runtime-startup.md), l’[identité et la connexion iPhone](reports/distribution-mobile-discovery.md) et les [injections natives après révocation](reports/native-input-residual.md). Les 45 scripts capturés correspondent au manifeste ; 29 corps natifs supplémentaires sont reliés à leurs octets. Les comportements documentés et les observations runtime restent distincts des implémentations serveur absentes.

La [nouvelle enquête sur les communications](reports/communications-investigation.md) couvre les messages entre bots et la continuité iPhone↔Mac, puis affine les inconnues de contexte et d’orchestration. Quatre rapports, 39 constats, 159 extraits et 22 contrats projetés sont conservés, avec [vue navigable](.lavish/communications-grokbot.html) et [vérification](evidence/communications-verification.json). La partie native iOS n’a pas été récupérée; les contrats mobiles ne sont pas présentés comme son implémentation.

Le [goal des neuf sujets](reports/goal-nine-topics.md) ajoute 68 constats et 255 extraits vérifiables sur contexte, mémoire, effets incertains, boucle outils, permissions/secrets, budgets, ressources, démonstrations et traces. [Vue navigable](.lavish/goal-neuf-sujets.html). Les rapports détaillent [contexte/mémoire/apprentissage](reports/goal-context-memory-learning.md), [exécution/autonomie](reports/goal-execution-autonomy.md), [permissions/secrets](reports/goal-permissions-secrets.md) et [ressources/traçabilité](reports/goal-resources-traceability.md). Le [ledger commun](evidence/goal-nine-topics.json) et le [vérificateur](tools/verify-goal-investigation.py) conservent les frontières client/serveur. Aucun runtime original authentifié n'est revendiqué.

La [suite de la rétro-ingénierie](reports/investigation-continuation.md) couvre désormais les routines, les ordinateurs, la compaction, la délégation, les runtimes, la personnalité des bots et les autres capacités peu détaillées. [Vue navigable](.lavish/enquete-grokbot.html). Les [routines](reports/cron-detail.md), [ordinateurs](reports/computer-detail.md), [runtimes](reports/compaction-runtimes.md), [délégations](reports/other-capabilities-detail.md), [personnalités](reports/bot-personality-detail.md) et [helper natif](reports/native-helper-detail.md) ont leurs rapports et preuves propres. Deux passes supplémentaires suivent [les cycles de délégation](reports/delegation-lifecycle-deep.md) et [l'admission au socket natif](reports/native-peer-deep.md).

Pour reprendre, commencer par [LIRE_EN_PREMIER.md](LIRE_EN_PREMIER.md), [PROCHAINE_SESSION.md](PROCHAINE_SESSION.md) et [MANQUES_COMPLETS.md](MANQUES_COMPLETS.md).

Ce dossier documente le client Grok Bot 0.70.2 installé sur le Mac et définit une architecture Rust indépendante pour reproduire ses familles fonctionnelles. Il contient les preuves, les contrats, les traces de comportement, les limites et les critères de validation. Quinze crates Rust constituent désormais un socle local, avec orchestration durable et un wrapper Mac utilisable. Le wrapper propose un mode simulé sans connexion et un adaptateur ChatGPT, ainsi que les sept capacités locales décrites ci-dessous. Les services cloud et les connecteurs restent à construire.

## Ouvrir Ezerbot, l’application locale

Double-cliquer sur `Ouvrir Ezerbot.command` ou sur `app/build/Ezerbot.app`. L’app propose les thèmes clair et sombre, les conversations, la recherche, la recherche dans le fil (⌘F), la notation locale et exportable des réponses, les favoris, le renommage et la reprise des envois incertains. L’historique et les préférences restent sur le Mac dans `~/Library/Application Support/Grokbot/workspace.sqlite`.

```sh
bash tools/build-app.sh
open app/build/Ezerbot.app
```

Le bot principal s’appelle Ezerbot. Au premier lancement après la mise à jour, seul son ancien nom par défaut « Grokbot » est migré : un nom personnalisé et les autres réglages sont conservés. Les identifiants internes (`grokbot-*`, `local.grokbot.workspace`, bot `grokbot`) et le dossier de données `~/Library/Application Support/Grokbot/` restent inchangés.

Le build requiert macOS 12.3 ou ultérieur, les outils de compilation Apple et Rust. Le bundle embarque le moteur Rust et ses ressources hors ligne. Une fine coque Cocoa héberge l’interface HTML dans WKWebView. Le [nouveau design](reports/mascotte-design.md) met la mascotte animée au centre : clignements, regard, saisie et réactions aux envois. Aucun compte Grok n’est connecté. Dans les paramètres, « Continuer avec ChatGPT » permet de choisir un modèle du catalogue du compte; sans connexion, les réponses restent explicitement simulées. L’adaptateur est testé contre un faux serveur OpenAI, pas avec un vrai compte. [Wrapper local](reports/wrapper-local.md) décrit le fonctionnement et les vérifications.

- [Architecture Rust](reports/architecture-rust.md) décrit les composants, propriétaires, types, flux, reprises et choix de conception.
- [Matrice de parité](reports/parity.md) couvre 45 familles et les scénarios qui devront prouver la future reconstruction.
- [Client Mac](reports/desktop.md) retrace démarrage, IPC, comptes, auth, persistance, fenêtres, notifications et updates.
- [Contrats et conversations](reports/protocol.md) retrace les schémas, envois, transcripts, mémoire, routines, skills, équipes, fichiers et voix.
- [Exécution et ordinateur](reports/execution.md) retrace le coordinateur, daemon, transports, autorisations, VNC, WebAuthn, 1Password et egress.
- [Vue interactive locale](vue/architecture.html) rassemble ces rapports, les diagrammes, une simulation des reprises et des extraits cités.

## Les sept capacités locales

Ces fonctions sont implémentées dans notre reconstruction, pas récupérées du serveur fermé de Grok Bot. Les références REA et les choix locaux sont détaillés dans [wrapper-local.md](reports/wrapper-local.md#les-sept-capacités).

| Capacité | Parcours disponible | Limite importante |
|---|---|---|
| Skills | Bibliothèque par bot, éditeur, versions et import texte/fichiers/dossiers | 250 skills; pas d’apprentissage Teach ni de marketplace |
| Mémoire | Faits privés/équipe, correction, oubli et partage par lots | « Équipe » = tous les bots locaux; oublier ne supprime pas vos messages |
| Runs et contexte | Boucle modèle/outils, reprise durable, arrêt, relance, tâches et compaction | Budgets locaux; tâche terminée seulement par `complete_task` |
| Formulaires, approbations, secrets | Cartes typées, reçu de reprise, coffre séparé; infrastructure d’approbation testée | Aucun outil livré n’exige encore de carte d’approbation; règles Toujours évaluées, réviseur en langage naturel absent |
| Routines | Cron, intervalle, date unique, webhook Bearer publié par Tailscale Funnel ([spec](docs/specs/routines.md#webhook-et-nat)), déclencheur GitHub (pull request, commentaire, issue d’un dépôt, [spec](docs/specs/routines.md#github-courriel-calendrier)) ou groupe de plusieurs déclencheurs, dont le premier qui part lance la routine ([spec](docs/specs/routines.md#groupe)); prompt libre ou skill épinglée; historique | Moteur ouvert requis; proposition du modèle activée après confirmation dans le chat principal, sans carte d’approbation séparée; le connecteur GitHub ne livre pas encore ses événements aux routines |
| Délégation | Enfants parallèles, contexte sélectionné, résultats au parent et arrêt en cascade | 4 actifs, 3 par racine, profondeur 3, file 16; fils de tâche en lecture seule dans l’interface |
| Images | Collage, dépôt/sélecteur, miniature et prédescription avec le brouillon | 20 Mio/image; vision dépendante du modèle; légendes aux tours suivants et à la compaction, pas encore transmises automatiquement aux enfants |

Les paramètres d’un bot donnent accès aux skills et à la mémoire; le panneau Routines se trouve aussi dans les paramètres généraux. Les cartes d’outil, de tâche et de résumé apparaissent dans le fil. Aucun outil shell, accès à l’ordinateur, MCP ou connecteur externe n’est encore fourni. Le routage des approbations vers une conversation de `grokbot`, pour décision par l’utilisateur, est en place et testé; aucun outil livré n’exige encore d’approbation. L’activation d’une routine passe par une confirmation dans le chat principal, pas par une carte. Les limites et suites restantes figurent dans [MANQUES_COMPLETS.md](MANQUES_COMPLETS.md#capacités-locales-livrées-et-suites).

## Essayer et vérifier le socle Rust

```sh
export PATH="$HOME/.cargo/bin:$PATH"
# Dans le dossier principal avec les archives REA (inputs/, artifacts/client/ et artifacts/formatted/) :
python3 tools/verify-rust.py
# Les commandes suivantes fonctionnent aussi dans une copie isolée :
cargo run --locked -p grokbot-try -- demo --db try-data/session.sqlite
cargo run --locked -p grokbot-try -- status --db try-data/session.sqlite
cargo run --locked -p grokbot-try -- flow --db try-data/flow.sqlite
cargo run --locked -p grokbot-try -- status --db try-data/flow.sqlite
cargo build --locked -p grokbot-wrapper
python3 tools/verify-wrapper.py
python3 tools/verify-app.py # contrôle natif séparé, après ouverture de l’app Mac
```

La démo exige un chemin neuf et refuse de remplacer une base existante. `status` relit le compte fictif de la démo. Les bibliothèques ouvrent SQLite avec un `AccountId` explicite. Le toolchain Rust est installé dans `~/.cargo/bin`, même si le shell ne l'inclut pas dans son PATH.

[Consolidation Rust](reports/consolidation-rust.md) décrit les premières corrections. [Orchestration durable](reports/orchestration-rust.md) décrit les commandes avant transport, les snapshots gardés et les essais de processus. La passe dans un clone sans les archives ignorées par Git a obtenu **791 tests passants** (doctests compris), fmt, clippy strict, scénarios SQLite et **60 contrôles HTTP/SQLite passants**, après build `--locked`. `verify-rust.py` doit cependant être lancé dans le **dossier principal contenant les archives REA** : ses contrôles `saved evidence` et `static dossier` ne peuvent pas aboutir dans une copie sans les 1 011 chemins archivés ni l’ASAR. Ce sont des limites d’environnement, pas des échecs du code Rust. Les JSON [Rust](evidence/rust-verification.json) et [HTTP](evidence/wrapper-verification.json) conservés dans le diff livré restent les versions de `main`, pas les rapports de cette passe isolée; ils ne certifient donc pas ces nouveaux comptes. Les preuves statiques de l’original restent distinctes des tests indépendants. Les contrôles natifs de cette livraison restent à faire par le capitaine.

Le dépôt Git versionne les sources, les ressources de l’interface, les scripts, les rapports et les preuves résumées. Les archives locales sont conservées sur disque et exclues de Git, comme les builds et captures de contrôle : `inputs/`, `artifacts/client/`, `artifacts/formatted/`, `artifacts/public-multibot/`, `artifacts/rea-focus/`, `artifacts/rea-original-profile-*/`, `artifacts/releases/`, `.lavish/`, `evidence/rea-bundle.json`, `evidence/multibot-rea-bundle.json`, `evidence/releases/` et `vue/qa/`. Après un clone, construire l’app ne nécessite pas ces archives. Les contrôles REA de `verify-rust.py` doivent être lancés dans le dossier principal qui les conserve; un clone utilisé pour reproduire toute la vérification nécessite l’ensemble de ces archives authentiques, pas seulement l’ASAR.

## Identité et couverture

La cible est `/Applications/Grok Bot.app`, version 0.70.2, distribution macOS x86_64. L'ASAR a l'empreinte SHA-256 `9fe94a66942f8511281f6d7c80a42432d749284c46e1e26f5b6d2f99b87652b0`.

673 fichiers sont extraits. REA a parsé 600 fichiers JavaScript, sans échec de parsing. Son graphe applicatif et sémantique reste partiel. Les lectures ciblées des bundles complètent les flux essentiels; elles ne prouvent pas l'exécution des branches.

L'inventaire des descripteurs comprend 7 services, 484 entrées de méthodes, 429 triplets RPC/types uniques, 2 313 messages et 287 enums. GrokBotService déclare 309 méthodes. L'initialisation du module `proto.cjs` est une évaluation contrôlée de JavaScript en Node VM, distincte de la lecture inerte REA et d'un lancement de l'application. Node VM n'est pas présenté comme une garantie de sandbox.

Six compagnons natifs présentent des contradictions avec les métadonnées ASAR. Leurs signatures Apple sont valides, mais le statut `observed-untrusted` est conservé. Le helper Computer Use est hors ASAR et ne fait pas partie de ces six contradictions.

Le lancement instrumenté Electron historique a échoué. Une observation ultérieure par lancement natif et Chromium CDP fonctionne sur un profil neuf : [preuve de démarrage](evidence/original-runtime-verification.json). Aucun scénario après authentification n'est observé. Les fuses `RunAsNode` et `EnableNodeCliInspectArguments` restent désactivées ; elles n’ont pas été modifiées pour cette observation.

## Preuves réutilisables

Le parent REA est `ev_a8b822a62831e30bd01eb4c4488b36f0bd50cb8ccc41d83e2dcff4e00cf82a4d`. Le [bundle portable](evidence/rea-bundle.json) conserve les records complets. [rea-summary.json](evidence/rea-summary.json) expose les comptes et limites. [manifest.json](artifacts/manifest.json) identifie les fichiers originaux; `artifacts/formatted` contient seulement des copies de lecture.

[protocol-contracts.json](evidence/protocol-contracts.json), [protocol-flows.json](evidence/protocol-flows.json) et [desktop-ledger.json](evidence/desktop-ledger.json) relient les conclusions à leurs noms et positions. [parity.json](evidence/parity.json) est la matrice canonique. Les vérifications se trouvent dans [protocol-verification.json](evidence/protocol-verification.json) et [dossier-verification.json](evidence/dossier-verification.json).

[investigation.json](evidence/investigation.json) clôt les questions avec leurs limites. [review.json](evidence/review.json) conserve la revue indépendante et ses corrections. [browser-verification.json](evidence/browser-verification.json) documente seulement la vérification de la vue interactive, distincte du runtime Grok Bot.

## Reproduire les vérifications

Exécuter depuis ce dossier:

```sh
node tools/protocol-index.cjs
python3 tools/protocol-traces.py
python3 tools/project-evidence.py
python3 tools/verify-dossier.py
python3 tools/build-review.py
```

`inspect-client.mjs` utilise le parseur officiel `@electron/asar` déjà installé avec REA. Ses arguments sont l'ASAR source, le dossier de sortie et le `package.json` qui permet de résoudre ce parseur. Il vérifie aussi que les fichiers déjà extraits correspondent toujours à la source. `format-client.py` utilise Prettier 3.6.2 sur des copies. Les offsets originaux et les lignes formatées ne sont pas interchangeables; ne pas reformater pendant une lecture ou revue.

`read-fuses.py` lit le framework en mode lecture seule. Il ne modifie ni les fuses ni les signatures.

## Limites d'une reproduction exacte

Le backend fermé, les prompts, les modèles, la fleet cloud, certaines garanties de transaction, les internes des helpers natifs et les clients mobiles ne sont pas établis complètement. Les 45 familles figurent dans le plan Rust avec des responsables et des scénarios. Les mécanismes absents devront être conçus et évalués indépendamment.

La matrice mesure la nature des preuves, pas un pourcentage de produit terminé. Le socle Rust est implémenté et vérifié localement. Les contrats des futures intégrations restent une proposition. Aucune CI distante, aucun déploiement ni équivalence complète du produit n'est revendiqué.
