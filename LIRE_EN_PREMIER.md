# Grok Bot en Rust — dossier de reprise

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
