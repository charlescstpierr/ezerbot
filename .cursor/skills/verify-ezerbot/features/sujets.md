# Sujets

Un sujet est un dossier nommé qui regroupe plusieurs fils (un bot seul ou un groupe). La barre latérale a une section « SUJETS » au-dessus de « CONVERSATIONS », qui garde les fils sans sujet. Un sujet a sa propre page, avec la liste de ses fils, « Nouveau fil » et la carte « Mémoire du sujet » (trois faits et « Tout voir »). Un bot peut proposer de retenir un fait pour tout le sujet : la carte « Retenir pour ce sujet ? » n'écrit rien avant la réponse de Charles.

## Sub-features

- `barre` montre la barre latérale : « SUJETS » avec son bouton « + », « Déménagement » déplié (deux fils décalés, « il y a 2 h » et « il y a 1 j »), « Impôts » replié, puis « CONVERSATIONS » avec les deux fils sans sujet.
- `nouveau-fil` ouvre la page de « Déménagement », clique « Nouveau fil » : un fil vide « Nouvelle conversation » s'ouvre, déjà rangé sous le sujet et actif dans la barre latérale.
- `sujet` montre la page du sujet : titre « Déménagement » (aussi en haut de page), description grise, « 2 fils » avec « Nouveau fil », les deux fils avec leur bot et leur temps relatif, et à droite (dessous en fenêtre étroite) la carte « Mémoire du sujet » vide.
- `memoire` retient quatre faits, puis « Tout voir » ouvre l'écran « Mémoire du sujet » : titre, phrase d'aide, champ de recherche, les faits avec la pastille de leur fil d'origine et leur date, une corbeille par fait et le lien gris « Supprimer le sujet… ».
- `oublier` clique la corbeille : le dialogue centré « Oublier ce fait ? Il sera retiré de tous les fils du sujet. » avec « Annuler » et « Oublier ». Rien n'est effacé tant que Charles n'a pas confirmé.
- `carte` ouvre le fil « Louer un camion » avec la carte « Retenir pour ce sujet ? » (« Retenir » / « Pas maintenant »).
- `retenu` clique « Retenir » : la carte devient « Retenu pour ce sujet », avec le lien « Mémoire du sujet ».
- `autre-fil` ouvre la page d'« Impôts », un autre sujet : sa carte « Mémoire du sujet » reste vide, aucun fait de « Déménagement » n'y paraît.

## How to get to it (user POV)

- Le « + » de la section SUJETS crée un sujet (nom, description facultative). Le nom d'un sujet ouvre sa page ; la flèche à sa gauche replie ou déplie ses fils.
- Le menu « Actions de la conversation » a « Classer dans un sujet… » (« Changer de sujet… » si le fil est déjà rangé) : un seul sujet à la fois, ou « Aucun sujet ».
- Sur la page : « Renommer », « Supprimer » (les fils ne sont pas supprimés : ils repassent dans Conversations) et « Nouveau fil ».
- La carte « Mémoire du sujet » de la page montre les trois derniers faits retenus ; « Tout voir » ouvre l'écran complet (recherche, corbeille par fait, « Supprimer le sujet… »). Supprimer un sujet qui a des faits les efface avec lui, après confirmation dans le dialogue ; la mémoire privée des bots reste.
- Un bot propose un fait pour le sujet avec l'outil `propose_topic_memory` : la carte « Retenir pour ce sujet ? » apparaît dans le fil. « Retenir » l'écrit pour tous les fils du sujet ; « Pas maintenant » n'écrit rien.

## Driving it with chrome-devtools-axi

Preconditions:

- `doctor: ok`, la pilule Modèle `#composer-tools [data-picker="model"]` est présente.

- **Préparation.** Le parcours crée deux sujets et cinq fils par l'API, vieillit leurs `updated_at` dans la base jetable (2 h, 26 h, 74 h, 5 h, 30 h), puis plie « Impôts » et déplie « Déménagement ».
- **Barre latérale.** `#sidebar-topics`, `#topic-list .topic-folder`, `.topic-toggle[aria-expanded]`, `.topic-name`, `.topic-thread` et `.topic-thread-time` ; les fils sans sujet restent dans `#conversation-list`.
- **Page.** `#topic-page` (`.topic-page-title`, `.topic-page-description`, `.topic-new-thread`, `.topic-memory`) ; `.main-panel[data-view="topic"]` cache le fil et le compositeur.
- **Mémoire du sujet.** `.topic-memory` (`.topic-memory-fact`, `.topic-memory-all`) sur la page ; l'écran `.topic-memory-screen` (`#topic-memory-title`, `#topic-memory-search`, `.topic-memory-item` avec `.topic-memory-chip` et `.topic-memory-date`, `.topic-memory-forget`, `.topic-memory-delete`) ; le dialogue `#topic-forget-dialog` ; la carte `[data-card-kind="topic_memory_offer"][data-status]` avec `.topic-memory-quote` et `.topic-memory-link`.
- **Préparation de la mémoire.** Aucun modèle n'est appelé : le parcours pose les offres en attente dans la base jetable (comme l'outil du bot le ferait), puis les retient par la vraie route `/api/topics/memory/accept` et date les faits. Chaque combinaison commence par oublier tous les faits (route d'oubli) et vider les offres.
- **Parcours groupé.** Run `bash .cursor/skills/verify-ezerbot/scripts/drive.sh --evidence <preuves> --sphere sujets`. Les trente-deux PNG (`barre-`, `nouveau-fil-`, `sujet-`, `memoire-`, `oublier-`, `carte-`, `retenu-` et `autre-fil-`, deux thèmes, deux tailles) existent.

## Gotchas

- Les trois captures partent du même état : avant chaque passage, le fil vide créé par « Nouveau fil » est supprimé.
- Les captures `memoire` à `autre-fil` se suivent : `memoire` repart d'une mémoire vide, `oublier` ne confirme pas, `carte` pose une nouvelle offre et `retenu` la retient (cinq faits ensuite).
- La page d'un sujet se ferme dès qu'un fil, l'accueil ou un lien est ouvert (`onNavigate`).
- Les sujets restent sur le Mac : le panneau ne se monte pas depuis l'iPhone appairé.
