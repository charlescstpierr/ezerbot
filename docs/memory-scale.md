# Mémoire à l’échelle (locale)

## Rappel ciblé

Le contexte contient un aperçu des faits récents (2 000 caractères, enregistrements JSON entiers), le nombre de faits actifs et une instruction de recherche. Il ne contient plus tout le catalogue. L’outil `recall_memory { query, limit }` cherche dans les faits privés du bot courant, les faits d’équipe locale et, si le fil est rangé dans un sujet, les faits de ce sujet (portée `topic`), classés ensemble. Il renvoie texte exact, identifiant et provenance ; les faits remplacés, oubliés ou privés d’un autre bot sont exclus avant le classement.

Recherche lexicale locale, sans réseau ni embeddings : mots exacts prioritaires, sous-chaînes ensuite, accents français ignorés, dates puis identifiants pour départager. `query` accepte 1–300 caractères, `limit` 1–20 ; les résultats ont un budget d’environ 12 000 caractères. Le modèle doit reformuler avec des mots-clés/synonymes en cas de résultat vide. Ce n’est pas une garantie de rappel sémantique ni de justesse d’un modèle particulier.

## Apprentissage et partage

L’apprentissage automatique ne s’arrête plus quand le catalogue grandit. Le petit modèle ne reçoit pas tout le catalogue : avant l’appel, `recall::search` (le même rappel lexical local que `recall_memory`) retient au plus **20 faits proches** du message, et c’est seulement avec eux que le modèle déduplique. Le prompt a donc la même taille à 5 ou à 500 faits. Un message qui répète un fait connu ne produit ni doublon ni nouveau reçu (le texte identique est aussi écarté localement, parmi tous les faits actifs, avant l’écriture). Le panneau n’affiche plus d’avis de pause ; le rappel, la correction et l’oubli continuent à fonctionner et aucune consolidation destructive n’a lieu.

Limite assumée : un fait que le rappel lexical ne rapproche pas du message (aucun mot en commun) n’est pas montré au modèle, donc une reformulation sans mot commun peut créer un fait voisin, que l’on peut oublier ou corriger dans le panneau.

Le jeton de routage **`@everyone` en texte brut** est retiré avant l’analyse, aux limites de mots, comme une mention enrichie. Les adresses e-mail, secrets, formulaires et autres contenus exclus restent filtrés.

Chaque nouvel apprentissage propose une carte **Partager à l’équipe ?** dans le fil. Seuls les faits cochés puis confirmés sont promus. « Équipe » désigne **les bots de cet espace local**, pas une équipe multi-comptes. Les données sensibles ne sont jamais partageables.

Le tri est une heuristique locale prudente et visible : personnel (prioritaire), travail, à vérifier. Seul le travail est présélectionné ; les préférences personnelles et les cas inconnus restent privés par défaut. Une sélection explicite peut partager un fait personnel non sensible. Ce tri n’est pas une détection exhaustive des données personnelles. Les confirmations et lots restent idempotents dans `memory_promotions`.

## Profil, versions, oubli

Le panneau propose un profil consolidé en trois rubriques (travail, personnel, à vérifier), trois faits récents par rubrique et les compteurs complets. `memory_profile_log` conserve les versions, actions, dates et **références** des neuf faits récents du résumé de chaque version. Les anciens catalogues reçoivent une version initiale à la migration. L’interface expose les huit dernières versions ; le journal SQLite est durable.

Aucun texte n’est dupliqué dans ce journal ni dans `memory_share_offers`. Les résumés historiques sont résolus à partir des seuls faits encore actifs et visibles : une correction ou un oubli retire immédiatement le texte de toutes les versions et cartes. Les mécanismes préexistants d’expurgation des traces et de lignée restent en place. Ce sont des résumés extractifs locaux, pas une nouvelle synthèse par modèle ni des fichiers exportés sur disque.

## Panneau

Recherche instantanée, insensible aux accents et à la casse, combinable avec portée (privé/équipe) et provenance. Le compteur indique les résultats sur le total. La sélection collective ne concerne que les faits de travail visibles, non sensibles et non déjà partagés. Modifier, oublier et le partage manuel restent disponibles.

## Tests ciblés

```sh
cargo test -p grokbot-memory --lib
cargo test -p grokbot-wrapper --test memory --test memory_auto --test memory_forget -- --test-threads=1
node --test app/web/src/memory-ui.test.js app/web/src/memory-scroll.test.js
cargo clippy -p grokbot-memory -p grokbot-wrapper --all-targets -- -D warnings
npm --prefix app/web run build
```

Les tests couvrent 500 faits, l’absence du fait ancien dans le premier prompt, l’aller-retour Responses de `recall_memory` avant une réponse, les portées, l’oubli, les versions, les faits proches du prompt d’apprentissage à 500 faits, `@everyone` simple/enrichi et en groupe, ainsi que les contrôles UI. Le fournisseur des tests est simulé ; ils ne constituent pas un essai avec un compte réel. `app/ui/chat.js` doit toujours être régénéré, jamais édité à la main.
