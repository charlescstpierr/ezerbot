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

L’interface est refaite autour d’un galet nacré animé, inspiré de la mascotte de Grokbot, avec des tons ivoire et sauge et des accents terracotta. Elle propose un composeur, les suggestions de départ, la recherche, les favoris, le renommage, la suppression, la copie, l’export JSON et les thèmes clair, sombre et système. Les raccourcis comprennent `⌘ N`, `⌘ K` et Entrée pour envoyer. Un premier message donne automatiquement son titre à une nouvelle conversation, sauf si le titre a été choisi explicitement. [Design de la mascotte](mascotte-design.md) décrit les références et les animations.

[app/native/main.m](../app/native/main.m) héberge la page dans WKWebView, démarre un seul moteur enfant et l’arrête à la fermeture de l’app. Il n’expose pas de pont JavaScript vers les capacités du Mac. L’export utilise le dialogue natif d’enregistrement. Les ressources [HTML](../app/ui/index.html), [CSS](../app/ui/styles.css) et [JavaScript](../app/ui/app.js) sont embarquées dans le binaire Rust; une reconstruction est nécessaire pour intégrer leurs modifications.

[grokbot-wrapper](../crates/wrapper/src/lib.rs) expose une API HTTP liée uniquement à `127.0.0.1`, sur un port dynamique. Il utilise le vrai journal durable et [SubmissionSession](../crates/session/src/submission.rs). Les titres, favoris, préférences et acceptations du transport simulé sont aussi stockés dans SQLite. Les messages restent immuables; répéter le même identifiant ne provoque pas un second envoi logique.

Dans les paramètres, « Tester une coupure » simule une réponse perdue après acceptation. Le message apparaît comme incertain. « Vérifier l’envoi » cherche son statut avant toute reprise et retrouve l’unique réponse. Un envoi resté en file dispose de « Reprendre l’envoi », avec le même identifiant. Le composeur attend la résolution d’un envoi en cours avant d’en créer un autre.

L’API vérifie le Host, l’Origin lorsqu’il est fourni, le marqueur local des POST et le type JSON. Les requêtes d’un site externe sont refusées. Les clients locaux sans Origin restent autorisés avec le marqueur requis. Cette API ne fournit pas d’authentification entre processus locaux du même poste.

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
