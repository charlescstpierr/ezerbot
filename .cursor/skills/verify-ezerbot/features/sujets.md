# Sujets

Un sujet est un dossier nommé qui regroupe plusieurs fils (un bot seul ou un groupe). La barre latérale a une section « SUJETS » au-dessus de « CONVERSATIONS », qui garde les fils sans sujet. Un sujet a sa propre page, avec la liste de ses fils, « Nouveau fil » et la carte « Mémoire du sujet ».

## Sub-features

- `barre` montre la barre latérale : « SUJETS » avec son bouton « + », « Déménagement » déplié (deux fils décalés, « il y a 2 h » et « il y a 1 j »), « Impôts » replié, puis « CONVERSATIONS » avec les deux fils sans sujet.
- `nouveau-fil` ouvre la page de « Déménagement », clique « Nouveau fil » : un fil vide « Nouvelle conversation » s'ouvre, déjà rangé sous le sujet et actif dans la barre latérale.
- `sujet` montre la page du sujet : titre « Déménagement » (aussi en haut de page), description grise, « 2 fils » avec « Nouveau fil », les deux fils avec leur bot et leur temps relatif, et à droite (dessous en fenêtre étroite) la carte « Mémoire du sujet » vide.

## How to get to it (user POV)

- Le « + » de la section SUJETS crée un sujet (nom, description facultative). Le nom d'un sujet ouvre sa page ; la flèche à sa gauche replie ou déplie ses fils.
- Le menu « Actions de la conversation » a « Classer dans un sujet… » (« Changer de sujet… » si le fil est déjà rangé) : un seul sujet à la fois, ou « Aucun sujet ».
- Sur la page : « Renommer », « Supprimer » (les fils ne sont pas supprimés : ils repassent dans Conversations) et « Nouveau fil ».

## Driving it with chrome-devtools-axi

Preconditions:

- `doctor: ok`, badge `ChatGPT`.

- **Préparation.** Le parcours crée deux sujets et cinq fils par l'API, vieillit leurs `updated_at` dans la base jetable (2 h, 26 h, 74 h, 5 h, 30 h), puis plie « Impôts » et déplie « Déménagement ».
- **Barre latérale.** `#sidebar-topics`, `#topic-list .topic-folder`, `.topic-toggle[aria-expanded]`, `.topic-name`, `.topic-thread` et `.topic-thread-time` ; les fils sans sujet restent dans `#conversation-list`.
- **Page.** `#topic-page` (`.topic-page-title`, `.topic-page-description`, `.topic-new-thread`, `.topic-memory`) ; `.main-panel[data-view="topic"]` cache le fil et le compositeur.
- **Parcours groupé.** Run `bash .cursor/skills/verify-ezerbot/scripts/drive.sh --evidence <preuves> --sphere sujets`. Les douze PNG (`barre-`, `nouveau-fil-` et `sujet-`, deux thèmes, deux tailles) existent.

## Gotchas

- Les trois captures partent du même état : avant chaque passage, le fil vide créé par « Nouveau fil » est supprimé.
- La page d'un sujet se ferme dès qu'un fil, l'accueil ou un lien est ouvert (`onNavigate`).
- Les sujets restent sur le Mac : le panneau ne se monte pas depuis l'iPhone appairé.
