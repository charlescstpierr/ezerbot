# Wrapper Mac local

Le wrapper ouvre une vraie fenêtre Mac et conserve les conversations dans SQLite. Les réponses sont simulées et identifiées comme telles dans le composeur et les paramètres. Aucun modèle, compte Grok, connecteur ou service cloud n’est configuré.

## Utiliser et reconstruire

Double-cliquer sur `Ouvrir Grokbot.command`, à la racine du projet, ou sur `app/build/Grokbot.app`. Le lanceur construit le bundle s’il manque. Pour reconstruire après une modification :

```sh
bash tools/build-app.sh
open app/build/Grokbot.app
```

La compilation utilise Cargo installé dans `~/.cargo/bin` et les outils Apple. Le bundle vise macOS 12.3 et l’architecture de la machine qui le construit. Il reçoit une signature ad hoc vérifiée localement; la distribution notarifiée et un build universel ne sont pas établis.

Les données sont dans `~/Library/Application Support/Grokbot/workspace.sqlite`. Les journaux du moteur sont dans le sous-dossier `logs`. Reconstruire l’app ne remplace pas ces données. Les préférences sont persistées par le backend, donc un changement de port au redémarrage ne réinitialise pas le thème.

## Interface et moteur

L’interface est refaite autour d’un galet sombre animé, aux yeux colorés, inspiré de la mascotte de Grokbot, avec des panneaux graphite et une couleur d’accent au choix dans les paramètres. Elle propose un composeur, les suggestions de départ, la recherche, les favoris, le renommage, la suppression, la copie, l’export JSON et les thèmes clair, sombre et système. Les raccourcis comprennent `⌘ N`, `⌘ K` et Entrée pour envoyer. Un premier message donne automatiquement son titre à une nouvelle conversation, sauf si le titre a été choisi explicitement. [Design de la mascotte](mascotte-design.md) décrit les références et les animations.

Le fil de conversation et le champ de message sont dessinés par un bundle React construit depuis `app/web` (`npm run build`), qui écrit `app/ui/chat.js` et `app/ui/chat.css`. Les réponses passent par `streamdown` (Markdown), avec `shiki` pour le code, `mermaid` pour les diagrammes et `katex` pour les formules; le champ de message utilise Tiptap en texte brut. `app.js` garde l’état, les envois et les brouillons, et appelle `window.GrokbotChat`. Les deux fichiers construits sont versionnés pour que `cargo build` fonctionne sans Node. Le moteur simulé renvoie un diagramme, du code ou un tableau quand le message contient l’un de ces mots.

## Abonnement ChatGPT

`crates/wrapper/src/provider.rs` implémente le parcours officiel [Sign in with ChatGPT pour les applications open source et locales](https://developers.openai.com/siwc/token-sharing-open-source) : enregistrement dynamique du client, PKCE, retour sur `127.0.0.1:1455` à `1465`, vérification du jeton d’identité contre le JWKS d’OpenAI, renouvellement sérialisé et révocation à la déconnexion. Les identifiants sont écrits dans `<base>.chatgpt-auth.json`, à côté de la base SQLite, en mode `0600`. Le catalogue vient de `GET /v1/models` avec le jeton du compte; aucun nom de modèle n’est codé en dur. Les réponses passent par `POST /v1/responses` avec `store: false` et `stream: true`, dans un fil d’arrière-plan qui écrit le texte partiel dans `wrapper_remote`; l’interface relit `/api/state` tant qu’une réponse est en cours. `wrapper_generation` porte l’état (`streaming`, `done`, `failed`, `stopped`), l’erreur et une note. `/api/stop` et `/api/regenerate` arrêtent et relancent une réponse. Après la première et la troisième réponse d’une conversation, une seconde demande au même modèle écrit un titre de 3 à 6 mots tiré du sujet; un titre choisi par l’utilisateur n’est jamais remplacé. Sans connexion, le moteur reste simulé. `crates/wrapper/tests/chatgpt.rs` rejoue tout le parcours contre un faux serveur OpenAI; aucun essai n’a été fait avec un vrai compte. Un build de développement accepte `GROKBOT_FAKE_OPENAI` pour viser un faux serveur; un build de production ignore cette variable.

[app/native/main.m](../app/native/main.m) héberge la page dans WKWebView, démarre un seul moteur enfant et l’arrête à la fermeture de l’app. Il n’expose pas de pont JavaScript vers les capacités du Mac. L’export utilise le dialogue natif d’enregistrement. Les ressources [HTML](../app/ui/index.html), [CSS](../app/ui/styles.css) et [JavaScript](../app/ui/app.js) sont embarquées dans le binaire Rust; une reconstruction est nécessaire pour intégrer leurs modifications.

[grokbot-wrapper](../crates/wrapper/src/lib.rs) expose une API HTTP liée uniquement à `127.0.0.1`, sur un port dynamique. Il utilise le vrai journal durable et [SubmissionSession](../crates/session/src/submission.rs). Les titres, favoris, préférences et acceptations du transport simulé sont aussi stockés dans SQLite. Les messages restent immuables; répéter le même identifiant ne provoque pas un second envoi logique.

Dans les paramètres, « Tester une coupure » simule une réponse perdue après acceptation. Le message apparaît comme incertain. « Vérifier l’envoi » cherche son statut avant toute reprise et retrouve l’unique réponse. Un envoi resté en file dispose de « Reprendre l’envoi », avec le même identifiant. Le composeur attend la résolution d’un envoi en cours avant d’en créer un autre.

L’API vérifie le Host, l’Origin lorsqu’il est fourni, le marqueur local des POST et le type JSON. Les requêtes d’un site externe sont refusées. Les clients locaux sans Origin restent autorisés avec le marqueur requis. Cette API ne fournit pas d’authentification entre processus locaux du même poste.

## Profils de bot

Un bot a un nom, une couleur prise parmi les accents, une description publique et des instructions privées. Chaque conversation appartient à un seul bot, choisi à sa création (`wrapper_conversations.bot_id`). Une ancienne base est migrée vers `grokbot`. Le moteur relit le bot à chaque réponse. Modifier un bot change donc aussi ses conversations existantes. `compose_instructions` assemble le contrat d’affichage de l’application, l’identité (« Tu t’appelles … »), la description puis les instructions privées. Le résultat part dans `instructions` vers `/v1/responses`.

Le bot par défaut vient d’un template versionné (`DEFAULT_BOT`). À chaque ouverture, `seed_default_bot` le crée s’il manque. Il met à niveau une copie restée intacte, et ne touche jamais une copie modifiée par l’utilisateur. Ce bot ne peut pas être supprimé. Un autre bot supprimé laisse une pierre tombale : ses conversations restent lisibles et il y répond toujours, mais on ne peut plus en commencer de nouvelle avec lui. L’API comprend `POST /api/bots` (l’identifiant est fourni par le client, ce qui rend la création idempotente), `/api/bots/update` et `/api/bots/delete`. `bots` s’ajoute à `/api/state`.

Dans l’interface, l’accueil propose « Avec qui ? » avec une puce par bot et « Nouveau bot ». La couleur du bot teinte les yeux de la mascotte, sans changer l’accent de l’application. La barre latérale montre un rond de cette couleur et la barre du haut affiche le nom du bot. Les paramètres listent les bots, et le menu d’une conversation propose « Modifier le bot ». Les réponses et le composeur portent le nom du bot. `/api/state` liste aussi les bots supprimés avec `deleted: true`. L’interface les retire des puces et des paramètres, mais leurs conversations gardent leur nom, suivi de « (supprimé) », et leur couleur.

## Vérifications et limites

```sh
python3 tools/verify-rust.py
~/.cargo/bin/cargo build --locked -p grokbot-wrapper
python3 tools/verify-wrapper.py
python3 tools/verify-app.py # app Mac déjà ouverte
```

- [Vérification Rust](../evidence/rust-verification.json) : 177 tests, format, lint et contrôle des preuves originales.
- [Vérification HTTP](../evidence/wrapper-verification.json) : 33 contrôles du vrai serveur, dont persistance après redémarrage, réponse perdue, reprise et déduplication.
- [Vérification native](../evidence/wrapper-native-verification.json) : signature, processus enfant, port local, ressources servies identiques aux sources et métadonnées de fenêtre.
- [Vérification de la mascotte](../evidence/wrapper-mascot-verification.json) : nouveau rendu, animations, expressions, reprise, mouvement réduit et petite fenêtre. La [première vérification navigateur](../evidence/wrapper-browser-verification.json) conserve aussi le test de contenu utilisateur rendu comme texte sur le design précédent.

La fermeture normale et la réouverture de l’app ont été exercées; l’enfant Rust s’arrête avec le host. La fenêtre a été observée à l’écran lors du lancement direct. Le premier lancement s’est terminé après la fermeture normale de sa fenêtre, confirmée par les journaux macOS; l’auteur de la fermeture n’est pas établi. Le contrôle des métadonnées de fenêtre n’est pas un audit complet de WKWebView, de l’accessibilité ou des méthodes de saisie. Le dialogue natif d’export n’a pas été exercé jusqu’à l’enregistrement du fichier.

Le wrapper constitue une interface locale du socle indépendant. Il ne démontre pas la parité des 45 familles avec Grok Bot. L’authentification, les modèles, les outils et les conversations distantes restent à implémenter. Les artefacts et preuves historiques de rétro-ingénierie restent conservés séparément.
