# Évaluation du tri de partage (synthétique)

[#249](https://github.com/charlescstpierr/ezerbot/issues/249) a mesuré
`recall::sharing_class` sans changer ses règles ; [#267](https://github.com/charlescstpierr/ezerbot/issues/267)
corrige la fuite qu'il avait constatée (section « Correction de #267 »). Les règles
de `promote` ne changent pas.
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
Il est étiqueté `personal` et **fuyait au départ** : le tri d'alors renvoyait `work`.
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

## Score de départ (avant #267)

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

## Correction de #267

Cause : un signal professionnel (« entreprise », « équipe », « client »…) suffisait
à classer `work`, et seuls quelques mots personnels (« famille », « santé »…)
l'emportaient. Un salaire, une dette ou un divorce mentionnant l'entreprise passait.

Règles de `recall::sharing_class`, dans l'ordre :

1. `sensitive = true`, ou un sujet privé (argent/banque/dettes, santé, vie
   intime/identité, pièces d'identité, confidentialité : liste de radicaux FR/EN
   comparés au début des mots) ou l'un des anciens mots personnels : `personal`.
   Un signal personnel l'emporte toujours sur un signal professionnel.
2. Sinon, un mot professionnel : `work`, **sauf si le fait contient un possessif de
   première personne** (« mon », « ma », « mes », « my », « mine ») : il s'abstient en
   `review`, car « Mon … chez l'entreprise » peut être une affaire privée qui cite
   seulement le métier. Un pronom seul (« Je travaille sur le projet Atlas ») reste
   `work` : il est trop courant dans les vrais faits professionnels.
3. Sinon : `review`. Un inconnu n'est jamais présélectionné.

La règle est lexicale et volontairement large : un faux positif coûte une
abstention, une fuite coûte une divulgation. Limite connue : un fait privé sans
aucun mot de la liste ni possessif, mais avec un mot professionnel
(« Je cache une situation à l'équipe »), reste `work`. Coût assumé : des mots
techniques courants (« traitement par lots », « héritage multiple », « mon projet
utilise Rust ») sortent de `work` vers `personal` ou `review`. C'est une exactitude `work` plus basse acceptée contre une fuite, pas un
défaut. Le sélecteur reste indicatif et l'utilisateur confirme toujours le partage.

Anti-sur-ajustement : quatorze paraphrases inventées hors du jeu (salaire, santé,
dettes, vie intime, avec un mot professionnel, dont deux sans aucun sujet
privé listé) ne doivent jamais être `work` ; quatre faits professionnels
(dont « Je travaille sur le projet Boréal ») doivent le rester. Elles sont dans les tests de `recall.rs`
(`private_paraphrases_with_work_words_are_never_preselected`), pas dans le jeu,
dont les étiquettes ne changent pas.

### Mesure après correction

| Étiquette \\ prédiction | personal | work | review | Exactitude par classe |
| --- | ---: | ---: | ---: | ---: |
| personal | 30 | 0 | 0 | 30/30 = 100,00 % |
| work | 5 | 20 | 5 | 20/30 = 66,67 % |
| review | 0 | 0 | 30 | 30/30 = 100,00 % |

**0 fuite** (10 au départ). Travail correctement présélectionné : **20/30**,
inchangé. L'exactitude `personal` passe de 50,00 % à 100,00 % ; les 10 erreurs
restantes de `work` (5 en `personal`, 5 en `review`) sont les abstentions prudentes
déjà présentes au départ (vocabulaire « maison », « santé »…, ou absence de mot-clé).

## Garde-fous

```sh
cargo test -p grokbot-memory --locked
cargo test -p grokbot-memory --locked --test sharing_class_evaluation zero_personal_or_sensitive_facts_preselected -- --exact --nocapture
```

La suite normale vérifie le format/calcul du rapport, le jeu et sa répétabilité,
que les faits inconnus restent `review`, que **chacun des 90 textes** devient
`personal` avec `sensitive = true`, et que `promote` laisse tous ces faits
sensibles privés, inchangés et invisibles à un autre bot. Un contrôle non sensible
est effectivement promu dans la même demande.

`zero_personal_or_sensitive_facts_preselected` est un test normal (plus ignoré) :
il exige strictement zéro fuite sur le jeu entier. Il ne doit pas être rendu vert
en modifiant les étiquettes ni en assouplissant les protections.
Les vrais faits de Charles, hors dépôt, relèvent d'un autre ticket.
