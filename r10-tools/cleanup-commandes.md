# Nettoyage limité à #294

Le reçu initial `../r10-before/cleanup.json` est conservé. Il montre la mauvaise session `verify-ezerbot` du script partagé. Le script partagé n'est pas modifié.

Le garde refuse toute session autre que `pm-i294`, toute commande autre que `stop` et tout argument supplémentaire. Test de refus exécuté et reçu dans `cleanup-guard-refusal.log`.

Commande exécutée pour l'ancien pont de ce run, avec reçu `cleanup-bridge-previous.log`.

```sh
CHROME_DEVTOOLS_AXI_SESSION=pm-i294 /Users/cstpierrecharles/.treehouse/pm/_orch/pages/evidence/i294/r10-tools/axi-cleanup-guard stop
```

La commande finale exécutée utilise `cleanup-safe.py`. Ce fichier charge le cleanup partagé, conserve ses vérifications d'identité des PID, de signatures, de chemins et de groupe. Il remplace seulement la constante de session dans le module chargé et impose l'exécutable garde. Une session par défaut ne peut donc pas être arrêtée, même si le module ignore l'environnement.

```sh
TMPDIR=/Users/cstpierrecharles/.treehouse/pm/_orch/pages/evidence/i294/r10-tmp CHROME_DEVTOOLS_AXI_SESSION=pm-i294 python3 /Users/cstpierrecharles/.treehouse/pm/_orch/pages/evidence/i294/r10-tools/cleanup-safe.py --evidence /Users/cstpierrecharles/.treehouse/pm/_orch/pages/evidence/i294/r10-final
```

Cette dernière commande a terminé avec sortie 0. Le reçu `r10-final/cleanup.json` confirme wrapper, fake et Chrome arrêtés, axi.session=pm-i294, code=0, le home jetable retiré et aucun processus encore attribuable au run.

Le pilote `drive.py` utilise le lanceur Chrome isolé de la skill pour enregistrer PID, identité et profil dans browser.json. Il relie ensuite chrome-devtools-axi uniquement au port local de ce Chrome. Aucun auto-connect ni profil utilisateur. Cleanup conserve ainsi ses contrôles d’identité pour Chrome, wrapper et faux fournisseur.
