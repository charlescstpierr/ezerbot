# Images, documents et bibliothèque

Statut : décisions du 21 octobre 2026, copie du comportement visible de Grok Bot 0.72.1 recalé sur le Mac de Charles. Aucune implémentation de ce lot n’est commencée. La première tranche qui doit marcher de bout en bout, avec le vrai compte, est la génération d’une image. Les tranches sont à créer comme issues enfants d’une carte `wayfinder:map` « Images, documents et bibliothèque ». Cette page ne crée pas les issues. L’annexe en donne le corps.

Vocabulaire déjà en usage : bot, fil, bulle, pièce jointe, document généré, délégation, sous-tâche, fil `task:`, coffre, trousseau. Une Page est le petit artefact de ce lot. La bibliothèque est l’ensemble de fichiers de contexte d’un bot, distinct des pièces jointes d’un fil.

## Contexte

Charles utilise Ezerbot sur son Mac. Joindre une image au fil est déjà là : au plus dix pièces, 20 Mo pour une image, et le bot les voit. Il n’existe pas d’outil pour en produire une. Grok Bot, lui, appelle `GenerateImageArgs` (`description`, `reference_image_paths`, `aspect_ratio`). L’écran dit « Generating a photo ». Aucune carte d’approbation n’est branchée côté client. Les erreurs prévues sont un modèle restreint et un refus de sécurité. La vidéo est derrière un drapeau désactivé. La galerie est une grille dans le message, puis une visionneuse. Il n’y a pas de page galerie, ni de retouche, ni de masque.

Demander à un sous-agent de décrire une photo jointe ne lui transmet pas l’image. Le schéma de Grok Bot prévoit `selected_images` dans le contexte choisi de l’enfant. Ezerbot s’arrête au nom.

Les documents générés existent : pdf, docx, pptx, md, csv, xlsx, svg. Le PDF est une police Courier, encodage WinAnsi. Les accents français passent. Un emoji est refusé par une phrase qui nomme Courier. Il n’y a pas de tableau dans ce PDF. Grok Bot ajoute les Pages : un fichier React vivant à côté du fil, compilé, renommable. Chez Grok ce fichier peut être partagé. Ezerbot n’a pas de serveur pour cela.

Aucun bot n’a de bibliothèque. Un fichier ne compte que s’il est joint au fil. Le scénario de parité DOC-3 dépose le PDF dans les fichiers du bot et attend une citation dans un nouveau fil.

## Énoncé du problème

Charles ne peut pas demander un dessin et le voir apparaître. Plusieurs images d’un message ne s’ouvrent pas en visionneuse. Un sous-agent à qui l’on confie une photo n’en reçoit que le nom.

Un PDF avec un emoji ou un tableau échoue ou reste en Courier. Une analyse qui devrait être un tableau interactif redevient du Markdown dans le fil. Un document utile pour tous les fils d’un bot doit être joint à chaque fois, et un autre bot peut le recevoir par erreur s’il est dans le mauvais fil.

## Solution

Le bot appelle lui-même `generate_image` quand la demande le justifie, avec la clé OpenAI de Charles, modèle gpt-image. L’image arrive dans le fil. Les images d’un message forment une grille, et un clic ouvre la visionneuse. Une photo jointe transmise à un enfant part avec ses octets.

Une analyse autonome devient une Page : fichier `.page.tsx` compilé sur le Mac, affiché à côté du fil. Le PDF embarque une police qui couvre les accents et l’emoji, et il sait faire un tableau. Le même contenu tient en docx et en pptx. Chaque bot a une bibliothèque : les fichiers déposés sont cherchés et cités dans ses nouveaux fils, et dans les siens seulement.

La vidéo, la retouche et le partage public d’une Page restent dehors. Les décisions déjà prises pour le Mac éteint ne bougent pas : pas d’ordinateur cloud dans ce lot.

## Décisions

Grill du 21 octobre 2026. Les réponses R1, R3 et R4 sont copiées. Aucune n’est rouverte.

1. **La génération n’a pas de carte d’approbation.** Le bot appelle `generate_image` quand la phrase le justifie. L’indicateur est « Génération d’une image… ». L’image remplace l’indicateur. Un refus `content_safety_blocked` ou un modèle restreint devient une phrase française, une fois, sans nouvel appel. Sans clé OpenAI, la carte « Connecte ton compte OpenAI » s’affiche et le fournisseur n’est pas appelé. La clé reste au trousseau. Elle ne va pas dans le fil, les journaux, ni le contexte du modèle.
2. **Les références et le cadre sont ceux de Grok Bot.** `reference_image_paths` désigne des images déjà jointes au fil. `aspect_ratio` vaut 1:1, 16:9, 9:16, 4:3 ou 3:4. Un autre cadre est refusé en français, sans appel. Pas de masque, pas de retouche, pas de variation.
3. **La galerie est dans le fil.** Plusieurs images d’un message sont une grille. Un clic ouvre la visionneuse plein écran : flèches, légende, « Enregistrer l’image… », « Copier l’image ». Le clavier circule. Il n’y a pas de page galerie.
4. **L’enfant reçoit l’image.** Quand `delegate` porte sur une image jointe, `selected_images` contient les octets ou le chemin et le type MIME. Le nom seul ne suffit pas. L’enfant décrit cette image. Un fil qui ne l’a pas ne l’envoie pas.
5. **Une Page est locale.** Quand le résultat est une analyse autonome (graphique, tableau interactif, outil réutilisable), le bot écrit un `.page.tsx`. Ezerbot le compile sur le Mac et l’affiche dans un cadre à côté du fil. Une erreur de compilation revient au bot avec la ligne et le caractère. Le bot corrige seul. Charles peut renommer, ouvrir en pleine page, et retrouver la Page dans la liste « Pages » du bot. Elle survit au redémarrage. Pas de lien public, pas de partage d’équipe.
6. **Le PDF ne perd plus les glyphes de ce lot.** Courier est remplacé par une police embarquée. Les accents et l’emoji 🎉 s’affichent. L’emoji peut être un glyphe ou une image de repli. Un tableau demandé est un tableau. Le docx et le pptx montrent les mêmes accents, le même emoji et le même tableau. Un caractère hors de cette police fait encore échouer le fichier en français, plutôt que d’imprimer une case vide.
7. **La bibliothèque appartient au bot.** Le panneau est dans les réglages du bot. On y dépose par glisser-déposer les formats déjà lus dans un fil : pdf, docx, xlsx, csv, pptx, md, txt. Le plafond annoncé est celui de 20 Mo déjà en vigueur pour un document. Au-delà, refus en français, rien n’est retenu. Un nouveau fil de ce bot peut chercher ces fichiers et les citer. Un autre bot ne les reçoit pas. Retirer un fichier le sort du rappel. Le remplacer met à jour ce qui sera cité.

## Récits

1. En tant que Charles, je veux écrire « Dessine un phare au crépuscule » et voir le bot générer l’image sans que je nomme un outil, afin d’obtenir le dessin.
2. En tant que Charles, je veux lire « Génération d’une image… » pendant l’appel, afin de savoir que le dessin est en cours.
3. En tant que Charles, je veux que l’indicateur cède la place à l’image, ou à la phrase d’erreur, afin de ne pas rester devant un appel qui aurait fini.
4. En tant que Charles, je veux, tant que la clé OpenAI manque, la carte « Connecte ton compte OpenAI » et aucun appel, afin de ne pas prendre une absence pour une image.
5. En tant que Charles, je veux qu’un refus de sécurité s’affiche en français, une seule fois, afin de comprendre le blocage.
6. En tant que Charles, je veux qu’un modèle restreint s’affiche en français, une seule fois, afin de le distinguer d’une panne réseau.
7. En tant que Charles, je veux que la clé reste absente du fil, des journaux et du contexte du modèle, afin qu’un export ne la contienne pas.
8. En tant que Charles, je veux qu’aucune carte d’approbation ne s’ouvre avant la génération, afin de garder le geste direct.
9. En tant que Charles, je veux joindre une photo et demander le même style, afin que cette photo soit envoyée comme référence.
10. En tant que Charles, je veux un cadre 1:1, 16:9, 9:16, 4:3 ou 3:4, afin de recevoir l’image dans ce rapport.
11. En tant que Charles, je veux qu’un cadre inconnu soit refusé en français, sans appel au fournisseur, afin de ne pas payer une requête vouée à l’échec.
12. En tant que Charles, je veux les images d’un message en grille, afin de les comparer sans ouvrir un autre écran.
13. En tant que Charles, je veux qu’un clic ouvre la visionneuse, avec les flèches et la légende, afin de regarder une image en grand.
14. En tant que Charles, je veux « Enregistrer l’image… », afin de récupérer le fichier.
15. En tant que Charles, je veux « Copier l’image », afin de la coller dans une autre application.
16. En tant que Charles, je veux passer à l’image suivante et à la précédente au clavier, afin de parcourir la grille sans la souris.
17. En tant que Charles, je veux que la grille reste dans le fil, afin de ne pas chercher une page galerie qui n’existe pas.
18. En tant que Charles, je veux déléguer la description d’une photo jointe, afin que la réponse du fil `task:` décrive cette photo.
19. En tant que Charles, je veux que l’enfant reçoive l’image elle-même, afin qu’un nom de fichier ne tienne pas lieu de contenu.
20. En tant que Charles, je veux qu’une délégation dans un fil sans cette image ne l’emporte pas, afin de ne pas mélanger les conversations.
21. En tant que Charles, je veux « compare mes 3 forfaits dans un tableau interactif » et voir une Page à côté du fil, afin d’obtenir un tableau vivant plutôt qu’un tableau Markdown.
22. En tant que Charles, je veux qu’une erreur de compilation soit corrigée par le bot, avec la ligne et le caractère, afin de ne pas ouvrir le fichier moi-même.
23. En tant que Charles, je veux renommer la Page, afin de la retrouver sous un titre clair.
24. En tant que Charles, je veux « Ouvrir en pleine page », afin de travailler sans le fil à côté.
25. En tant que Charles, je veux la liste « Pages » du bot, afin de rouvrir une Page plus tard.
26. En tant que Charles, je veux que la Page soit encore là après un redémarrage, afin de ne pas la perdre en quittant.
27. En tant que Charles, je veux un PDF avec é, ç, 🎉 et un tableau, sans glyphe manquant, afin de l’envoyer tel quel.
28. En tant que Charles, je veux le même emoji et le même tableau dans le docx et le pptx, afin que les trois formats disent la même chose.
29. En tant que Charles, je veux qu’un PDF dont le texte tient dans les accents français continue de s’ouvrir, afin de ne pas casser les fichiers déjà demandés.
30. En tant que Charles, je veux déposer un fichier dans la bibliothèque d’un bot par glisser-déposer, afin qu’il serve dans tous les nouveaux fils de ce bot.
31. En tant que Charles, je veux qu’une question dans un nouveau fil cite le fichier, afin de voir d’où vient la réponse.
32. En tant que Charles, je veux qu’un autre bot ignore cette bibliothèque, afin de séparer les contextes.
33. En tant que Charles, je veux retirer un fichier et entendre le bot dire qu’il ne l’a plus, afin que le retrait soit réel.
34. En tant que Charles, je veux remplacer un fichier et voir la nouvelle version citée, afin de corriger un tarif sans laisser l’ancien.
35. En tant que Charles, je veux lire la taille maximale sur le panneau, et un refus en français au-delà, afin de savoir pourquoi le dépôt n’a pas eu lieu.
36. En tant que Charles, je veux continuer à joindre une image au fil et à la faire lire, afin de garder le geste déjà livré.
37. En tant que Charles, je veux qu’une Page ne propose pas de lien public, afin qu’elle reste sur ce Mac.

## Décisions d’implémentation

- L’outil s’appelle `generate_image`. Les arguments sont `description`, `aspect_ratio` optionnel, et, à partir de la tranche références, `reference_image_paths`. Le modèle envoyé au fournisseur est gpt-image. Le cadre omis vaut 1:1. La clé est celle du compte OpenAI de Charles, lue dans le trousseau déjà utilisé pour les secrets de compte. Ce lot n’ajoute pas un second coffre ni un formulaire « colle ta clé ».
- Le fournisseur de production est l’API d’images OpenAI. Les tests visent un fournisseur de boucle. Tout autre hôte est refusé. La clé ne sort pas de cet hôte.
- Une image générée est une pièce du message, au même titre qu’une image jointe. La grille et la visionneuse lisent les deux. Le plafond de dix images et de 20 Mo déjà en vigueur pour une pièce jointe reste celui d’un dépôt. Une image générée qui dépasserait ce plafond est une phrase française, pas un fichier tronqué.
- `selected_images` est un champ du contexte de `delegate`. Chaque entrée a les octets, ou un chemin local plus le type MIME. Le résumé texte du parent peut accompagner l’image. Il ne la remplace pas.
- Une Page est un fichier du bot, pas un message. Le compilateur est local. Le diagnostic (ligne, caractère, message) est le texte que le bot reçoit pour réécrire le fichier. Charles ne voit pas la clé, et il ne voit pas un cadre vide présenté comme une Page réussie. La liste « Pages » est celle de ce bot.
- La génération pdf, docx et pptx reste `create_document`. La police embarquée du PDF remplace Courier. Le tableau est reconnu dans le contenu déjà accepté par cet outil. Les coupes de longueur déjà écrites pour une diapositive restent, sauf à manger un emoji ou une cellule : ceux-là doivent apparaître.
- La bibliothèque est une liste de fichiers du bot. Le rappel dans un nouveau fil réutilise la recherche lexicale locale déjà utilisée pour la mémoire quand l’ensemble ne tient pas dans le prompt. Il n’y a pas d’index distant. La citation nomme le fichier. Retirer ou remplacer change cette liste avant le prochain fil.

## Décisions de test

Un bon test observe le comportement visible : l’indicateur, l’image dans le fil, la phrase française, l’absence de la clé dans le transcript et les journaux, le nombre d’appels au fournisseur, la grille, le fichier enregistré, le contenu reçu par l’enfant, le cadre de la Page, le texte extrait du PDF, la citation, et le refus de l’autre bot. Il ne vérifie pas l’ordre interne des fonctions.

La couture image est un fournisseur de boucle, puis le rendu de la bulle. On n’ajoute pas de harnais. Le test compte un appel pour une demande, zéro appel sans clé, zéro appel pour un cadre inconnu, et un seul appel pour un refus de sécurité. La couture Page injecte un diagnostic de compilation et observe le fichier réécrit, puis la Page encore présente après redémarrage du moteur. La couture PDF extrait é, ç et les cellules du tableau, et constate l’emoji au rendu. La couture bibliothèque parle à deux bots.

Chaque tranche commence par un test rouge sur sa couture, puis le code qui le rend vert. Le fournisseur scripté ne prouve pas le compte réel. Le juge visuel sur le Mac de Charles est le critère d’acceptation des parcours ci-dessous. Une capture immobile ne suffit pas.

## Critères d’acceptation

Le juge coche ces parcours dans l’app, avec le compte OpenAI de Charles capable d’appeler gpt-image, et des fichiers à lui. La clé n’apparaît nulle part à l’écran.

1. **Génération.** Phrase : « Dessine un phare au crépuscule. » L’indicateur « Génération d’une image… » apparaît, puis une image dans le fil. Aucune carte d’approbation. Déconnecter le compte, ou partir d’un profil sans clé : la carte « Connecte ton compte OpenAI » s’affiche et aucune image n’est ajoutée. Un refus de sécurité scripté, rejoué sur le fournisseur de boucle devant le juge si le compte réel ne le produit pas, affiche une phrase française et ne relance pas l’appel.
2. **Référence et cadre.** Joindre une photo. Phrase : « Le même style, en 16:9. » L’image résultante est en 16:9 et le juge reconnaît le style de la photo. Demander un cadre « 2:1 » affiche un refus français et n’ajoute pas d’image.
3. **Grille.** Un message qui contient trois images les montre en grille. Le clic ouvre la visionneuse, la légende est lisible, les flèches et les touches changent d’image. « Enregistrer l’image… » produit un fichier ouvrable. « Copier l’image » dépose une image dans le presse-papiers, collable dans un autre document. Aucun écran « Galerie » n’est ajouté au menu.
4. **Enfant.** Joindre une capture connue. Phrase : « Délègue la description de cette photo. » Le fil `task:` décrit cette capture. Une délégation dans un autre fil, sans la photo, ne décrit pas cette capture.
5. **Page.** Phrase : « Compare mes 3 forfaits dans un tableau interactif. » Une Page s’affiche à côté du fil. Le fil ne se contente pas d’un tableau Markdown à la place de cette Page. Renommer, puis « Ouvrir en pleine page », puis quitter et rouvrir : la Page est dans « Pages » et se rouvre. Une compilation cassée, injectée une fois, est corrigée sans que Charles édite le fichier. Aucun bouton de partage public.
6. **PDF.** Phrase : « Fais un PDF avec é, ç, 🎉 et un tableau, puis le même contenu en DOCX et en PPTX. » Les trois fichiers s’ouvrent. Le PDF montre les accents, l’emoji et le tableau. Le docx et le pptx aussi. Aucune case de glyphe manquant.
7. **Bibliothèque.** Déposer `tarifs.pdf` dans la bibliothèque du bot A. Nouveau fil : « Quel est le tarif X ? » La réponse cite `tarifs.pdf` et le tarif juste. Le bot B, nouveau fil, même question, dit qu’il n’a pas ce document.
8. **Retrait.** Retirer `tarifs.pdf`. Nouveau fil du bot A : le bot dit qu’il ne l’a plus. Déposer une version corrigée sous le même usage : la citation suit la nouvelle version. Déposer un fichier au-delà de la taille affichée : phrase française, le panneau ne gagne pas ce fichier.

## Hors-périmètre

- Générer une vidéo, joindre une vidéo, et le drapeau vidéo de Grok Bot.
- Retouche, masque, inpainting, variation.
- Une page galerie hors du fil.
- Partager une Page, la publier, ou la compiler sur un serveur.
- Un second coffre de clés, et un champ pour coller une clé API dans le fil.
- Changer la limite de dix pièces et de 20 Mo d’une pièce jointe.
- Réécrire la lecture des images jointes déjà livrée.
- L’ordinateur cloud, le Mac éteint qui générerait quand même, l’iPhone comme lieu de génération.
- Faire de la bibliothèque une mémoire d’équipe ou un index distant.
- Notion, les connecteurs, les routines. Le mémo vocal est le fichier `docs/specs/memo-vocal.md`, enfant de la carte [#63](https://github.com/charlescstpierr/ezerbot/issues/63), pas de cette carte.

## Risques

- **La clé est un secret de compte.** Elle suit la règle déjà prise pour les jetons : trousseau, pas de repli fichier pour un secret de compte quand le trousseau est actif. Le juge du parcours 1 vérifie qu’elle n’est pas à l’écran. Les tests vérifient qu’elle n’est pas dans le transcript.
- **Un refus de sécurité mal classé relancerait l’appel.** Le test compte un seul appel, puis la phrase. Pas de boucle.
- **gpt-image peut refuser le compte connecté.** La phrase « modèle restreint » est le comportement visible. Le juge du parcours 1 a besoin d’un compte qui produit une vraie image. Les tests, eux, tournent sur la boucle.
- **Une Page exécute du code local écrit par le bot.** Elle n’est pas partagée. Une compilation en échec ne doit pas être affichée comme une Page vide réussie : le diagnostic reste visible tant que le bot n’a pas réécrit un fichier qui compile.
- **Le PDF change de police.** Les accents qui passaient par WinAnsi doivent encore passer. Le parcours 6 et le test d’extraction sont là pour cela. Un glyphe hors police échoue encore en français.
- **La bibliothèque peut être volumineuse.** Le plafond de 20 Mo par fichier et la recherche lexicale locale évitent d’envoyer tous les fichiers à chaque tour. Un fichier retiré doit disparaître du rappel au fil suivant : le parcours 8 le coche.
- **L’enfant et la vie privée.** `selected_images` ne prend que les images du fil d’origine. Le test du parcours 4 couvre l’autre fil.

## Tranches

Chaque tranche traverse le parcours visible et le test. Le détail est dans l’annexe. Les blocages seront aussi des dépendances natives quand les issues existeront. Labels des tranches : `wayfinder:task`, `enhancement`, `ready-for-agent`. La première à prendre est T1.

Carte à créer : « Images, documents et bibliothèque ».

1. T1. Le bot génère une image. Aucun bloquant.
2. T2. Images de référence et format. Bloquée par T1.
3. T3. Galerie et visionneuse. Aucun bloquant : les images jointes suffisent. L’image générée entre dans la même grille une fois T1 livrée.
4. T4. Transmettre une image jointe à un sous-agent. Aucun bloquant.
5. T24. Créer une Page. Aucun bloquant.
6. T5. PDF Unicode, emoji et tableau. Aucun bloquant.
7. T6. Bibliothèque : ajouter. Aucun bloquant.
8. T7. Bibliothèque : retirer et remplacer. Bloquée par T6.

## Annexe — tickets prêts à créer

Ces corps sont prêts à coller dans le suivi. Cette PR ne crée aucune issue. Le parent de chacun est la carte « Images, documents et bibliothèque » une fois créée. Spec : `docs/specs/images-documents.md`.

### T1 — Le bot génère une image

## Parent

Carte « Images, documents et bibliothèque ». Spec : `docs/specs/images-documents.md`, décision 1 et parcours juge 1.

## What to build

Le bot appelle `generate_image` quand Charles demande un dessin. L’argument est la description. Le cadre omis vaut 1:1. Le modèle est gpt-image. La clé OpenAI de Charles est lue dans le trousseau. Elle n’est pas écrite dans le fil, les journaux, ni le contexte.

Pendant l’appel, le fil affiche « Génération d’une image… ». L’image le remplace. Un refus `content_safety_blocked` ou un modèle restreint affiche une phrase française et n’appelle pas le fournisseur une seconde fois. Sans clé, la carte « Connecte ton compte OpenAI » s’affiche et le compte d’appels reste à zéro. Aucune carte d’approbation.

Démo : « Dessine un phare au crépuscule », puis l’image.

TDD : d’abord un test rouge du wrapper contre le fournisseur de boucle (un appel, image dans le fil, refus de sécurité affiché, zéro appel sans clé, clé absente du transcript).

## Acceptance criteria

- [ ] TDD : le test du comportement visible est rouge avant le changement et vert après.
- [ ] « Dessine un phare au crépuscule » produit un appel et une image dans le fil, avec l’indicateur pendant l’appel.
- [ ] Aucune carte d’approbation n’est proposée.
- [ ] Sans clé, la carte « Connecte ton compte OpenAI » s’affiche et le fournisseur n’est pas appelé.
- [ ] Un refus de sécurité et un modèle restreint sont des phrases françaises, avec un seul appel chacun.
- [ ] La clé est absente du transcript, des journaux et de l’export.
- [ ] Joindre une image et la faire décrire fonctionne encore.
- [ ] Juge visuel sur le Mac, vrai compte : parcours 1 de la spec.

## Blocked by

- None (can start immediately)

### T2 — Images de référence et format

## Parent

Carte « Images, documents et bibliothèque ». Spec : `docs/specs/images-documents.md`, décision 2 et parcours juge 2.

## What to build

`generate_image` accepte `reference_image_paths` (images jointes au fil) et `aspect_ratio` parmi 1:1, 16:9, 9:16, 4:3, 3:4. La référence part avec la demande. Un cadre hors de cette liste est refusé en français, sans appel. Pas de masque ni de retouche.

Démo : une photo jointe, « le même style, en 16:9 », une image 16:9 à côté.

TDD : d’abord un test rouge (la référence est dans l’appel, le cadre 16:9 est celui de l’image rendue, un cadre « 2:1 » fait zéro appel).

## Acceptance criteria

- [ ] TDD : le test est rouge avant et vert après.
- [ ] Une image jointe citée comme style est envoyée dans `reference_image_paths`.
- [ ] 16:9, 1:1, 9:16, 4:3 et 3:4 sont acceptés.
- [ ] Un cadre inconnu est une phrase française et zéro appel.
- [ ] Juge visuel : parcours 2 de la spec. La photo et le résultat sont lisibles ensemble, et le rapport 16:9 se voit.

## Blocked by

- T1 — Le bot génère une image

### T3 — Galerie et visionneuse dans le fil

## Parent

Carte « Images, documents et bibliothèque ». Spec : `docs/specs/images-documents.md`, décision 3 et parcours juge 3.

## What to build

Plusieurs images d’un message s’affichent en grille. Un clic ouvre la visionneuse plein écran : flèches, légende, « Enregistrer l’image… », « Copier l’image ». Les touches image suivante et précédente circulent. Enregistrer écrit le fichier. Copier met l’image dans le presse-papiers. Il n’y a pas de page galerie. Les images jointes suffisent pour livrer cette tranche. Une image produite par T1 utilise la même grille.

Démo : trois images, grille, visionneuse, fichier enregistré, collage.

TDD : d’abord un test rouge du rendu (grille de trois, libellés, enregistrement, presse-papiers).

## Acceptance criteria

- [ ] TDD : le test est rouge avant et vert après.
- [ ] Trois images d’un message forment une grille.
- [ ] La visionneuse montre la légende et change d’image à la flèche et au clavier.
- [ ] « Enregistrer l’image… » produit le fichier. « Copier l’image » alimente le presse-papiers.
- [ ] Le menu de l’app ne gagne pas d’entrée Galerie.
- [ ] Juge visuel : parcours 3 de la spec.

## Blocked by

- None (can start immediately). Le juge d’une image générée dans la grille attend T1. Les images jointes débloquent le reste.

### T4 — Transmettre une image jointe à un sous-agent

## Parent

Carte « Images, documents et bibliothèque ». Spec : `docs/specs/images-documents.md`, décision 4 et parcours juge 4.

## What to build

Quand `delegate` vise une image jointe au fil, l’enfant reçoit `selected_images` : les octets, ou le chemin local et le type MIME. Le nom du fichier ne suffit pas. L’enfant décrit cette image dans le fil `task:`. Une délégation ouverte dans un fil qui n’a pas l’image ne la contient pas.

Démo : « Délègue la description de cette photo », description juste.

TDD : d’abord un test rouge qui observe la requête de l’enfant et constate l’image, puis l’absence d’image dans un autre fil.

## Acceptance criteria

- [ ] TDD : le test est rouge avant et vert après. L’enfant reçoit l’image, pas seulement son nom.
- [ ] Le fil `task:` décrit l’image jointe.
- [ ] Un autre fil, sans cette image, ne la place pas dans `selected_images`.
- [ ] Juge visuel : parcours 4 de la spec.

## Blocked by

- None (can start immediately)

### T24 — Créer une Page

## Parent

Carte « Images, documents et bibliothèque ». Spec : `docs/specs/images-documents.md`, décision 5 et parcours juge 5.

## What to build

Quand le bot produit une analyse autonome, il écrit un fichier `.page.tsx`. Ezerbot le compile sur le Mac et l’affiche dans un cadre à côté du fil. Une erreur de compilation est renvoyée au bot avec la ligne et le caractère. Le bot réécrit le fichier sans que Charles l’édite. Charles peut renommer la Page, choisir « Ouvrir en pleine page », et la retrouver dans la liste « Pages » du bot. La Page survit au redémarrage. Aucun lien de partage.

Démo : « Compare mes 3 forfaits dans un tableau interactif », la Page à côté du fil.

TDD : d’abord un test rouge (la Page s’affiche, une erreur injectée est corrigée, le fichier est encore là après redémarrage, pas de URL publique).

## Acceptance criteria

- [ ] TDD : le test est rouge avant et vert après.
- [ ] La demande de tableau interactif affiche une Page, et le fil ne s’arrête pas à un tableau Markdown à sa place.
- [ ] Une erreur de compilation injectée est corrigée sans édition manuelle.
- [ ] Renommer et « Ouvrir en pleine page » sont disponibles. La liste « Pages » retrouve la Page après redémarrage.
- [ ] Aucun contrôle de partage public n’est proposé.
- [ ] Juge visuel : parcours 5 de la spec. Capture du fil avec la Page, puis la pleine page.

## Blocked by

- None (can start immediately)

### T5 — PDF Unicode, emoji et tableau

## Parent

Carte « Images, documents et bibliothèque ». Spec : `docs/specs/images-documents.md`, décision 6 et parcours juge 6.

## What to build

`create_document` en PDF n’utilise plus Courier comme seule police. Une police embarquée couvre les accents français et l’emoji. L’emoji peut être dessiné en image si la police n’a pas le glyphe. Un tableau demandé est un tableau. Le même contenu en docx et en pptx montre les accents, l’emoji et le tableau. Un caractère encore hors couverture fait échouer la génération en français, sans case vide.

Démo : « Fais un PDF avec é, ç, 🎉 et un tableau », puis les trois formats ouverts.

TDD : d’abord un test rouge d’extraction (é, ç, cellules) et de rendu (emoji présent, aucun glyphe manquant).

## Acceptance criteria

- [ ] TDD : le test est rouge avant et vert après.
- [ ] Le PDF contient é, ç, le tableau, et l’emoji visible.
- [ ] Le docx et le pptx montrent les mêmes éléments.
- [ ] Un PDF limité aux accents français déjà acceptés s’ouvre encore.
- [ ] Un caractère hors couverture échoue en français, sans fichier à glyphe manquant.
- [ ] Juge visuel : parcours 6 de la spec, fichiers ouverts dans Aperçu ou l’application habituelle.

## Blocked by

- None (can start immediately)

### T6 — Bibliothèque : ajouter des fichiers de contexte

## Parent

Carte « Images, documents et bibliothèque ». Spec : `docs/specs/images-documents.md`, décision 7 et parcours juge 7.

## What to build

Les réglages du bot gagnent un panneau Bibliothèque. Charles y dépose par glisser-déposer un pdf, un docx, un xlsx, un csv, un pptx, un md ou un txt. Le plafond annoncé est 20 Mo. Le fichier est retenu pour ce bot. Un nouveau fil de ce bot peut s’en servir, et la réponse cite le fichier. Un autre bot ne le reçoit pas.

Démo : `tarifs.pdf` sur le bot A, question sur le tarif, citation. Le bot B ne sait pas.

TDD : d’abord un test rouge à deux bots (citation chez A, absence chez B).

## Acceptance criteria

- [ ] TDD : le test est rouge avant et vert après.
- [ ] Le panneau accepte le dépôt des formats listés et affiche le fichier.
- [ ] Un nouveau fil du bot A cite le fichier et répond juste.
- [ ] Le bot B ne cite pas ce fichier et ne prétend pas l’avoir.
- [ ] Juge visuel : parcours 7 de la spec.

## Blocked by

- None (can start immediately)

### T7 — Bibliothèque : retirer et remplacer

## Parent

Carte « Images, documents et bibliothèque ». Spec : `docs/specs/images-documents.md`, décision 7 et parcours juge 8.

## What to build

Retirer un fichier de la bibliothèque le sort du rappel : le bot dit ensuite qu’il ne l’a plus. Remplacer le fichier met à jour ce qui est cité. La taille maximale est annoncée sur le panneau. Un fichier plus gros est refusé en français et n’est pas retenu.

Démo : avant et après le retrait, puis un remplacement.

TDD : d’abord un test rouge (après retrait, la réponse dit que l’information n’est plus là ; après remplacement, la nouvelle valeur est citée ; le fichier trop gros est absent de la liste).

## Acceptance criteria

- [ ] TDD : le test est rouge avant et vert après.
- [ ] Après retrait, un nouveau fil dit que le bot n’a plus l’information.
- [ ] Après remplacement, la citation suit la nouvelle version.
- [ ] Au-delà de la taille annoncée, refus français, fichier absent du panneau.
- [ ] Juge visuel : parcours 8 de la spec, avant et après.

## Blocked by

- T6 — Bibliothèque : ajouter des fichiers de contexte
