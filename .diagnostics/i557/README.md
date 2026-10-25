# Collecte temporaire du cas exact 557

Le workflow pousse seulement sur `pm/ez-mv-i557`. Il mesure le sujet `6aca5529b049d04b3534c97414b329143fbf7a1e` sur `macos-latest`. Le SHA du véhicule reste distinct. Les sources produit locales demeurent celles de c721. Ce véhicule ne crée ni PR ni revue et ne modifie aucun compteur de #515.

La séquence comporte une référence propre, une configuration instrumentée puis un témoin restauré utilisant l'exécutable propre archivé. Chaque configuration exécute une fois le cas exact. Les trois résultats conservent leurs codes, compteurs et charges. Une erreur de préparation arrête la séquence. Un échec attendu de l'assertion de fluidité est enregistré et maintient le job rouge après export. La cible complète et toute nouvelle poussée attendent le coordinateur.

`run.py` ne transmet aux fixtures qu'une liste fermée de variables nécessaires à Rust, au dossier isolé et au capteur. Les credentials Actions et les variables GITHUB ne sont pas transmis. Le verrou Cargo est propre au job. Chaque commande Cargo est séquentielle et bornée. Aucun exécutable Intel n'est transporté.

Les logs bruts, exécutables, fichiers d'auth factices et diagnostics d'erreurs restent privés dans la VM. `export.py` reconstruit les métadonnées par schéma, copie seulement les TSV numériques et les analyses validées, puis contrôle les reçus de flush et les empreintes. Une première passe Gitleaks valide la charge utile. Le reçu de scan et le manifeste sont ajoutés, puis une seconde passe contrôle tous les octets finalement uploadés. Toute incohérence bloque l'upload. Aucun fichier de log ne fait partie du dossier exporté.

Le fichier `receipt.json` distingue résultat produit, préparation et validité de la collecte. `finalize.py` remet le job en échec pour tout rouge, même si l'export a réussi. Le succès du job ne représente jamais un workspace vert.

Le capteur et ses limites sont conservés. Les maxima seq décrivent le vecteur retenu sans prouver individuellement tous ses membres. Les intervalles COMMIT chevauchés restent ambigus. La collecte ajoute des horloges, allocations et écritures de fin de thread. Un job dédié ne recrée pas automatiquement la charge des anciens runs workspace. Sans saut supérieur à 300 ms, l'attribution reste inconnue.

Les pins officiels checkout v4.2.2 et upload-artifact v4.6.2 ont été relus via leurs refs Git. Le scanner Gitleaks 8.30.1 Darwin ARM64 est contrôlé par le SHA-256 du manifeste officiel. Le label runner n'est pas une image historique figée, comme le précise la [documentation GitHub des runners](https://docs.github.com/en/actions/reference/runners/github-hosted-runners).
