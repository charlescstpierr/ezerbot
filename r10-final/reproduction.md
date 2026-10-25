# Exécution de la preuve r10

Tête du binaire et de la PR `e84053c9eb08aff8997724ad856c6535fe88f609`.

Le pilote utilise chrome-devtools-axi relié au Chrome isolé enregistré par verify-ezerbot. Le wrapper et le faux fournisseur n’ont que des sockets loopback. Les 11 captures ont été ouvertes et inspectées.

Les refus 400/409 sont injectés à la frontière HTTP du navigateur. Les clics, l’éditeur, le panneau Pages, les transferts, la création de conversation et les réessais utilisent l’application réelle. Les fichiers sont des documents synthétiques. Les scénarios sont nommés dans cases.json.

Le scénario avant journal écrit la même transaction de métadonnées que le test Rust dans la base jetable, après une interruption avant réception serveur. Il constate l’absence du message dans le DTO, recharge le navigateur, puis utilise Actions et Renvoyer. Aucun processus sain n’est tué pour simuler un crash.

Les variantes de renvoi sur un message déjà livré affichent temporairement queued dans le DTO navigateur pour atteindre le menu. Ce sont des fixtures explicites. Le renvoi serveur réel utilise ensuite la commande de développement Tester une coupure.

Le cas T23 supprime artificiellement bot_pages dans la base jetable. Aucun chemin de suppression de cette table n’est établi en production. La suppression réelle du fil d’origine est testée séparément dans les cas orphelins.

Cinq ajustements du pilote ont été nécessaires. Le premier appel au pont a échoué avant toute action; la reconnexion suivante a réussi. Une requête directe omettait X-Grokbot-Request, exigé par la frontière HTTP de lib.rs; le contrat attendu 400 est ensuite observé avec cet en-tête. Une attente renvoyait un nœud React non sérialisable au lieu d’un booléen. Enfin, après rechargement, le pilote devait rouvrir la Page par son lien avant une nouvelle citation. Le contrôle de coupure exige aussi son événement change; un simple checked=true ne l’active pas. L’attente bornée a expiré, puis ce seul cas a été rejoué avec cet événement. Ces ajustements n’ont changé aucun fichier produit ou test du dépôt.

Les sorties échouées restent dans drive-startup-failed.log, drive-header-failed.log, drive-dom-failed.log et drive.log. Les reprises partielles ont gardé les mêmes PID et le même binaire. Après ces ajustements, le parcours corrigé complet a été rejoué depuis son début et a terminé avec sortie 0. Son journal est drive-full.log. Les captures et cases.json publiés proviennent de ce dernier parcours intégral. Les résultats partiels antérieurs sont conservés localement sous segments/.

Pour T23, le parcours complet vérifie le DTO avec anchor=null et present=true, puis le refus 409 d’un envoi direct sans ancre. Après rechargement, Actions puis Renvoyer utilise la route serveur réelle avec succès. Aucun repli client n’est utilisé pour cette dernière action.

proof.py contient le parcours complet corrigé. Les scripts proof-resume.py, proof-resume-final.py et proof-resume-t23.py gardent les étapes de reprise. drive-resume-t23.py archive les résultats partiels avant de relancer proof.py au complet. La commande finale exécutée, sous test-gate, est la suivante.

```sh
/Users/cstpierrecharles/.treehouse/pm/_orch/bin/test-gate python3 /Users/cstpierrecharles/.treehouse/pm/_orch/pages/evidence/i294/r10-tools/drive-resume-t23.py /Users/cstpierrecharles/.treehouse/pm/_orch/pages/evidence/i294/r10-final
```

Doctor après parcours termine avec sortie 0. Le rapport owner porte PASS sur la tête exacte. Cleanup termine avec sortie 0 et confirme pm-i294 comme seule session du pont arrêtée. Les échecs antérieurs du pilote ne sont pas présentés comme des exécutions vertes.
