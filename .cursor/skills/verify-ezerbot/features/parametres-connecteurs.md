# Paramètres et connecteurs

Les Paramètres regroupent le compte, l’apparence et les sections ajoutées par les panneaux. Les connecteurs sont la section MCP : catalogue, serveurs personnels et formulaire « Ajouter un serveur MCP ».

## Sub-features

- `parametres-ouvrir` ouvre le dialogue Paramètres depuis le bas de la barre latérale.
- `connecteurs-section` montre le titre Connecteurs et l’ajout d’un serveur, sans en enregistrer.
- `connecteurs-vide` laisse la liste des serveurs personnels vide sur une base neuve.
- `connecteurs-reunions` montre l’entrée Réunions du catalogue : un seul champ de clé (mot de passe) et le bouton Connecter, sans OAuth.
- `connecteurs-recherche` filtre le catalogue : « git » ne laisse que GitHub, une recherche vide rend toutes les cartes, une chaîne absente affiche « Aucun service ne correspond à … ».
- `connecteurs-mcp-https` : le formulaire « Ajouter un serveur MCP » propose « Commande locale » (par défaut) ou « Adresse HTTPS » (adresse, jeton en champ mot de passe, case « sur ce Mac » visible seulement pour une adresse `http:`). Un serveur ajouté par adresse montre son adresse, « Jeton privé conservé dans le coffre », jamais le jeton, et « Connecter… » ouvre l’approbation avec « Adresse du service ».
- `connecteurs-github` montre la carte GitHub sans champ d’identifiant ni de secret ; sans `client_id` de build, elle dit que l’application GitHub d’Ezerbot n’est pas configurée.
- `connecteurs-gmail` montre la carte Gmail sans champ d’identifiant ni de secret ; sans client Google de build (`EZERBOT_GOOGLE_CLIENT_ID`), elle dit que le client Google d’Ezerbot n’est pas configuré. La carte dit que le même consentement lit l’agenda et Drive en lecture, sans second formulaire. Connecter ouvre le navigateur sur Google : la preuve automatisée n’y va pas.
- `connecteurs-slack` montre la carte Slack sans champ d’identifiant ni de secret, sans URL de retour à copier ; sans `client_id` de build (`EZERBOT_SLACK_CLIENT_ID`), elle dit que le client Slack d’Ezerbot n’est pas configuré. Connecter un vrai espace et lire un canal exigent le client Slack de Charles, configuré avec les portées utilisateur `channels:history groups:history channels:read groups:read users:read chat:write` et le retour `http://127.0.0.1:52187/oauth/slack/callback` : la preuve automatisée n’y va pas.
- `connecteurs-gmail-comptes` : une fois un compte Google connecté, la carte Gmail offre « Ajouter un autre compte ». Chaque compte a sa ligne `[data-account]` (adresse, Déconnecter, activation par bot) ; en attente, la ligne offre Annuler. La preuve automatisée n’y va pas : elle exige deux comptes Google.
- `connecteurs-github-comptes` : une fois un compte connecté, la carte GitHub offre « Connecter un autre compte ». Chaque compte a sa ligne `[data-account]` (login, code à entrer pendant l’attente, Déconnecter, activation par bot). La preuve automatisée n’y va pas : elle exige deux comptes GitHub.

## How to get to it (user POV)

- Choisir le bouton du bas de barre « Paramètres de l’application » (`#settings-button`).
- Faire défiler jusqu’à la section Connecteurs.

## Driving it with chrome-devtools-axi

Preconditions:

- `doctor: ok`.
- Badge `ChatGPT`.
- Base neuve : aucun serveur MCP enregistré.

- **Ouvrir Paramètres.** Run `bash .cursor/skills/verify-ezerbot/scripts/browser.sh --evidence <preuves> eval '() => { document.getElementById("settings-button").click(); return document.getElementById("settings-dialog").open; }'`. `result:` vaut `true`. Le titre `#settings-title` est `Paramètres`.
- **Aller aux connecteurs.** Run `bash .cursor/skills/verify-ezerbot/scripts/browser.sh --evidence <preuves> eval '() => { const panel = document.querySelector("[data-panel=mcp]"); panel.scrollIntoView({block:"start"}); return panel.querySelector("h3").textContent; }'`. `result:` vaut `"Connecteurs"`.
- **Formulaire d’ajout.** Le `summary` « Ajouter un serveur MCP » est dans `[data-panel="mcp"]`. Ne pas le soumettre.
- **Entrée Réunions.** `[data-panel="mcp"] [data-connector="reunions"]` contient un seul `input` de type `password` et le bouton « Connecter ». Ne pas le soumettre : la clé et les liens de réunion ne passent que par un fournisseur de boucle (`GROKBOT_FAKE_RECALL`), jamais par cette sphère.
- **Recherche.** Le champ `[data-panel="mcp"] [name="catalogue-search"]` filtre par nom et description, sans tenir compte de la casse ni des accents. Saisir `git` (événement `input`) : seul `[data-connector="github"]` n’est pas `hidden`. Vider : toutes les cartes reviennent. Saisir `zzzz-absent` : le `[role="status"]` de l’en-tête du catalogue dit qu’aucun service ne correspond.
- **Serveur par adresse.** Choisir la case `input[type=radio][aria-label="Adresse HTTPS"]` : `[name=url]`, `[name=token]` (mot de passe) et, après une adresse `http://127.0.0.1…`, `[name=on-this-mac]` apparaissent ; le champ `[name=command]` disparaît. Un test de bout en bout exige un serveur MCP HTTP local sur `127.0.0.1` (non fourni par la preuve automatisée) : `prove.sh` ne soumet pas ce formulaire.
- **Liste vide.** `[data-panel="mcp"] .mcp-list` ne contient pas d’article de serveur personnalisé.
- **Capture.** Pour chaque thème et chaque taille (`1140x780`, `820x560`), Run `bash .cursor/skills/verify-ezerbot/scripts/browser.sh --evidence <preuves> resize <largeur> <hauteur>` puis `screenshot <preuves>/spheres/parametres-connecteurs/<theme>-<largeur>x<hauteur>.png`.
- **Parcours groupé.** Run `bash .cursor/skills/verify-ezerbot/scripts/drive.sh --evidence <preuves> --sphere parametres-connecteurs`.

## Gotchas

- Le dialogue est modal. Un clic sur la marque ne ferme pas Paramètres ; utiliser `[data-close-dialog]` ou « Terminé ».
- La section est montée dans `#settings-extensions` avec `data-panel="mcp"`. Chercher ce panneau, pas un lien de navigation séparé.
- Enregistrer un connecteur lancerait une commande locale. Cette sphère ne soumet pas le formulaire.
- Pour jouer deux comptes sans compte réel, lancer un wrapper de debug à part (HOME et `--db` jetables) avec `GROKBOT_FAKE_CONNECTORS=http://127.0.0.1:<port>` et `EZERBOT_GITHUB_CLIENT_ID` : le fournisseur de boucle sert `/github/device`, `/github/token` et `/github/user`. Ces variables ne sont pas transmises par `launch.sh`.
- La carte Gmail se connecte par la page Google (client public d’Ezerbot, PKCE, retour sur `127.0.0.1`) avec un vrai compte : la preuve automatisée ne clique pas sur Connecter. Pour voir la carte configurée sans compte, lancer un wrapper de debug à part (HOME et `--db` jetables) avec `EZERBOT_GOOGLE_CLIENT_ID` ; `launch.sh` ne transmet pas cette variable.
- Pour jouer deux comptes Google sans compte réel, lancer le même wrapper de debug à part (HOME et `--db` jetables) avec `GROKBOT_FAKE_CONNECTORS` et `EZERBOT_GOOGLE_CLIENT_ID` ; le fournisseur de boucle doit répondre à `/gmail/token` avec un utilisateur différent à chaque connexion (le retour du navigateur se joue en appelant le `redirect_uri` de l’URL d’autorisation), à `/gmail/api/gmail/v1/users/me/profile` avec l’adresse du porteur du jeton, et servir une boîte par adresse. Avec deux comptes et sans choix dans la conversation, l’outil de lecture (courriel, agenda ou Drive) répond qu’il faut demander lequel ; la lecture suivante, dans la même conversation, garde le compte nommé pour ces trois services.
- Le courriel que le bot prépare passe par une carte brouillon dans le fil (De, À, Cc, Objet, Corps, Envoyer, Annuler, jamais « Toujours »), pas par cette section. Pour la voir, lancer un wrapper de debug à part (HOME et `--db` jetables) avec `GROKBOT_FAKE_CONNECTORS`, `EZERBOT_GOOGLE_CLIENT_ID` et un faux OpenAI qui appelle `gmail_send_email` : `prove.sh` ne la déclenche pas. Le fournisseur de boucle sert `/gmail/token`, `/gmail/api/gmail/v1/users/me/profile` et `…/messages/send`.
- Pour jouer Slack sans espace réel, lancer le même wrapper de debug à part avec `GROKBOT_FAKE_CONNECTORS=http://127.0.0.1:<port>` et `EZERBOT_SLACK_CLIENT_ID` : le fournisseur de boucle des tests (`crates/wrapper/tests/connectors.rs`) sert `/slack/token`, `/slack/api/auth.test`, `/slack/api/conversations.list`, `/slack/api/conversations.history`, `/slack/api/users.info` et `/slack/api/chat.postMessage`. Un message Slack préparé par le bot (`slack_post_message`) ouvre dans le chat une carte brouillon (Espace, Cible, Fil, texte modifiable, Envoyer, Annuler, sans « Toujours ») : rien ne part avant Envoyer, et un second Envoyer ne republie pas. Sans `EZERBOT_SLACK_CLIENT_ID`, un clic sur Connecter est refusé avec une phrase, sans carte d’approbation.
- La carte GitHub se connecte par un code à 8 caractères validé chez GitHub avec un vrai compte : la preuve automatisée n’y va pas. Sans `EZERBOT_GITHUB_CLIENT_ID`, un clic sur Connecter est refusé avec une phrase, sans carte d’approbation.
- Le catalogue peut être long. La capture doit avoir fait défiler le titre Connecteurs dans le dialogue, pas seulement ouvert Paramètres en haut.
