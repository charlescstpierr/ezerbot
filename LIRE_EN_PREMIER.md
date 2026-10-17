# Grok Bot en Rust — dossier de reprise

La dernière passe REA suit [Messages après panne](reports/messages-worker-recovery.md) jusqu’au [helper natif](reports/native-messages-recovery.md). Une [sonde de tranches originales avec fixtures](reports/messages-worker-slice-replay.md) vérifie UUID et retry, sans lancer l’app originale ni envoyer de message. Vérifier avec `python3 tools/verify-messages-recovery-followup.py`. Le redémarrage du worker ne rejoue pas ses opérations ; les protections B4 restent volatiles.

La dernière demande vise la parité fonctionnelle de notre app et insiste sur REA. La [suite groupes et mentions](reports/multibot-membership-paths.md) sépare `@`, modification des membres et création d’une room. Les [tours côté hôte](reports/multibot-turn-paths.md) et la [passe REA ciblée](reports/multibot-rea-followup.md) conservent les preuves et les inconnues serveur. Vérifier avec `python3 tools/verify-multibot-investigation.py`. La proposition de nouveau moteur n’est pas approuvée; les rapports ne sont pas du code produit livré.

La suite [contrôle local Mac 0.71.2](reports/local-cua-path-0712.md) relie maintenant le provider authentifié, le consentement d’app, la file par conversation et l’enfant MCP au [companion natif](reports/native-0712-mcp.md). Le [retrait d’une autorisation](reports/local-cua-consent-0712.md) n’arrête pas rétroactivement un appel admis. Un [échantillon arm64 .70](reports/native-arm64-residual.md) confirme aussi six propriétés natives ciblées. Vérification : `python3 tools/verify-local-control-followup.py`. La vue navigable comporte un nouvel onglet « Contrôle du Mac .71 ».

La [nouvelle recherche des preuves manquantes](reports/remaining-evidence-progress.md) obtient un démarrage réel de l’original : écran de connexion et **45 scripts chargés identiques au manifeste**. Elle identifie aussi l’app iPhone et son parcours de connexion, puis relie la révocation native aux événements clavier/souris. Les comportements décrits publiquement restent distingués du code récupéré. Le moteur serveur et l’IPA iOS restent à obtenir. Vérifications : `python3 tools/verify-original-runtime.py` et `python3 tools/verify-remaining-evidence.py`.

La [passe communications et iPhone](reports/communications-investigation.md) ajoute les échanges entre bots, mentions, réponses, retours vocaux et le chemin iPhone↔Mac accessible côté client. Elle conserve 39 constats et 159 extraits dans quatre rapports, avec [vue navigable](.lavish/communications-grokbot.html). L’app native iOS et le moteur serveur restent absents. Vérification : `python3 tools/verify-communications.py`.

Le [goal des neuf sujets](reports/goal-nine-topics.md) approfondit contexte, mémoire et oubli, actions incertaines, boucle outils, permissions et secrets, budgets, ressources partagées, démonstrations et traçabilité. Il ajoute quatre rapports, 68 constats et 255 extraits hashés, ainsi que huit corps natifs de contrôle de session. La [nouvelle vue navigable](.lavish/goal-neuf-sujets.html) relie les réponses et les inconnues. Vérifier avec `python3 tools/verify-goal-investigation.py`.

La [suite de l'enquête](reports/investigation-continuation.md) approfondit maintenant les cronjobs, les ordinateurs, la compaction, la délégation, les runtimes et la personnalité des bots. La [vue navigable](.lavish/enquete-grokbot.html) présente les résultats et leurs limites. Les nouvelles preuves restent statiques; le moteur distant n'est pas récupéré.

Ce dossier conserve l’analyse de Grok Bot 0.70.2, les preuves et l’architecture Rust proposée. Il contient aussi une copie de l’application signée analysée. Un socle Rust local existe maintenant : domaine, contrats, journal, réplique, SQLite, orchestration durable, scénarios CLI et wrapper Mac. Double-cliquer sur `Ouvrir Grokbot.command` pour ouvrir l’app indépendante; ses réponses restent simulées. [reports/wrapper-local.md](reports/wrapper-local.md) décrit cette nouvelle interface.

Ouvre [MANQUES_COMPLETS.md](MANQUES_COMPLETS.md) pour voir tout ce qui reste à établir, à choisir, à construire et à tester. Lis [PROCHAINE_SESSION.md](PROCHAINE_SESSION.md) pour reprendre le travail sans recommencer l’analyse.

- [Wrapper local](reports/wrapper-local.md) : app Mac, conversations persistantes, thèmes et reprise des envois.
- [Orchestration durable](reports/orchestration-rust.md) : commandes avant transport, réponses tardives et reprise de processus.
- [Consolidation Rust](reports/consolidation-rust.md) : correctifs de reprise, tests et limites du socle.
- [Vérification du socle](tools/verify-rust.py) : commande `python3 tools/verify-rust.py`.
- [Vue interactive](vue/architecture.html) : diagrammes, scénarios de reprise, 45 familles et extraits de source.
- [Architecture Rust](reports/architecture-rust.md) : responsabilités, types, persistance et flux d’échec.
- [Matrice de parité](reports/parity.md) : les 45 familles et leurs critères de validation futurs.
- [Index des preuves](README.md) : rapports, empreintes et scripts de vérification.
- [État de session](SESSION_STATE.json) : contexte et limites lisibles par la prochaine session.

Le dossier est autonome pour consulter les rapports, vérifier les fichiers extraits et relire les preuves REA. Les données d’un compte Grok et les profils de navigateur ne font pas partie de cette sauvegarde.

La vue HTML reprend les couleurs du client Grok Bot. Son affichage du code utilise un module CDN et nécessite Internet. Les sources et les rapports Markdown restent consultables sans ce module.

Pour la prochaine session, tu peux donner cette instruction :

> Lis /Users/cstpierrecharles/Desktop/grokbot-rust/LIRE_EN_PREMIER.md, PROCHAINE_SESSION.md et MANQUES_COMPLETS.md. Reprends à partir des preuves conservées. Distingue les faits du client, les inconnues du serveur et les choix de conception Rust.
