# Horaires : calcul livré vérifié aux changements d’heure

Le calcul candidat livré dans Grok Bot .70 peut produire deux instants pour une heure locale répétée à l’automne. Il saute l’heure inexistante du printemps et renvoie `null` si la prochaine occurrence dépasse sa recherche de 366 jours. Ces comportements, déduits dans [CR11](cron-detail.md), sont maintenant observés dans des fonctions originales exécutées sur des dates explicites.

Cette preuve concerne le calcul candidat du client. Son utilisation pour déclencher les routines réelles n’est toujours pas établie. Elle ne donne pas la politique de rattrapage, de déduplication ou de changement d’heure du scheduler serveur.

## Source et capture

La [sonde](../tools/probe-cron-candidate-slices.mjs) parse le membre brut `dist/renderer/assets/index.eager-app-BfVhIMYf.js` de l’ASAR 0.70.2, vérifie son SHA-256 contre le manifeste, puis sélectionne 26 fonctions ou déclarations exactes. Le fichier complet n’est jamais évalué. Les textes sélectionnés, complétés par les séparateurs nécessaires aux déclarations isolées, s’exécutent dans Node VM avec `Date` et `Intl`. Aucun compte, gateway ou ordonnanceur n’est appelé.

REA capture cette exécution sous l’Evidence `ev_ec367c9e76a03287bd7c8cc1e891d989cf2b9d47671f790d1136a8d95dd608ab`. La [capture complète](../evidence/cron-candidate-capture.json) contient les 10 frames PTY, soit 9 709 octets, et les 10 rendus, sans troncature. Le processus sort avec code 0, le fournisseur indique `quiesced`, groupe possédé vérifié et répertoire temporaire retiré. Trois processus non attribués sont laissés intacts. Deux échantillons de processus sont retenus ; ils ne constituent pas une trace exhaustive d’accès système.

Les [résultats](../evidence/cron-candidate-results.json) conservent les hashes du membre, du manifeste, de la sonde et de chacune des 26 tranches. Ils distinguent les positions UTF-16 des plages d’octets. L’environnement est Node 24.18.0, ICU 78.3 et tzdata 2026b. Les cas cron emploient tous une zone explicite, même si la zone de l’hôte est America/Toronto.

Node VM n’est pas présenté comme une sandbox. Le fournisseur ne surveille ni le réseau ni les appels système et n’attribue pas lui-même un hash de cible à cette observation. Le lien au code livré vient des contrôles de source de la sonde et du vérificateur. L’implémentation `Intl` de Node est distincte de celle du Chromium embarqué par l’app originale.

## Résultats qui précisent les limites

| Calcul et référence | Résultat observé | Conséquence pour ce calcul |
|---|---|---|
| `30 2 * * *`, New York, juste avant le saut du 8 mars 2026 | 9 mars à 02:30 locale, soit `2026-03-09T06:30Z` | Aucune minute 02:30 n’existe le 8 mars dans cette zone |
| `30 1 * * *`, New York, avant le repli du 1er novembre 2026 | `05:30Z`, puis `06:30Z` lorsque la référence avance au premier résultat | Les deux instants ont l’heure locale 01:30 ; ce calcul n’élimine pas le second |
| `0 0 29 2 *`, UTC, référence au 1er janvier 2026 | `null` | Le prochain 29 février, en 2028, dépasse l’horizon de recherche |
| Même expression, référence au 1er janvier 2028 | 29 février 2028 à 00:00Z | `null` n’était pas un rejet de la syntaxe du 29 février |
| Jour du mois 13 et lundi tous deux restreints | Mardi 13 octobre, puis lundi 19 octobre 2026 | Les deux champs sont reliés par une disjonction dans ce calcul |
| Dimanche écrit `7` | Dimanche 18 octobre 2026 | `7` est normalisé vers dimanche |
| Minute courante exacte ou à 59,999 secondes | Minute strictement suivante | L’instant de référence n’est jamais renvoyé comme occurrence cron |
| `@every 0.5s` | Référence plus 500 ms | Les intervalles acceptent les composantes décimales |
| `@every 1h/15m`, référence exactement à 00:15Z | 01:15Z | La phase s’aligne sur l’époque et avance strictement |

Les quatre autres cas vérifient le rejet du préfixe timezone avec `@every`, d’une zone explicite inconnue et d’une expression à six champs, ainsi que l’alias `@DAILY` insensible à la casse. Les 16 entrées et sorties exactes figurent dans les résultats.

## Ce qui ne change pas dans la conclusion serveur

Le seul appel direct nommé `mne` retrouvé dans ce bundle reste celui de la description d’un intervalle horaire avec phase, à [A:31243](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:31243). Ce chemin emploie la branche intervalle. La présence et l’exécution contrôlée des fonctions cron n’établissent pas leur emploi par l’ordonnanceur du produit. Les appels dynamiques ou distants restent hors de cette recherche nommée.

`null` décrit le résultat de la fonction dans un cas et un environnement donnés. Il ne démontre ni une erreur de création de routine, ni une prochaine date serveur absente. De même, deux instants candidats ne prouvent pas deux départs réels.

La [vérification hors ligne](../tools/verify-cron-candidate-slices.py), `python3 tools/verify-cron-candidate-slices.py`, contrôle les frames, le scénario capturé, les sources, les tranches et les sorties conservées. Elle ne réévalue pas la cible. La preuve manquante reste un enregistrement d’occurrences serveur, corrélé à la zone effective, à une clé d’occurrence et aux départs réellement exécutés.
