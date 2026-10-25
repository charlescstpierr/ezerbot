# Ronde 12 du ticket 294

Point de contrôle. Test rouge, correctif de cinq lignes, merge nécessaire, vérifications complètes, push, preuve exacte puis nettoyage. Aucun verifier ou pret par l’owner. Le coordinateur garde ces deux commandes.

La base attendue était 638abad5340b6737c16a6bc24f95f28e990cd717, worktree propre. La décision Firstmate du 25 octobre à 17:12 autorise seulement la citation perdue pendant la création. La découpe Tampon r11 reste intacte.

## Rouge et vert

Commande depuis le worktree.

```
/Users/cstpierrecharles/.treehouse/pm/_orch/bin/test-gate node --test app/ui/draft-transfer.test.cjs app/ui/panels/pages.test.cjs
```

`red.log` conserve le rouge réel sur la base attendue avec le test ajouté. 56 tests, 54 réussis, deux échecs sur la puce cachée après refus. La croix seule passe. `red-receipt.json` fixe la commande et la tête. `test.patch` contient le test et le clonage des réponses HTTP. Chaque réponse possède son instantané indépendant. Aucune assertion existante n’a été retirée ou affaiblie.

Le commit fdae7a92 enregistre le test avant le correctif. Le commit bd1c0673 ajoute cinq lignes dans onState de pages.js. `green-before-merge.log` donne 56/56, zéro saut. Le correctif garde orphan.anchor lors du passage du brouillon sans fil au fil créé, seulement pendant une opération occupée, pour la même Page et le même bot, hors tâche.

## Merge et contrôles

GitHub indiquait CONFLICTING. Merge normal de origin/main c721c313. Seul conflit dans app/ui/chat.js, bundle généré. `npm run build` depuis app/web résout ce conflit avec les sources fusionnées. Aucun changement de pages.js, app.js ou du test de citation pendant ce merge. Tête finale 7427e46b36ee558878440fead366c74c511b778e.

```
/Users/cstpierrecharles/.treehouse/pm/_orch/bin/test-gate node --test app/ui/*.test.cjs app/ui/panels/*.test.cjs app/web/src/*.test.js
/Users/cstpierrecharles/.treehouse/pm/_orch/bin/cargo-gate /Users/cstpierrecharles/.cargo/bin/cargo test --locked -p grokbot-wrapper --test pages -- --test-threads=4
gitleaks git --redact --log-opts=origin/main..HEAD
git push origin pm/ez-i294
```

Les logs sont node.log, rust.log, gitleaks.log et push.log. Le diff de la PR est dans stat.txt. La portée du correctif R12 est de deux fichiers, 33 ajouts et cinq retraits, dont cinq lignes de produit.

## Preuve isolée

Les scripts de ce tour sont dans tools. run-proof.py impose pm-i294, TMPDIR et HOME axi dans ce dossier de preuves et cargo-gate. Il conserve les chemins du toolchain installé. Le wrapper reçoit son propre HOME jetable par Launch. proof.py passe toutes les commandes Chrome par browser.sh, qui vérifie l’attribution signée du run.

Le Cleanup de main ne force plus la session par défaut. cleanup-safe.py garde aussi axi-cleanup-guard, qui refuse tout sauf pm-i294 stop. cleanup-guard-refusal.log conserve son refus 91 avec une session différente. Aucun script partagé modifié.

Principes lus. principle-fix-root-causes place la correction avant choose, au premier effacement indu. principle-model-the-domain garde les structures quote et orphan existantes. principle-prove-it-works impose le rouge observé et les preuves du binaire final.

Le premier passage complet Node a donné 1 124 réussites et deux annulations par délai dépassé dans shell-stop.test.js. Aucun échec d’assertion, skip ou todo. Son résultat non vert reste dans node.log. La suite entière a été relancée avec une seule tâche Node à la fois, sans modifier les tests, leurs assertions ou leurs délais.

```
/Users/cstpierrecharles/.treehouse/pm/_orch/bin/test-gate node --test --test-concurrency=1 app/ui/*.test.cjs app/ui/panels/*.test.cjs app/web/src/*.test.js
/Users/cstpierrecharles/.treehouse/pm/_orch/bin/test-gate python3 /Users/cstpierrecharles/.treehouse/pm/_orch/pages/evidence/i294/r12/tools/live.py
python3 /Users/cstpierrecharles/.treehouse/pm/_orch/pages/evidence/i294/r12/tools/run-proof.py bash .cursor/skills/verify-ezerbot/scripts/verdict.sh --evidence /Users/cstpierrecharles/.treehouse/pm/_orch/pages/evidence/i294/r12/final --ref HEAD --status pass
python3 /Users/cstpierrecharles/.treehouse/pm/_orch/pages/evidence/i294/r12/tools/run-proof.py python3 /Users/cstpierrecharles/.treehouse/pm/_orch/pages/evidence/i294/r12/tools/cleanup-safe.py --evidence /Users/cstpierrecharles/.treehouse/pm/_orch/pages/evidence/i294/r12/final
```

live.py contient les commandes exactes Launch, Doctor, Drive et compare. Elles passent toutes par l’environnement isolé run-proof.py. Les verrous globaux CARGO_GATE_DIR et TEST_GATE_DIR restent ceux du Mac malgré le HOME axi isolé. La preuve réelle termine avec six captures et PASS owner sur la tête finale. Le refus 400 et l’attente de création sont injectés à la frontière HTTP du navigateur. La suppression du fil, la création du nouveau fil et le réessai utilisent le serveur réel isolé. Les réponses HTTP réelles ne partagent aucun objet avec le client.

Doctor est vert avant et après. Le wrapper et le faux fournisseur n’ont que des sockets TCP sur 127.0.0.1. Les six images ont été ouvertes et comparées visuellement aux captures Galet existantes de docs/captures. visual-inspection.md précise cette comparaison et ses limites. Aucun changement de CSS dans R12.

Cleanup a terminé avec code 0. final/cleanup.json confirme auth, wrapper, fake et Chrome arrêtés, pm-i294 arrêté et le HOME jetable absent. Les contrôles d’identité et d’attribution signée du script de main sont conservés. Aucun accès au compte réel ou aux données de Charles.
