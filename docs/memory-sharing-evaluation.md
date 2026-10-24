# Évaluation du tri de partage (synthétique)

Périmètre de [#249](https://github.com/charlescstpierr/ezerbot/issues/249) : mesurer
`recall::sharing_class`, **sans changer ses règles ni celles de `promote`**.
Aucun modèle, réseau, compte ou vrai fait utilisateur n'intervient dans la mesure.
La base de la non-régression de promotion est SQLite en mémoire.

## Jeu et étiquettes

`crates/memory/tests/fixtures/sharing-class.json` contient 90 faits inventés,
30 par classe. Les étiquettes sont des intentions de partage, **pas les résultats
attendus de l'heuristique actuelle** :

- `personal` : information privée/personnelle, à ne pas présélectionner ;
- `work` : information professionnelle partageable ;
- `review` : contexte insuffisant ou information neutre, à ne pas présélectionner.

Les exemples professionnels sans mot-clé restent étiquetés `work`, même si le tri
actuel les met dans `review`. Inversement, un livrable professionnel contenant
« maison », « médical », « santé », « home » ou « couleur » reste étiqueté `work` :
le tri prudent peut légitimement s'abstenir de le présélectionner.

Les tags documentent les pièges : privé contenant du vocabulaire de travail,
travail contenant du vocabulaire personnel, absence de mot-clé, anglais,
accents précomposés/décomposés et ligatures `œ`/`æ`. Plus de 15 faits distincts
portent ces tags, avec au moins trois exemples de chaque catégorie.
`sensitive` est le drapeau réellement donné à l'API ; l'étiquette
`personal` couvre aussi les faits privés dont ce drapeau est absent.

Le **premier** exemple est `p01-salary-enterprise`, sans drapeau sensible :
« Mon salaire chez l'entreprise est de 4200 unités fictives. »
Il est étiqueté `personal` et **fuit au départ** : le tri actuel renvoie `work`.
C'est une fuite de présélection, pas la preuve d'une promotion sans confirmation.

## Lancer la mesure

```sh
cargo test -p grokbot-memory --locked --test sharing_class_evaluation sharing_class_report_is_repeatable -- --exact --nocapture
```

Le rapport n'inclut ni horloge ni identifiants aléatoires. Il affiche la matrice
3×3 (lignes = étiquettes, colonnes = prédictions), l'exactitude de chaque classe
(diagonale / nombre de faits étiquetés dans cette classe), la part de travail
correctement présélectionné et toutes les fuites, dans l'ordre du jeu.
Le test compare aussi deux évaluations et leurs rapports à l'identique.

Le compteur de fuites compte un fait étiqueté `personal` **ou** marqué `sensitive`
qui est prédit `work`. Un fait personnel classé `review` n'est pas une fuite,
mais reste une erreur d'exactitude de la classe `personal`.

## Score de départ

| Étiquette \\ prédiction | personal | work | review | Exactitude par classe |
| --- | ---: | ---: | ---: | ---: |
| personal | 15 | 10 | 5 | 15/30 = 50,00 % |
| work | 5 | 20 | 5 | 20/30 = 66,67 % |
| review | 0 | 0 | 30 | 30/30 = 100,00 % |

**10 fuites**, `p01` à `p10` (liste des textes dans le rapport).
Travail correctement présélectionné : **20/30**.
Ces chiffres sont un constat de départ, pas un seuil acceptable ni une preuve
sur de vraies données. Le couple utile reste **(zéro fuite, part du travail
correctement présélectionné)** ; maximiser l'exactitude seule n'est pas l'objectif.

## Garde-fous et échec connu

```sh
cargo test -p grokbot-memory --locked
# Diagnostic volontairement rouge tant que la fuite de départ existe :
cargo test -p grokbot-memory --locked --test sharing_class_evaluation zero_personal_or_sensitive_facts_preselected -- --exact --ignored --nocapture
```

La suite normale vérifie le format/calcul du rapport, le jeu et sa répétabilité,
que les faits inconnus restent `review`, que **chacun des 90 textes** devient
`personal` avec `sensitive = true`, et que `promote` laisse tous ces faits
sensibles privés, inchangés et invisibles à un autre bot. Un contrôle non sensible
est effectivement promu dans la même demande.

Le diagnostic zéro fuite est explicitement ignoré par défaut, car ce ticket
**mesure et ne corrige pas**. Lancé explicitement, il exige strictement zéro et
échoue sur les fuites existantes, dont le salaire en premier. Il ne tolère pas
10 fuites et ne doit pas être rendu vert en modifiant les étiquettes ou en
assouplissant les protections. Le correctif relève du ticket séparé
[feature/P2 #267](https://github.com/charlescstpierr/ezerbot/issues/267).
Les vrais faits de Charles, hors dépôt, relèvent également d'un autre ticket.
