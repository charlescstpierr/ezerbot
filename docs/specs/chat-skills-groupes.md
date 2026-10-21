# Chat, skills, groupes et secrets

Statut : décisions du 21 octobre 2026, copie du comportement visible de Grok Bot 0.72.1 recalé sur le Mac de Charles. Aucune implémentation de ce lot n’est commencée. La première tranche est la réaction et le renvoi d’un envoi échoué. Les tranches sont à créer comme issues enfants d’une carte `wayfinder:map` « Chat, skills, groupes et secrets ». Cette page ne crée pas les issues. L’annexe en donne le corps.

Vocabulaire déjà en usage : fil, message, bulle, bot, groupe, skill, formulaire, coffre, trousseau. Une réaction est un emoji posé sur un message. Un sous-fil est la réponse attachée à un message. Une notation est le jugement local d’une réponse. Un secret de bot est une valeur nommée, jamais réaffichée.

## Contexte

Le menu d’un message sait copier. ⌘K place le curseur dans la recherche globale des conversations. Il n’y a pas de réaction, pas de « Répondre en fil », pas de Renvoyer ni de Supprimer sur un envoi qui n’est pas parti, pas de ⌘F dans le fil, pas de « Bon résultat ».

Grok Bot, lui, pose une réaction (`ReactToGrokBotMessageRequest`), ouvre un fil (`CreateGrokBotThreadFromMessageRequest`), cherche avec ⌘F (`sand.findInChat`), et note via un vote. Renvoyer et Supprimer n’existent que sur un envoi échoué. Supprimer ne demande pas de confirmation. Un message déjà livré ne se supprime pas et ne s’édite pas. La notation propose « Good result » et « Bad result », puis « How can we improve? ». Chez Grok ce vote part au serveur. Ezerbot n’a pas ce serveur.

L’import d’une skill lit le `SKILL.md` et garde un historique de versions. Le champ des fichiers compagnons existe, et l’import le laisse vide : un `modele.md` posé à côté n’est pas lu quand la skill s’applique. ⌘⇧W ne fait rien. Chez Grok, ⌘⇧W ouvre le panneau Skills (`sand.openSkills`), y compris depuis le champ de saisie. La publication d’une skill à une équipe n’a pas de place ici : il n’y a pas d’équipe.

Un groupe réunit de deux à six bots. On choisit les membres. On ne nomme pas le groupe, on ne lui donne ni description ni avatar. La barre dit « Groupe · » suivi des noms. Il n’y a pas de bandeau de coût. Grok Bot ajoute des humains à la création, au plus dix-neuf, et au plus trois bots dans ce cas. Ezerbot est mono-utilisateur. Ce choix est déjà tranché : pas d’humains. Le bandeau, lui, se copie.

Un secret demandé par le bot va au coffre, au trousseau sur l’app signée, et la valeur ne revient pas dans le fil. Les réglages montrent où les secrets sont stockés. Ils ne montrent pas, par bot, une liste nom, description, dates, sans la valeur. Grok Bot a ce panneau (« No secrets yet », ajouter, supprimer). Il n’a pas d’expiration côté client. Le seul refus de ce genre est un formulaire périmé. Les approbations d’outils ont déjà une échéance. Les secrets n’en gagnent pas une.

## Énoncé du problème

Charles ne peut pas réagir, répondre en fil, renvoyer un message perdu, chercher dans le fil, ni noter une réponse. Une skill importée ignore ses annexes, et le raccourci du panneau manque. Un groupe n’a pas de nom, et rien ne prévient qu’il coûte plus cher. Les secrets d’un bot ne se gèrent pas dans un panneau : on ne les voit pas, on ne les retire pas sans passer par une demande du bot.

## Solution

Le menu du message gagne la réaction, le sous-fil, et, seulement si l’envoi a échoué, Renvoyer et Supprimer. ⌘F cherche dans le fil. La notation est locale et exportable. Un dossier de skill emporte ses annexes, le bot les lit quand la skill s’applique, et ⌘⇧W ouvre le panneau. Un groupe a un nom, une description, un avatar, et le bandeau de coût. Chaque bot a un panneau Secrets.

Les humains dans un groupe, la publication de skill à une équipe, l’expiration d’un secret et l’envoi de la notation à un serveur restent dehors.

## Décisions

Grill du 21 octobre 2026. R5, R6, R9, R10 et R11 sont copiés. Aucune n’est rouverte.

1. **La réaction est une bascule.** Le sélecteur pose un emoji sur le message et le retire si on le rechoisit. L’affichage est immédiat. La réaction est encore là après un redémarrage. Copier reste dans le menu.
2. **Renvoyer et Supprimer ne visent qu’un envoi qui n’est pas parti.** L’état affiché est « Échec de l’envoi » ou « Pas encore livré ». Renvoyer rejoue le même message, pièces jointes comprises, une fois. Supprimer le retire, sans confirmation. Un message livré n’a ni Renvoyer, ni Supprimer, ni édition.
3. **⌘F cherche dans le fil ouvert.** Le champ, le nombre de résultats, suivant, précédent et Fermer. ⌘K continue d’ouvrir la recherche globale des conversations. Les deux raccourcis ne se remplacent pas.
4. **La notation est locale.** « Bon résultat » enregistre. « Mauvais résultat » ouvre « Comment s’améliorer ? », avec « Pas ce que j’ai demandé », « Inexact », « Qualité faible », « Trop lent », un champ « Détails » et « Envoyer ». Fermer sans envoyer annule. Le stockage est sur ce Mac. On peut l’exporter. Rien ne part vers un serveur.
5. **Répondre en fil ouvre un sous-fil.** Il est lié au message. Le bot reçoit ce message parent comme contexte. Le fil principal reste lisible. L’entrée de menu arrive avec le menu de la tranche réaction.
6. **Une skill emporte ses annexes.** Importer un dossier prend le `SKILL.md` et les fichiers à côté. Quand la skill s’applique, le bot lit l’annexe. Le panneau montre une version précédente et la restaure, annexes comprises. ⌘⇧W ouvre Skills, y compris quand le curseur est dans le champ de saisie. La publication à une équipe n’existe pas.
7. **Un groupe a un nom, et le bandeau se copie.** Nom et description à la création et au renommage, plus un avatar. Dans tout groupe, un bandeau dit : « Les groupes consomment plus de ton usage, car plusieurs bots traitent chaque message. » et « Pour la plupart des tâches, parle plutôt à un seul bot. » On peut le fermer. Il reste fermé pour la session de ce groupe. Un autre groupe l’affiche encore. Aucune personne ne s’ajoute. La borne de deux à six bots reste.
8. **Les secrets se gèrent par bot, sans durée de vie.** Le panneau dit « Aucun secret » quand la liste est vide. « Ajouter un secret » demande le nom, la description et la valeur. La liste montre le nom, la description et les dates, jamais la valeur. Supprimer retire le secret. La valeur va au trousseau. Un formulaire périmé, parce que le bot a redémarré ou que la demande a été remplacée, est refusé : « Ce formulaire n’a pas été accepté. » Laisser la demande ouverte trente minutes ne la périme pas. L’écran du compte ChatGPT et le bloc « Stockage des secrets » restent distincts.

## Récits

1. En tant que Charles, je veux poser un emoji sur un message et le voir tout de suite, afin de réagir sans écrire.
2. En tant que Charles, je veux rechoisir cet emoji pour le retirer, afin d’annuler la réaction.
3. En tant que Charles, je veux retrouver la réaction après un redémarrage, afin qu’elle ne soit pas un effet d’affichage.
4. En tant que Charles, je veux, sur un envoi qui n’est pas parti, lire « Échec de l’envoi » ou « Pas encore livré », afin de savoir qu’il manque.
5. En tant que Charles, je veux Renvoyer et obtenir le même message une fois, pièces jointes comprises, afin de ne pas le retaper.
6. En tant que Charles, je veux Supprimer cet envoi échoué sans confirmation, afin de nettoyer le fil d’un geste.
7. En tant que Charles, je veux qu’un message déjà livré n’offre ni Renvoyer, ni Supprimer, ni édition, afin de ne pas réécrire l’historique.
8. En tant que Charles, je veux que Copier reste disponible, afin de garder le geste déjà livré.
9. En tant que Charles, je veux que ⌘F ouvre « Chercher dans le fil », afin de trouver une phrase de cette conversation.
10. En tant que Charles, je veux le champ, le nombre de résultats, suivant, précédent et Fermer, afin de circuler dans les trouvailles.
11. En tant que Charles, je veux que ⌘K ouvre encore la recherche globale des conversations, afin de ne pas perdre ce raccourci.
12. En tant que Charles, je veux marquer « Bon résultat », afin de garder une trace de ce qui a marché.
13. En tant que Charles, je veux que « Mauvais résultat » ouvre « Comment s’améliorer ? », avec les quatre motifs, « Détails » et « Envoyer », afin de dire pourquoi.
14. En tant que Charles, je veux que Fermer sans Envoyer n’enregistre rien, afin d’abandonner la notation.
15. En tant que Charles, je veux exporter les notations de ce Mac, afin de les relire hors de l’app.
16. En tant que Charles, je veux que la notation ne parte pas vers un serveur, afin qu’elle reste un fichier local.
17. En tant que Charles, je veux « Répondre en fil » et obtenir un sous-fil lié au message, afin de ne pas mélanger la suite avec le fil principal.
18. En tant que Charles, je veux que le bot reçoive le message parent, afin qu’il réponde à cette phrase-là.
19. En tant que Charles, je veux que le fil principal reste lisible pendant que le sous-fil est ouvert, afin de garder le contexte.
20. En tant que Charles, je veux importer un dossier `SKILL.md` plus `modele.md`, afin que l’annexe fasse partie de la skill.
21. En tant que Charles, je veux que la skill appliquée utilise le texte de l’annexe, afin que le modèle ne soit pas ignoré.
22. En tant que Charles, je veux voir une version précédente et la restaurer, afin de revenir en arrière.
23. En tant que Charles, je veux que ⌘⇧W ouvre Skills même depuis le champ de saisie, afin d’y aller sans la souris.
24. En tant que Charles, je veux cocher les quatre points du scénario d’import, afin de voir l’écart se fermer.
25. En tant que Charles, je veux nommer et décrire un groupe à la création, afin de le reconnaître.
26. En tant que Charles, je veux le renommer et changer sa description plus tard, afin de corriger le titre.
27. En tant que Charles, je veux lui donner un avatar, afin de le distinguer dans la liste.
28. En tant que Charles, je veux lire le bandeau sur le coût des groupes, afin de savoir pourquoi un groupe consomme plus.
29. En tant que Charles, je veux fermer ce bandeau pour la session de ce groupe, afin qu’il ne revienne pas à chaque message.
30. En tant que Charles, je veux que le bandeau d’un autre groupe reste visible, afin que la fermeture ne soit pas globale.
31. En tant que Charles, je veux qu’aucune personne ne puisse être ajoutée au groupe, afin de rester dans un usage mono-utilisateur.
32. En tant que Charles, je veux ouvrir Secrets sur un bot vide et lire « Aucun secret », afin de voir que le panneau est le bon.
33. En tant que Charles, je veux ajouter un secret avec un nom, une description et une valeur, afin de le ranger moi-même.
34. En tant que Charles, je veux une liste qui montre le nom, la description et les dates, jamais la valeur, afin de ne pas la révéler.
35. En tant que Charles, je veux que la valeur aille au trousseau et reste absente du fil, afin qu’un export ne l’emporte pas.
36. En tant que Charles, je veux Supprimer un secret, afin qu’il ne soit plus utilisable.
37. En tant que Charles, je veux qu’un formulaire de secret périmé réponde « Ce formulaire n’a pas été accepté. », afin de ne pas enregistrer une vieille demande.
38. En tant que Charles, je veux qu’une demande laissée trente minutes reste acceptable, afin de ne pas inventer une expiration que Grok Bot n’a pas.
39. En tant que Charles, je veux que les secrets d’un autre bot, et le compte ChatGPT, restent hors de ce panneau, afin de ne pas mélanger les coffres.

## Décisions d’implémentation

- La réaction, le sous-fil, Renvoyer, Supprimer et Copier sont des actions du message. Renvoyer et Supprimer ne sont pas rendus si l’envoi est livré. Renvoyer réutilise le corps et les pièces déjà tentés. Un second Renvoyer après succès n’envoie pas une copie de plus.
- ⌘F ne change pas l’écoute de ⌘K. La recherche du fil surligne l’occurrence courante et affiche le compte.
- La notation est une table locale du Mac, exportable en un fichier que Charles déclenche. Elle n’entre pas dans le contexte du modèle, sauf le jour où une autre spec le demanderait. Ce lot ne le demande pas.
- L’import de dossier remplit les fichiers compagnons déjà prévus sur la skill, au lieu de les laisser vides. Appliquer la skill place le texte de l’annexe avec le corps. Une version restaurée ramène le corps et les annexes de cette version. Les paquets d’import de 8 Mio déjà annoncés restent la borne d’un envoi.
- Le groupe enregistre nom, description et avatar à côté des membres. La borne de deux à six bots ne change pas. Le bandeau fermé est un état de la session de ce groupe, pas un réglage de compte.
- Le secret de bot est une entrée du coffre déjà utilisé, étiquetée par le bot. Le panneau ne lit pas les identifiants ChatGPT, ni les variables des connecteurs. La valeur n’est pas dans la réponse du panneau. Le formulaire de `request_secret` déjà livré continue d’écrire dans ce coffre. S’il est périmé, il échoue avec la phrase de la décision 8, et il n’écrit rien.

## Décisions de test

Un bon test observe le menu, l’emoji encore là après redémarrage, le nombre d’envois, l’absence de Renvoyer sur un message livré, le texte trouvé par ⌘F, la notation exportée et absente du réseau, le sous-fil, le texte de l’annexe utilisé dans la réponse, le panneau Skills ouvert au raccourci, le bandeau, et la liste de secrets sans valeur. Il ne vérifie pas l’ordre interne des fonctions.

La couture du chat est celle des tests de fil déjà joués. Une coupure scriptée produit l’état d’échec. Le test compte un envoi après Renvoyer, zéro après Supprimer, zéro Renvoyer sur un message livré. La couture skill importe un dossier de fixture. La couture groupe lit le bandeau. La couture secret lit le panneau et le coffre, et vérifie l’absence de la valeur dans la liste, le fil et l’export.

Chaque tranche commence par un test rouge, puis le code qui le rend vert. Le juge visuel sur le Mac est le critère d’acceptation. Une capture immobile ne suffit pas.

## Critères d’acceptation

1. **Réaction.** Sur un message livré, poser un emoji. Il apparaît sans recharger. Le rechoisir le retire. Quitter et rouvrir : l’emoji posé une fois est encore là. Copier copie encore le texte.
2. **Envoi échoué.** Couper le réseau, envoyer une phrase et une image. L’état est « Échec de l’envoi » ou « Pas encore livré ». Renvoyer, réseau revenu : le fil montre ce message une fois, avec l’image. Supprimer sur un autre échec le retire, sans dialogue de confirmation. Un message déjà livré n’a ni Renvoyer ni Supprimer, et son texte ne s’édite pas.
3. **Recherche.** Dans un fil qui contient un mot rare, ⌘F ouvre « Chercher dans le fil », annonce le nombre, et suivant / précédent circulent. Fermer rend le fil. ⌘K ouvre encore la recherche globale des conversations, pas ce champ.
4. **Notation.** « Bon résultat » enregistre. « Mauvais résultat » ouvre « Comment s’améliorer ? » avec les quatre motifs, « Détails » et « Envoyer ». Fermer sans Envoyer n’ajoute rien. L’export contient la notation envoyée. Aucune requête vers un serveur de notation n’est partie.
5. **Sous-fil.** « Répondre en fil » ouvre un sous-fil lié au message. Une question dans ce sous-fil reçoit une réponse qui tient compte du message parent. Le fil principal montre toujours ce message.
6. **Skill.** Importer un dossier avec `SKILL.md` et `modele.md`, le modèle contenant une consigne reconnaissable. Lancer la skill : la réponse suit cette consigne. Le panneau montre une version précédente et la restaurer ramène l’ancien texte. Depuis le champ de saisie, ⌘⇧W ouvre Skills. Les quatre points se comptent.
7. **Groupe.** Créer un groupe avec un nom, une description et un avatar. La liste les montre. Les renommer met à jour la liste. Le bandeau affiche les deux phrases. Le fermer : il ne revient pas dans ce groupe pendant la session. Ouvrir un autre groupe : le bandeau y est. Le dialogue des membres ne propose pas d’ajouter une personne. Deux à six bots restent la borne.
8. **Secrets.** Le panneau du bot A dit « Aucun secret ». Ajouter un secret. La liste montre le nom, la description et les dates, pas la valeur. La valeur est au trousseau. Elle n’est pas dans le fil. Supprimer la retire. Rejouer un formulaire de secret après un redémarrage du bot : « Ce formulaire n’a pas été accepté. », et la liste n’a pas gagné cette valeur. Une demande laissée ouverte bien au-delà de trente minutes s’accepte encore. Le bot B ne liste pas le secret de A. L’écran du compte ChatGPT est encore le sien.

## Hors-périmètre

- Supprimer ou éditer un message déjà livré.
- Envoyer la notation à un serveur, et l’injecter dans le prompt.
- Publier une skill à une équipe, ou la partager.
- Ajouter des humains à un groupe, changer la borne de deux à six bots, ou le fil `task:` en lecture seule déjà livré.
- Une durée de vie client sur un secret, et un affichage de la valeur.
- Mélanger dans ce panneau les identifiants ChatGPT ou les variables privées des connecteurs MCP.
- Les connecteurs Gmail et Slack. Ils sont dans `docs/specs/connecteurs-google-slack.md`, enfants de la carte [#44](https://github.com/charlescstpierr/ezerbot/issues/44).
- L’ordinateur cloud.

## Risques

- **Renvoyer deux fois.** Un second clic après succès doublerait le message. Le test compte un envoi. Les pièces doivent être celles de la tentative, pas une sélection nouvelle.
- **Supprimer trop large.** Le bouton n’est rendu que sur l’échec. Un message livré qui l’afficherait casserait la décision 2. Le test le vérifie.
- **⌘F et ⌘K.** Les deux doivent rester distincts. Le parcours 3 coche les deux.
- **Annexe absente du prompt.** Importer le fichier sans le donner au bot quand la skill s’applique laisserait le scénario à moitié fait. Le test exige que la consigne de `modele.md` se voie dans la réponse.
- **Bandeau fermé pour tout le monde.** L’état est par groupe et par session. Le parcours 7 ouvre un second groupe.
- **Valeur de secret dans la liste.** La réponse du panneau ne contient pas la valeur. Le test compare la liste, le fil et l’export. Le trousseau verrouillé suit la règle déjà prise : pas de repli fichier pour un secret de compte quand le trousseau est actif.
- **Formulaire périmé accepté.** Le test de redémarrage écrit zéro secret. L’absence de TTL à trente minutes est l’autre moitié : une demande seulement vieille doit encore passer, sinon on invente l’expiration écartée.

## Tranches

Labels : `wayfinder:task`, `enhancement`, `ready-for-agent`. La première à prendre est T17.

Carte à créer : « Chat, skills, groupes et secrets ».

1. T17. Réagir, renvoyer ou supprimer un envoi échoué. Aucun bloquant.
2. T18. Chercher dans le fil et noter un résultat. Aucun bloquant.
3. T19. Répondre en fil. Bloquée par T17.
4. T20. Fichiers annexes de skill et raccourci. Aucun bloquant.
5. T21. Options de groupe. Aucun bloquant.
6. T22. Panneau Secrets par bot. Aucun bloquant.

## Annexe — tickets prêts à créer

Ces corps sont prêts à coller. Cette PR ne crée aucune issue. Le parent est la carte « Chat, skills, groupes et secrets » une fois créée. Spec : `docs/specs/chat-skills-groupes.md`.

### T17 — Réagir à un message et renvoyer ou supprimer un envoi échoué

## Parent

Carte « Chat, skills, groupes et secrets ». Spec : `docs/specs/chat-skills-groupes.md`, décisions 1 et 2, parcours juge 1 et 2.

## What to build

Le menu d’un message permet une réaction emoji. La rechoisir la retire. L’affichage est immédiat et survit au redémarrage. Copier reste.

« Échec de l’envoi » ou « Pas encore livré » affiche Renvoyer et Supprimer. Renvoyer rejoue le même message, pièces jointes comprises, une fois. Supprimer le retire sans confirmation. Un message livré n’a ni Renvoyer, ni Supprimer, ni édition.

Démo : une réaction, un envoi coupé du réseau, Renvoyer, Supprimer.

TDD : d’abord un test rouge (réaction persistante, un envoi après Renvoyer, zéro après Supprimer, message livré sans ces deux actions).

## Acceptance criteria

- [ ] TDD : le test est rouge avant et vert après.
- [ ] La réaction apparaît tout de suite, se retire en la rechoisissant, et reste après redémarrage.
- [ ] Un envoi échoué propose Renvoyer et Supprimer. Renvoyer livre une fois, pièces comprises. Supprimer retire sans confirmation.
- [ ] Un message livré n’a ni Renvoyer, ni Supprimer, ni édition. Copier fonctionne encore.
- [ ] Juge visuel sur le Mac : parcours 1 et 2 de la spec.

## Blocked by

- None (can start immediately)

### T18 — Chercher dans le fil et noter un résultat

## Parent

Carte « Chat, skills, groupes et secrets ». Spec : `docs/specs/chat-skills-groupes.md`, décisions 3 et 4, parcours juge 3 et 4.

## What to build

⌘F ouvre « Chercher dans le fil » : champ, nombre de résultats, suivant, précédent, Fermer. ⌘K reste la recherche globale des conversations.

« Bon résultat » enregistre. « Mauvais résultat » ouvre « Comment s’améliorer ? » avec « Pas ce que j’ai demandé », « Inexact », « Qualité faible », « Trop lent », « Détails » et « Envoyer ». Fermer sans Envoyer annule. Le stockage est local, exportable, et aucune requête de notation ne part.

Démo : trouver un mot rare, noter une mauvaise réponse, exporter.

TDD : d’abord un test rouge (le mot est trouvé, Fermer n’enregistre pas, l’export contient l’envoi, zéro requête de notation).

## Acceptance criteria

- [ ] TDD : le test est rouge avant et vert après.
- [ ] ⌘F cherche dans le fil ouvert et annonce le nombre. Suivant et précédent circulent.
- [ ] ⌘K ouvre encore la recherche globale.
- [ ] Les libellés de notation sont ceux de la décision 4. Fermer sans Envoyer n’ajoute rien.
- [ ] L’export contient la notation. Aucune requête vers un serveur de notation n’est partie.
- [ ] Juge visuel : parcours 3 et 4 de la spec.

## Blocked by

- None (can start immediately)

### T19 — Répondre en fil à un message

## Parent

Carte « Chat, skills, groupes et secrets ». Spec : `docs/specs/chat-skills-groupes.md`, décision 5 et parcours juge 5.

## What to build

« Répondre en fil », dans le menu du message, ouvre un sous-fil lié à ce message. Le bot reçoit le message parent comme contexte de sa réponse. Le fil principal reste lisible.

Démo : répondre en fil à une phrase précise, la réponse en tient compte.

TDD : d’abord un test rouge (le sous-fil cite le parent, le fil principal contient encore le message).

## Acceptance criteria

- [ ] TDD : le test est rouge avant et vert après.
- [ ] « Répondre en fil » ouvre un sous-fil lié au message choisi.
- [ ] La réponse du bot tient compte du message parent.
- [ ] Le fil principal montre toujours ce message.
- [ ] Juge visuel : parcours 5 de la spec.

## Blocked by

- T17 — Réagir à un message et renvoyer ou supprimer un envoi échoué

### T20 — Fichiers annexes de skill et raccourci

## Parent

Carte « Chat, skills, groupes et secrets ». Spec : `docs/specs/chat-skills-groupes.md`, décision 6 et parcours juge 6.

## What to build

Importer un dossier enregistre le `SKILL.md` et les fichiers annexes. Quand la skill s’applique, le bot lit l’annexe. Le panneau montre une version précédente et la restaurer ramène le corps et les annexes de cette version. ⌘⇧W ouvre le panneau Skills, y compris depuis le champ de saisie. Pas de publication à une équipe.

Le scénario SKL-3 se compte n/4 : import, annexe utilisable, version précédente restaurable, raccourci.

Démo : `modele.md` réellement suivi par la réponse, puis ⌘⇧W depuis le champ.

TDD : d’abord un test rouge (l’annexe est dans le contexte quand la skill s’applique, la restauration ramène l’ancienne annexe, le raccourci ouvre le panneau).

## Acceptance criteria

- [ ] TDD : le test est rouge avant et vert après.
- [ ] L’import d’un dossier avec `SKILL.md` et `modele.md` garde l’annexe.
- [ ] La réponse qui applique la skill suit la consigne de `modele.md`.
- [ ] Une version précédente est visible et restaurable, annexes comprises.
- [ ] ⌘⇧W ouvre Skills depuis le champ de saisie.
- [ ] Juge visuel : parcours 6 de la spec, les quatre points cochés.

## Blocked by

- None (can start immediately)

### T21 — Options de groupe

## Parent

Carte « Chat, skills, groupes et secrets ». Spec : `docs/specs/chat-skills-groupes.md`, décision 7 et parcours juge 7.

## What to build

Créer un groupe demande un nom et une description, et permet un avatar. Le renommage met à jour les trois. Le bandeau de chaque groupe affiche « Les groupes consomment plus de ton usage, car plusieurs bots traitent chaque message. » et « Pour la plupart des tâches, parle plutôt à un seul bot. » Le fermer le masque pour la session de ce groupe seulement. Aucune personne ne s’ajoute. Deux à six bots restent la borne.

Démo : un groupe nommé, le bandeau, la fermeture, un second groupe qui l’affiche encore.

TDD : d’abord un test rouge (nom affiché, bandeau, fermeture locale au groupe, aucun champ humain).

## Acceptance criteria

- [ ] TDD : le test est rouge avant et vert après.
- [ ] Le nom, la description et l’avatar s’enregistrent à la création et au renommage.
- [ ] Le bandeau montre les deux phrases. Fermé, il ne revient pas dans ce groupe pendant la session. Un autre groupe l’affiche.
- [ ] Le dialogue ne propose pas d’ajouter une personne. La borne de deux à six bots tient toujours.
- [ ] Juge visuel : parcours 7 de la spec.

## Blocked by

- None (can start immediately)

### T22 — Panneau Secrets par bot

## Parent

Carte « Chat, skills, groupes et secrets ». Spec : `docs/specs/chat-skills-groupes.md`, décision 8 et parcours juge 8.

## What to build

Le bot a un panneau Secrets. Vide, il dit « Aucun secret ». « Ajouter un secret » demande le nom, la description et la valeur. La liste montre le nom, la description et les dates, jamais la valeur. Supprimer retire le secret. La valeur va au trousseau et n’entre pas dans le fil ni l’export. Un formulaire périmé (bot redémarré, ou demande remplacée) répond « Ce formulaire n’a pas été accepté. » et n’écrit rien. Une demande seulement ancienne, y compris au-delà de trente minutes, reste acceptable. Les secrets d’un autre bot ne sont pas listés. L’écran du compte ChatGPT et le bloc « Stockage des secrets » restent distincts.

Démo : ajouter, constater l’absence de la valeur, supprimer, rejouer un formulaire après redémarrage.

TDD : d’abord un test rouge (liste sans valeur, trousseau, phrase du formulaire périmé, demande ancienne encore acceptée, bot B ignorant).

## Acceptance criteria

- [ ] TDD : le test est rouge avant et vert après.
- [ ] Le panneau vide dit « Aucun secret ». Ajouter enregistre nom, description et valeur.
- [ ] La liste montre le nom, la description et les dates, pas la valeur. La valeur est absente du fil et de l’export, présente au trousseau.
- [ ] Supprimer retire le secret.
- [ ] Un formulaire périmé affiche « Ce formulaire n’a pas été accepté. » et n’écrit rien.
- [ ] Une demande laissée ouverte plus de trente minutes s’accepte encore.
- [ ] Le bot B ne voit pas les secrets du bot A. Le compte ChatGPT n’est pas géré dans ce panneau.
- [ ] Juge visuel : parcours 8 de la spec.

## Blocked by

- None (can start immediately)
