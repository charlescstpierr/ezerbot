# Connecteurs Google et Slack

Complément de `docs/specs/connecteurs.md`. Pas une nouvelle carte. Les tranches sont des enfants de [Connecteurs](https://github.com/charlescstpierr/ezerbot/issues/44). Cette page ne crée pas les issues. L’annexe en donne le corps.

`docs/specs/connecteurs.md` laisse Gmail, Google Calendar, Google Drive, Slack et Notion au parcours actuel : Charles crée le client OAuth et en saisit l’identifiant et le secret. Ce complément spécifie le trou pour Gmail, Calendar, Drive et Slack. Il ne réécrit pas le device flow GitHub, la carte brouillon GitHub, ni le MCP HTTPS. Notion reste au parcours actuel.

Vocabulaire déjà en usage : catalogue, connecteur, courtier, coffre, trousseau, compte, carte brouillon. Un compte Google est un utilisateur Google lié à Gmail, Calendar et Drive. Le client public est le `client_id` embarqué d’Ezerbot, sans `client_secret`.

## Contexte

Le catalogue propose six services. GitHub est spécifié à part : GitHub App, device flow, `client_id` public, aucun secret dans l’app, jetons au trousseau sans repli fichier. Gmail, Calendar, Drive et Slack demandent encore un client créé par Charles. Les lectures n’ont pas été jouées avec un vrai compte. Un service ne garde qu’un compte, et le multi-comptes spécifié l’est pour GitHub ([#49](https://github.com/charlescstpierr/ezerbot/issues/49)). La carte brouillon spécifiée l’est pour un commentaire GitHub ([#47](https://github.com/charlescstpierr/ezerbot/issues/47)).

Grok Bot ne met pas de `client_id` Google dans l’application : le serveur le fournit. Il embarque un `client_id` Slack de repli. Le retour peut être un loopback forcé en `127.0.0.1`. Ezerbot n’a pas de serveur. La décision déjà prise est donc la même que pour la GitHub App : `client_id` public embarqué, PKCE, retour sur `127.0.0.1`, jetons au trousseau.

Les routines attendent ces connecteurs. [#59](https://github.com/charlescstpierr/ezerbot/issues/59) consomme un nouveau courriel déjà normalisé. [#60](https://github.com/charlescstpierr/ezerbot/issues/60) consomme un événement de calendrier. Ni l’un ni l’autre ne parlent à l’API du fournisseur.

## Énoncé du problème

Charles ne peut pas connecter Gmail ou Slack sans fabriquer un client OAuth. Il ne lit pas ses courriels, son agenda ni un fichier Drive avec un vrai compte. Un envoi de courriel ou de message Slack ne passe pas par une carte brouillon. Un second compte Google n’existe pas. Tant que ces trous restent ouverts, les routines courriel et calendrier n’ont rien à consommer.

## Solution

Gmail se connecte en un clic, comme GitHub : client public d’Ezerbot, PKCE, loopback `127.0.0.1`, aucun formulaire d’identifiant. La lecture part sans carte. L’envoi passe par une carte brouillon, le même geste que [#47](https://github.com/charlescstpierr/ezerbot/issues/47). Slack suit le même parcours de connexion, puis sa propre carte. Calendar et Drive se lisent avec le même consentement Google. Plusieurs comptes Google coexistent, sur le modèle de [#49](https://github.com/charlescstpierr/ezerbot/issues/49).

Charles prépare une fois, hors de l’app, le client Google « application de bureau » d’Ezerbot et le client Slack, comme il prépare la GitHub App. Ce prérequis bloque le juge, pas les tests sur fournisseur factice.

## Décisions

La section Preuves rattache chaque geste copié à son fichier, sa fonction ou son symbole, et marque les inférences. Le `client_id` public embarqué et le PKCE sur le Mac sont le choix Ezerbot de R2, pas le courtier serveur de Grok.

1. **Gmail se connecte sans client à saisir.** La carte n’a ni identifiant ni secret. Connecter lance le courtier déjà livré, avec le `client_id` public, PKCE, et un retour sur `127.0.0.1`. Aucun `client_secret` n’est lu, envoyé ou stocké. Sans `client_id`, la carte explique que le client Google d’Ezerbot n’est pas configuré. Les portées Gmail sont celles lues chez Grok Bot : `gmail.readonly`, `gmail.compose`, `gmail.modify`. Les jetons vont au trousseau. Si le trousseau refuse l’écriture, ils ne suivent pas le repli fichier : la connexion échoue, la carte le dit, rien n’est conservé. « Résume mes 5 derniers courriels » lit cinq objets réels, sans carte d’approbation. Le jeton n’est pas dans le fil.
2. **Un nouveau courriel est un événement normalisé.** L’identifiant du message, le compte Google qui l’a reçu, l’expéditeur, l’objet, la date et le texte. Le même identifiant ne produit qu’une livraison par compte. [#59](https://github.com/charlescstpierr/ezerbot/issues/59) consomme cet événement. Cette spec ne crée pas la routine.
3. **L’envoi Gmail est une carte brouillon.** De, À, Cc, Objet, Corps, modifiables. Envoyer une fois, Annuler, pas de « Toujours ». Zéro envoi avant Envoyer, un après, zéro après Annuler, zéro au second clic sur la même carte. Ce qui part est le texte validé, sous le compte de Charles. Les états difficiles sont Refusé, Trop de messages, Envoi non confirmé. Dans ces trois cas, le courriel n’est pas considéré envoyé, ou la carte dit que l’accusé manque. Cette tranche débloque [#59](https://github.com/charlescstpierr/ezerbot/issues/59).
4. **Slack se connecte comme Gmail, avec son propre client public.** Le catalogue a déjà Slack, aujourd’hui avec un client à créer. Le formulaire disparaît. Le `client_id` public d’Ezerbot est embarqué, remplaçable par une variable d’environnement pour les tests, comme celui de GitHub. Sans lui, la carte explique que le client Slack n’est pas configuré. Lire un canal nomme les messages, sans carte. Les jetons suivent la même règle de trousseau.
5. **Le message Slack est une carte brouillon.** Espace, cible, fil, texte. Les règles d’envoi sont celles de la décision 3 : une fois, Annuler, pas de « Toujours », mêmes états.
6. **Calendar et Drive se lisent avec le consentement Google de Gmail.** Pas un second formulaire. « Mes rendez-vous de demain » et la lecture d’un fichier Drive partent sans carte. Les portées ajoutées sont en lecture. Ce lot n’écrit ni dans l’agenda ni dans Drive. L’événement de calendrier normalisé porte un identifiant, un cas parmi créé, modifié, annulé et commence, l’instant de début et le fuseau. [#60](https://github.com/charlescstpierr/ezerbot/issues/60) consomme cet événement. Cette spec ne crée pas la routine. Cette tranche débloque #60.
7. **Plusieurs comptes Google.** « Ajouter un autre compte », sur le modèle de #49. Un seul compte est utilisé sans question. Plusieurs comptes, et aucun choix dans la conversation : le bot demande. Le choix vaut pour Gmail, Calendar et Drive de cet utilisateur. Déconnecter l’un laisse l’autre. Slack n’est pas multi-comptes dans ce lot.

## Récits

1. En tant que Charles, je veux connecter Gmail en un clic, sans champ d’identifiant ni de secret, afin de ne pas créer un client OAuth.
2. En tant que Charles, je veux un retour sur `127.0.0.1` en PKCE, afin que l’échange reste sur ce Mac.
3. En tant que Charles, je veux que les jetons aillent au trousseau, et qu’un trousseau verrouillé échoue sans fichier de secours, afin qu’un fichier du Mac ne les contienne pas.
4. En tant que Charles, je veux, si le `client_id` manque, une phrase qui dit que le client Google d’Ezerbot n’est pas configuré, afin de ne pas voir revenir le formulaire.
5. En tant que Charles, je veux « Résume mes 5 derniers courriels » et lire cinq objets réels, sans carte, afin de vérifier le compte.
6. En tant que Charles, je veux que le jeton reste absent du fil, afin qu’un export ne l’emporte pas.
7. En tant que Charles, je veux qu’un nouveau message soit livré une fois, avec l’expéditeur, l’objet, la date et le texte, afin que la routine courriel ait un contrat.
8. En tant que Charles, je veux une carte De, À, Cc, Objet, Corps, modifiable, afin de relire le courriel avant qu’il parte.
9. En tant que Charles, je veux qu’Annuler n’envoie rien, afin d’abandonner le brouillon.
10. En tant que Charles, je veux qu’Envoyer envoie une fois, sous mon nom, afin que le destinataire voie Charles.
11. En tant que Charles, je veux qu’un second clic n’envoie pas un second courriel, afin d’éviter le double envoi.
12. En tant que Charles, je veux que la carte n’offre pas « Toujours », afin que le prochain courriel attende encore mon clic.
13. En tant que Charles, je veux voir Refusé, Trop de messages ou Envoi non confirmé quand l’envoi ne s’est pas terminé, afin de ne pas croire que le courriel est parti.
14. En tant que Charles, je veux connecter Slack en un clic, sans client à créer, afin de suivre le même geste que Gmail.
15. En tant que Charles, je veux lire un canal et voir les messages dans le fil, sans carte, afin de vérifier la lecture.
16. En tant que Charles, je veux une carte Slack avec l’espace, la cible, le fil et le texte, afin de valider le message avant publication.
17. En tant que Charles, je veux les mêmes règles d’envoi que pour le courriel, afin de ne pas apprendre une seconde carte.
18. En tant que Charles, je veux « Mes rendez-vous de demain » en lecture réelle, afin de voir l’agenda du compte connecté.
19. En tant que Charles, je veux lire un fichier Drive nommé, afin de vérifier que le même consentement suffit.
20. En tant que Charles, je veux un événement de calendrier avec son identifiant, son cas, son début et son fuseau, afin que la routine calendrier ait un contrat.
21. En tant que Charles, je veux ajouter un second compte Google et qu’on me demande lequel quand ce n’est pas clair, afin de séparer deux boîtes.
22. En tant que Charles, je veux déconnecter un compte Google et garder l’autre, afin de ne pas tout refaire.
23. En tant que Charles, je veux que GitHub et Notion gardent leur parcours déjà spécifié ou actuel, afin que ce lot ne les casse pas.

## Décisions d’implémentation

- Le `client_id` Google et le `client_id` Slack sont des valeurs publiques du build, remplaçables par variable d’environnement. Le point d’échange de production est celui du fournisseur. Les tests visent le fournisseur de boucle déjà utilisé par le catalogue. Tout autre hôte est refusé. Aucun `client_secret` n’est lu.
- Le courtier local déjà livré fait l’échange. Le retour est `127.0.0.1`. PKCE est obligatoire. Le `code_verifier` ne quitte pas la mémoire d’attente.
- Chaque compte a son secret dans le coffre des connecteurs : jeton d’accès, rafraîchissement, échéance. L’adresse affichée n’est pas un secret. La règle de `docs/specs/connecteurs.md` sur le repli fichier des jetons de compte s’applique telle quelle.
- Les outils Gmail de ce lot sont fermés : lire les derniers messages, livrer l’événement nouveau message, préparer un envoi. Pas de suppression de courriel, même si la portée `gmail.modify` est demandée parce que Grok Bot la demande. Calendar et Drive : lire l’agenda, livrer l’événement normalisé, lire un fichier nommé. Pas d’écriture. Slack : lire un canal, préparer un message.
- La carte brouillon réutilise la demande à usage unique de #47 : empreinte du texte validé, du compte et de la cible. Changer le texte change l’empreinte. L’envoi n’a lieu qu’après le clic.
- Le compte résolu est celui de #45 et #49. Zéro compte : le chat dit de connecter. Un compte : celui-là. Plusieurs : la question, puis le choix gardé dans la conversation.
- L’événement courriel et l’événement calendrier sont le contrat d’entrée de #59 et #60. Ils ne contiennent pas de jeton.

## Décisions de test

Un bon test observe la carte du catalogue, la bulle, les boutons, les appels comptés vers le fournisseur factice, l’événement normalisé, et l’absence du jeton dans la base, le transcript et les journaux. Il ne vérifie pas l’ordre interne des fonctions.

La couture est celle du catalogue et du courtier. On n’ajoute pas de harnais. Gmail, Calendar, Drive et Slack sont joués par le fournisseur de boucle. Le coffre des tests wrapper reste le coffre fichier jetable. Le refus de repli fichier, trousseau verrouillé, se vérifie sur la couture trousseau déjà réservée à macOS.

Chaque tranche commence par un test rouge, puis le code qui le rend vert. Le juge visuel sur le Mac, avec les vrais comptes de Charles, est le critère d’acceptation. Une capture immobile ne suffit pas. Le jeton ne doit apparaître nulle part à l’écran.

## Critères d’acceptation

Le juge a le client Google d’Ezerbot et le client Slack préparés, les `client_id` publics configurés, et des données à lui : au moins cinq courriels, un rendez-vous demain, un fichier Drive, un canal Slack.

1. **Gmail.** La carte n’a pas de champ identifiant ni secret. Connecter aboutit, le login est affiché. « Résume mes 5 derniers courriels » montre cinq objets réels, sans carte d’approbation. Le jeton n’est pas à l’écran. Sans `client_id`, la phrase de configuration s’affiche et le formulaire ne revient pas. GitHub garde son parcours device flow.
2. **Brouillon Gmail.** « Prépare un courriel à moi-même, objet Test parité, corps Bonjour. Ne l’envoie pas. » La carte montre De, À, Cc, Objet, Corps, sans « Toujours ». Annuler : rien dans la boîte. Corriger le corps en « Bonjour validé », Envoyer une fois : un seul courriel, expéditeur le compte de Charles, ce corps. Un second clic n’en crée pas un autre.
3. **Slack.** Connecter en un clic, sans formulaire de client. Lire un canal nommé : les messages apparaissent sans carte. Sans `client_id` Slack, la phrase de configuration s’affiche.
4. **Brouillon Slack.** Une demande d’envoi affiche espace, cible, fil et texte, sans « Toujours ». Annuler ne publie pas. Envoyer une fois publie ce texte. Un second clic ne republie pas.
5. **Agenda et Drive.** « Mes rendez-vous de demain » nomme le rendez-vous connu, sans carte. Lire le fichier Drive connu en donne un extrait. L’événement normalisé disponible pour une routine porte l’identifiant, le cas, le début et le fuseau. Rien n’est écrit dans l’agenda ni dans Drive.
6. **Deux comptes Google.** « Ajouter un autre compte » aboutit. Les deux adresses sont visibles. Une lecture sans choix préalable demande le compte. Chaque choix lit avec le compte nommé. Déconnecter le second laisse le premier capable de lire.

## Hors-périmètre

- Réécrire GitHub, le device flow, la recherche du catalogue, le MCP HTTPS, ou la confirmation de déconnexion GitHub.
- Notion, Outlook, et tout autre service du catalogue de Grok Bot.
- Supprimer un courriel, modifier un événement, écrire dans Drive.
- Le multi-comptes Slack.
- Créer les routines #59 et #60. Ce lot publie le contrat et les débloque. Il ne les implémente pas.
- Un `client_secret` dans l’app, un callback hébergé, un courtier tiers.
- L’ordinateur cloud, et un partage de connexions entre Mac.
- Appeler Google ou Slack depuis les tests automatisés. Le compte réel est réservé au juge.

## Risques

- **`gmail.modify` est plus large que la lecture.** Grok Bot demande cette portée. La liste d’outils est la barrière : pas d’outil de suppression. Un outil classé lecture par erreur enverrait ou détruirait sans carte. Le test de la carte compte zéro envoi avant Envoyer, un après, zéro après Annuler.
- **Le `client_id` est public.** C’est voulu, comme pour GitHub. Le secret absent est le `client_secret`. Le `code_verifier` reste en mémoire et tombe au redémarrage.
- **Le juge dépend des clients préparés par Charles.** Sans le client Google et sans le client Slack, les parcours réels ne se cochent pas. Les tests de la couture tournent quand même.
- **Deux specs de connecteurs.** `docs/specs/connecteurs.md` dit que ces services restent au parcours actuel. Cette page est le lot suivant, pas une correction du parcours GitHub. Le jour où les deux sont sur main, ce fichier gagne pour Gmail, Calendar, Drive et Slack. Notion reste à l’autre fichier.
- **Le contrat de routine bouge.** #59 et #60 consomment les événements décrits ici. Si un champ manque, on l’ajoute dans ce fichier, pas dans la routine.
- **Compte ambigu.** Avec deux comptes Google, une lecture sans question enverrait le mauvais. Le test de la tranche multi-comptes exige la question.

## Preuves

Source : Grok Bot 0.72.1, `grokbot-rea.md` §c et `tickets-parite.md` R2. Chemins relatifs à `dist/`. [P] est une chaîne citée. [I] est une inférence.

- **Pas de client Google dans le binaire.** [P] aucun `*.apps.googleusercontent.com`. [P] le `client_id` est lu dans `/.well-known/oauth-protected-resource`, champ `cursor_client_id`, pour les URL `^/rest-mcp/([^/]+)/mcp`. [P] document `https://cursor.com/oauth/mcp-client.json`. [I] les client_id sont détenus côté serveur et les jetons restent sur le backend (`api.cursor.com`, `connectors-gateway.grok.com`). Embarquer un client public est le choix Ezerbot, le même geste que la GitHub App de #44.
- **Loopback.** [P] callback `http://localhost:8787/callback`, ports `18787`, `28787`, `31787`, forcé en `127.0.0.1`. Grok a aussi des callbacks hébergés et un deep link. Ce lot ne copie que le loopback.
- **Portées Gmail.** [P] `provider:"google-workspace"`, `clientRegistration:"static"`, `access_type:"offline"`, `prompt:"select_account consent"`, portées `gmail.readonly`, `gmail.compose`, `gmail.modify`.
- **Slack.** [P] repli embarqué `slack_mcp_client_id` (valeur masquée) dans `electron-main/main-app.cjs`, à côté de `shopify_mcp_client_id`. Le client Slack d’Ezerbot est un `client_id` public de build. [I] l’aligner sur ce repli ne copie pas la valeur masquée.
- **Carte courriel.** [P] `GrokBotEmailDraft` (`from`, `to`, `cc`, `subject`, `body`) dans `electron-main/proto.cjs`. [P] `SendGrokBotDraftRequest` (champ `email`) et `DiscardGrokBotDraftRequest`. [P] carte `email-draft` dans `renderer/assets/index.eager-app-BBYQQe1M.js`. [P] états `pending`, `sending`, `refused`, `rate_limited`, `unconfirmed` dans `renderer/assets/chunk-draft-send-error-*`. Refusé, Trop de messages et Envoi non confirmé sont les libellés français de `refused`, `rate_limited`, `unconfirmed`. [P] la chaîne « Always allow » existe. Retirer « Toujours » est la politique de publication de #47, pas une absence chez Grok.
- **Carte Slack.** [P] `GrokBotSlackDraft` (`workspace`, `target`, `thread`, `body`). [P] `SendGrokBotDraftRequest` champ `slack`. [P] carte `slack-draft` dans le même renderer.
- **Lecture sans carte.** [P] `McpArgs` porte `smart_mode_approval` et `skip_approval`. [I] en déduire que toute lecture Gmail part sans carte irait au-delà de ces champs. La classe « lecture sans carte » est celle de `docs/specs/connecteurs.md`, appliquée aux outils de lecture de ce lot.
- **Agenda et Drive.** [P] hôtes `calendarmcp.googleapis.com` → `googlecalendar`, `drivemcp.googleapis.com` → `googledrive`, `gmailmcp.googleapis.com` → `gmail`. [P] rea §b : le déclencheur calendrier est absent de l’union des routines. Il n’existe que pour la récurrence des réunions. [I] le contrat créé, modifié, annulé, commence, avec fuseau, est celui de `docs/specs/routines.md` (#60), pas un symbole de routine Grok.
- **Plusieurs comptes.** [P] « Add another account » dans `renderer/assets/index.eager-common-BH0hmUYG.js`. [P] `accountKey`, `accountEmail`, `singleAccount` dans `electron-main/main-app.cjs`. [P] `DeleteMcpOAuthAccountRequest` (`server_id`, `account_key`) et `RenameMcpOAuthAccountRequest` dans `electron-main/proto.cjs`. Certains connecteurs ont `singleAccount:!0`. [I] Gmail n’est pas nommé dans cette liste. T16 le traite comme multi-comptes, sur le modèle de #49.

## Tranches

Labels : `wayfinder:task`, `enhancement`, `ready-for-agent`. Parent : [#44](https://github.com/charlescstpierr/ezerbot/issues/44). La première à prendre, dès que #45 est là, est T11.

1. T11. Connecter Gmail et lire les courriels. Bloquée par [#45](https://github.com/charlescstpierr/ezerbot/issues/45).
2. T12. Carte brouillon Gmail. Bloquée par T11 et par [#47](https://github.com/charlescstpierr/ezerbot/issues/47). Débloque [#59](https://github.com/charlescstpierr/ezerbot/issues/59).
3. T13. Connecter Slack et lire un canal. Bloquée par [#45](https://github.com/charlescstpierr/ezerbot/issues/45).
4. T14. Carte brouillon Slack. Bloquée par T13 et par [#47](https://github.com/charlescstpierr/ezerbot/issues/47).
5. T15. Google Calendar et Drive en lecture. Bloquée par T11. Débloque [#60](https://github.com/charlescstpierr/ezerbot/issues/60).
6. T16. Plusieurs comptes Google. Bloquée par T11 et par [#49](https://github.com/charlescstpierr/ezerbot/issues/49).

## Annexe — tickets prêts à créer

Ces corps sont prêts à coller. Cette PR ne crée aucune issue. Le parent de chacun est la carte [#44](https://github.com/charlescstpierr/ezerbot/issues/44). Spec : `docs/specs/connecteurs-google-slack.md`.

### T11 — Connecter Gmail sans créer de client OAuth et lire les courriels

## Parent

Carte [Connecteurs](https://github.com/charlescstpierr/ezerbot/issues/44). Spec : `docs/specs/connecteurs-google-slack.md`, décisions 1 et 2, parcours juge 1.

## What to build

La carte Gmail n’a ni identifiant ni secret. Connecter utilise le `client_id` public d’Ezerbot, PKCE, retour `127.0.0.1`. Aucun `client_secret`. Sans `client_id`, la carte explique que le client Google d’Ezerbot n’est pas configuré et n’affiche pas le formulaire. Les portées sont `gmail.readonly`, `gmail.compose`, `gmail.modify`. Les jetons vont au trousseau. Un trousseau qui refuse l’écriture ne produit pas de fichier de secours.

« Résume mes 5 derniers courriels » écrit les objets dans la bulle, sans carte. Le jeton est absent du fil. Un nouveau message est aussi un événement normalisé : identifiant, expéditeur, objet, date, texte. Le même identifiant ne sort qu’une fois. Cet événement est celui que [#59](https://github.com/charlescstpierr/ezerbot/issues/59) consommera. La routine, elle, attend T12.

GitHub garde son parcours. Notion garde le sien.

Démo : Connecter, cinq objets réels dans la bulle.

TDD : d’abord un test rouge du wrapper contre le fournisseur de boucle (PKCE, absence de secret, cinq objets dans la bulle, événement une fois, zéro jeton après annulation).

## Acceptance criteria

- [ ] TDD : le test est rouge avant et vert après.
- [ ] La carte n’a pas de champ client. Sans `client_id`, la phrase de configuration s’affiche.
- [ ] L’échange envoie le `client_id` public et PKCE, pas de `client_secret`, retour sur `127.0.0.1`.
- [ ] Au succès, le compte est affiché. Le jeton est au coffre de cette connexion, absent du transcript et des journaux.
- [ ] Sur le trousseau macOS, un refus d’écriture ne crée pas de fichier de secours contenant le jeton.
- [ ] La lecture des cinq derniers courriels part sans carte.
- [ ] L’événement nouveau message porte identifiant, expéditeur, objet, date et texte, une fois par identifiant.
- [ ] GitHub et Notion répondent comme avant cette tranche.
- [ ] Juge visuel sur le Mac, vrai compte : parcours 1 de la spec.

## Blocked by

- [Connecter GitHub et lister les issues d’un dépôt](https://github.com/charlescstpierr/ezerbot/issues/45)

### T12 — Carte brouillon Gmail

## Parent

Carte [Connecteurs](https://github.com/charlescstpierr/ezerbot/issues/44). Spec : `docs/specs/connecteurs-google-slack.md`, décision 3 et parcours juge 2.

## What to build

Préparer un courriel affiche une carte De, À, Cc, Objet, Corps. Le texte est modifiable. Envoyer publie le texte validé, une fois, sous le compte de Charles. Annuler n’envoie rien. Pas de « Toujours ». Un second Envoyer sur la même carte ne rappelle pas le fournisseur. Les états Refusé, Trop de messages et Envoi non confirmé ne comptent pas comme un courriel parti, ou ils disent que l’accusé manque.

Cette tranche débloque [#59](https://github.com/charlescstpierr/ezerbot/issues/59) : l’événement à consommer est celui de T11, et le connecteur courriel est alors complet pour cette attente.

La lecture reste sans carte.

Démo : Annuler, puis Envoyer un corps corrigé, un seul courriel reçu, expéditeur Charles.

TDD : d’abord un test rouge (zéro appel d’envoi avant Envoyer, un après, zéro après Annuler, zéro au second clic, corps corrigé = corps publié).

## Acceptance criteria

- [ ] TDD : le test est rouge avant et vert après.
- [ ] La carte montre De, À, Cc, Objet et Corps, sans « Toujours ».
- [ ] Avant Envoyer, le fournisseur n’a reçu aucun envoi. Annuler n’envoie rien.
- [ ] Corriger puis Envoyer publie ce texte, une fois, sous le compte affiché.
- [ ] Un second Envoyer ne crée pas un second courriel.
- [ ] Refusé, Trop de messages et Envoi non confirmé sont visibles, et ne sont pas présentés comme un envoi réussi silencieux.
- [ ] La lecture des courriels ne fait toujours pas apparaître de carte.
- [ ] Juge visuel : parcours 2 de la spec. L’expéditeur reçu est le compte de Charles.

## Blocked by

- T11 — Connecter Gmail sans créer de client OAuth et lire les courriels
- [Commenter une issue GitHub depuis une carte brouillon](https://github.com/charlescstpierr/ezerbot/issues/47)

### T13 — Connecter Slack sans client propre et lire un canal

## Parent

Carte [Connecteurs](https://github.com/charlescstpierr/ezerbot/issues/44). Spec : `docs/specs/connecteurs-google-slack.md`, décision 4 et parcours juge 3.

## What to build

Slack quitte le formulaire de client. Connecter utilise le `client_id` public d’Ezerbot, PKCE, retour `127.0.0.1`, sans `client_secret`. Sans `client_id`, la carte explique que le client Slack n’est pas configuré. Les jetons suivent la règle de trousseau de T11. Lire un canal écrit les messages dans la bulle, sans carte. Le jeton est absent du fil.

Démo : Connecter, puis les messages d’un canal nommé.

TDD : d’abord un test rouge (absence de formulaire, lecture sans carte, jeton absent du transcript).

## Acceptance criteria

- [ ] TDD : le test est rouge avant et vert après.
- [ ] La carte Slack n’a pas de champ client. Sans `client_id`, la phrase de configuration s’affiche.
- [ ] L’échange n’envoie pas de `client_secret`. Le jeton absent du transcript va au coffre, sans repli fichier si le trousseau refuse.
- [ ] La lecture d’un canal part sans carte et nomme les messages.
- [ ] Juge visuel : parcours 3 de la spec.

## Blocked by

- [Connecter GitHub et lister les issues d’un dépôt](https://github.com/charlescstpierr/ezerbot/issues/45)

### T14 — Carte brouillon Slack

## Parent

Carte [Connecteurs](https://github.com/charlescstpierr/ezerbot/issues/44). Spec : `docs/specs/connecteurs-google-slack.md`, décision 5 et parcours juge 4.

## What to build

Préparer un message Slack affiche une carte : espace, cible, fil, texte. Les règles sont celles de T12. Envoyer une fois, Annuler, pas de « Toujours », second clic sans nouvel envoi, états Refusé, Trop de messages, Envoi non confirmé. La lecture d’un canal reste sans carte.

Démo : Annuler, puis publier un texte corrigé, un seul message dans le canal.

TDD : d’abord un test rouge (zéro publication avant Envoyer, une après, zéro après Annuler, zéro au second clic).

## Acceptance criteria

- [ ] TDD : le test est rouge avant et vert après.
- [ ] La carte montre l’espace, la cible, le fil et le texte, sans « Toujours ».
- [ ] Annuler ne publie rien. Envoyer publie le texte validé, une fois.
- [ ] Un second Envoyer ne republie pas.
- [ ] Les trois états d’échec sont visibles et ne comptent pas comme une publication silencieuse.
- [ ] Juge visuel : parcours 4 de la spec.

## Blocked by

- T13 — Connecter Slack sans client propre et lire un canal
- [Commenter une issue GitHub depuis une carte brouillon](https://github.com/charlescstpierr/ezerbot/issues/47)

### T15 — Google Calendar et Drive en lecture réelle

## Parent

Carte [Connecteurs](https://github.com/charlescstpierr/ezerbot/issues/44). Spec : `docs/specs/connecteurs-google-slack.md`, décision 6 et parcours juge 5.

## What to build

Calendar et Drive utilisent le compte Google connecté par T11. Pas un second formulaire de client. Les portées ajoutées sont en lecture. « Mes rendez-vous de demain » et la lecture d’un fichier Drive nommé partent sans carte. Rien n’est écrit dans l’agenda ni dans Drive.

L’événement normalisé porte l’identifiant, le cas (créé, modifié, annulé, commence), l’instant de début et le fuseau. C’est le contrat de [#60](https://github.com/charlescstpierr/ezerbot/issues/60). Cette tranche débloque #60. Elle n’implémente pas la routine.

Démo : le rendez-vous de demain dans la bulle, puis un extrait du fichier Drive.

TDD : d’abord un test rouge (lecture sans carte, zéro appel d’écriture, événement complet, même consentement que Gmail).

## Acceptance criteria

- [ ] TDD : le test est rouge avant et vert après.
- [ ] Connecter Gmail suffit pour lire l’agenda et un fichier Drive. Aucun second formulaire.
- [ ] « Mes rendez-vous de demain » et la lecture du fichier partent sans carte.
- [ ] Aucun appel d’écriture agenda ou Drive n’est exposé.
- [ ] L’événement porte identifiant, cas, début et fuseau.
- [ ] Juge visuel : parcours 5 de la spec.

## Blocked by

- T11 — Connecter Gmail sans créer de client OAuth et lire les courriels

### T16 — Plusieurs comptes Google

## Parent

Carte [Connecteurs](https://github.com/charlescstpierr/ezerbot/issues/44). Spec : `docs/specs/connecteurs-google-slack.md`, décision 7 et parcours juge 6.

## What to build

« Ajouter un autre compte » ajoute un second utilisateur Google, avec son propre secret. Un seul compte est utilisé sans question. Plusieurs comptes, sans choix dans la conversation : le bot demande. Le choix vaut pour Gmail, Calendar et Drive de cet utilisateur. Déconnecter l’un laisse l’autre capable de lire. Slack n’est pas concerné.

Démo : deux adresses, la question, la déconnexion ciblée.

TDD : d’abord un test rouge (question quand c’est ambigu, lecture avec le compte choisi, l’autre compte intact après déconnexion).

## Acceptance criteria

- [ ] TDD : le test est rouge avant et vert après.
- [ ] Deux comptes Google coexistent, adresses affichées, secrets séparés.
- [ ] Sans choix, le bot demande. Avec un choix, la lecture porte le compte nommé.
- [ ] Déconnecter le second laisse le premier capable de lire.
- [ ] Slack n’a pas gagné un second compte dans cette tranche.
- [ ] Juge visuel : parcours 6 de la spec.

## Blocked by

- T11 — Connecter Gmail sans créer de client OAuth et lire les courriels
- [Plusieurs comptes GitHub dans le chat](https://github.com/charlescstpierr/ezerbot/issues/49)
