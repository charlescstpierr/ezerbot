# Tâches de fond

Statut : décisions du 21 octobre 2026, copie du comportement visible de Grok Bot 0.72.1 pour le shell long, recalé sur le Mac de Charles. Aucune implémentation de ce lot n’est commencée. La première tranche est le lancement en arrière-plan. Les tranches sont à créer comme issues enfants d’une carte `wayfinder:map` « Tâches de fond ». Cette page ne crée pas les issues. L’annexe en donne le corps.

L’ordinateur cloud (T10, SCR-4, IPH-3) est reporté. Il est décrit au hors-périmètre et n’a pas de ticket.

Vocabulaire déjà en usage : bot, fil, bulle, approbation, shell, contrôle de l’écran. Une tâche de fond est une commande lancée par le bot qui continue pendant que le fil reste utilisable. Arrêter est le bouton de cette tâche.

## Contexte

Le shell du bot existe. `run_shell` part après l’approbation déjà livrée. La description de l’outil dit de ne pas lancer de tâche détachée. Le délai par défaut est trente secondes. La valeur acceptée va de 100 ms à 120 s. La sortie est plafonnée (1 Kio à 1 Mio, 128 Kio par défaut). Pendant ce temps, la commande occupe le tour : Charles n’a pas une carte de tâche à consulter, et le bot n’enchaîne pas une autre réponse comme si la commande vivait à côté.

Le contrôle de l’écran est déjà à parité sur les scénarios SCR-1 et SCR-2 : session visible, pause si Charles reprend la souris, champs mot de passe refusés. Ce lot n’y touche pas.

Grok Bot sait lancer une commande longue, la suivre, y écrire, et prévenir quand elle se termine. Le scénario SCR-3 (« sleep 120 && echo fini ») échoue ici sur le délai. Le scénario SCR-4, l’ordinateur du bot dans le cloud, est un autre sujet : les specs des routines et des réunions le laissent pour plus tard.

## Énoncé du problème

Une commande plus longue que le délai du shell meurt, ou bloque le tour. Charles ne voit pas un état En cours, ne lit pas la sortie à son rythme, et ne peut pas l’arrêter par un bouton. Il ne peut pas non plus répondre à une invite du programme. Quitter Ezerbot ne doit pas laisser croire que la commande tourne encore.

## Solution

`run_shell` accepte un mode arrière-plan. La commande n’est plus coupée à 30 s ni à 120 s. Elle apparaît dans le fil : En cours, puis Terminée avec le code de sortie, sortie consultable, bouton Arrêter. L’approbation actuelle reste exigée avant le départ. Charles peut continuer à parler au bot pendant que la tâche tourne.

Il peut envoyer une ligne sur l’entrée standard d’une tâche En cours. Une invite de mot de passe est refusée, comme un champ sécurisé de SCR-2. Quitter l’app tue la tâche et la marque Arrêtée.

## Décisions

1. **L’arrière-plan est un mode du shell, pas un second outil.** L’approbation déjà livrée précède le lancement. Le mode au premier plan garde son délai : 30 s par défaut, 120 s au plus. Le mode arrière-plan n’a pas ce plafond de temps. La carte montre En cours, puis Terminée et le code, y compris un code différent de zéro. La sortie se lit sur la carte. Le plafond de taille de sortie déjà en vigueur reste. S’il est atteint, la carte le dit.
2. **Arrêter tue le processus.** Le bouton de la carte envoie l’arrêt tout de suite, sans seconde approbation et sans « Toujours ». L’état passe à Arrêtée. Une autre tâche du fil n’est pas touchée. Le code affiché n’est pas un 0 inventé.
3. **Quitter l’app arrête la tâche.** Le processus ne survit pas à la fermeture. Au retour, la carte dit Arrêtée. Elle ne dit pas En cours. Un arrêt brutal du Mac a le même affichage au rallumage : la tâche n’est pas relancée, et elle n’est pas laissée marquée En cours.
4. **L’entrée standard est une ligne, pas un secret.** Écrire dans une tâche En cours envoie cette ligne. Elle apparaît dans la sortie. Une tâche Terminée ou Arrêtée refuse la saisie en français. Une invite qui demande un mot de passe, ou un champ sécurisé au sens de SCR-2, ne reçoit rien : la phrase le dit, et les caractères ne partent pas.
5. **Le cloud reste reporté.** #63, #53 et `docs/specs/routines.md` fixent « Mac seulement, cloud plus tard ». Cette spec ne prépare pas de VM, pas d’exécuteur distant, pas de chat Mac éteint. Le détail est au hors-périmètre, pour qu’on ne rouvre pas la décision en croyant qu’elle manque.

## Récits

1. En tant que Charles, je veux lancer « sleep 90 && echo fini » en arrière-plan et continuer à écrire dans le fil, afin de ne pas attendre les yeux sur un tour bloqué.
2. En tant que Charles, je veux voir la carte passer de En cours à Terminée, avec le code 0, afin de savoir que la commande a fini.
3. En tant que Charles, je veux lire « fini » dans la sortie de la carte, afin de vérifier le résultat sans ouvrir un terminal.
4. En tant que Charles, je veux un code de sortie autre que 0 affiché tel quel, afin de distinguer une fin normale d’un échec du programme.
5. En tant que Charles, je veux Arrêter, afin que le processus meure et que la carte dise Arrêtée.
6. En tant que Charles, je veux que Arrêter sur une carte laisse l’autre tâche tranquille, afin de couper la bonne commande.
7. En tant que Charles, je veux quitter Ezerbot et retrouver la tâche Arrêtée, le processus absent, afin de ne pas croire qu’elle tourne encore.
8. En tant que Charles, je veux la même approbation qu’aujourd’hui avant le départ, afin qu’une commande longue ne parte pas toute seule.
9. En tant que Charles, je veux qu’une commande au premier plan s’arrête encore à son délai, afin de garder le shell court.
10. En tant que Charles, je veux envoyer « y » à une invite et le voir dans la sortie, afin de répondre au programme.
11. En tant que Charles, je veux qu’une invite de mot de passe ne reçoive pas ce que je tape, afin de ne pas donner un secret au shell.
12. En tant que Charles, je veux qu’une saisie sur une tâche déjà finie soit refusée en français, afin de voir que rien n’est parti.
13. En tant que Charles, je veux que la carte dise quand la sortie a été coupée par le plafond déjà en vigueur, afin de ne pas prendre un extrait pour le texte complet.
14. En tant que Charles, je veux que le contrôle d’écran déjà livré, y compris le refus des champs mot de passe, continue, afin que ce lot ne le remplace pas.

## Décisions d’implémentation

- Le mode est un argument de `run_shell`. Les commandes déjà émises sans ce mode restent au premier plan, avec les bornes 100 ms–120 s. Le texte d’outil qui interdit une tâche détachée vise le contournement (`&`, `nohup`) : le mode arrière-plan est le seul suivi.
- La carte appartient au fil. Elle survit aux messages suivants. Deux tâches ont deux cartes. Arrêter vise le groupe de processus de cette carte, enfants compris, pour ne pas laisser un orphelin.
- La fermeture propre de l’app et l’arrêt du moteur passent les tâches En cours à Arrêtée et tuent leur groupe. Au redémarrage, aucune de ces cartes n’est En cours, et aucune n’est relancée.
- La saisie est une ligne écrite sur l’entrée standard de la tâche En cours. Elle est aussi recopiée dans la sortie visible, pour que le juge voie ce qui a été envoyé. Le plafond de sortie s’applique à cet écho comme au reste.
- Le refus de mot de passe réutilise la classe déjà utilisée par le contrôle d’écran : champ sécurisé, ou invite qui demande un mot de passe. La ligne n’est pas écrite. Elle n’est pas ajoutée à la sortie.

## Décisions de test

Un bon test observe la carte, l’état, le code, la sortie, la possibilité d’envoyer un autre message pendant la tâche, le processus encore vivant ou déjà mort, et le refus d’une invite secrète. Il ne vérifie pas l’ordre interne des appels.

La couture est celle des tests de shell déjà joués. L’horloge n’est pas un substitut au processus : le test lance une commande courte scriptée qui dort assez pour prouver que le tour du bot n’est pas bloqué, puis qui écrit une ligne. Un second test tue par Arrêter et vérifie que le processus n’existe plus. Un troisième écrit sur l’entrée standard et refuse l’invite de mot de passe.

La tranche commence par un test rouge, puis le code qui le rend vert. Le juge visuel sur le Mac est le critère d’acceptation. Une capture immobile ne suffit pas : il faut voir En cours, puis Terminée, et constater qu’Arrêter a bien tué.

## Critères d’acceptation

1. **Commande longue.** Avec le shell autorisé pour le bot, demander : « Lance sleep 90 && echo fini en arrière-plan, et réponds-moi tout de suite bonjour. » L’approbation du shell s’affiche encore. Après accord, le bot écrit bonjour avant la fin des 90 s. La carte est En cours, puis Terminée, code 0. La sortie contient fini.
2. **Arrêt et fermeture.** Relancer une commande qui dort deux minutes. Arrêter : l’état devient Arrêtée, et le processus n’est plus là. Le code affiché n’est pas 0. Relancer, quitter Ezerbot pendant En cours, rouvrir : la carte est Arrêtée, le processus est absent, la commande n’a pas redémarré. Une seconde tâche lancée à côté n’a pas été tuée par l’arrêt de la première.
3. **Saisie.** Un script qui lit une ligne et l’écrit. Envoyer « oui » depuis la carte. La sortie montre oui. Envoyer encore après Terminée : phrase française, la sortie ne gagne pas cette seconde ligne.
4. **Mot de passe.** Un script qui demande un mot de passe. La saisie proposée n’est pas écrite dans le programme, et la carte le dit. Le contrôle d’écran d’un champ mot de passe reste refusé comme avant cette tranche. Une commande au premier plan de 30 s s’arrête encore à son délai.

## Hors-périmètre

- **T10, ordinateur cloud, Mac éteint (SCR-4, IPH-3).** Reporté. #63 et la spec des routines disent que le cloud viendra plus tard, et #53 réserve l’identité de créneau à un exécuteur qui n’existe pas. Chez Grok Bot, Mac éteint, le chat, les routines, les réunions, les connecteurs et les images continuent parce que le bot vit sur l’ordinateur du compte. Ezerbot ne le copie pas dans ce lot. Pas de VM par compte, pas d’exécuteur distant, pas de ticket `needs-decision`. Les outils du téléphone restent hors périmètre : la liste n’est pas dans l’app, et le drapeau est désactivé.
- Réécrire SCR-1 ou SCR-2. La session d’écran, la pause humaine et le refus des champs mot de passe à l’écran restent.
- Un shell sans approbation, un « Toujours » nouveau, ou une tâche qui survivrait à la fermeture de l’app.
- Relancer au réveil une tâche tuée par la fermeture.
- Lever le plafond de taille de sortie.
- Notifications push quand une tâche finit. La carte du fil suffit.
- Routines, connecteurs, images. Une routine a déjà son historique. Elle n’est pas une tâche de fond de ce lot.

## Risques

- **Orphelin.** Un `&` dans la commande, ou un enfant du processus, peut survivre à Arrêter si seul le parent est tué. La décision d’implémentation vise le groupe. Le juge du parcours 2 vérifie l’absence du processus.
- **Secret sur l’entrée standard.** La décision 4 refuse l’invite de mot de passe. Une invite qui ne se déclare pas comme telle peut encore recevoir une ligne. C’est la même limite que le contrôle d’écran : on refuse la classe connue, on ne devine pas tous les secrets.
- **Longue sortie.** Le plafond actuel évite une carte sans fin. La carte doit dire que la sortie est coupée, sinon Charles lit un extrait comme s’il était complet.
- **Fermeture brutale.** Le processus meurt avec le Mac. Au retour, afficher En cours serait faux. Le parcours 2 couvre la fermeture propre. L’arrêt brutal est la même règle d’affichage : pas En cours, pas de relance.
- **Le cloud rouvert par accident.** Un mode arrière-plan n’est pas un ordinateur distant. Rien dans cette spec ne fait tourner une commande quand le Mac est éteint.

## Tranches

Labels : `wayfinder:task`, `enhancement`, `ready-for-agent`. La première à prendre est T8. T10 n’est pas une tranche.

Carte à créer : « Tâches de fond ».

1. T8. Lancer une commande en tâche de fond. Aucun bloquant.
2. T9. Envoyer une saisie à une tâche de fond. Bloquée par T8.

## Annexe — tickets prêts à créer

Ces corps sont prêts à coller. Cette PR ne crée aucune issue. Le parent est la carte « Tâches de fond » une fois créée. Spec : `docs/specs/taches-de-fond.md`.

### T8 — Lancer une commande en tâche de fond

## Parent

Carte « Tâches de fond ». Spec : `docs/specs/taches-de-fond.md`, décisions 1 à 3 et parcours juge 1 et 2.

## What to build

`run_shell` accepte un mode arrière-plan, sans le plafond de 30 s ni celui de 120 s. L’approbation existante reste exigée. La tâche apparaît dans le fil : En cours, sortie consultable, bouton Arrêter, puis Terminée et le code de sortie. Charles peut envoyer un autre message pendant que la tâche tourne.

Arrêter tue le groupe de processus de cette carte et passe l’état à Arrêtée, sans inventer un code 0, sans seconde carte. Quitter l’app fait de même. Au retour, la carte n’est pas En cours et la commande n’a pas redémarré. Le mode au premier plan et son délai ne changent pas. Le plafond de taille de sortie reste, et la carte le signale s’il est atteint.

Démo : `sleep 90 && echo fini`, le bot répond pendant l’attente, puis Terminée, code 0.

TDD : d’abord un test rouge (un message du bot part avant la fin des 90 s, état Terminée et code 0, Arrêter tue le processus, fermeture de l’app marque Arrêtée).

## Acceptance criteria

- [ ] TDD : le test du comportement visible est rouge avant le changement et vert après.
- [ ] `sleep 90 && echo fini` laisse le bot répondre pendant l’attente. La carte finit Terminée, code 0, sortie « fini ».
- [ ] L’approbation du shell est encore demandée avant le départ.
- [ ] Arrêter tue le processus et affiche Arrêtée, sans code 0 inventé. Une autre tâche du fil continue.
- [ ] Quitter l’app, puis rouvrir : la tâche est Arrêtée, le processus est absent, elle ne redémarre pas.
- [ ] Une commande au premier plan respecte encore 30 s par défaut et 120 s au plus.
- [ ] Juge visuel sur le Mac : parcours 1 et 2 de la spec.

## Blocked by

- None (can start immediately)

### T9 — Envoyer une saisie à une tâche de fond

## Parent

Carte « Tâches de fond ». Spec : `docs/specs/taches-de-fond.md`, décision 4 et parcours juge 3 et 4.

## What to build

Une tâche En cours accepte une ligne sur son entrée standard. La ligne apparaît dans la sortie. Une tâche Terminée ou Arrêtée refuse en français, et la ligne n’est pas écrite. Une invite de mot de passe, ou un champ sécurisé au sens du contrôle d’écran, ne reçoit rien. Le refus des champs mot de passe à l’écran (SCR-2) reste celui d’aujourd’hui.

Démo : un script qui lit une ligne, la saisie « oui » visible dans la sortie.

TDD : d’abord un test rouge (la ligne écrite est celle lue par le script, zéro octet sur une invite de mot de passe, refus après Terminée).

## Acceptance criteria

- [ ] TDD : le test est rouge avant et vert après.
- [ ] La saisie envoyée à une tâche En cours apparaît dans sa sortie et a bien été lue par le processus.
- [ ] Après Terminée ou Arrêtée, la saisie est refusée en français et la sortie ne change pas.
- [ ] Une invite de mot de passe ne reçoit pas la ligne. Le champ mot de passe du contrôle d’écran est encore refusé.
- [ ] Juge visuel sur le Mac : parcours 3 et 4 de la spec.

## Blocked by

- T8 — Lancer une commande en tâche de fond
