# Ticket 295, ronde 2

La correction de [#574](https://github.com/charlescstpierr/ezerbot/pull/574) est figée sur `904f03fb3a9c6369a351fcfe8ace575d6b9acdf8`. Cette tête est locale et distante. Le worktree `/Users/cstpierrecharles/.treehouse/pm/ez-i295`, branche `pm/ez-i295`, est propre. [Identité](head.json), [confirmation GitHub](pr-readback.json) et [intégration de main](sync.log).

Le retour vers une Page affiche son titre et Restauration pendant l'attente, puis relit après réussite ou refus. La relecture d'une citation préfère l'instantané Page installé depuis son départ. Elle ne réinstalle pas la source annulée, un passage disparu ou une citation quittée. Le renommage remplace son instantané au lieu de le modifier en place.

Les classes restent `navigation-perdue@app/ui/panels` et `reponse-perimee@app/ui/panels`. Leur [inventaire des sites et liens vers les tests](sites.md) couvre les deux GET, les résultats de restauration et de renommage, les résumés d'état, la navigation et le composeur. Il distingue les réponses avant, pendant, après, refusées et tardives.

## Rouges puis verts

Les deux défauts ont été reproduits avant modification du produit sur `caaf0734037a4bd3536e291d365b111f8c9ed219`. [Commande, code et empreintes](red.json), [source des tests](red-tests.cjs) et [sortie rouge](red.log). Le commit `5d237b47` enregistre les tests avant le correctif.

```text
AssertionError [ERR_ASSERTION]: la Page redevient visible avant la réponse de restauration
false !== true
AssertionError [ERR_ASSERTION]: la version annulée ne doit jamais être repeinte
true !== false
```

Les autres sites ont donné des rouges sur le [renommage](inventory-rename-red3.log), le [bouton d'attente](pending-red.log) et le [titre au retour d'une autre Page](return-title-red.log). Les erreurs de fixture et la correction d'une attente de focus sont documentées dans [fixture-notes.md](fixture-notes.md). Aucun test n'est sauté, ignoré, filtré ou mis en quarantaine.

Sur la tête finale, [checks.json](checks.json) conserve les commandes et leurs codes.

- [81 tests UI](ui-head.log), dont 24 nouveaux cas avec le vrai DOM, `app.js`, `chat.js`, `pages.js` et des réponses HTTP indépendantes.
- [36 tests Rust Pages](rust-pages-head.log), zéro ignoré ou filtré.
- [Format](fmt-head.log), [deux jobs CI sur leur SHA exact](ci.json) et [gitleaks avant push](gitleaks-code.log) réussis.

```text
ℹ tests 81
ℹ pass 81
ℹ fail 0
ℹ cancelled 0
ℹ skipped 0
ℹ todo 0
```

## Parcours du binaire final

Le [parcours complet](final3/cases.json) et le [complément traité](treated/cases.json) portent sur la même tête. Leurs verdicts owner sont [PASS](final3/report.md) et [PASS](treated/report.md). Les [32 captures](captures.json) couvrent clair et sombre, à 1140×780 et 820×560. La [comparaison Galet](visual-inspection.md) précise sa méthode et ses limites.

Doctor passe avant et après chaque parcours. Les [Cleanup](final3/cleanup.json) [des deux runs](treated/cleanup.json) et leurs [audits PID](final3/cleanup-audit.json) [par un processus distinct](treated/cleanup-audit.json) confirment les racines signées, leur suppression et l'arrêt des quatre processus attribués. Seule la session `pm-i295` est arrêtée. Le [HOME du lanceur](launcher-cleanup.json) est aussi retiré.

Deux premiers lancements omettaient `--sphere pages` et recevaient OK sans `write_page`. Leur cause, leurs nettoyages et le parcours corrigé sont dans [attempts.json](attempts.json). Le parcours complet a retardé la livraison du corps HTTP réel de restauration pour photographier le retour. Les refus et les lectures concurrentes du composeur sont prouvés par la suite HTTP contrôlée, sans injection de refus dans le binaire final.

## Diff et verdict

La ronde 2 ajoute 16 lignes produit et en retire 7 dans `pages.js`. Elle ajoute 303 lignes de tests. Le [diff total](diff.txt) contient 838 ajouts et 18 retraits dans sept fichiers, dont 628 ajouts de tests. Le repère d'environ 300 lignes reste dépassé. Le nouveau serveur de test et son pilote navigateur sont nécessaires pour traverser le vrai composeur et observer les repeints transitoires. Aucun nouveau moteur ni fonction de transfert de brouillon n'est ajouté.

`principle-fix-root-causes` a imposé les rouges produit et l'inventaire des chemins voisins. `principle-model-the-domain` a conduit à conserver des instantanés Page et les jetons de citation existants. `principle-prove-it-works` a imposé le rejeu sur cette tête après la fusion de main. `principle-laziness-protocol` a limité le correctif produit à un seul module.

Verdict owner PASS. Aucun verdict officiel PROPRE ni statut PRÊT n'est revendiqué. Aucun `codex-review`, `verifier`, `pret` ou sous-agent de revue produit n'a été lancé. La revue indépendante et le statut final restent au coordinateur. La fusion reste à Firstmate. Aucune suite globale du dépôt ni conduite de l'application Cocoa n'est revendiquée.
