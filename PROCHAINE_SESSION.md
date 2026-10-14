# Reprendre le travail

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

Préparer un compte et des bots de test, puis observer les parcours après connexion. Le lancement instrumenté précédent a échoué. Il n’existe pas de preuve runtime authentifiée à réutiliser. La présence de fichiers dans le profil temporaire de lancement ne transforme pas cet échec en scénario instrumenté réussi.

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
