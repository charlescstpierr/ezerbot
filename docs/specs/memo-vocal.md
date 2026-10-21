# Mémo vocal — vitesse et saut au mot

Complément de `docs/specs/reunions-voix-iphone.md`. Pas une nouvelle carte. La tranche est un enfant de [Réunions, voix et iPhone](https://github.com/charlescstpierr/ezerbot/issues/63). Cette page ne crée pas l’issue. L’annexe en donne le corps.

La spec des réunions décide : « Le mémo ne change pas de voix dans ce lot : il garde la lecture déjà livrée. » Ce complément couvre le trou VX-2 laissé par cette phrase : la vitesse choisie et le saut au mot. Il ne rouvre ni la dictée, ni l’appel, ni Recall.

Vocabulaire déjà en usage : mémo vocal, transcription, bot, fil, voix du bot. Un horodatage de mot est l’instant de ce mot dans l’audio du mémo.

## Contexte

Le mémo joint l’audio au message et affiche sa transcription. La lecture livrée aujourd’hui n’a pas de vitesse à choisir, et un mot de la transcription n’est pas un point de lecture. La durée reste d’une minute. L’audio du mémo reste sur le Mac.

Grok Bot demande une synthèse avec `with_timestamps`. Les temps par mot permettent de sauter à un mot. La vitesse de voix va de 0,5 à 2. La carte [#68](https://github.com/charlescstpierr/ezerbot/issues/68) donne déjà cette plage à chaque bot, pour le mode voix. Le mémo n’en hérite pas encore.

## Énoncé du problème

Charles lit un mémo d’une traite, à une seule vitesse. Il ne peut pas cliquer un mot de la transcription pour reprendre à cet endroit. Le scénario VX-2 note la vitesse et le saut au mot, et les deux manquent.

## Solution

La lecture du mémo prend une vitesse entre 0,5 et 2. La transcription est cliquable. Un clic sur un mot place la lecture à l’horodatage de ce mot. Les temps viennent de la transcription du mémo, sur le Mac, sur le modèle des horodatages par mot de Grok Bot.

## Décisions

1. **La vitesse est celle du mémo, dans la plage déjà décidée.** Le contrôle va de 0,5 à 2. Une valeur hors plage est refusée et laisse la vitesse précédente. La plage est celle de la voix du bot (#68). Le mémo ne gagne pas une autre échelle.
2. **Le mot est un instant.** Chaque mot affiché a un début. Le clic lance la lecture à cet instant. Sans horodatage pour un mot, le clic ne saute pas et la phrase le dit.
3. **Le mémo reste local et court.** L’audio ne part pas vers un serveur. La minute maximale, la dictée, le mode voix, l’appel et Recall ne changent pas. Cette lecture ne devient pas l’appel vocal.

## Récits

1. En tant que Charles, je veux choisir la vitesse de lecture d’un mémo entre 0,5 et 2, afin d’accélérer ou de ralentir sans réenregistrer.
2. En tant que Charles, je veux qu’une vitesse hors de cette plage soit refusée, afin de garder la vitesse qui marchait.
3. En tant que Charles, je veux voir la transcription du mémo mot à mot, afin de viser un endroit.
4. En tant que Charles, je veux cliquer un mot et entendre la suite à partir de ce mot, afin de ne pas réécouter le début.
5. En tant que Charles, je veux que l’audio du mémo reste sur le Mac, afin de garder la règle déjà livrée.
6. En tant que Charles, je veux que la dictée, le mode voix et l’appel se comportent comme avant, afin que ce lot ne les mélange pas avec le mémo.

## Décisions d’implémentation

- Le lecteur du mémo déjà livré gagne la vitesse et le clic. Il ne devient pas un second lecteur.
- Les horodatages sont produits avec la transcription du mémo, un début par mot, comme `with_timestamps`. Ils sont gardés avec le message. Ils ne sont pas un fichier audio de plus.
- La vitesse affichée au départ est la vitesse du bot une fois #68 livrée. Avant ce réglage, le contrôle du mémo part de 1 et reste dans 0,5–2. #68 reste le bloquant de la tranche : le juge coche la vitesse du bot et celle du mémo ensemble.
- Un mot sans temps n’est pas cliquable. La lecture en cours, elle, continue.

## Décisions de test

Un bon test observe la vitesse annoncée, l’instant de lecture après un clic, le refus hors plage, et l’absence d’un envoi de l’audio du mémo. Il ne vérifie pas la forme interne du décodeur.

La couture est celle des tests de mémo déjà joués sans microphone réel. Le temps de chaque mot est injectable. Le test place la lecture au début du mot cliqué.

La tranche commence par un test rouge, puis le code qui le rend vert. Le juge visuel sur le Mac est le critère d’acceptation. Une capture immobile ne suffit pas : il faut entendre le changement de vitesse et le saut.

## Critères d’acceptation

1. **Vitesse et mot.** Envoyer un mémo dont la transcription contient une phrase connue. Passer la vitesse à 2, écouter : la lecture est plus rapide qu’à 1. Revenir à 0,5 : elle est plus lente. Cliquer un mot au milieu : la lecture reprend à ce mot, pas au début. Une vitesse 3 est refusée et la vitesse affichée ne change pas. L’écran du mémo ne dit pas que l’audio part au fournisseur. La dictée d’une phrase courte fonctionne encore.

## Hors-périmètre

- Changer la voix du mémo, la durée d’une minute, la dictée, le choix du microphone, l’appel vocal, le mode voix, Recall.
- Les cinquante-sept langues de dictée de Grok Bot, et les noms de voix de Grok.
- Jouer le mémo depuis la PWA comme un geste nouveau. Une fois le message dans le fil, la PWA le lit comme aujourd’hui, Mac allumé.
- Produire les horodatages par un serveur si la transcription du mémo est aujourd’hui locale. Elle le reste.
- L’ordinateur cloud.

## Risques

- **Un clic sans temps donnerait un saut au début.** La décision 2 l’interdit : le mot sans temps n’est pas une cible. Le test injecte un mot sans début et vérifie que la position ne change pas.
- **La vitesse du bot et celle du mémo pourraient diverger.** La plage unique 0,5–2 et le bloquant #68 sont la parade. Le juge écoute le mémo après avoir réglé la voix du bot.
- **Confondre mémo et appel.** L’appel transmet l’audio le temps de la session. Le mémo non. Le parcours 1 vérifie que l’écran du mémo ne reprend pas la phrase de l’appel.

## Tranches

Label : `wayfinder:task`, `enhancement`, `ready-for-agent`. Parent : [#63](https://github.com/charlescstpierr/ezerbot/issues/63).

1. T23. Lire un mémo à la vitesse choisie et sauter au mot. Bloquée par [#68](https://github.com/charlescstpierr/ezerbot/issues/68).

## Annexe — ticket prêt à créer

Ce corps est prêt à coller. Cette PR ne crée pas l’issue.

### T23 — Lire un mémo à la vitesse choisie et sauter au mot

## Parent

Carte [Réunions, voix et iPhone](https://github.com/charlescstpierr/ezerbot/issues/63). Spec : `docs/specs/memo-vocal.md`. Complète `docs/specs/reunions-voix-iphone.md` sans en faire une nouvelle carte.

## What to build

La lecture d’un mémo a une vitesse de 0,5 à 2. Une valeur hors plage est refusée et la vitesse précédente reste. La transcription est cliquable. Chaque mot qui a un début saute la lecture à cet instant. Un mot sans début n’est pas une cible. L’audio du mémo reste sur le Mac. La minute maximale, la dictée, le mode voix et l’appel ne changent pas.

Démo : mémo de vingt secondes, vitesse 2 puis 0,5, clic sur un mot au milieu, la suite part de ce mot.

TDD : d’abord un test rouge (vitesse annoncée, position égale au début du mot cliqué, refus au-delà de 2, audio du mémo non envoyé).

## Acceptance criteria

- [ ] TDD : le test du comportement visible est rouge avant le changement et vert après.
- [ ] Le contrôle affiche une vitesse de 0,5 à 2 et la lecture suit.
- [ ] Une vitesse hors plage ne remplace pas la vitesse affichée.
- [ ] Un clic sur un mot place la lecture au début de ce mot.
- [ ] Un mot sans horodatage ne déplace pas la lecture.
- [ ] L’audio du mémo n’est pas envoyé. La dictée et le mode voix se comportent comme avant cette tranche.
- [ ] Juge visuel sur le Mac : parcours 1 de `docs/specs/memo-vocal.md`. Il faut entendre la vitesse et le saut.

## Blocked by

- [Donner une voix à chaque bot](https://github.com/charlescstpierr/ezerbot/issues/68)
