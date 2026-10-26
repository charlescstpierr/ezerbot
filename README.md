# Preuve du rail Galet pour #561

PR https://github.com/charlescstpierr/ezerbot/pull/587.
Tête exacte be2cedd617db0161bab4d3192170dc9369497056.
Avant compilé sur eff4b4134a02c549583dff0e96a95f3a9518c325.

357 tests passent sur cette tête. Aucun échec, annulation, test sauté ou todo. Les deux checks CI compile sont SUCCESS. Les résultats sont dans final-tests.log et ci.json.

Le wrapper est compilé puis lancé avec HOME, base, profil Chrome et session pm-i561 isolés. Le lancement de l’accueil utilise explicitement --sphere accueil. Le parcours Pages a son propre lancement --sphere pages et le faux fournisseur prévu pour écrire une Page.

## Captures Mac

Les quatre captures après contiennent cinq bots et 32 fils. Le texte capturé accompagne chaque PNG. Ces quatre images proviennent du nouveau run final-dimensions avec viewport explicite et IHDR vérifié. Les grands après précédents, réellement 1440×855, sont rejetés. Le reçu receipts/png-dimensions.json couvre tous les PNG publiés. Les avant de grande taille sont étiquetés 1440×855 selon leur IHDR réel.

| Taille | Sombre | Clair |
| --- | --- | --- |
| 1440×900 | [Après sombre](after/dark-1440x900.png) | [Après clair](after/light-1440x900.png) |
| 820×560 | [Après sombre](after/dark-820x560.png) | [Après clair](after/light-820x560.png) |

Les captures avant sont dans before/. Le board réel rendu est dans references/page-2-rendered.png. [comparison.md](comparison.md) explique les différences de largeur, les accès conservés et la portée de la comparaison.

## Comportements

Le parcours Mac ajoute 20 contrôles DOM et clavier. Les cinq destinations ont leur nom accessible et leur état actif. Sujets mène à sa section réelle et à son formulaire. Routines ouvre le même dialogue avec Atelier sélectionné. Notifications, Réglages et Mémoire ouvrent leurs vrais panneaux. Tab, Entrée, Échap, ⌘K, la recherche et ⌘N sont pilotés dans Chrome.

[rail-checks.json](rail-checks.json) conserve les résultats et les rectangles de la liste. Le dernier fil est entièrement visible, puis sélectionné après défilement. Le rail reste séparé. Les textes de la barre latérale sont au moins à 11,5 px. La carte de compte et l’indicateur Local sont absents.

Le test du rail utilise le vrai wrapper et une Page créée par write_page. Il vérifie Pages absent, présent puis absent en changeant de bot. Les groupes de dates sont contrôlés avec quatre dates connues. Les captures Pages supplémentaires sont dans pages/. Le complément dans creation/ crée un bot par le vrai formulaire ouvert avec le plus, garde ce bot avec ⌘N et enregistre une conversation en tapant puis en envoyant un message par Entrée.

[Les montages et leurs tests](mounts-and-tests.md) recensent tous les chemins modifiés. Le fil, ses cartes et le composer ne sont pas modifiés.

## Reçus et nettoyage

Les doctor avant et après, les verdicts des parcours et les nettoyages sont dans receipts/. Les signatures des runs sont validées localement par la skill. Chaque HOME jetable est supprimé et les processus attribués aux runs sont arrêtés. Les métadonnées signées privées, marqueurs, profils, bases et fichiers d’authentification ne sont pas publiés.

Les pilotes complémentaires sont dans replay/. Ils s’exécutent sous test-gate avec la skill verify-ezerbot de ce worktree. Le manifest donne le SHA-256 de chaque fichier public.

La revue canonique du produit et pret restent à lancer par le coordinateur. Ce dossier ne constitue pas leur verdict.
