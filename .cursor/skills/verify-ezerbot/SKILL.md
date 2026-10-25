---
name: verify-ezerbot
description: "Lance Ezerbot isolé (grokbot-wrapper, interface web locale) avec un faux OpenAI, le pilote dans Chrome comme un utilisateur et produit un verdict PASS ou FIX avec captures. À utiliser pour prouver un changement visible d’Ezerbot, pour /verify-ezerbot, ou quand un agent doit montrer le vrai parcours plutôt qu’un test unitaire."
---

# Vérifier Ezerbot

Surface principale : l’interface web locale servie par `grokbot-wrapper` (coque `app/ui`, fil construit depuis `app/web`). Le binaire écoute seulement `127.0.0.1`. L’app Cocoa `Ezerbot.app` n’est pas la surface de cette skill : ne pas l’ouvrir.

Deux instances peuvent tourner ensemble si chacune a son port, son `HOME`, sa base et sa session Chrome dédiée. Ne jamais piloter une instance que ce run n’a pas lancée. Ne jamais utiliser un compte ChatGPT réel.

Les chemins du Mac du capitaine sont les défauts ci-dessous. Chaque chemin s’écrase avec une variable d’environnement.

| Variable | Défaut |
|---|---|
| `EZERBOT_VERIFY_REAL_HOME` | `/Users/cstpierrecharles` |
| `EZERBOT_VERIFY_APP_SUPPORT` | `/Users/cstpierrecharles/Library/Application Support/Grokbot` |
| `EZERBOT_VERIFY_DATA_ROOT` | `/Users/cstpierrecharles/firstmate/data/gb-verify-skill` |
| `EZERBOT_VERIFY_EVIDENCE_DIR` | `$EZERBOT_VERIFY_DATA_ROOT/evidence/<session>` quand ce home existe, sinon un dossier hors dépôt sous `$TMPDIR` |
| `EZERBOT_VERIFY_REF_SHOTS` | `/Users/cstpierrecharles/firstmate/data/gb-visual-qa/shots` |
| `EZERBOT_VERIFY_REF_ANNEX` | `/Users/cstpierrecharles/firstmate/data/gb-visual-qa2/report.md` |
| `EZERBOT_VERIFY_REA_REPORTS` | `<dépôt>/reports` |
| `EZERBOT_VERIFY_REA_EVIDENCE` | `<dépôt>/evidence` |
| `EZERBOT_VERIFY_REA_FORMATTED` | `<dépôt>/artifacts/formatted` |
| `EZERBOT_VERIFY_CARGO` | `~/.cargo/bin/cargo` |
| `EZERBOT_VERIFY_CARGO_GATE` | vide, chemin du verrou Cargo quand le Mac le requiert |
| `EZERBOT_VERIFY_AXI` | `~/.local/bin/chrome-devtools-axi` |
| `EZERBOT_VERIFY_CHROME` | `/Applications/Google Chrome.app/Contents/MacOS/Google Chrome` |
| `CHROME_DEVTOOLS_AXI_SESSION` | obligatoire, identifiant du run |
| `EZERBOT_VERIFY_PATCH_ID` | vide ; si présent, recopié tel quel en `patch=<id>` |
| `EZERBOT_VERIFY_BROWSER` | `chrome-devtools-axi` |

## Launch

Depuis la racine du dépôt. Le build n’exporte jamais `CARGO_TARGET_DIR`. Un `CARGO_HOME` étranger est retiré seulement pour le binaire par défaut, afin que `~/.cargo/bin/cargo` utilise le toolchain de ce dossier.

```bash
~/.cargo/bin/cargo build --locked -p grokbot-wrapper
```

Le lanceur crée un `HOME` jetable et y pointe `HOME` et `CFFIXED_USER_HOME`. Il génère 32 octets cryptographiques par lancement et les transmet au wrapper par un canal stdin privé fermé après écriture. Un pipe anonyme ou une paire de sockets Unix anonymes convient. Les sockets réseau et les fichiers réguliers sont refusés. Un lancement HTTP sans ce canal échoue avant l'ouverture de la base et du listener. Les modes CLI sans listener restent indépendants.

Le secret reste en mémoire dans le wrapper et un gardien de navigateur. Il ne passe ni par argv, ni par l'environnement, ni par une URL, ni par un fichier ou une sortie. Le gardien reçoit les mêmes octets par un second pipe. Son interception CDP ajoute `X-Ezerbot-Launch` seulement à l'origine exacte du wrapper, port compris, pour les fetch, médias et navigations. Il refuse les redirections authentifiées et désactive le cache. Il n'expose aucune route de récupération du secret.

Les GET des assets de la liste fermée restent publics après contrôle Host. Les API locales exigent l'identité, même en GET ou HEAD. iPhone et webhook gardent leurs protections sur des listeners distincts.

L’instance est prête quand sa première ligne stdout est `{"url":"http://127.0.0.1:<port>"}` et que `GET /` contient `<title>Ezerbot</title>`. La pilule Modèle `#composer-tools [data-picker="model"]` apparaît ensuite : le jeton fictif est lu, le catalogue vient du faux OpenAI. Tant qu’elle est absente, l’instance n’est pas utilisable pour les sphères qui dépendent du compte.

Le lanceur refuse de démarrer si `HOME` résout vers le vrai dossier personnel (le défaut Mac, le home du compte Unix, ou `HOME` au moment de l’appel) ou si un chemin touche `~/Library/Application Support/Grokbot`. `Grokbot Local` sous le répertoire jetable n’est pas ce dossier. Les preuves sont hors du dépôt et hors de ce `HOME`.

Le faux serveur n’écoute que `127.0.0.1`. `GROKBOT_FAKE_OPENAI` reçoit son URL ; il sert aussi le catalogue `/backend-api/codex/models`. Le fichier `<throwaway>/workspace.codex-auth.json` (mode `0600`) contient une session factice : un JWT sans signature pour `account-fake`, `fake-refresh-verify`, l’e-mail `vous@exemple.com` et `welcomed: true`. Aucune variable d’identifiant du parent n’est transmise au wrapper.

Invocation :

```bash
CHROME_DEVTOOLS_AXI_SESSION=verify-ezerbot \
  bash .cursor/skills/verify-ezerbot/scripts/launch.sh --evidence <dossier-de-preuves>
```

Le script écrit `<preuves>/run.json` (pid, port, URL, commit, empreinte du binaire). Il lie aussi la session axi et son `HOME` au répertoire du run par une signature HMAC. Il ne lance pas `Ezerbot.app`.

Choisir un dossier de preuves neuf pour chaque lancement. Un dossier qui contient déjà `run.json`, même après nettoyage, est refusé.

## Doctor

Contrôle en lecture seule, à lancer dès que quelque chose semble faux. Il ne tue rien et n’écrit rien.

```bash
bash .cursor/skills/verify-ezerbot/scripts/doctor.sh --evidence <dossier-de-preuves>
```

Il exige, ensemble :

- le PID du wrapper est vivant et sa ligne de commande est encore celle enregistrée au lancement ;
- le port annoncé est en écoute et appartient à ce PID ;
- `git rev-parse --verify HEAD^{commit}` égale le commit enregistré au build, et `git cat-file -e <sha>^{commit}` réussit ;
- l’empreinte du binaire n’a pas changé ;
- les sources compilées (`crates/`, `app/ui/`, `app/web/`, manifestes Cargo) étaient propres au build ;
- `HOME` est toujours le répertoire jetable ;
- la page répond avec le titre Ezerbot ;
- le faux OpenAI et le gardien authentifié lancés par ce run sont encore vivants ;
- un GET externe de `/api/state` sans identité donne 403.

La dernière ligne est `doctor: ok` ou `doctor: echec`.

## Drive

Harness : `chrome-devtools-axi`, avec `CHROME_DEVTOOLS_AXI_SESSION` égal à l’identifiant du run. Les commandes observées dans ce dépôt sont `open`, `resize`, `screenshot`, `eval`, `pages`, `selectpage`, `newpage`, `closepage`. `eval` imprime `result: <json>`. Si cette valeur est une chaîne qui contient encore du JSON, elle est décodée jusqu’à l’objet.

Utiliser `browser.sh` pour attribuer la session axi au run. Le pilote valide la session signée et le `HOME` axi du lancement avant toute commande. Il enregistre cette utilisation dans `browser.json` sous le répertoire signé. Un changement de session ou de `HOME` est refusé.

Lancement, commandes navigateur et nettoyage partagent un verrou conservé dans le dossier de preuves. Les métadonnées sont publiées par remplacement atomique. Le nettoyage attend la fin d'une commande déjà commencée. Dès que le nettoyage commence, le pilote refuse toute nouvelle commande et tout changement de mode navigateur.

```bash
bash .cursor/skills/verify-ezerbot/scripts/browser.sh --evidence <preuves> open http://127.0.0.1:<port>
bash .cursor/skills/verify-ezerbot/scripts/browser.sh --evidence <preuves> resize 1140 780
bash .cursor/skills/verify-ezerbot/scripts/browser.sh --evidence <preuves> eval '() => document.title'
bash .cursor/skills/verify-ezerbot/scripts/browser.sh --evidence <preuves> screenshot <preuves>/spheres/accueil/dark-1140x780.png
```

Sur Mac, le lanceur crée un Chrome propre et son port CDP. Le gardien authentifié reste connecté à ce Chrome. Le pilote `chrome-devtools-axi` s’y connecte avec la session et le HOME axi signés du run. Il conserve les PID et le port du navigateur dans son attribution. Aucun secret de lancement n’est transmis à axi en argument ou dans son environnement.

Le substitut `EZERBOT_VERIFY_BROWSER=chrome-headless` parle le même vocabulaire via le protocole Chrome. Il ne remplace pas une session `chrome-devtools-axi` sur le Mac. Il sert quand ce binaire n’existe pas (preuve Linux). Il lance `google-chrome-stable` (pas un wrapper qui épingle un profil du home réel), profil `--user-data-dir=<HOME jetable>/chrome-profile` en un seul argument, sans URL initiale, avec `--headless=new`. Ce Chrome est un processus du run, tué au nettoyage. Un run headless n’arrête aucune session axi. Aucun processus n’est tué par son nom.

Les sélecteurs stables viennent de `app/ui/index.html`, `app/ui/app.js`, `app/ui/panels/` et `app/web/src` :

- accueil : `#home-link` (« Ezerbot, accueil »), `#welcome`, titre `Une idée en tête`, `#conversation-title` = `Nouvelle conversation`, `.gb-editor[aria-label="Votre message"]` ;
- thème : `#settings-button`, puis `[data-theme-choice="light|dark"]` dans `[aria-label="Thème"]` ; ne pas choisir `system` ;
- connecteurs : `[data-panel="mcp"]`, titre `Connecteurs`, `summary` `Ajouter un serveur MCP` ;
- modèles : `[data-picker="model"]` (`aria-label` commence par `Modèle`), `#picker h4` = `Modèle`, `[role="menuitemradio"]` contient `GPT Test` ;
- réunions : `#meeting-retention` sous le titre `Réunions et rétention audio` ;
- routines : `[data-panel="routines"]`, bouton `Gérer les routines par bot`, puis `#routines-title` = `Routines` ;
- mémoire : `[data-panel="memory"]` (`Mémoire automatique`, `input[name="automatic-grokbot"]`), puis `[aria-label="Modifier Ezerbot"]`, puis `[aria-label="Mémoire du bot"]` et `[aria-label="Rechercher dans la mémoire"]`.

Parcours complet d’une sphère, ou `all` :

```bash
bash .cursor/skills/verify-ezerbot/scripts/drive.sh --evidence <preuves> --sphere accueil
```

Chaque sphère est capturée en sombre et en clair, à `1140x780` et à `820x560`. Le thème est choisi par le bouton des Paramètres, puis rouvert pour lire `aria-pressed`. La couleur vient de `--canvas` (`#0e0f12` sombre, `#ffffff` clair).

## Evidence

Les preuves vont dans le dossier donné par l’appelant. Défaut : sous le data root Mac, sinon un dossier temporaire hors dépôt. Elles ne sont jamais commitées.

Pour chaque capture : le PNG, le texte visible (`.txt`) et l’action qui a mené à cet écran. Une sphère n’est pas prouvée par le dernier écran seulement : le doctor, la pilule Modèle et l’état DOM attendu comptent. Le faux OpenAI est la frontière déjà prévue par les builds de développement (`GROKBOT_FAKE_OPENAI`) ; il ne contacte pas OpenAI. Le vérifier en observant que le processus n’a pas d’autre socket que `127.0.0.1`.

`reunions-routines` produit deux PNG par thème et par taille : `reunions-…png` (rétention) et `routines-…png` (dialogue Routines).

La comparaison lit, sans écrire :

- `reports/**`, `evidence/**`, `artifacts/formatted/**` ;
- les captures `EZERBOT_VERIFY_REF_SHOTS`.

```bash
bash .cursor/skills/verify-ezerbot/scripts/compare.sh --evidence <preuves>
```

Un corpus absent est listé dans `deviations.md`. Ce n’est pas un défaut produit. Un défaut produit va dans `findings.json` (`screenshot`, `defect`, `bloquant` ou `mineur`, `fix`).

Le verdict résout le commit avec `git rev-parse --verify <ref>^{commit}` et `git cat-file -e <sha>^{commit}`. Si l’un des deux échoue, aucune ligne `PASS` ou `FIX` n’est émise.

```bash
bash .cursor/skills/verify-ezerbot/scripts/verdict.sh --evidence <preuves> --ref HEAD --status pass
```

Dernière ligne de la sortie et de `report.md` : `PASS <sha>` ou `FIX <sha>`. Un `patch` d’appelant n’est accepté que s’il est un patch-id git : 40 hexadécimaux minuscules, sans caractère de contrôle. Il est alors recopié tel quel : `PASS <sha> patch=<id>`. Un finding `bloquant` force `FIX`. Une erreur d’outil (`tool-error.json`) force aussi `FIX`. Elle apparaît dans `report.md` sous « Erreur d’outil » et n’est pas un défaut numéroté d’Ezerbot.

## Cleanup

Ne jamais tuer par nom de processus. Le nettoyage ne signale `SIGTERM` puis `SIGKILL` qu’aux PID dont l’identité (instant de démarrage et ligne de commande) est encore celle enregistrée : wrapper, gardien du navigateur, faux OpenAI, et Chrome seulement si ce run l’a lancé. Le groupe de processus n’est tué que si `os.getpgid` confirme le pgid enregistré et que ce n’est pas le groupe de l’appelant. Un `HOME` ou un `run_root` vide est refusé (ce n’est pas le répertoire courant). La suppression ne vise qu’un répertoire `verify-ezerbot.*` créé par ce run : marqueur HMAC dans `run.json` et sur le disque, sous le `TMPDIR` du système, et `assert_throwaway` sur `run_root` comme sur `HOME`.

À la fin d’un nettoyage accepté, la commande `chrome-devtools-axi stop` utilise la session et le `HOME` axi signés du run. Le nettoyage exige aussi un `browser.json` attribué au même run par `browser.sh`. Les variables courantes de l’appelant ne choisissent pas la session à arrêter. Un nettoyage refusé n’envoie pas cette commande.

Sans attribution validable, aucun arrêt axi n’est envoyé. Cela couvre les anciens runs sans signature de session, les runs sans utilisation d’axi et les métadonnées incohérentes ou illisibles. Le nettoyage des PID enregistrés et du dossier signé reste possible. Ne pas déduire une session à fermer de l’environnement. Un second nettoyage n’envoie aucune nouvelle commande. Le dossier de preuves, y compris `report.md` et les PNG, reste.

Si axi est absent ou si son arrêt échoue, le nettoyage échoue et conserve le répertoire signé et l'attribution pour une nouvelle tentative. Le navigateur reste interdit pendant cet état. Relancer `cleanup.sh` avec axi disponible. Après un nettoyage réussi, un second nettoyage n'envoie aucune nouvelle commande.

```bash
bash .cursor/skills/verify-ezerbot/scripts/cleanup.sh --evidence <preuves>
```

Après un échec, lancer ce nettoyage avant de relancer, pour ne pas laisser de port ni de `HOME`.

## Helpers

Tous les scripts vivent dans `.cursor/skills/verify-ezerbot/scripts/` et s’exécutent tels quels.

| Script | Rôle |
|---|---|
| `scripts/launch.sh` | build, faux OpenAI, wrapper isolé |
| `scripts/doctor.sh` | contrôle lecture seule |
| `scripts/browser.sh` | `chrome-devtools-axi` ou substitut headless |
| `scripts/drive.sh` | une sphère, deux thèmes, deux tailles |
| `scripts/compare.sh` | écarts de référence, lecture seule |
| `scripts/verdict.sh` | `report.md` et ligne `PASS` / `FIX` |
| `scripts/cleanup.sh` | arrêt ciblé, `HOME` retiré, preuves gardées |
| `scripts/prove.sh` | enchaînement complet |
| `scripts/verify_lib.py self-check` | refuse le vrai home et le dossier Grokbot |

Carte des sphères : `features/README.md`.

## Preuve sur le Mac

Une seule commande, depuis la racine du dépôt, rejoue la preuve : build, lancement, doctor, sphère accueil en clair et en sombre aux deux tailles, comparaison, verdict, nettoyage, puis contrôle que les PNG sont encore là.

```bash
CHROME_DEVTOOLS_AXI_SESSION=verify-ezerbot \
  bash .cursor/skills/verify-ezerbot/scripts/prove.sh --sphere accueil
```

Elle suppose `~/.cargo/bin/cargo`, `~/.local/bin/chrome-devtools-axi` (ou `PATH`) et Google Chrome. Elle n’ouvre pas l’app du capitaine.

Sous Linux, la même commande bascule toute seule sur `EZERBOT_VERIFY_BROWSER=chrome-headless` et écrit `stand-in.md`. Ce fichier liste ce que le substitut a fait et que le Mac ne fera pas. Ces points restent non prouvés tant que la commande ci-dessus n’a pas tourné sur le Mac.

## Maintenance

Quand l’interface change, mettre à jour la carte `features/` avec `/maintain-verification-skill`. Une preuve qui ne couvre qu’une entrée alors que la carte en liste d’autres est incomplète.
