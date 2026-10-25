# Commandes de découpe r11

Depuis `/Users/cstpierrecharles/.treehouse/pm/ez-i294`.

1. `git revert --no-edit 48c6e404`, sortie 0, commit f88d55c3.
2. Tests ajoutés avant le correctif. `test-gate node --test app/ui/draft-transfer.test.cjs app/ui/panels/pages.test.cjs`, 53 tests, 51 verts et deux rouges dans r11-red.log. Les pertes orpheline et passage disparu échouent. Les trois protections de retrait et de citation récente sont des non-régressions déjà vertes.
3. Même commande après réapplication citation, 53/53 dans r11-green.log.
4. `/Users/cstpierrecharles/.treehouse/pm/_orch/bin/test-gate node --test app/ui/*.test.cjs app/ui/panels/*.test.cjs app/web/src/*.test.js`, r11-node.log.
5. `/Users/cstpierrecharles/.treehouse/pm/_orch/bin/cargo-gate /Users/cstpierrecharles/.cargo/bin/cargo test --locked -p grokbot-wrapper --lib --test storage --test delegation --test delegation_success --test pages --test state_scale -- --test-threads=4`, r11-rust.log.
6. `npm run build` depuis app/web, r11-web-build.log. Aucun script npm test dans ce package. La commande Node précédente exécute ses tests.

Le test donne des identifiants de fil croissants même après suppression. La fixture distingue ainsi le fil d’origine supprimé du fil créé par l’envoi. L’avertissement de restauration est notifié après l’erreur d’envoi pour rester visible, sans notifyDraft ni draftWarnings.

Le principe principle-prove-it-works impose des tests rouges observés puis les captures sur la tête finale. Le point de contrôle sépare revert, correctif citation, vérifications, commit et push, puis preuve finale. Aucun nouveau correctif hors portée, aucune revue officielle ni commentaire public.


Les commandes Node et Rust complètes ont terminé avec sortie 0, respectivement 1105 et 449 tests réussis. Aucun échec, test sauté ou ignoré.

`git commit -m "fix(pages): remettre la citation au refus d'un envoi (#294)"` produit 638abad5340b6737c16a6bc24f95f28e990cd717. `gitleaks git --redact --log-opts=origin/main..HEAD` termine avec sortie 0 avant `git push origin pm/ez-i294`. La tête distante est confirmée par `gh pr view 335 --json headRefOid`. Le JSON utilise gh car gh-axi ne prend pas ce format pour pr view.

Le `git diff --stat origin/main...HEAD` est conservé dans r11-stat.txt. Il compare depuis la base commune b93ae7bf. La référence partagée origin/main a avancé pendant la découpe. Aucun nouveau merge n’a été effectué.

Commande de lancement sur la tête exacte, cargo uniquement via le wrapper cargo-gate.

```sh
TMPDIR=/Users/cstpierrecharles/.treehouse/pm/_orch/pages/evidence/i294/r11-tmp CHROME_DEVTOOLS_AXI_SESSION=pm-i294 EZERBOT_VERIFY_CARGO=/Users/cstpierrecharles/.treehouse/pm/_orch/pages/evidence/i294/r11-tools/cargo bash .cursor/skills/verify-ezerbot/scripts/launch.sh --evidence /Users/cstpierrecharles/.treehouse/pm/_orch/pages/evidence/i294/r11-final --sphere pages
```


Les étapes de preuve suivantes ont toutes terminé avec sortie 0.

```sh
bash .cursor/skills/verify-ezerbot/scripts/doctor.sh --evidence /Users/cstpierrecharles/.treehouse/pm/_orch/pages/evidence/i294/r11-final
TMPDIR=/Users/cstpierrecharles/.treehouse/pm/_orch/pages/evidence/i294/r11-tmp CHROME_DEVTOOLS_AXI_SESSION=pm-i294 /Users/cstpierrecharles/.treehouse/pm/_orch/bin/test-gate python3 /Users/cstpierrecharles/.treehouse/pm/_orch/pages/evidence/i294/r11-tools/drive.py /Users/cstpierrecharles/.treehouse/pm/_orch/pages/evidence/i294/r11-final
bash .cursor/skills/verify-ezerbot/scripts/doctor.sh --evidence /Users/cstpierrecharles/.treehouse/pm/_orch/pages/evidence/i294/r11-final
bash .cursor/skills/verify-ezerbot/scripts/compare.sh --evidence /Users/cstpierrecharles/.treehouse/pm/_orch/pages/evidence/i294/r11-final
bash .cursor/skills/verify-ezerbot/scripts/verdict.sh --evidence /Users/cstpierrecharles/.treehouse/pm/_orch/pages/evidence/i294/r11-final --ref HEAD --status pass
TMPDIR=/Users/cstpierrecharles/.treehouse/pm/_orch/pages/evidence/i294/r11-tmp CHROME_DEVTOOLS_AXI_SESSION=pm-i294 python3 /Users/cstpierrecharles/.treehouse/pm/_orch/pages/evidence/i294/r11-tools/cleanup-safe.py --evidence /Users/cstpierrecharles/.treehouse/pm/_orch/pages/evidence/i294/r11-final
```

Le PASS appartient à la preuve owner verify-ezerbot. Aucun bin/verifier ni pret n’a été lancé. Les six captures ont été inspectées. Les sockets du wrapper et du faux fournisseur sont exclusivement loopback. Les corpus evidence et formatted sont absents et restent non comparés.

Le nettoyage charge le script du worktree avec sa constante de session limitée à pm-i294 et son exécutable limité à la seule commande stop de cette session. Ses vérifications de PID, d’identité, de groupe, de chemin et de signature restent en place. cleanup.json confirme wrapper, fake et Chrome arrêtés, axi.session=pm-i294, code=0 et le home jetable retiré. Le script partagé n’est pas modifié. Le garde a déjà été vérifié en r10, son refus est conservé dans r10-tools/cleanup-guard-refusal.log.
