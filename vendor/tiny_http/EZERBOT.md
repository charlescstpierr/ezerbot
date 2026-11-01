# tiny_http 0.12.0, corrigé pour Ezerbot

Copie de `tiny_http` 0.12.0 (crates.io, la dernière version publiée), branchée par
`[patch.crates-io]` dans le `Cargo.toml` racine. Seul `src/util/task_pool.rs` change ;
les exemples, bancs et tests du paquet d’origine sont retirés.

## Le défaut

Chaque connexion TCP occupe un fil du pool tant qu’elle reste ouverte (keep-alive).
`TaskPool::spawn` mettait la connexion en file dès qu’un fil était compté « au repos ».
Ce compteur ne baissait qu’au moment où le fil réveillé reprenait le verrou : quand
plusieurs connexions arrivaient avant ce réveil (Chrome en ouvre six d’un coup), elles
voyaient toutes le même fil libre. Les connexions en trop restaient dans la file, sans
fil pour lire leur requête, tant que les autres connexions keep-alive restaient ouvertes.

Symptôme : une feuille de style ou un script de `index.html` ne répond jamais, les
scripts `defer` ne s’exécutent pas et l’app reste vide. Visible surtout sur une machine
chargée (CI Linux : `skill-folder-import.test.cjs`, « bouton introuvable »).

## La correction

La file et le nombre de fils au repos encore libres sont sous le même verrou. Mettre une
tâche en file réserve un fil libre ; sans fil libre, un nouveau fil est créé.

Pour revenir à la version publiée : retirer `[patch.crates-io]` et `exclude` du
`Cargo.toml` racine, puis ce dossier, quand une version de tiny_http corrige le pool.
