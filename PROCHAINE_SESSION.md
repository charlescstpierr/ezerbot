# Reprendre le travail

## Objectif produit actuel

L’utilisateur veut pouvoir faire dans son app ce que fait Grok Bot. Il a précisé les échanges entre bots, `@` dans un fil existant et les conversations à plusieurs bots. Lire [app-parity-delivery.md](reports/app-parity-delivery.md) puis la [proposition de premier lot](docs/superpowers/specs/2026-10-17-multibot-design.md). Le code audité a déjà profils et chemin ChatGPT, mais `Conversation` et SQLite rattachent encore le fil à un unique bot. La proposition n’est pas approuvée et aucun code produit de ce lot n’a été écrit pendant cette passe. Préserver les changements concurrents dans app/ui et wrapper.

La nouvelle suite statique [contrôle Mac .71](reports/local-cua-path-0712.md) et les rapports liés se vérifient avec `python3 tools/verify-local-control-followup.py`. Ils établissent la gate, les 14 outils, les queues par conversation, la façade stdio puis le socket companion et la portée du retrait d’app. Un résultat done n’établit pas toujours une action réussie. Le mode natif MCP existait déjà en .70; la composition JavaScript observée est .71.

Dernier steering : « utilise REA », sans approbation du design produit. Lire [multibot-rea-followup.md](reports/multibot-rea-followup.md), [multibot-membership-paths.md](reports/multibot-membership-paths.md) et [multibot-turn-paths.md](reports/multibot-turn-paths.md). Le `@` ne modifie pas les membres dans la chaîne inspectée; Members conserve l’ID de room; nouveau groupe crée une room puis transfère le brouillon. Un nouveau bundle REA de 15 records est conservé. Le focus d’un seul module a une couverture sémantique unknown et un nettoyage producteur initialement incomplet; le worker exact a été arrêté et le répertoire vide retiré. Réutiliser ces preuves au lieu de relancer le même focus. Vérifier avec `python3 tools/verify-multibot-investigation.py`.

## Dernière passe de rétro-ingénierie

Lire [messages-worker-recovery.md](reports/messages-worker-recovery.md), [native-messages-recovery.md](reports/native-messages-recovery.md) et [messages-worker-slice-replay.md](reports/messages-worker-slice-replay.md). La chaîne .71 relie caches B4, worker, enveloppe RPC et osascript. La capture REA `ev_e0e22e161c96d6a1db6661fbe68e99e006c3a35204228b02374fce15a8585934` exécute seulement 16 tranches exactes avec frontières fictives. Ne pas la présenter comme un envoi ou un crash réel. Son stdout PTY est complet ; ses rendus terminal sont partiels et les processus non attribués sont laissés intacts. Vérifier hors ligne avec `python3 tools/verify-messages-recovery-followup.py`, sans relancer le scénario.

Commencer désormais par [remaining-evidence-progress.md](reports/remaining-evidence-progress.md). Un démarrage original signé a été observé avec un profil neuf et le port CDP Chromium, sans Node Inspector ni Playwright launch : 45 sources capturées correspondent exactement au manifeste. Le processus d’observation et la connexion Chrome dédiée ont été arrêtés. Lire [original-runtime-startup.md](reports/original-runtime-startup.md), [distribution-mobile-discovery.md](reports/distribution-mobile-discovery.md) et [native-input-residual.md](reports/native-input-residual.md). Ne pas répéter l’ancien lancement rejeté ni confondre cet écran sans compte avec les scénarios authentifiés. Vérifier avec `tools/verify-original-runtime.py` et `tools/verify-remaining-evidence.py`.

Lire d’abord [communications-investigation.md](reports/communications-investigation.md) et ses quatre rapports. L’utilisateur a demandé les inconnues restantes, les échanges entre bots, puis la connexion à son iPhone. La passe ajoute grouping/panneau interbot, mentions riches, réponses scoped et rails vocaux, contrats directs/rooms, roster iOS et flux transcript Mac, partage de mémoire par lots, réveil de formulaire et import texte vers skills. `python3 tools/verify-communications.py` vérifie ces preuves; `python3 tools/build-communications-review.py` reconstruit l’index et la vue.

Garder les limites : aucune capture native iOS ou connexion directe téléphone–Mac établie; les routes mobiles sont des contrats et les consommateurs Mac. Le serveur qui choisit un pair, enrichit le contexte, compacte, impose les budgets et produit une skill depuis Teach n’est pas livré. La liste des corps ou observations nécessaires est dans les rapports résiduels. Ne pas refaire une analyse globale ni convertir les valeurs de repli 32/70 % en réglages effectifs.

Commencer par [goal-nine-topics.md](reports/goal-nine-topics.md) et ses quatre rapports. La [vue neuf sujets](.lavish/goal-neuf-sujets.html) complète la vue précédente. La passe ajoute 68 constats, 255 extraits et huit corps natifs de leases, sans modifier le produit Rust. `python3 tools/verify-goal-investigation.py` vérifie les preuves; `python3 tools/build-goal-review.py` reconstruit la synthèse et la vue. Les détails natifs sont reproductibles avec `python3 tools/inspect-native-lease.py`.

Précisions à garder : le rappel vocal est une projection de huit textes; reset ne prouve pas un oubli durable; un timeout d'outil peut suivre un effet appliqué; retirement d'approbation et cancel sont distincts; les plafonds locaux ne sont pas les budgets du moteur texte; teach ne prouve pas la création d'une skill; le bail natif du Mac ne prouve pas l'arbitrage du bureau cloud; les traces peuvent être abandonnées ou non échantillonnées. Les neuf sujets restent partiellement répondus au niveau du produit complet.

Lire [investigation-continuation.md](reports/investigation-continuation.md), puis les huit rapports de détail liés. La [vue navigable](.lavish/enquete-grokbot.html) les rassemble. Les preuves incluent désormais personnalité, cycle de délégation et huit corps natifs x86_64 désassemblés. `python3 tools/verify-investigation-detail.py` vérifie les hashes, extraits et liens. `python3 tools/build-investigation-detail.py` reconstruit la vue.

Le MCP REA a fonctionné pour les métadonnées natives et le démanglage. Aucun moteur profond Hopper, Ghidra ou IDA n'était disponible. La session native est fermée. Les preuves sont conservées sur disque. Les erreurs d'instrumentation historiques ne sont pas des résultats de cette passe.

Corrections à garder : 70 % est un repli sans consommateur Grok de compaction établi; les cartes de sous-agents ne prouvent pas un écran isolé; `takeControl` change l'état du viewer sans verrou serveur démontré; création/update/test de routines suivent la gateway; un exécuteur SDK déclaré n'est pas un handler actif. Scheduler et contexte délégué restent inconnus.

## Objectif et autorisation conservés

L’utilisateur a demandé de rétro-ingénierer Grok Bot afin de concevoir un système équivalent en Rust. Il a choisi explicitement « Analyse complète et architecture Rust ». Il a ensuite demandé de conserver tout le dossier et la liste des manques sur le Bureau, dans `grokbot-rust`.

L’analyse et la conception sont conservées. La reprise du 13 octobre 2026 ajoute une consolidation du socle Rust, explicitement choisie par l’utilisateur, avec des sous-agents. L’utilisateur a ensuite demandé un beau wrapper et choisi une app locale. Le bundle `app/build/Grokbot.app` existe; `Ouvrir Grokbot.command` l’ouvre. Lire [wrapper-local.md](reports/wrapper-local.md) pour sa portée et ses limites. Les esquisses Rust historiques dans les rapports ne constituent pas toutes une implémentation compilée. Suivre la nouvelle instruction de l’utilisateur pour la suite; ne pas interpréter cette sauvegarde comme une demande de déploiement ou d’actions sur des comptes tiers.

## Lire et vérifier avant de reprendre

1. Lire [LIRE_EN_PREMIER.md](LIRE_EN_PREMIER.md).
2. Lire [MANQUES_COMPLETS.md](MANQUES_COMPLETS.md), puis [reports/architecture-rust.md](reports/architecture-rust.md).
3. Lire [evidence/investigation.json](evidence/investigation.json), [evidence/review.json](evidence/review.json) et [evidence/runtime-status.json](evidence/runtime-status.json).
4. Vérifier la sauvegarde depuis ce dossier :

```sh
python3 tools/verify-handoff.py --evidence-only
python3 tools/verify-rust.py
```

La première commande vérifie les artefacts et les preuves de la sauvegarde historique, sans exiger que le code et les documents en cours restent figés. Le mode strict historique compare toujours le dossier entier; il échoue après les ajouts Rust et la présence du lien local `Forge compagnon.app`. La seconde vérifie le socle Rust puis l’ASAR conservé, les 673 fichiers extraits, les contrats et les références des rapports. Elle utilise la copie de `inputs/Grok Bot.app`; l’application installée n’est pas requise pour ce contrôle.

## Si la suite est une implémentation Rust

La phase 1 est partiellement implémentée dans les huit crates du workspace. Lire [orchestration-rust.md](reports/orchestration-rust.md) et [consolidation-rust.md](reports/consolidation-rust.md), puis lancer `python3 tools/verify-rust.py`. Rust et Cargo sont installés dans `~/.cargo/bin`; ajouter ce dossier au PATH du shell si nécessaire. Ne pas recréer le workspace.

Les tests du modèle de domaine et de reprise existent. Les scénarios couverts incluent la répétition d’un nonce, l’incertitude d’acceptation, un trou dans les événements, un corps absent, un changement de compte et une réponse tardive. Ces scénarios locaux ne nécessitent aucune clé de modèle. L’orchestration durable et le transport simulé existent dans `grokbot-session`. La commande `flow` montre une réponse perdue, une recherche de statut avant renvoi et des snapshots gardés. Le wrapper Mac local relie déjà cette orchestration à une interface de conversation, avec un serveur HTTP sur loopback et des réponses simulées. La prochaine étape possible est un control-plane local avec une API indépendante, une acceptation idempotente, une outbox durable et un worker simulé. Le socle actuel ne contient aucun transport distant réel ni moteur de modèle.

Les modules proposés sont des limites de responsabilités. Ils ne demandent pas de créer 23 microservices. Le premier ensemble de programmes est `desktop`, `control-plane`, `worker`, puis `local-exec` pour les capacités locales activées.

Le wrapper actuel est une coque Cocoa/WKWebView et une interface web hors ligne, pas une UI Iced. Pour un client entièrement Rust, valider les risques d’Iced : éditeur riche, mentions, IME, accessibilité et affichage distant. Le choix est un candidat, pas une parité démontrée. Tonic définit notre contrat gRPC; sa présence ne garantit pas une compatibilité avec les API Connect de Grok Bot.

## Si la suite est une observation de l’original

Préparer un compte et des bots de test, puis observer les parcours après connexion. Le démarrage par l’exécutable natif et Chromium CDP fonctionne désormais sur profil neuf ; les preuves réutilisables sont séparées de l’ancien lancement instrumenté échoué. Il n’existe toujours pas de preuve runtime authentifiée à réutiliser. La présence de fichiers dans un profil ne suffit pas : la nouvelle capture conserve un vrai renderer, ses sources et un écran de connexion.

Le MCP REA a répondu `Transport closed`. Le fallback CLI public a répondu `Process failed to launch!`. Les fuses désactivées rendent une incompatibilité plausible; la cause complète n’est pas prouvée. Conserver la signature et les fuses originales.

Une lecture des sources suffit aux questions déjà résolues. Pour un nouveau point natif ou runtime, utiliser le skill [reprise/skills/reverse-engineer-anything/SKILL.md](reprise/skills/reverse-engineer-anything/SKILL.md) et vérifier les outils disponibles dans la nouvelle session. Les sessions de décompilateur et les connexions MCP ne sont pas portables; le bundle de preuves l’est.

## Éviter de perdre ou de fausser les preuves

- Garder les records bruts REA et les fichiers de `artifacts/client` inchangés.
- Garder la distinction entre offset original en unités UTF-16 et ligne de la copie formatée, comptée avec LF.
- Ne pas reformater les bundles pendant une revue : les numéros de ligne changeraient.
- Garder les six contradictions ASAR au statut `observed-untrusted`, même avec des signatures Apple valides.
- Garder le helper Computer Use hors ASAR distinct de ces six contradictions.
- Un descripteur RPC, un appel de propriété candidat et un handler actif sont trois niveaux de preuve différents.
- `desktop-mount` est un panneau flottant. Le nom ne prouve pas un montage de fichiers.
- Les garanties Rust sur les effets externes, les grants et le shell sont des choix proposés, parfois plus stricts que le client observé.

## Rouvrir la vue

Ouvrir `vue/architecture.html`, ou lancer une nouvelle session locale Lavish :

```sh
~/.local/bin/lavish-axi vue/architecture.html
```

Utiliser l’URL renvoyée par cette commande. L’ancienne URL `http://127.0.0.1:4387/session/1491f4f66df720c6` appartient au dossier précédent et dépend d’un serveur local actif.

Le Chrome de vérification par défaut est headless. Une navigation réussie dans ce navigateur ne prouve pas une fenêtre visible chez l’utilisateur. Une session dédiée avec `CHROME_DEVTOOLS_AXI_HEADED=1` a pu charger la page. Le contrôle natif CUA retournait `apps: []` et `browsers: []`; le contrôle des fenêtres via macOS a auparavant refusé l’accès avec l’erreur `-25211`. L’ouverture visible n’a pas été confirmée par l’utilisateur.

Garder les profils de navigateur hors du dossier de la vue : leurs écritures peuvent déclencher des révisions et des analyses de mise en page inutiles. Les captures `vue/qa` concernent l’HTML de présentation, jamais le runtime de Grok Bot.
