# Owner du ticket 295, ronde 1

La PR [#574](https://github.com/charlescstpierr/ezerbot/pull/574) est livrée pour la vérification du coordinateur. La tête distante et la tête locale sont `caaf0734037a4bd3536e291d365b111f8c9ed219`. Le worktree `/Users/cstpierrecharles/.treehouse/pm/ez-i295` est propre. Le reçu de fetch puis merge de main est dans [sync.log](sync.log).

Le panneau affiche les passages soulignés, les numéros, la liste et les états. La restauration reprend la seule source précédente et relit la Page. Les ancres portent le vrai couple fil/message, y compris après suppression du fil d’origine. L’index par Page évite le parcours des ancres étrangères.

Le diff total contient 523 ajouts et 15 retraits dans six fichiers. Le repère d’environ 300 lignes n’est pas satisfait. Il comprend 195 ajouts produit, 325 ajouts de tests et trois lignes de carte de vérification. Le contrat ajouté se limite au fil de l’ancre. Les moteurs de création, de modification, de restauration et de citation restent ceux déjà présents.

## Contrôles sur la tête finale

Les commandes exactes, leurs codes de sortie et leur tête sont dans [checks.json](checks.json). Elles sont rejouables par [checks.py](checks.py).

- `cargo-gate cargo test --locked -p grokbot-wrapper --test pages -- --nocapture` passe, 36 tests, zéro échec, zéro ignoré, zéro filtré.
- `cargo-gate cargo fmt --all -- --check` passe.
- `test-gate node --test app/ui/panels/pages.test.cjs app/ui/panels/pages-browser.test.cjs app/ui/draft-transfer.test.cjs` passe, 57 tests, aucun échec, skip, todo ou annulation.
- Les deux jobs CI de compilation passent. Voir [ci.txt](ci.txt) et [tête distante](pr-head.json).
- Gitleaks passe avant chacun des deux pushes produit. La publication des preuves passe aussi par gitleaks.

Le vrai GET mesure 44 instructions SQLite avec 1 000 ancres d’autres Pages, puis 44 avec 10 000. Il renvoie le même résultat avec la même ancre cible. Ce contrôle porte sur les instructions, sans revendication de latence. [Plan avant et après](anchors-query-plan.txt). Avant, SCAN avec tri temporaire. Après, SEARCH USING INDEX wrapper_page_anchors_page.

Le test navigateur utilise le vrai DOM et des réponses JSON servies par HTTP. Il couvre la sélection préservée, la citation et sa croix, les deux repères, le traitement et le passage disparu, le lien au message, la restauration, le refus serveur, le double clic, le polling avant et pendant la mutation, les réponses tardives, le changement de Page et le retour pendant une restauration. Les suites existantes conservent les refus et les accusés tardifs de #294.

Le défaut de largeur du titre a été reproduit par [un test rouge](title-width-red.log), puis corrigé avec [57 tests verts](title-width-green.log). Le run préliminaire [final4](final4/report.md) porte FIX sur l’ancienne tête. Les trois erreurs précédentes du pilote et leurs nettoyages sont décrits dans [attempts.json](attempts.json).

## Preuve Mac

La session isolée est `pm-i295`. Le build de lancement, Doctor avant et après, la pilule Modèle et les valeurs de thème ont été contrôlés. Le faux OpenAI n’a que des sockets 127.0.0.1. Le [parcours réel](final5/cases.json) crée deux commentaires ouverts, rejoint le vrai message, modifie Confort de 19 € à 21 € avec edit_page, traite le commentaire, restaure la version initiale, garde 21 € dans la liste sans repère et rejoint le commentaire d’un nouveau fil après suppression du fil d’origine.

Les [24 captures](final5/spheres/pages) ont été inspectées. Leur [comparaison Galet](visual-inspection.md) nomme les deux références ouvertes et les corpus absents ou non comparés. Les captures restent attachées à la tête exacte par [captures.json](captures.json).

La preuve owner est [PASS caaf0734037a4bd3536e291d365b111f8c9ed219](final5/report.md). Le [Cleanup](final5/cleanup.json) confirme la suppression du HOME jetable, l’absence de processus encore attribués au run et l’arrêt du pont pm-i295. Les gardes de répertoire signé et d’identité PID ont été lus et vérifiés. [Reçu des identités](final5/owned-processes.json). [Self-check](cleanup-guards.log).

## Choix et limites

`principle-model-the-domain` a conduit à conserver le couple fil/message de l’ancre et à réunir chaque numéro avec sa plage DOM. `principle-sequence-verifiable-units` a séparé les contrôles du contrat, des comportements et de l’interface réelle. `principle-fix-root-causes` a conduit à corriger la largeur du champ dans la règle flex. `principle-prove-it-works` a imposé le rejeu complet après ce dernier commit.

Le contrat d’ancre contient un passage sans offset. Si le texte apparaît plusieurs fois, le premier passage correspondant dans le DOM porte le repère. Les repères restent limités à la partie visible du canevas. Les refus et les courses sont prouvés dans le navigateur avec un serveur contrôlé, sans injection de refus dans le parcours Mac final. Aucun test de l’ensemble du dépôt ni parcours Cocoa natif n’est revendiqué.

La tête est figée. Aucun codex-review, verifier ou pret officiel n’a été lancé par l’owner. Aucun verdict indépendant ni statut PRÊT n’est revendiqué. Le coordinateur garde l’unique revue officielle et le statut final. La fusion reste à Firstmate.
