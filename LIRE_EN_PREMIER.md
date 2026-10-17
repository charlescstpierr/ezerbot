# Grok Bot en Rust — dossier de reprise

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
