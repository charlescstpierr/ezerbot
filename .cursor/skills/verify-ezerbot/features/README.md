# Ezerbot verification map

This directory is the maintained source for verifying the user-facing behavior of Ezerbot. Read the index before driving the app, then use the matching feature file as the recipe.

## Baseline preconditions

- Lancer l’instance avec `bash .cursor/skills/verify-ezerbot/scripts/launch.sh --evidence <preuves>`.
- `HOME` et `CFFIXED_USER_HOME` sont un répertoire `mktemp` neuf. Le refus est immédiat si ce `HOME` est le vrai dossier personnel ou si un chemin touche `~/Library/Application Support/Grokbot`.
- Le faux OpenAI de bouclage et le fichier `workspace.codex-auth.json` fictif sont déjà en place. Ne pas copier d’identifiants ChatGPT.
- Poser `CHROME_DEVTOOLS_AXI_SESSION` sur l’identifiant du run.
- Lancer `bash .cursor/skills/verify-ezerbot/scripts/doctor.sh --evidence <preuves>` et exiger `doctor: ok`, la bonne URL, le bon port et le commit extrait.
- Ne jamais piloter une instance qui n’a pas été démarrée par ce run, ni ouvrir `Ezerbot.app`.

## Driving conventions

- Partir de l’état de base, sauf précondition contraire du fichier de sphère.
- Préférer les rôles ARIA, les noms accessibles et les identifiants stables (`#settings-button`, `[data-panel]`, `[data-theme-choice]`, `[data-picker]`) aux coordonnées.
- Traiter chaque commande comme littérale.
- Passer les actions navigateur par `bash .cursor/skills/verify-ezerbot/scripts/browser.sh --evidence <preuves>`.
- Le harness sous-jacent est `chrome-devtools-axi`. Le substitut `EZERBOT_VERIFY_BROWSER=chrome-headless` parle les mêmes commandes et ne prouve pas le Mac.
- Restaurer l’écran d’accueil après une sphère modale. Ne pas supprimer les preuves au nettoyage.

## Proof and skip reporting

- Capturer l’action et l’état qui en résulte, pas seulement le dernier écran.
- Chaque thème (`dark`, `light`) et chaque taille (`1140x780`, `820x560`) produit un PNG et le texte visible à côté.
- Une préférence de thème n’est prouvée qu’une fois `aria-pressed` relu après fermeture et réouverture des Paramètres, et une fois `--canvas` égal à `#14161a` (sombre) ou `#ffffff` (clair).
- Noter l’identifiant de sphère et l’entrée utilisée avec chaque artefact.
- Un chemin inaccessible se rapporte avec la commande tentée et la précondition manquante.
- Ne pas déclarer une entrée sautée comme vérifiée par un autre chemin.

## Feature entry contract

Chaque fichier commence par un titre H1 et un paragraphe. Il utilise ensuite exactement quatre H2, dans cet ordre : `Sub-features`, `How to get to it (user POV)`, `Driving it with chrome-devtools-axi`, `Gotchas`.

## Features

- [Accueil](./accueil.md) couvre la marque, la nouvelle conversation vide, le compositeur et les deux thèmes.
- [Paramètres et connecteurs](./parametres-connecteurs.md) couvre l’ouverture des Paramètres et la section Connecteurs.
- [Modèles](./modeles.md) couvre le menu Modèle du compositeur branché sur le faux catalogue.
- [Réunions et routines](./reunions-routines.md) couvre la rétention audio et le dialogue Routines.
- [Mémoire](./memoire.md) couvre la mémoire automatique et la mémoire du bot Ezerbot.
- [Bibliothèque](./bibliotheque.md) couvre la bibliothèque de fichiers d’un bot : dépôt, liste, retrait et avertissement de troncature.
- [Images générées](./images.md) couvre la demande d’un dessin, l’image dans la carte et la visionneuse.
- [Images de référence et cadre](./images-reference.md) couvre la photo jointe envoyée en référence et l’image large rendue à son rapport dans la carte.
- [Pages](./pages.md) couvre une Page locale écrite par le bot : affichage à côté du fil, choix interactif et texte décodé.
- [Dictée et microphone](./dictee.md) couvre le réglage du microphone, son repli quand il disparaît et la dictée qui pose le texte dans le brouillon.
- [Voix par bot](./voix-bots.md) couvre la voix, la vitesse et la langue de chaque bot, l’attribution de deux voix différentes et le refus d’une vitesse au-delà de 2.
- [Mémo vocal](./memo-vocal.md) couvre la vitesse de lecture d’un mémo de 0,5 à 2, le refus d’une vitesse hors plage et le saut au mot de la transcription.
- [Réponse depuis la notification iPhone](./iphone-reponse.md) couvre la feuille Répondre ouverte depuis la notification d’un bot et l’envoi qui n’arrive qu’une fois dans le fil.
- [Appel vocal](./voix-appel.md) couvre l’appel en direct : la phrase de transmission de l’audio, le transcript affiché ou masqué, la voix du bot coupée par la parole ou Échap, et le transcript rangé dans le fil.
- [Mode YOLO](./mode-yolo.md) couvre l'activation par défaut, la commande et l'écriture de fichier faites sans carte (« Fait automatiquement »), le journal d'activité, le mode par bot, le retour à « Demander avant d'agir » et les garde-fous.
- [Tâches de fond](./taches-de-fond.md) couvre la carte d’une commande en arrière-plan : la saisie d’une ligne sur son entrée standard, sa recopie dans la sortie, le refus après Terminée et sur une invite de mot de passe.
- [Répondre en fil](./reponse-fil.md) couvre l’entrée « Répondre en fil » du menu d’un message, le sous-fil lié avec son bandeau, la réponse du bot qui tient compte du message parent et le retour au fil principal.
- [Approbation non bloquante](./approbation-non-bloquante.md) couvre le réglage « Continuer sans attendre mes approbations » dans Paramètres : l’état par défaut, la durée de validité, le rappel et l’état coupé où les choix sont grisés.
- [Approbation tardive](./approbation-tardive.md) couvre la carte décidée après la fin du tour du bot : « Autorisé une fois », la reprise du bot dans le même fil avec le résultat, et la carte Expirée grise sans action.
- [Approbation : le rappel](./approbation-rappel.md) couvre la demande en attente qui revient vers l’utilisateur : la ligne de rappel dans Notifications, la ligne grise « En attente de vous depuis… · expire dans… » au-dessus de la carte et le bandeau « N demandes attendent votre décision » à la reprise de l’app.
- [Sujets](./sujets.md) couvre la section SUJETS de la barre latérale (dossiers repliables, fils décalés avec temps relatifs), « Nouveau fil » dans un sujet, la page d’un sujet avec sa carte « Mémoire du sujet », l’écran « Mémoire du sujet » (recherche, oubli d’un fait) et la carte « Retenir pour ce sujet ? ».
