# Messages .71 : identifiants et pertes de réponse vérifiés avec des fixtures

La reconnexion interne conserve le même `requestID`. Un nouvel appel à l’adaptateur en génère un autre. Une fermeture après l’effet fictif rejette le premier appel ; un second appel atteint de nouveau la frontière remplacée. Ce résultat vérifie le comportement de fonctions originales sélectionnées. Il ne démontre pas un doublon dans Messages réel ni le comportement de toute l’application.

Cette passe complète l’[analyse statique du worker](messages-worker-recovery.md) et le [chemin natif d’envoi](native-messages-recovery.md). REA a capturé le processus Node qui exécute la sonde. Ni le worker complet, ni le helper, ni l’application originale n’ont été lancés par cette sonde.

## Source et exécution contrôlée

Le membre brut `dist/local-exec-daemon/messages-worker.cjs` de la distribution 0.71.2 mesure 50 767 octets, SHA-256 `acf68b6f7e419c830da7cd8d28059da37fd7fbd63e14495acbdd042e089bb5cc`. Son hash est vérifié contre l’[inventaire ASAR](../artifacts/releases/0.71.2/asar-inventory.json). Le parseur `@babel/parser` 8.0.4 déjà installé avec REA sélectionne 16 fonctions ou déclarations de variables au niveau supérieur. Il parse le fichier sans en exécuter le bootstrap.

La [sonde](../tools/probe-messages-worker-slices.mjs) évalue ces tranches exactes dans Node VM, avec leur texte inchangé. Elle ajoute seulement `var` et les séparateurs nécessaires aux déclarations isolées, puis expose trois fonctions. Les [résultats](../evidence/messages-worker-slice-results.json) conservent les plages UTF-16 et octets, les hashes de chaque tranche et du programme assemblé, le hash de la sonde et les requêtes fictives observées.

`Ze`, `xr`, `rn.homedir`, `g.readFile`, `Cr`, `Br`, `kr` et `sn.setTimeout` sont remplacés par des fixtures. Elles renvoient des chemins `/fixture-only`, une configuration fictive et des réponses déterminées. L’effet est un compteur. Aucun fichier texte ni socket de Messages n’est ouvert. Les UUID viennent de `node:crypto.randomUUID`. Le constructeur `Error` du processus est injecté pour conserver le comportement des tests `instanceof` à la frontière des erreurs fictives.

Node VM sert à isoler les dépendances du test. Ni VM ni capture REA ne constituent une garantie de sandbox. La capture ne surveille pas le réseau ni les accès système. Les limites d’exécution reposent ici sur les seules tranches évaluées et les frontières explicitement remplacées.

## Résultats des quatre scénarios

| Scénario | Observation dans la sonde | Portée |
|---|---|---|
| Deux appels indépendants avec les mêmes données et des champs supplémentaires `sendKey` et `requestId` | Deux UUID distincts, `id:1` dans les deux enveloppes, deux effets fictifs. `Hr` ne conserve que `to`, `service` et `textFile` | Projection et absence de cache dans cet adaptateur. Le cache B4 du daemon n’est pas exécuté |
| `ECONNREFUSED` une fois avant l’effet fictif, puis succès | Deux tentatives portent une ligne JSON strictement identique. Une installation et un lancement fictifs, un délai fictif de 200 ms, un effet fictif | Réutilisation de l’enveloppe dans la boucle `Ur`, sans véritable reconnexion réseau |
| Effet fictif puis erreur `sidecar_closed`, suivi d’un nouvel appel identique | Le premier appel rejette sans nouvelle tentative interne. Le second porte un UUID différent et produit le deuxième effet fictif | Le niveau adaptateur ne réconcilie pas l’effet incertain. Aucun doublon du produit réel n’est observé |
| Appels directs à `Vr` | Send : `undefined`. Snapshot : 10 000 ms. Résolution de handles : 4 000 ms. Présence de la propriété `prompt`, même fausse : `undefined` | Sélecteur de délai seulement. L’adaptateur ordinaire ne transmet `prompt` que lorsque sa valeur est vraie |

Le troisième scénario sépare deux notions souvent confondues : une erreur après effet et une preuve d’absence d’effet. Il confirme la frontière décrite par les corps. Les protections B4, le worker_threads, le service Swift et Messages restent hors de cette exécution.

## Capture REA et couverture

Evidence : `ev_e0e22e161c96d6a1db6661fbe68e99e006c3a35204228b02374fce15a8585934`, conservée intégralement dans la [capture](../evidence/messages-worker-slice-capture.json). Le processus Node sort avec code 0. Ses 9 frames PTY, soit 9 075 octets, sont conservées sans perte. Le JSON complet contient quatre scénarios et `assertions_passed:true`.

Le champ global `truncated:true` est conservé : le budget des rendus successifs du terminal a omis deux frames rendues sur neuf. Cela n’a supprimé aucun octet PTY brut. La capture indique `settlement.state:alive_at_deadline` et `exit.reason:exited`; les deux champs sont préservés sans convertir le premier en garantie de stabilité. Elle indique le groupe possédé vérifié et le répertoire temporaire retiré. Quatorze autres processus n’ont pas pu être attribués et ont été laissés intacts. Aucun échantillon de processus n’a été retenu pendant cette exécution courte.

Le fournisseur n’attribue pas lui-même un hash de cible à cette observation, dont le sujet est `null`. Le lien au membre livré vient des contrôles explicites de la sonde et du vérificateur. Cette attribution ne transforme pas la capture en exécution complète du produit.

## Vérification hors ligne

`python3 tools/verify-messages-worker-slices.py` relit le JSON depuis les frames brutes, vérifie son égalité avec les résultats extraits, puis contrôle les hashes et les quatre scénarios. Il n’évalue aucun JavaScript cible et n’utilise pas le réseau. Il ne relance pas la capture.

La preuve répond à la question des identifiants et retries de ces fonctions. La reprise durable de bout en bout, la livraison au destinataire et le comportement après une panne réelle restent à établir.
