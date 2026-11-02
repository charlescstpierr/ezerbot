# Vérifier un bundle Linux sans compiler

Ouvrez une exécution réussie de [Bundle Linux](https://github.com/charlescstpierr/ezerbot/actions/workflows/linux-bundle-artifact.yml).
Vérifiez le dépôt, la branche et le SHA source attendu. Téléchargez l'artefact
`ezerbot-linux-x86_64-ubuntu-24.04-<SHA>` depuis cette exécution, avec une connexion GitHub.
Décompressez le ZIP GitHub dans un dossier vide. Il contient une archive `.tar.gz`,
`manifest.json` et `SHA256SUMS`.

Dans ce dossier, vérifiez les sommes avant d'extraire :

```sh
sha256sum --check SHA256SUMS
cat manifest.json
source_sha=SHA_COMPLET
tar -xzf "ezerbot-linux-x86_64-ubuntu-24.04-${source_sha}.tar.gz"
```

Remplacez `SHA_COMPLET` par le SHA attendu. Vérifiez que `sourceSHA` et `runUrl` dans
le manifeste correspondent à la tête de la branche et à l'exécution. Pour une PR, le bundle utilise sa tête,
pas le commit de fusion synthétique de GitHub. Le manifeste décrit la distribution,
l'architecture, la taille et l'empreinte de l'archive, puis les chemins relatifs,
types, modes, tailles et empreintes de ses fichiers.

Pour une QA avec des données séparées, lancez depuis ce même dossier :

```sh
qa_dir=$(mktemp -d)
"./linux/Ouvrir Ezerbot.sh" --data-dir "$qa_dir"
```

Le bundle contient tout `app/build/linux`, avec ses modes exécutables. Sur la base
`b070306b`, il n'a pas d'installateur et son fichier `.desktop` pointe vers le runner.
Utilisez le lanceur Bash. Après fusion de la PR #648, le workflow embarquera aussi
l'installateur produit par le script de build, sans liste de fichiers à modifier.

Le build release cible Ubuntu 24.04 x86_64 avec les bibliothèques dynamiques GTK 3,
WebKitGTK 4.1 et PipeWire de cette distribution. Elles ne sont pas embarquées.
Un bureau graphique et ses services sont nécessaires. Les autres distributions,
anciennes glibc, ARM64, les fonctions natives et les conversations ne sont pas
validés par ce job. Le faux fournisseur de développement est absent en release.

Le workflow tourne sur les pushes de `main`, les PR qui changent le workflow ou
le script de build, et sur demande. Il limite le job à 45 minutes, le build à
35 minutes et le bundle à 1 Gio avant compression. Chaque fichier de sortie est
limité à 1 Gio. Le script de build exige 25 Gio libres au départ. Aucun cache de
compilation n'est conservé. Les artefacts expirent après 7 jours.

Le job ouvre le tar.gz et compare tous les chemins, types, modes, tailles et
empreintes au dossier construit avant l'upload. Un échec interdit cet upload.
Le SHA et les sommes permettent de contrôler la provenance du run et l'intégrité
du téléchargement. Ils ne constituent pas une signature. Cet artefact temporaire
sert à la QA ; il ne crée ni release publique ni mise à jour automatique.
