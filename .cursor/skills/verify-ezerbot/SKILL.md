---
name: verify-ezerbot
description: "Lance Ezerbot isolé (grokbot-wrapper, interface web locale) avec un faux OpenAI, le pilote dans Chrome comme un utilisateur et produit un verdict PASS ou FIX avec captures. À utiliser pour prouver un changement visible d’Ezerbot, pour /verify-ezerbot, ou quand un agent doit montrer le vrai parcours plutôt qu’un test unitaire."
---

# Vérifier Ezerbot

Surface principale : l’interface web locale servie par `grokbot-wrapper` (coque `app/ui`, fil construit depuis `app/web`). Le binaire écoute seulement `127.0.0.1`. L’app Cocoa `Ezerbot.app` n’est pas la surface de cette skill : ne pas l’ouvrir.

Deux instances peuvent tourner ensemble si chacune a son port, son `HOME` et sa base. Ne jamais piloter une instance que ce run n’a pas lancée. Ne jamais utiliser un compte ChatGPT réel.

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

Puis, avec `HOME` et `CFFIXED_USER_HOME` pointés sur un répertoire neuf (`mktemp -d`) :

```bash
target/debug/grokbot-wrapper --port 0 --db <throwaway>/workspace.sqlite
```

L’instance est prête quand sa première ligne stdout est `{"url":"http://127.0.0.1:<port>"}` et que `GET /` contient `<title>Ezerbot</title>`. Le badge `ChatGPT` apparaît ensuite : le jeton fictif est lu, le catalogue vient du faux OpenAI. Tant que le badge reste `Local`, l’instance n’est pas utilisable pour les sphères qui dépendent du compte.

Le lanceur refuse de démarrer si `HOME` résout vers le vrai dossier personnel (le défaut Mac, le home du compte Unix, ou `HOME` au moment de l’appel) ou si un chemin touche `~/Library/Application Support/Grokbot`. `Grokbot Local` sous le répertoire jetable n’est pas ce dossier. Les preuves sont hors du dépôt et hors de ce `HOME`.

Le faux serveur n’écoute que `127.0.0.1`. `GROKBOT_FAKE_OPENAI` reçoit son URL. Le fichier `<throwaway>/workspace.chatgpt-auth.json` (mode `0600`) contient uniquement `fake-access-verify` / `fake-refresh-verify`, le scope `chatgpt.tokens.use.direct`, l’e-mail `vous@exemple.com` et `welcomed: true`. Aucune variable d’identifiant du parent n’est transmise au wrapper.

Invocation :

```bash
CHROME_DEVTOOLS_AXI_SESSION=verify-ezerbot \
  bash .cursor/skills/verify-ezerbot/scripts/launch.sh --evidence <dossier-de-preuves>
```

Le script écrit `<preuves>/run.json` (pid, port, URL, commit, empreinte du binaire). Il ne lance pas `Ezerbot.app`.

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
- le faux OpenAI lancé par ce run est encore vivant.

La dernière ligne est `doctor: ok` ou `doctor: echec`.

## Drive

Harness : `chrome-devtools-axi`, avec `CHROME_DEVTOOLS_AXI_SESSION` égal à l’identifiant du run. Les commandes observées dans ce dépôt sont `open`, `resize`, `screenshot`, `eval`, `pages`, `selectpage`, `newpage`, `closepage`. `eval` imprime `result: <json>`. Si cette valeur est une chaîne qui contient encore du JSON, elle est décodée jusqu’à l’objet.

```bash
bash .cursor/skills/verify-ezerbot/scripts/browser.sh --evidence <preuves> open http://127.0.0.1:<port>
bash .cursor/skills/verify-ezerbot/scripts/browser.sh --evidence <preuves> resize 1140 780
bash .cursor/skills/verify-ezerbot/scripts/browser.sh --evidence <preuves> eval '() => document.title'
bash .cursor/skills/verify-ezerbot/scripts/browser.sh --evidence <preuves> screenshot <preuves>/spheres/accueil/dark-1140x780.png
```

Le substitut `EZERBOT_VERIFY_BROWSER=chrome-headless` parle le même vocabulaire via le protocole Chrome. Il ne remplace pas une session `chrome-devtools-axi` sur le Mac. Il sert quand ce binaire n’existe pas (preuve Linux). Il lance `google-chrome-stable` (pas un wrapper qui épingle un profil du home réel), profil `--user-data-dir=<HOME jetable>/chrome-profile` en un seul argument, sans URL initiale, avec `--headless=new`. Ce Chrome est un processus du run, tué au nettoyage. La session `chrome-devtools-axi` de ce run est arrêtée à la fin, et seulement elle : `CHROME_DEVTOOLS_AXI_SESSION=verify-ezerbot chrome-devtools-axi stop`. Aucune autre session n’est arrêtée, et aucun processus n’est tué par son nom.

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

Chaque sphère est capturée en sombre et en clair, à `1140x780` et à `820x560`. Le thème est choisi par le bouton des Paramètres, puis rouvert pour lire `aria-pressed`. La couleur vient de `--canvas` (`#14161a` sombre, `#ffffff` clair).

## Evidence

Les preuves vont dans le dossier donné par l’appelant. Défaut : sous le data root Mac, sinon un dossier temporaire hors dépôt. Elles ne sont jamais commitées.

Pour chaque capture : le PNG, le texte visible (`.txt`) et l’action qui a mené à cet écran. Une sphère n’est pas prouvée par le dernier écran seulement : le doctor, le badge `ChatGPT` et l’état DOM attendu comptent. Le faux OpenAI est la frontière déjà prévue par les builds de développement (`GROKBOT_FAKE_OPENAI`) ; il ne contacte pas OpenAI. Le vérifier en observant que le processus n’a pas d’autre socket que `127.0.0.1`.

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

Ne jamais tuer par nom de processus. Le nettoyage ne signale `SIGTERM` puis `SIGKILL` qu’aux PID dont l’identité (instant de démarrage et ligne de commande) est encore celle enregistrée : wrapper, faux OpenAI, et Chrome seulement si ce run l’a lancé. Le groupe de processus n’est tué que si `os.getpgid` confirme le pgid enregistré et que ce n’est pas le groupe de l’appelant. Un `HOME` ou un `run_root` vide est refusé (ce n’est pas le répertoire courant). La suppression ne vise qu’un répertoire `verify-ezerbot.*` créé par ce run : marqueur HMAC dans `run.json` et sur le disque, sous le `TMPDIR` du système, et `assert_throwaway` sur `run_root` comme sur `HOME`. À la fin d’un nettoyage accepté, et seulement alors, la session `verify-ezerbot` est arrêtée par `chrome-devtools-axi stop`. Un nettoyage refusé n’envoie pas cette commande. Le dossier de preuves, y compris `report.md` et les PNG, reste.

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
