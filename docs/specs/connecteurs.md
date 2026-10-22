# Connecteurs

Statut : décisions du grill du 21 octobre 2026. Aucune implémentation de ce lot n’est commencée. La première tranche qui doit marcher de bout en bout, avec un vrai compte, est GitHub. Les tranches sont des issues enfants de la carte [Connecteurs](https://github.com/charlescstpierr/ezerbot/issues/44). Le complément [`connecteurs-google-slack.md`](connecteurs-google-slack.md) change ensuite Gmail, Calendar, Drive et Slack : « installation actuelle » désigne ici l’état de ce lot.

## Contexte

Charles utilise Ezerbot, l’app Mac en Rust, pour qu’un bot lise et agisse sur ses services. Le catalogue existe déjà. Il propose six services : Gmail, Google Calendar, Google Drive, Slack, Notion et GitHub. Un courtier OAuth local ouvre le navigateur, échange le code et range les jetons dans le coffre (trousseau macOS sur l’app signée, fichier de secours sinon). Connecter un service ouvre une approbation, puis le navigateur. Les outils d’un connecteur restent soumis aux approbations, avec un « Toujours » possible.

Cet existant ne suffit pas pour un usage réel :

- Cinq services sur six exigent que Charles crée lui-même un client OAuth et en saisisse l’identifiant et le secret. Notion est la seule exception. GitHub demande une OAuth App, l’URL de retour `http://127.0.0.1/oauth/github/callback`, l’identifiant et le secret.
- Le parcours n’a jamais été joué avec un vrai compte. Les tests parlent à un fournisseur de boucle locale.
- Un service ne garde qu’un compte. L’identité affichée est une adresse, pas une liste de comptes.
- Les serveurs ajoutés à la main sont des commandes locales (stdio). Il n’y a pas de serveur MCP en HTTPS à coller par son URL.
- Il n’y a pas de carte brouillon. Depuis `4f77a4c` (#130), un outil dont le nom dit un envoi sous le nom de Charles (courriel, message, commentaire) ou une suppression demande une confirmation à chaque fois, sans « Toujours ». Ce classement lit le nom de l’outil. Rien ne montre un texte modifiable, lié à l’empreinte de ce qui part, et un outil dont le nom ne dit pas l’envoi garde son « Toujours ».

Le catalogue et le courtier sont sur main (`b5d9219`, `c88bdd8`, `c63488a`). Les connecteurs MCP locaux avec approbation par outil sont `2bb6070`. Les empreintes `6acead7`, `749a35e` et `9ba1aa2` citées au grill ne sont pas dans `charlescstpierr/ezerbot` ; le comportement qu’elles désignent est celui de ces commits.

Le coffre refuse déjà d’écrire un secret dans SQLite. Sur l’app Mac signée, il écrit dans le trousseau. Si le trousseau est verrouillé ou refuse, l’écriture échoue, sans dialogue, et rien n’est copié dans le fichier de secours (`e8895d5`, #124). Les connecteurs passent par ce coffre.

## Verdict d’accès

GitHub documente deux façons d’obtenir un jeton utilisateur pour une GitHub App : le flux web, et le device flow. Le flux web échange le code avec un `client_secret`, obligatoire. Le device flow n’envoie que le `client_id` public, le `device_code` et le type `urn:ietf:params:oauth:grant-type:device_code`. La documentation GitHub dit que les applications de bureau doivent utiliser le device flow, et qu’il faut l’activer dans le réglage de l’app. Le jeton obtenu est un jeton utilisateur (`ghu_`). Les actions sont donc attribuées à Charles, pas au compte robot de l’app. Par défaut ce jeton expire au bout de 8 heures ; le rafraîchissement (`ghr_`) dure 6 mois. Les permissions ne sont pas des scopes OAuth : ce sont les permissions fines de la GitHub App, croisées avec les droits de Charles et avec les dépôts sur lesquels l’app est installée.

Une OAuth App en device flow aurait aussi un `client_id` public, sans secret. Son scope `repo` ouvrirait en lecture et en écriture tous les dépôts privés accessibles au compte. Ce lot ne la retient pas.

Un jeton d’installation (clé privée de la GitHub App) ferait parler le robot de l’app. Ce lot ne le retient pas : ce qui part doit porter le nom de Charles.

Le serveur MCP hébergé `https://api.githubcopilot.com/mcp/` n’est pas le connecteur GitHub de ce lot. Sa surface d’outils est large, et son accès est celui de Copilot. Ezerbot ne peut pas y classer lecture, publication et suppression. Le connecteur GitHub est un adaptateur à nous, qui appelle l’API REST GitHub avec le jeton utilisateur.

**Ce que Charles prépare une fois, hors de l’app.** Il crée une GitHub App nommée Ezerbot, active le device flow, et ne copie aucun client secret dans le dépôt, le binaire ou le trousseau. Le `client_id` public (pas l’App ID) est une valeur de configuration du build Mac, remplaçable par une variable d’environnement pour les tests et pour le juge. Permissions de dépôt : Metadata en lecture, Issues en lecture et écriture, Pull requests en lecture et écriture, Contents en lecture. Pas de permission d’administration. Pas de webhook dans ce lot. Charles installe l’app sur le compte du juge et limite l’installation au dépôt d’essai. Tant que le `client_id` manque, « Connecter » explique que l’application GitHub d’Ezerbot n’est pas configurée. Le formulaire identifiant / secret ne revient pas.

## Décisions

1. **GitHub se connecte par le device flow d’une GitHub App Ezerbot.** Un clic sur Connecter ouvre l’approbation déjà utilisée pour une connexion : elle nomme les permissions (issues, pull requests, contenu en lecture, dépôts choisis chez GitHub). Après accord, l’app affiche le code à 8 caractères, permet de le copier, et ouvre `https://github.com/login/device`. Charles s’identifie, autorise l’app et choisit les dépôts sur les pages de GitHub. Ezerbot interroge le jeton au rythme indiqué par GitHub, y compris après un `slow_down`, et s’arrête à l’expiration (15 minutes par défaut). Annuler, un code expiré, un refus GitHub ou un redémarrage de l’app pendant l’attente ne conservent aucun jeton. Le code mort ne peut plus aboutir.
2. **Le connecteur GitHub est à nous, avec une liste d’outils fermée.** Lecture, sans carte : lister les issues d’un dépôt `propriétaire/dépôt`, lire une pull request, lire un dépôt (description, branche par défaut, fichier nommé). Publication, carte brouillon : commenter une issue, créer une issue. Suppression, carte de confirmation : déconnecter un compte. Aucun autre outil GitHub n’est exposé. Fermer une issue, la supprimer, fusionner une pull request, pousser du code ou laisser une revue de code sont hors de cette liste.
3. **La politique d’action a trois classes.** Une lecture connue part seule. Une publication prépare une carte brouillon : texte exact, dépôt, numéro ou titre, compte GitHub. « Envoyer » publie ce texte, une seule fois, sous le compte affiché. « Annuler » ne publie rien. Le texte de la carte est modifiable avant l’envoi ; ce qui part est le texte validé. Il n’y a pas de « Toujours ». Une suppression affiche une carte de confirmation qui nomme la cible, avec « Confirmer » et « Annuler », sans « Toujours ». Un refus laisse le compte et les jetons en place. Cette carte brouillon est celle qu’un connecteur ultérieur devra utiliser pour un courriel, un message, un commentaire ou une issue. Ce lot la branche sur GitHub seulement.
4. **Plusieurs comptes par service, prouvé sur GitHub.** Une connexion est un service plus un utilisateur GitHub (identifiant numérique), avec son login affiché et son propre secret dans le coffre. Deux connexions GitHub coexistent. S’il n’y en a qu’une, le bot l’utilise. S’il y en a plusieurs, il demande laquelle avant d’agir, et la conversation garde ce choix jusqu’à ce que Charles en nomme une autre. Déconnecter l’une ne touche pas à l’autre.
5. **Le catalogue se cherche, et GitHub s’installe en un clic.** Le champ filtre le nom et la description des six services. Une recherche vide les montre tous. Aucun résultat le dit. La carte GitHub n’a ni identifiant ni secret. Les cinq autres services gardent leur installation actuelle, y compris la saisie d’un client quand elle existe aujourd’hui.
6. **Un serveur MCP HTTPS s’ajoute par son URL.** Le formulaire des serveurs personnalisés garde la commande locale et ajoute l’URL. HTTPS est exigé. `http` n’est accepté que pour `127.0.0.1`, après une confirmation explicite que le serveur est sur ce Mac. Pas de redirection. Le jeton éventuel est un champ mot de passe, rangé dans le coffre, renvoyé seulement vers l’origine approuvée, jamais réaffiché, jamais donné au modèle. Les outils découverts ne sont pas des lectures automatiques : le premier appel passe par l’approbation existante, arguments visibles, « Toujours » possible pour cet outil précis, sauf quand son nom dit un envoi ou une suppression (#130 : confirmation à chaque fois). Les classer en brouillon ou en suppression est hors de ce lot.
7. **Les jetons de compte ne suivent jamais un repli fichier.** Le coffre l’impose déjà à tout secret (`e8895d5`). Sur l’app Mac qui active le trousseau, un jeton d’accès ou de rafraîchissement de connecteur qui ne peut pas être écrit dans le trousseau n’est pas écrit dans le fichier de secours. La connexion échoue, la carte le dit, rien n’est conservé. Ce lot n’ajoute aucun chemin qui contourne ce refus. Le `device_code` ne quitte pas la mémoire d’attente. Le coffre garde sa limite actuelle : un secret qui ne tient pas fait échouer la connexion, sans valeur tronquée.

## Récits

1. En tant que Charles, je veux chercher « git » dans le catalogue, afin de trouver GitHub sans parcourir toute la liste.
2. En tant que Charles, je veux qu’une recherche vide montre les six services, afin de ne pas croire qu’ils ont disparu.
3. En tant que Charles, je veux qu’une recherche sans résultat le dise, afin de distinguer « rien trouvé » d’un panneau cassé.
4. En tant que Charles, je veux installer GitHub en un clic, afin de ne pas créer d’application OAuth.
5. En tant que Charles, je veux que la carte GitHub n’affiche ni identifiant ni secret, afin qu’on ne me demande pas un secret que je n’ai pas à fournir.
6. En tant que Charles, je veux que Gmail, Calendar, Drive, Slack et Notion gardent leur installation actuelle, afin que ce lot ne les casse pas.
7. En tant que Charles, je veux voir les permissions GitHub avant l’ouverture du navigateur, afin de refuser la connexion sans qu’un jeton soit gardé.
8. En tant que Charles, je veux un code court et la page `https://github.com/login/device`, afin de me connecter avec mon compte GitHub.
9. En tant que Charles, je veux copier ce code, afin de le coller chez GitHub.
10. En tant que Charles, je veux que l’app termine la connexion toute seule après mon autorisation, afin de ne pas coller un jeton en retour.
11. En tant que Charles, je veux qu’une annulation, un refus ou un code expiré laisse GitHub déconnecté, sans jeton.
12. En tant que Charles, je veux un nouveau code si le précédent a expiré, afin de réessayer.
13. En tant que Charles, je veux voir mon login GitHub sur la carte connectée, afin de savoir quel compte est lié.
14. En tant que Charles, je veux un second compte GitHub à côté du premier, afin de séparer le personnel et le travail.
15. En tant que Charles, je veux choisir le compte quand il y en a plusieurs, afin que la lecture et l’écriture visent le bon.
16. En tant que Charles, je veux que déconnecter un compte laisse l’autre intact, et n’efface que les jetons du compte nommé.
17. En tant que Charles, je veux demander les issues de `propriétaire/dépôt` et les voir dans le chat (numéro, titre, état, auteur), afin de lire le dépôt sans ouvrir le site.
18. En tant que Charles, je veux qu’un dépôt absent ou interdit soit expliqué en français, sans tempête de nouvelles tentatives.
19. En tant que Charles, je veux que cette lecture parte sans carte, parce qu’une lecture est sans risque.
20. En tant que Charles, je veux lire une pull request de la même façon (titre, état, auteur, branche de base).
21. En tant que Charles, je veux lire un dépôt (description, branche par défaut, fichier nommé) de la même façon.
22. En tant que Charles, je veux que la réponse nomme le compte GitHub utilisé.
23. En tant que Charles, je veux qu’un commentaire préparé s’affiche en carte brouillon, avec le texte exact, le dépôt, l’issue et le compte.
24. En tant que Charles, je veux qu’un clic sur Envoyer publie ce texte exact, sous mon nom, sur GitHub.
25. En tant que Charles, je veux qu’Annuler ne publie rien.
26. En tant que Charles, je veux corriger le texte sur la carte avant l’envoi, afin que les mots publiés soient ceux que j’ai validés.
27. En tant que Charles, je veux qu’un second clic sur Envoyer ne crée pas un second commentaire.
28. En tant que Charles, je veux la même carte pour créer une issue, titre et corps, afin de la publier d’un clic sous mon nom.
29. En tant que Charles, je veux qu’aucune carte brouillon n’offre « Toujours », afin que le prochain commentaire attende encore mon clic.
30. En tant que Charles, je veux que déconnecter un compte GitHub demande une confirmation qui nomme ce compte.
31. En tant que Charles, je veux que cette confirmation n’offre pas « Toujours ».
32. En tant que Charles, je veux qu’un refus de confirmation laisse le compte et ses jetons en place.
33. En tant que Charles, je veux qu’une confirmation acceptée retire les jetons locaux même si GitHub ne confirme pas la révocation, et que la carte le dise.
34. En tant que Charles, je veux qu’un outil classé suppression ne parte jamais seul et ne puisse pas être couvert par « Toujours ».
35. En tant que Charles, je veux ajouter un serveur MCP par une URL HTTPS, sans commande locale.
36. En tant que Charles, je veux qu’un serveur `http://127.0.0.1` ne soit accepté qu’après une confirmation « sur ce Mac ».
37. En tant que Charles, je veux que le secret de ce serveur reste dans le coffre, sans réaffichage et sans passage dans le modèle.
38. En tant que Charles, je veux voir ses outils après la connexion.
39. En tant que Charles, je veux qu’un outil de ce serveur demande l’approbation existante, arguments visibles, et ne soit pas traité comme une lecture GitHub.
40. En tant que Charles, je veux qu’une redirection ou un autre hôte soit refusé, afin que le jeton ne parte pas ailleurs.
41. En tant que Charles, je veux que les jetons GitHub du build signé vivent dans le trousseau, afin qu’un fichier du Mac ne les contienne pas.
42. En tant que Charles, je veux qu’un trousseau verrouillé fasse échouer la connexion sans écrire le jeton dans le fichier de secours.
43. En tant que Charles, je veux que le chat, les journaux et le contexte du modèle ne contiennent ni jeton, ni `device_code`, ni secret de client.
44. En tant que Charles, je veux qu’un jeton d’accès expiré se renouvelle seul à la lecture suivante.
45. En tant que Charles, je veux qu’un rafraîchissement mort me demande de reconnecter, sans réutiliser l’ancien jeton d’accès.

## Décisions d’implémentation

- Le catalogue garde ses six entrées. Seule l’entrée GitHub change de parcours : plus de client saisi, device flow, une connexion par utilisateur GitHub. Les entrées Gmail, Calendar, Drive, Slack et Notion restent sur le courtier actuel.
- Le `client_id` est public. Le point d’échange des jetons de production est celui de GitHub. Les tests et le juge local peuvent viser le fournisseur de boucle déjà utilisé par le catalogue. Tout autre hôte est refusé. Aucun `client_secret` n’est lu, envoyé ou stocké.
- Chaque connexion GitHub a son secret dans le coffre déjà utilisé pour les connecteurs. Le secret contient le jeton d’accès, le jeton de rafraîchissement et l’échéance. Le login et l’identifiant GitHub sont des attributs de la connexion, pas des secrets. Remplacer les jetons au rafraîchissement retire l’ancien rafraîchissement dans la même écriture.
- Tous les outils GitHub passent par « le compte que cette demande a résolu ». Avec un seul compte, c’est celui-là. Avec zéro compte, le chat dit de connecter GitHub. Avec plusieurs comptes et aucun choix dans la conversation, le bot demande. La première tranche implémente le cas d’un seul compte. La tranche multi-comptes ajoute la question, sans changer l’outil.
- Lister les issues lit une page (30) du dépôt nommé `propriétaire/dépôt`. La bulle affiche numéro, titre, état, auteur, et le compte. S’il y en a d’autres, elle le dit. Elle ne déverse pas le JSON. Un dépôt introuvable, un accès refusé ou une limitation de débit GitHub produisent une phrase française, sans nouvel essai en boucle.
- La carte brouillon est une demande humaine à usage unique, liée à l’empreinte du texte validé, du compte et de la cible. Un second Envoyer sur la même carte ne rappelle pas GitHub. Changer le texte avant l’envoi change l’empreinte. L’envoi n’a lieu qu’après le clic, jamais pendant la préparation de la carte.
- La carte de confirmation de suppression est une autre demande, sans règle « Toujours ». La déconnexion retire le secret local même quand la révocation distante échoue, et la phrase actuelle de révocation non confirmée reste la phrase montrée dans ce cas.
- Le transport HTTPS des serveurs ajoutés réutilise le client MCP HTTP déjà limité (pas de redirection, session bornée). L’URL et l’origine approuvées sont celles saisies. Le mode commande locale ne change pas.
- Aucun secret de connexion n’entre dans le contexte du modèle, la mémoire, ni l’export.

## Décisions de test

Un bon test observe le comportement visible : l’état du catalogue, le texte de la bulle, les boutons de la carte, les appels comptés vers le fournisseur factice, et l’absence du jeton dans la base, le transcript et les journaux. Il ne vérifie pas l’ordre interne des fonctions.

La couture unique est celle du catalogue et du courtier : un test du wrapper contre un fournisseur de boucle locale, puis le test de présentation qui rend la carte ou la bulle à partir de cet état. On n’ajoute pas de harnais. Le device flow et l’API REST GitHub sont joués par ce fournisseur. Le coffre des tests wrapper reste le coffre fichier jetable. L’absence de repli fichier pour un jeton de compte, quand le trousseau est verrouillé, se vérifie sur la couture trousseau déjà réservée à macOS.

Chaque tranche commence par un test rouge sur cette couture, puis le code qui le rend vert. Le fournisseur scripté ne prouve pas le compte réel. Le juge visuel sur Mac, avec le vrai compte GitHub de Charles et le build qui active le trousseau, est le critère d’acceptation des parcours ci-dessous.

## Critères d’acceptation

Un juge visuel sur Mac coche ces parcours dans l’app signée, avec la GitHub App Ezerbot préparée, le `client_id` public configuré, et un dépôt du compte qui contient au moins une issue ouverte, une pull request ouverte et un fichier lisible. Il envoie les phrases, attend la bulle, clique les cartes. Une capture immobile ne suffit pas. Le jeton ne doit apparaître nulle part à l’écran.

1. **Connexion et issues.** La carte GitHub n’a pas de champ identifiant ni secret. Connecter affiche les permissions, puis un code et la page GitHub. Après autorisation, la carte dit connecté et montre le login. Phrase : « Liste les issues de `propriétaire/dépôt`. » La bulle montre numéro, titre, état et auteur de l’issue connue, et le login, sans carte d’approbation supplémentaire. Gmail propose encore ses champs de client. Annuler pendant le code, puis reconnecter, aboutit. Quitter l’app pendant le code et la rouvrir ne connecte pas avec l’ancien code.
2. **Pull request et dépôt.** « Montre la pull request de `propriétaire/dépôt`. » La bulle donne titre, état, auteur et branche de base. « Lis le README de `propriétaire/dépôt`. » La bulle donne la description du dépôt, la branche par défaut et un extrait du fichier. Aucune carte brouillon.
3. **Commentaire.** « Commente l’issue N de `propriétaire/dépôt` avec : Bonjour du brouillon. » Une carte brouillon montre ce texte, le dépôt, le numéro, le login. Elle n’a pas de « Toujours ». Annuler : l’issue n’a pas de nouveau commentaire sur GitHub. Rejouer la demande, corriger le texte en « Bonjour validé », Envoyer une fois : GitHub montre un seul commentaire, auteur le login de Charles, corps « Bonjour validé ». Un second clic sur la même carte ne crée pas un second commentaire.
4. **Issue.** « Ouvre une issue “Essai Ezerbot” sur `propriétaire/dépôt`, corps : créée depuis la carte. » La carte montre titre et corps. Annuler ne crée rien. Envoyer crée l’issue sous le login de Charles. La bulle donne son numéro. L’issue est visible sur GitHub.
5. **Suppression.** Déconnecter le compte affiche une confirmation qui nomme le login, sans « Toujours ». Annuler laisse la carte connectée. Confirmer retire le compte. La phrase dit si GitHub a confirmé la révocation ou non. Une lecture suivante dit de reconnecter.
6. **Deux comptes.** Connecter un second compte GitHub. La carte montre les deux logins. « Liste les issues de `propriétaire/dépôt`. » Le bot demande quel compte. Chaque choix liste avec le login choisi. Déconnecter le second laisse le premier capable de lister.
7. **Recherche.** Taper « git » ne laisse que GitHub dans la liste, et Connecter y est en un clic. Effacer la recherche rend les six services. Taper « zzzz-absent » affiche l’état vide.
8. **MCP HTTPS.** Ajouter l’URL HTTPS d’un serveur MCP de test contrôlé par Charles, avec un secret. Le secret n’est plus affiché. Les outils apparaissent. Demander d’en utiliser un ouvre l’approbation existante, arguments visibles, et n’exécute rien avant l’accord. Une URL `http` autre que `127.0.0.1` est refusée. Une redirection est refusée. La commande locale d’un serveur personnalisé fonctionne encore.
9. **Trousseau.** Après le parcours 1, le jeton n’est pas dans un fichier de coffre en clair du répertoire de l’app ; il est dans le trousseau. Verrouiller le trousseau et reconnecter échoue avec une phrase française, sans créer ce fichier pour l’occasion.

## Hors-périmètre

- Faire marcher Gmail, Google Calendar, Google Drive, Slack ou Notion sans client créé par Charles. Ils restent au parcours actuel.
- Le serveur MCP hébergé de GitHub Copilot, une OAuth App au scope `repo`, et un jeton d’installation qui parlerait au nom du robot de l’app.
- Fermer ou supprimer une issue, fusionner ou commenter en revue une pull request, pousser des commits, gérer les réglages du dépôt.
- Le classement automatique des outils d’un MCP ajouté en carte brouillon ou en carte de confirmation propre à ce lot. Ils restent sur l’approbation existante, avec le classement par nom de #130, qui ne change pas.
- Les webhooks GitHub, l’iPhone, l’ordinateur cloud, et un partage de connexions entre Mac.
- Un « Toujours » sur une carte brouillon ou une confirmation de suppression.
- Appeler `github.com` depuis les tests automatisés. Le compte réel est réservé au juge.
- Changer le comportement du coffre quand le trousseau refuse, déjà fermé sur main.

## Risques

- **Un jeton hors du trousseau.** Un jeton `ghu_` ou `ghr_` dans un fichier est lisible par tout processus du compte macOS. Le coffre de main refuse déjà le repli fichier quand le trousseau refuse. Le risque est qu’une tranche contourne ce refus. Décision 7 le garde, et le juge du parcours 9 est le moment de vérité. Les tests Linux ne voient pas le trousseau : ils prouvent l’absence du jeton dans la base et les journaux ; le refus se prouve sur Mac.
- **Le `client_id` est public.** C’est voulu. Le secret qui ne doit jamais apparaître est le `client_secret`, et il n’est pas dans l’app. Un `device_code` volé pendant les 15 minutes d’attente permettrait de finir le flux si Charles entre aussi le code. L’attente est en mémoire, annulable, et invalidée au redémarrage.
- **Les permissions de l’app sont plus larges que la lecture.** Issues et pull requests sont en écriture pour que le commentaire et la création d’issue soient possibles. La classe publication est la barrière locale. Un bug qui classerait mal un outil enverrait du texte sans carte. La liste d’outils est fermée, et le test de la carte compte les appels au fournisseur : zéro avant Envoyer, un après.
- **Un « Toujours » existe encore pour les outils MCP ajoutés et pour les anciens connecteurs,** sauf ceux dont le nom dit un envoi ou une suppression (#130). Ce classement lit le nom : un outil qui envoie sous un nom neutre peut repartir sans carte brouillon. Ce lot ne le classe pas. La mitigation est l’approbation du premier appel, arguments visibles, portée limitée à cet outil.
- **SSO d’organisation.** Un dépôt d’org invisible après autorisation se dit en français (session SAML ou installation à élargir). Le bot ne réessaie pas en boucle.
- **Limite de 4 Ko du coffre.** Les jetons GitHub tiennent. Un secret MCP trop gros fait échouer l’ajout, sans troncature et sans secret partiel.
- **Renouvellement.** Un rafraîchissement qui échoue ne réutilise pas l’accès expiré. La carte passe à « reconnecter ». Deux rafraîchissements concurrents ne doivent pas garder un ancien `ghr_` que GitHub aurait déjà invalidé.
- **Le juge dépend de la GitHub App.** Sans le `client_id` et sans l’installation sur le dépôt d’essai, les parcours 1 à 6 ne peuvent pas être cochés. Les tests de la couture, eux, tournent avec le fournisseur factice.

## Tranches

Chaque tranche traverse le parcours visible, le coffre et le test. Le détail et les critères vivent dans l’issue. Les blocages sont aussi des dépendances natives sur le suivi. La première à prendre est la connexion GitHub et la liste des issues.

Carte : [Connecteurs](https://github.com/charlescstpierr/ezerbot/issues/44). Les tranches Google et Slack ([#89](https://github.com/charlescstpierr/ezerbot/issues/89), [#90](https://github.com/charlescstpierr/ezerbot/issues/90), [#100](https://github.com/charlescstpierr/ezerbot/issues/100), [#101](https://github.com/charlescstpierr/ezerbot/issues/101), [#102](https://github.com/charlescstpierr/ezerbot/issues/102), [#103](https://github.com/charlescstpierr/ezerbot/issues/103)) relèvent du complément cité plus haut.

1. [Connecter GitHub et lister les issues d’un dépôt](https://github.com/charlescstpierr/ezerbot/issues/45). Aucun bloquant. Première tranche à prendre.
2. [Lire une pull request et un dépôt GitHub](https://github.com/charlescstpierr/ezerbot/issues/46). Bloquée par 45.
3. [Commenter une issue GitHub depuis une carte brouillon](https://github.com/charlescstpierr/ezerbot/issues/47). Bloquée par 45.
4. [Créer une issue GitHub depuis une carte brouillon](https://github.com/charlescstpierr/ezerbot/issues/52). Bloquée par 47.
5. [Confirmer avant de déconnecter un compte GitHub](https://github.com/charlescstpierr/ezerbot/issues/48). Bloquée par 45.
6. [Plusieurs comptes GitHub dans le chat](https://github.com/charlescstpierr/ezerbot/issues/49). Bloquée par 45.
7. [Chercher dans le catalogue de connecteurs](https://github.com/charlescstpierr/ezerbot/issues/50). Bloquée par 45.
8. [Ajouter un serveur MCP en HTTPS](https://github.com/charlescstpierr/ezerbot/issues/51). Aucun bloquant.
