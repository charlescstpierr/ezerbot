# Connecteurs : matrice service → authentification → actions

Complément de [`connecteurs.md`](connecteurs.md) et de [`connecteurs-google-slack.md`](connecteurs-google-slack.md). Pas une nouvelle carte : issue [#172](https://github.com/charlescstpierr/ezerbot/issues/172), enfant de [Connecteurs](https://github.com/charlescstpierr/ezerbot/issues/44). Cette page fixe le contrat des trois connecteurs ajoutés au catalogue. Elle ne code rien. Les tranches qui la réalisent sont [#175](https://github.com/charlescstpierr/ezerbot/issues/175) (Linear), [#176](https://github.com/charlescstpierr/ezerbot/issues/176) (Microsoft 365) et [#177](https://github.com/charlescstpierr/ezerbot/issues/177) (Jira Cloud), à prendre après ce document.

## Périmètre

Le catalogue compte six manifests : Gmail, Google Calendar, Google Drive, Slack, Notion, GitHub (`crates/wrapper/src/features/mcp/catalogue.rs`, `MANIFESTS`). Les 46 identifiants de Grok Bot ne prouvent pas 46 connecteurs opérationnels : une parité chiffrée n’est pas certifiable. Le périmètre validé par Charles est borné à trois services : **Linear**, **Microsoft 365** (Outlook courrier et calendrier) et **Jira Cloud**. Salesforce, HubSpot, Stripe, Snowflake et Databricks restent hors périmètre.

Le moteur, le courtier OAuth et le coffre restent locaux. Chaque service reprend les règles déjà décidées pour GitHub, Gmail et Slack :

- **Client public.** Le `client_id` est embarqué dans le build, remplaçable par une variable d’environnement pour les tests. Aucun `client_secret` dans l’app, le binaire, le dépôt ou le trousseau. Sans `client_id`, la fiche dit que le client du service n’est pas configuré, et ne montre pas de formulaire. L’exception de Jira Cloud est posée plus bas : c’est un point ouvert pour Charles, pas une décision.
- **Retour sur `127.0.0.1`.** Le courtier écoute `http://127.0.0.1:<port>/oauth/<service>/callback`, avec un port libre choisi à chaque connexion.
- **Jetons au trousseau**, sans repli fichier si le trousseau refuse. Le jeton n’apparaît ni dans le fil, ni dans un export, ni dans les journaux.
- **Lecture sans carte. Effet par carte brouillon** : texte modifiable, Envoyer une fois, Annuler, pas de « Toujours ». Zéro effet avant Envoyer, un après, zéro après Annuler, zéro au second clic sur la même carte.
- **Liste d’outils fermée.** Aucun outil hors de ce tableau n’est offert au bot. Un outil désactivé n’est plus invocable. Après déconnexion, les appels échouent.

## Matrice

| Service | Compte ou application OAuth à créer | Parcours d’authentification | Permissions | Outils de lecture (sans carte) | Outils à effet (carte brouillon) | Fixture réelle de validation |
| --- | --- | --- | --- | --- | --- | --- |
| **Linear** | Une application OAuth2 dans l’espace Linear de Charles. Fournir son `client_id` public. URL de retour à y déclarer : voir « À vérifier ». | Code d’autorisation + PKCE (`S256`), sans `client_secret`. Autorisation `https://linear.app/oauth/authorize`, jeton `https://api.linear.app/oauth/token`. Jeton d’accès valable 24 h, renouvelé par le jeton de rafraîchissement avec le seul `client_id`. | Portées `read`, `issues:create`, `comments:create`. Pas de `write` large, pas d’`admin`. | `list_teams` (équipes visibles), `list_issues` (issues d’une équipe), `get_issue` (une issue par identifiant, ex. `ENG-12`, avec ses commentaires). | `create_issue` (équipe, titre, description). `comment_issue` (issue, texte). | Un espace Linear de test avec une équipe et une issue connue (identifiant, titre, état, assigné relevés à la main). Un commentaire et une issue créés sont retrouvés dans l’espace. |
| **Microsoft 365** | Une inscription d’application dans Microsoft Entra, plateforme « Applications mobiles et de bureau » (client public, flux public autorisé). Types de comptes : choisis pour couvrir le compte de test. Fournir son `client_id` (application) public. | Code d’autorisation + PKCE (`S256`), sans `client_secret`, point d’entrée `https://login.microsoftonline.com/<tenant>/oauth2/v2.0/` (`authorize`, `token`). `offline_access` pour le rafraîchissement. Retour sur `127.0.0.1` : voir « À vérifier ». | Déléguées Microsoft Graph : `User.Read`, `Mail.Read`, `Mail.Send`, `Calendars.Read`, `offline_access`. Pas de `Mail.ReadWrite`, pas de `Calendars.ReadWrite`. | `list_recent_messages` (les 5 derniers courriels de la boîte de réception : expéditeur, objet, date, aperçu). `list_events` (rendez-vous d’un jour, par défaut demain, fuseau du compte). | `send_mail` (De, À, Cc, Objet, Corps). Pas d’écriture dans le calendrier. | Un compte Microsoft 365 de test avec au moins cinq courriels reçus dont les objets sont connus, et un rendez-vous demain. Le courriel envoyé arrive une fois dans une seconde boîte. |
| **Jira Cloud** | Une application OAuth 2.0 (3LO) dans la console développeur Atlassian. Fournir son `client_id`. **Un `client_secret` est exigé par la documentation Atlassian : décision de Charles requise, voir « Points ouverts ».** | Code d’autorisation 3LO. Autorisation `https://auth.atlassian.com/authorize` (`audience=api.atlassian.com`, `prompt=consent`), jeton `https://auth.atlassian.com/oauth/token`. Les sites accessibles viennent de `https://api.atlassian.com/oauth/token/accessible-resources`, et le compte de `https://api.atlassian.com/me`. Jeton de rafraîchissement à rotation, expiré après 90 jours sans usage. | `read:jira-work`, `write:jira-work`, `read:me`, `offline_access`. Portées classiques ; une portée plus fine par outil est un resserrement à étudier dans la tranche. | `list_issues` (issues d’un projet, via `POST /rest/api/3/search/jql`), `get_issue` (une issue par clé, ex. `PROJ-12`, avec ses commentaires). | `comment_issue` (issue, texte, envoyé comme document Atlassian à un paragraphe). | Un site Jira Cloud de test avec un projet et une issue connue (clé, résumé, état). Un commentaire publié est retrouvé sur l’issue. |

Les noms d’outils ci-dessus suivent le catalogue existant (`list_recent_messages`, `list_events`…) : ils sont indicatifs. La tranche les fixe, et ils restent une liste fermée.

## Parcours visible, commun aux trois

Chercher le nom du service dans le catalogue, ouvrir la fiche : elle affiche les permissions en français et les actions, séparées entre lecture et effets. Connecter ouvre le navigateur, puis la fiche montre le compte (et le site pour Jira). L’approbation d’un outil à effet est la carte brouillon, jamais un « Toujours ». Déconnecter retire le jeton du trousseau et dit si le service a confirmé la révocation ou non, comme `REVOCATION_WARNING` aujourd’hui.

## Critères par connecteur

Repris tels quels dans les issues. Un résultat simulé seul ne valide aucun de ces critères : la fixture réelle de la matrice fait foi.

### Linear — [#175](https://github.com/charlescstpierr/ezerbot/issues/175)

- [ ] Chercher « Linear » : la fiche affiche les permissions et les actions.
- [ ] Connecter : le compte est affiché, le jeton est au trousseau et absent du transcript.
- [ ] La lecture d'une issue fixture réelle est conforme, sans carte.
- [ ] Créer ou commenter passe par une carte et produit un effet unique vérifiable. Annuler donne 0 effet.
- [ ] Un outil désactivé n'est plus invocable. Après déconnexion, les appels échouent.
- [ ] Une donnée simulée seule ne valide pas le ticket. Captures fiche, connexion et carte.

### Microsoft 365 — [#176](https://github.com/charlescstpierr/ezerbot/issues/176)

- [ ] Chercher « Microsoft » ou « Outlook » : la fiche affiche les permissions et les actions.
- [ ] Connecter : le compte est affiché et le jeton est absent du transcript.
- [ ] Les objets des courriels réels et le rendez-vous de demain sont lus sans carte.
- [ ] Le courriel passe par une carte : un seul envoi reçu. Annuler donne 0 envoi.
- [ ] Un outil désactivé n'est plus invocable. Après déconnexion, les appels échouent.
- [ ] Captures fiche, connexion et carte.

### Jira Cloud — [#177](https://github.com/charlescstpierr/ezerbot/issues/177)

- [ ] Chercher « Jira » : la fiche affiche les permissions et les actions.
- [ ] Connecter : le compte et le site sont affichés, et le jeton est absent du transcript.
- [ ] La lecture d'une issue fixture réelle est conforme, sans carte.
- [ ] Le commentaire passe par une carte et produit un effet unique. Annuler donne 0 effet.
- [ ] Un outil désactivé n'est plus invocable. Après déconnexion, les appels échouent.
- [ ] Captures fiche, connexion et carte.

## En attente de Charles

- **Linear** : créer l’application OAuth et fournir le `client_id` public ; fournir un espace Linear de test.
- **Microsoft 365** : enregistrer l’application dans Microsoft Entra et fournir le `client_id` public ; fournir un compte Microsoft 365 de test.
- **Jira Cloud** : créer l’application OAuth Atlassian et fournir le `client_id` ; fournir un site Jira de test ; trancher le secret (ci-dessous).

Ce prérequis bloque le juge visuel avec un vrai compte, pas les tests sur fournisseur factice.

## Points ouverts

- **Jira Cloud : le `client_secret`.** La page Atlassian du flux 3LO (<https://developer.atlassian.com/cloud/jira/platform/oauth-2-3lo-apps/>) marque `client_secret` comme obligatoire dans l’échange du code et dans le rafraîchissement, et ne mentionne ni PKCE ni client public. Si cela se confirme avec un vrai compte, la règle « aucun secret dans l’app » ne tient pas pour Jira. Les options à trancher par Charles : un secret rangé au trousseau de ce Mac, saisi une fois par Charles (comme les clients Google d’avant [`connecteurs-google-slack.md`](connecteurs-google-slack.md)) ; ou un secret embarqué dans le build, qui n’est alors plus secret ; ou retirer Jira du lot. Aucune n’est retenue ici.
- **Microsoft Entra : le retour `127.0.0.1`.** Entra ignore le port d’un retour sur loopback. Le portail refuse pourtant d’enregistrer `http://127.0.0.1` : il faut modifier l’attribut `replyUrlsWithType` du manifeste, ou enregistrer `http://localhost`. Le courtier d’Ezerbot écoute sur `127.0.0.1`. À vérifier avec une inscription réelle laquelle des deux Entra accepte avec un port libre ; la tranche s’aligne sur le résultat.
- **Linear : l’URL de retour.** La documentation Linear n’énonce aucune règle pour un retour loopback à port variable. À vérifier avec l’application réelle : un port fixe, déclaré à la création, est le repli (c’est ce que fait déjà Slack, port `52187`).
- **Jira Cloud : le retour loopback.** La même page n’énonce pas non plus de règle pour `127.0.0.1`. Même vérification, même repli.
- **Microsoft 365 : le consentement.** `Mail.Read`, `Mail.Send` et `Calendars.Read` se consentent d’ordinaire par l’utilisateur, mais un locataire peut restreindre ce consentement et exiger un administrateur. Le compte de test doit le permettre ; sinon la fiche dit en français que l’administrateur doit autoriser l’application.
- **Microsoft 365 : le champ « De ».** Graph envoie sous le compte connecté. La carte affiche « De » mais ne le rend pas modifiable ; envoyer au nom d’une autre boîte est hors-périmètre.
- **Linear : durée du jeton de rafraîchissement.** La documentation donne 24 h pour le jeton d’accès, pas de durée pour le jeton de rafraîchissement. Un rafraîchissement qui échoue passe la fiche à « reconnecter », sans réutiliser un accès expiré.

## Hors-périmètre

- Salesforce, HubSpot, Stripe, Snowflake, Databricks, et tout service hors des trois ci-dessus.
- Écrire dans le calendrier Microsoft, supprimer ou déplacer un courriel, répondre en fil, joindre un fichier.
- Fermer, assigner ou déplacer une issue Linear ou Jira ; créer une issue Jira ; modifier une issue existante.
- Les webhooks, l’iPhone, l’ordinateur cloud et le partage de connexions entre Mac.
- Plusieurs comptes par service dans ce lot.
- Un « Toujours » sur une carte brouillon.
- Appeler Linear, Microsoft ou Atlassian depuis les tests automatisés. Le compte réel est réservé au juge.

## Preuves

Consultées le 23 octobre 2026. Une inférence est marquée [I].

- **Linear.** Documentation OAuth 2.0 (<https://linear.app/developers/oauth-2-0-authentication>) : `code_challenge` et `code_challenge_method` (`plain` ou `S256`) pour PKCE ; `client_secret` facultatif à l’échange ; au rafraîchissement, « si votre jeton a été obtenu avec PKCE, passez simplement `client_id` » ; portées `read`, `write`, `issues:create`, `comments:create`, `admin` ; jeton d’accès valable 24 h, rafraîchissement rotatif avec 30 minutes de grâce.
- **Microsoft.** Page des URI de redirection (<https://learn.microsoft.com/en-us/entra/identity-platform/reply-url>) : `http` accepté seulement pour localhost ; le port est ignoré pour la comparaison d’un retour localhost ; le portail refuse `http://127.0.0.1`, le manifeste l’accepte ; `[::1]` non pris en charge ; la plateforme « Applications mobiles et de bureau » est celle d’une application native. Les portées Graph `Mail.Read`, `Mail.Send` et `Calendars.Read` sont des permissions déléguées standard [I] ; leur détail est à relire dans la référence des permissions Graph au moment de la tranche.
- **Atlassian.** Page OAuth 2.0 (3LO) (<https://developer.atlassian.com/cloud/jira/platform/oauth-2-3lo-apps/>) : `client_secret` marqué obligatoire à l’échange ; `offline_access` pour obtenir un jeton de rafraîchissement ; rotation et expiration à 90 jours d’inactivité ; `accessible-resources` pour le site ; `read:me` pour l’identité. Le chemin `POST /rest/api/3/search/jql` et le corps de commentaire en document Atlassian sont à confirmer dans la référence REST Jira à la tranche [I].
- **Existant Ezerbot.** Courtier : `crates/wrapper/src/features/mcp/catalogue/oauth.rs` (PKCE, retour `127.0.0.1:<port>/oauth/<service>/callback`). Manifests : `catalogue.rs`, `MANIFESTS`. Slack, port fixe `52187` : même fichier.
