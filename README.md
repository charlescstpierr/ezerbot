# Analyse de Grok Bot et architecture de reconstruction en Rust

La [reprise Messages .71](reports/messages-worker-recovery.md), le [chemin natif jusqu’à osascript](reports/native-messages-recovery.md) et les [scénarios REA avec fixtures](reports/messages-worker-slice-replay.md) précisent les pertes de résultat après effet. Le cache reste volatil et aucune garantie de déduplication durable n’est établie. Vérification hors ligne : `python3 tools/verify-messages-recovery-followup.py`.

L’utilisateur vise désormais la parité fonctionnelle de son app avec Grok Bot. Le [découpage de livraison](reports/app-parity-delivery.md) part de l’app actuelle et la [proposition du premier lot](docs/superpowers/specs/2026-10-17-multibot-design.md) couvre groupes, mentions et délégation dans le fil. Cette proposition est à relire; ce lot n’est pas encore implémenté.

La [nouvelle analyse des membres](reports/multibot-membership-paths.md), les [tours de room](reports/multibot-turn-paths.md) et le [complément REA](reports/multibot-rea-followup.md) approfondissent le `@` dans une conversation existante et les groupes. Les sources produit actuelles sont [conservées séparément](reports/multibot-public-usage.md). `python3 tools/verify-multibot-investigation.py` vérifie la suite sans exécuter la cible.

Le [contrôle local .71](reports/local-cua-path-0712.md), son [consentement par app](reports/local-cua-consent-0712.md) et sa [façade native MCP](reports/native-0712-mcp.md) sont maintenant reliés. L’original .70 possède aussi une [vérification arm64 ciblée](reports/native-arm64-residual.md). Vérifier cette suite avec `python3 tools/verify-local-control-followup.py`.

La [nouvelle passe de preuves](reports/remaining-evidence-progress.md) ajoute un [démarrage original observé](reports/original-runtime-startup.md), l’[identité et la connexion iPhone](reports/distribution-mobile-discovery.md) et les [injections natives après révocation](reports/native-input-residual.md). Les 45 scripts capturés correspondent au manifeste ; 29 corps natifs supplémentaires sont reliés à leurs octets. Les comportements documentés et les observations runtime restent distincts des implémentations serveur absentes.

La [nouvelle enquête sur les communications](reports/communications-investigation.md) couvre les messages entre bots et la continuité iPhone↔Mac, puis affine les inconnues de contexte et d’orchestration. Quatre rapports, 39 constats, 159 extraits et 22 contrats projetés sont conservés, avec [vue navigable](.lavish/communications-grokbot.html) et [vérification](evidence/communications-verification.json). La partie native iOS n’a pas été récupérée; les contrats mobiles ne sont pas présentés comme son implémentation.

Le [goal des neuf sujets](reports/goal-nine-topics.md) ajoute 68 constats et 255 extraits vérifiables sur contexte, mémoire, effets incertains, boucle outils, permissions/secrets, budgets, ressources, démonstrations et traces. [Vue navigable](.lavish/goal-neuf-sujets.html). Les rapports détaillent [contexte/mémoire/apprentissage](reports/goal-context-memory-learning.md), [exécution/autonomie](reports/goal-execution-autonomy.md), [permissions/secrets](reports/goal-permissions-secrets.md) et [ressources/traçabilité](reports/goal-resources-traceability.md). Le [ledger commun](evidence/goal-nine-topics.json) et le [vérificateur](tools/verify-goal-investigation.py) conservent les frontières client/serveur. Aucun runtime original authentifié n'est revendiqué.

La [suite de la rétro-ingénierie](reports/investigation-continuation.md) couvre désormais les routines, les ordinateurs, la compaction, la délégation, les runtimes, la personnalité des bots et les autres capacités peu détaillées. [Vue navigable](.lavish/enquete-grokbot.html). Les [routines](reports/cron-detail.md), [ordinateurs](reports/computer-detail.md), [runtimes](reports/compaction-runtimes.md), [délégations](reports/other-capabilities-detail.md), [personnalités](reports/bot-personality-detail.md) et [helper natif](reports/native-helper-detail.md) ont leurs rapports et preuves propres. Deux passes supplémentaires suivent [les cycles de délégation](reports/delegation-lifecycle-deep.md) et [l'admission au socket natif](reports/native-peer-deep.md).

Pour reprendre, commencer par [LIRE_EN_PREMIER.md](LIRE_EN_PREMIER.md), [PROCHAINE_SESSION.md](PROCHAINE_SESSION.md) et [MANQUES_COMPLETS.md](MANQUES_COMPLETS.md).

Ce dossier documente le client Grok Bot 0.70.2 installé sur le Mac et définit une architecture Rust indépendante pour reproduire ses familles fonctionnelles. Il contient les preuves, les contrats, les traces de comportement, les limites et les critères de validation. Huit crates Rust constituent désormais un socle local, avec orchestration durable et un wrapper Mac utilisable. Les réponses sont simulées; les modèles, services cloud et intégrations restent à construire.

## Ouvrir l’application locale

Double-cliquer sur `Ouvrir Grokbot.command` ou sur `app/build/Grokbot.app`. L’app propose les thèmes clair et sombre, les conversations, la recherche, les favoris, le renommage et la reprise des envois incertains. L’historique et les préférences restent sur le Mac dans `~/Library/Application Support/Grokbot/workspace.sqlite`.

```sh
bash tools/build-app.sh
open app/build/Grokbot.app
```

Le build requiert macOS 12.3 ou ultérieur, les outils de compilation Apple et Rust. Le bundle embarque le moteur Rust et ses ressources hors ligne. Une fine coque Cocoa héberge l’interface HTML dans WKWebView. Le [nouveau design](reports/mascotte-design.md) met la mascotte animée au centre : clignements, regard, saisie et réactions aux envois. Aucun modèle ni compte Grok n’est connecté. [Wrapper local](reports/wrapper-local.md) décrit le fonctionnement et les vérifications.

- [Architecture Rust](reports/architecture-rust.md) décrit les composants, propriétaires, types, flux, reprises et choix de conception.
- [Matrice de parité](reports/parity.md) couvre 45 familles et les scénarios qui devront prouver la future reconstruction.
- [Client Mac](reports/desktop.md) retrace démarrage, IPC, comptes, auth, persistance, fenêtres, notifications et updates.
- [Contrats et conversations](reports/protocol.md) retrace les schémas, envois, transcripts, mémoire, routines, skills, équipes, fichiers et voix.
- [Exécution et ordinateur](reports/execution.md) retrace le coordinateur, daemon, transports, autorisations, VNC, WebAuthn, 1Password et egress.
- [Vue interactive locale](vue/architecture.html) rassemble ces rapports, les diagrammes, une simulation des reprises et des extraits cités.

## Essayer et vérifier le socle Rust

```sh
export PATH="$HOME/.cargo/bin:$PATH"
python3 tools/verify-rust.py
cargo run --locked -p grokbot-try -- demo --db try-data/session.sqlite
cargo run --locked -p grokbot-try -- status --db try-data/session.sqlite
cargo run --locked -p grokbot-try -- flow --db try-data/flow.sqlite
cargo run --locked -p grokbot-try -- status --db try-data/flow.sqlite
cargo build --locked -p grokbot-wrapper
python3 tools/verify-wrapper.py
python3 tools/verify-app.py # après ouverture de l’app Mac
```

La démo exige un chemin neuf et refuse de remplacer une base existante. `status` relit le compte fictif de la démo. Les bibliothèques ouvrent SQLite avec un `AccountId` explicite. Le toolchain Rust est installé dans `~/.cargo/bin`, même si le shell ne l'inclut pas dans son PATH.

[Consolidation Rust](reports/consolidation-rust.md) décrit les premières corrections. [Orchestration durable](reports/orchestration-rust.md) décrit les commandes avant transport, les snapshots gardés et les essais de processus. [rust-verification.json](evidence/rust-verification.json) conserve les commandes, résultats et empreintes du code vérifié. Les preuves statiques de l'original restent distinctes de ces tests du système indépendant.

Le dépôt Git versionne les sources, les ressources de l’interface, les scripts, les rapports et les preuves résumées. Les archives locales `inputs/`, `artifacts/client/`, `artifacts/formatted/` et `evidence/rea-bundle.json` sont conservées sur disque et exclues de Git, comme les builds et captures de contrôle. Après un clone, construire l’app ne nécessite pas ces archives; les vérifications de rétro-ingénierie nécessitent de les recopier depuis ce dossier de sauvegarde.

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
