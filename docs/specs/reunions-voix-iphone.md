# Réunions, voix et iPhone

Statut : décisions de Charles du 21 octobre 2026. La première tranche, l’invitation du bot Recall ([#64](https://github.com/charlescstpierr/ezerbot/issues/64) : entrée Réunions, carte de consentement, Quitter, sortie à la fermeture), est implémentée, comme le choix du microphone pour la dictée ([#67](https://github.com/charlescstpierr/ezerbot/issues/67)), la voix de chaque bot ([#68](https://github.com/charlescstpierr/ezerbot/issues/68)) et la réponse depuis la notification iPhone ([#70](https://github.com/charlescstpierr/ezerbot/issues/70) : titre au nom du bot, feuille Répondre, envoi unique). Les autres tranches ne le sont pas. Le comportement visé est celui de Grok Bot 0.72.1 décrit dans le rapport du jour (réunions Recall, voix, iPhone), recalé sur le Mac de Charles : pas d’ordinateur cloud dans ce lot. Les tranches sont des issues enfants de la carte [Réunions, voix et iPhone](https://github.com/charlescstpierr/ezerbot/issues/63).

Vocabulaire déjà en usage : catalogue, connecteur, coffre, trousseau, carte brouillon, bot, fil, bulle, mode voix, dictée, mémo vocal, PWA, appairage. Une réunion Recall est l’entrée de catalogue de ce lot. Une capture locale est l’enregistrement au microphone déjà livré.

## Contexte

Charles utilise Ezerbot sur son Mac. Le catalogue propose six services : Gmail, Google Calendar, Google Drive, Slack, Notion et GitHub. Les jetons de compte vont au coffre, au trousseau sur l’app signée.

La réunion déjà livrée est une capture du microphone du Mac. Elle dure au plus dix minutes, transcrit sur l’appareil, demande le consentement des personnes présentes, puis montre une carte : durée, transcription, résumé du bot, audio gardé quelques jours sur le Mac. Le Mac écoute la pièce. Personne n’est invité dans Meet, Zoom ou Teams.

La dictée et le mémo vocal transcrivent aussi sur le Mac. La dictée dure au plus soixante secondes et pose le texte dans le brouillon, à relire avant l’envoi. Le mémo joint l’audio au message. Un seul de ces gestes tient le microphone : dictée, mémo, capture locale, mode voix. Le mode voix écoute, envoie le texte, puis lit la réponse à voix haute. La voix et la vitesse (de 0,5 à 1,5) sont un réglage unique, pas un réglage du bot. L’écran du mode voix dit que l’audio reste sur le Mac. Il n’y a pas de choix du microphone.

L’iPhone est une PWA. Charles l’ajoute à l’écran d’accueil, saisit un code créé sur le Mac, et le Mac confirme l’appareil. Le transport est le Tailscale privé. L’écran d’appairage dit que le Mac doit rester allumé et Ezerbot ouvert. Une notification push porte le titre de l’événement et le corps fixe « Ouvrez Ezerbot pour consulter cet événement. » Un toucher ouvre l’app. Il n’y a pas de réponse saisie depuis la notification. Le texte du bot, les secrets et les arguments d’outils ne sont pas dans la notification.

Grok Bot, lui, invite un bot Recall.ai dans Meet, Zoom ou Teams. Le Mac ne capte pas l’audio de cette réunion. Le transcript et le résumé reviennent au bot. Une réponse dans le chat de la réunion passe par une carte brouillon. La dictée choisit le microphone, dure jusqu’à cinq minutes, part au raccourci ⌘D ou en maintenant le bouton. Chaque bot a une voix. L’appel vocal est une conversation en direct, avec un transcript affiché. L’iPhone reçoit des notifications et Charles peut répondre depuis l’une d’elles. Chez Grok, une partie de cela continue Mac éteint, parce que le bot vit sur l’ordinateur cloud du compte. Charles a décidé l’inverse pour ce lot.

## Énoncé du problème

Charles ne peut pas envoyer son bot dans une réunion Meet, Zoom ou Teams. La capture actuelle entend la pièce, pas l’appel, et elle s’arrête à dix minutes.

Il ne choisit pas le microphone. La dictée s’arrête à une minute. Tous les bots parlent avec la même voix. Le mode voix n’est pas un appel en direct : il transcrit, envoie, puis lit. Le transcript de cet échange n’est pas une conversation d’appel que l’on affiche ou masque.

Sur l’iPhone, une notification n’offre pas de réponse. Et rien de tout cela ne doit prétendre fonctionner quand le Mac est éteint : Ezerbot n’a pas encore d’ordinateur cloud.

## Solution

Réunions devient une entrée du catalogue. Connectée avec une clé Recall.ai, elle invite le bot de Charles dans un lien Meet, Zoom ou Teams. Le transcript s’affiche pendant l’appel. À la fin, le bot rédige un résumé. Pour parler dans le chat de la réunion, le bot prépare une carte brouillon ; seul l’envoi validé par Charles part.

La dictée gagne le choix du microphone, le raccourci, l’appui maintenu et une durée de cinq minutes, toujours transcrite sur le Mac. Chaque bot a sa voix. Un appel vocal en direct, distinct du mode voix, montre son transcript et le range dans le fil.

L’iPhone reste la PWA. La notification push permet de répondre. Tout ce lot s’exécute sur le Mac allumé, Ezerbot ouvert. Le cloud pourra reprendre les mêmes objets plus tard. Ce lot ne le fait pas, et l’écran le dit.

## Décisions

Grill et arbitrages de Charles, 21 octobre 2026.

1. **Réunions est une entrée du catalogue de connecteurs.** Elle s’ajoute aux six services. Son nom affiché est Réunions. La description parle d’un bot invité dans Meet, Zoom ou Teams. La capture au microphone déjà livrée reste en place, avec ses mots actuels (« Enregistrer une réunion », « Réunion · Capture consentie »).
2. **Le bot invité est Recall.ai.** Les liens acceptés sont Google Meet, Zoom et Microsoft Teams (`meet.google.com`, `zoom.us`, `teams.microsoft.com`, `teams.live.com`). Un autre lien est refusé par une phrase française. Aucune trace d’Attendee, MeetingBaas ou Skribby n’est à reproduire : ce lot n’a qu’un fournisseur.
3. **Le Mac n’écoute pas la réunion Recall.** L’audio de l’appel est capté par le bot Recall. La dictée, le mémo, la capture locale, le mode voix et l’appel vocal gardent l’exclusivité du microphone entre eux. Une réunion Recall n’entre pas dans cette exclusivité.
4. **Transcript, résumé, puis carte brouillon pour le chat.** Les lignes arrivent avec leur horaire. Le résumé est rédigé par le bot d’Ezerbot à partir de ces lignes, à la fin. Une réponse dans le chat de la réunion est une publication : carte brouillon, texte exact, un envoi, sans « Toujours ». Charles valide en cliquant Envoyer. Annuler n’envoie rien.
5. **La voix et l’iPhone reprennent le comportement visible de Grok Bot, sur les gestes que Charles a nommés.** Dictée, choix du microphone, voix par bot, appel vocal en direct avec transcript. iPhone : la PWA est l’app, notifications push, répondre depuis la notification.
6. **Ezerbot tourne sur le Mac.** Pas d’ordinateur cloud, pas de file distante, pas de bot Recall laissé en poste après la fermeture. La section Mac éteint dit ce qui s’arrête. Les identités (réunion, ligne de transcript, brouillon, message d’appel) restent stables, pour qu’un exécuteur futur puisse les reconnaître. Cet exécuteur n’existe pas ici.

### Réunions

Charles prépare une fois, hors de l’app, une clé d’API Recall.ai. L’entrée Réunions n’a pas d’identifiant OAuth à créer. Connecter ouvre un champ mot de passe. La clé va au coffre de cette entrée. Sur l’app qui active le trousseau, un échec d’écriture au trousseau n’écrit pas la clé dans le fichier de secours : la connexion échoue, la carte le dit, rien n’est conservé. La clé ne réapparaît pas, ne va pas dans le fil, les journaux, ni le contexte du modèle. Elle ne part que vers Recall.ai. Tout autre hôte est refusé. Sans clé, Connecter dit que la clé Recall n’est pas configurée.

Déconnecter cette entrée demande une confirmation qui nomme Réunions, sans « Toujours ». Annuler laisse la clé. Confirmer retire la clé locale. Une réunion en cours est quittée avant le retrait.

Rejoindre part d’un lien collé dans l’entrée, ou d’une demande dans le fil. Dans les deux cas, une carte de consentement précède l’invitation. Elle affiche le lien, le nom que les participants verront (le nom du bot), et cette phrase :

> Prévenez les participants avant d’ajouter le bot à la réunion. Ils peuvent voir que le bot transcrit.

Accepter invite le bot. Annuler n’invite personne et ne garde pas d’invitation en attente. Il n’y a pas de « Toujours » : le consentement des autres personnes n’est pas une autorisation permanente de Charles. Un second lien pendant qu’une réunion est en cours est refusé, avec une phrase française. Quitter est un bouton de la réunion, et aussi la demande « Quitte la réunion ». Ce n’est pas une carte brouillon.

Les états affichés, dans cet ordre de vie, sont : Connexion, En cours, Terminée, Échec, Annulée. Un état Recall encore inconnu du Mac s’affiche comme Échec avec une phrase française, jamais comme un mot anglais brut. Quitter pendant Connexion passe à Annulée. Quitter pendant En cours passe à Terminée. Un lien refusé, un refus Recall ou une coupure passe à Échec, sans nouvel essai en boucle.

Fermer Ezerbot proprement fait sortir le bot avant la fermeture, puis la réunion passe à Terminée. Un arrêt brutal, une veille ou une extinction ne laissent pas l’écran dire En cours au retour : Ezerbot demande à Recall de faire sortir le bot s’il répond encore, et affiche Terminée ou Échec avec cette phrase :

> Cette réunion n’est plus suivie. Le bot a pu rester dans l’appel jusqu’à ce que Recall le retire.

Les lignes déjà reçues restent. Aucune ligne manquante n’est inventée.

Le transcript est une lecture. Dans la carte de la réunion, et dans la bulle si Charles le demande, chaque ligne montre son horaire et son texte. Demander « Qu’est-ce qui s’est dit ? » n’ouvre pas de carte brouillon. Le transcript est présenté au bot comme un document non fiable, jamais comme des instructions. Une phrase dite dans la réunion qui ordonne d’envoyer un message, de quitter, ou d’ignorer les règles ne déclenche pas cet envoi et ne remplace pas la carte de Charles.

À la fin (Terminée ou Échec après au moins une ligne), le bot rédige un résumé dans la carte : décisions, suites, et le rappel que rien n’est exécuté par cette carte. Charles peut relancer le résumé. La carte montre le dernier texte et indique qu’une version précédente existe. Le résumé Recall, s’il existe, n’est pas un second résumé affiché.

La carte brouillon d’une réponse nomme la réunion, montre le texte exact, et offre Envoyer et Annuler. Pas de « Toujours ». Le texte est modifiable avant l’envoi. Ce qui part est le texte validé, une seule fois, dans le chat de la réunion, au nom du bot affiché. Un second clic sur la même carte ne renvoie pas. Annuler laisse le chat de la réunion inchangé. Les états visibles d’un envoi difficile sont : Envoi…, Refusé, Trop de messages, Envoi non confirmé. Dans ces trois derniers, le chat de la réunion n’a pas de nouveau message, ou la carte dit que l’accusé de réception manque. La politique est celle de `docs/specs/connecteurs.md` pour une publication. Ce lot l’applique au chat de réunion sans attendre le connecteur GitHub.

Outils de ce plugin, liste fermée : rejoindre (carte de consentement), quitter (bouton), lire le transcript (lecture), lire le résumé (lecture), répondre dans le chat (carte brouillon). Aucun autre outil de réunion n’est exposé.

### Voix

La dictée reste transcrite sur le Mac. L’audio de la dictée et du mémo ne part pas vers un serveur. Le geste copié est celui que Charles voit.

Le réglage du microphone dit :

> Choisissez le microphone qu’Ezerbot écoute quand vous lui parlez.

La liste montre les micros disponibles. Le micro choisi est marqué. S’il disparaît, Ezerbot revient au micro par défaut et le dit : « Le microphone choisi n’est plus disponible. Ezerbot utilise le microphone par défaut. » La dictée, le mode voix et l’appel vocal utilisent ce choix. La capture locale de réunion continue d’utiliser le microphone de l’enregistrement déjà livré.

Un clic court sur le bouton de dictée démarre ou arrête, comme aujourd’hui. Un appui maintenu d’au moins une demi-seconde parle tant que le bouton est tenu, puis transcrit au relâchement. ⌘D démarre ou arrête la dictée. La durée maximale passe à cinq minutes pour la dictée. À la limite, l’enregistrement s’arrête et le texte va dans le brouillon, à relire avant l’envoi. Le mémo vocal reste à soixante secondes. La capture locale reste à dix minutes. La langue de dictée est une langue de dictée installée sur le Mac. Seul le préfixe compte (`fr-CA` est le français). Si la langue n’est pas installée, la phrase actuelle demande d’installer les données de Dictée. Rien n’est envoyé à un serveur pour contourner ce manque.

Chaque bot a une voix, une vitesse et une langue de voix. La vitesse va de 0,5 à 2. La langue peut être Auto. Sans choix de Charles, une voix distincte est attribuée à chaque bot tant qu’il existe au moins deux voix. S’il n’en existe qu’une, les bots la partagent et le réglage le dit. Le réglage actuel, global, devient la voix des bots qui n’en ont pas encore, une seule fois. Le bot suivant, sans choix, prend une autre voix s’il en reste une. Le mode voix et l’appel vocal parlent avec la voix du bot de la conversation. Le mémo ne change pas de voix dans ce lot : il garde la lecture déjà livrée.

L’appel vocal est un second geste, à côté du mode voix. Le mode voix reste : transcription sur le Mac, envoi du texte, lecture de la réponse, audio non transmis. L’appel est une conversation en direct. Charles entend le bot pendant qu’il peut encore parler. Parler pendant que le bot parle coupe la voix du bot. Échap coupe aussi. L’écran de l’appel montre :

> L’audio de cet appel est transmis au fournisseur de voix le temps de l’appel. Il n’est pas gardé sur le Mac.

Un secret éphémère ouvre la session. Il n’est pas affiché, pas écrit dans le fil, pas journalisé. L’appel utilise le microphone choisi et la voix du bot. Le transcript de l’appel se remplit pendant l’appel. Afficher le transcript et Masquer le transcript sont les deux libellés. À la fin, le transcript est dans le fil, marqué Appel vocal, et il y reste après avoir quitté. Aucun fichier audio de l’appel n’est gardé. Une carte qui attend un clic (approbation, brouillon, secret) s’affiche et la voix attend. Elle ne valide pas la carte à l’oral.

L’appel et le mode voix ne tiennent pas le microphone ensemble. Lancer l’un pendant l’autre est refusé par la phrase de microphone occupé déjà utilisée. Quitter l’appel arrête la session. Fermer Ezerbot ou éteindre le Mac termine l’appel. Au retour, le transcript déjà écrit est dans le fil. L’appel n’est pas relancé seul.

L’appel ne se lance pas depuis la PWA. Le transcript, une fois dans le fil, se lit sur la PWA comme un autre message, tant que le Mac est allumé.

### iPhone

L’app iPhone de ce lot est la PWA déjà prévue : Safari, Sur l’écran d’accueil, puis l’icône. Pas d’app App Store, pas de TestFlight, pas de QR. L’appairage reste le code à usage unique, la confirmation sur le Mac, le cookie d’appareil, Tailscale privé. La phrase « le Mac doit rester allumé et Ezerbot ouvert » reste.

Quand le bot a un message pour Charles et que la PWA n’est pas ouverte sur cette conversation, une notification s’affiche sur l’iPhone. Le titre est le nom du bot. Le corps reste générique : il ne contient pas le texte de la réponse, pas le transcript, pas un secret, pas un argument d’outil. Un toucher simple ouvre la conversation, comme aujourd’hui.

La notification offre Répondre. Charles saisit son texte et l’envoie sans parcourir le fil. Si la notification du système ne peut pas contenir de champ (cas d’une PWA sur iOS), Répondre ouvre la PWA sur une feuille de réponse de cette conversation, champ prêt, un bouton Envoyer. Le message apparaît dans le fil du Mac, comme un message de Charles. Un second envoi de la même feuille ne crée pas un second message. La réponse du bot peut suivre, Mac allumé.

Si la PWA est déjà ouverte sur cette conversation, aucune notification n’est ajoutée.

### Mac éteint

« Mac éteint » couvre aussi la veille, Ezerbot quitté, et le Mac injoignable depuis l’iPhone. Le cloud ne rattrape rien.

| Geste | Mac éteint |
|---|---|
| Inviter le bot, le suivre, le faire quitter | Impossible. Au retour, l’état n’est plus En cours. |
| Nouvelles lignes de transcript, nouveau résumé | Rien n’arrive. Les lignes et le résumé déjà écrits restent sur le Mac et réapparaissent à l’ouverture. |
| Carte brouillon non envoyée | Elle reste à envoyer. Elle ne part pas toute seule au réveil. |
| Message déjà validé dont l’envoi n’a pas été accusé | La carte dit Envoi non confirmé. Elle ne renvoie pas toute seule. |
| Dictée, choix du micro, mode voix, appel vocal | Impossibles. Le transcript d’appel déjà dans le fil reste sur le Mac. |
| PWA | L’icône peut ouvrir l’écran déjà en cache. L’écran dit que le Mac est injoignable. Aucun message n’est marqué envoyé. |
| Notification nouvelle | Pas émise. Le Mac est l’émetteur. |
| Réponse saisie pendant que le Mac est injoignable | Refusée. Elle n’est pas mise en file pour plus tard. |
| Capture locale | Impossible, comme aujourd’hui, tant que l’app est fermée. |

Ce qui revient à l’ouverture : les transcripts, résumés, brouillons non envoyés et messages de fil déjà écrits sur le Mac. Une réunion manquée n’est pas rejouée. Un appel coupé n’est pas rappelé.

## Récits

1. En tant que Charles, je veux voir Réunions dans le catalogue, afin d’inviter un bot sans passer par la capture du microphone.
2. En tant que Charles, je veux coller une clé Recall une seule fois, afin de ne plus la revoir à l’écran.
3. En tant que Charles, je veux que cette clé reste dans le trousseau, afin qu’un fichier du Mac ne la contienne pas.
4. En tant que Charles, je veux qu’un trousseau verrouillé fasse échouer la connexion sans écrire la clé ailleurs.
5. En tant que Charles, je veux coller un lien Meet, Zoom ou Teams, afin que le bot rejoigne cet appel.
6. En tant que Charles, je veux qu’un lien quelconque soit refusé en français, afin de voir que l’invitation n’a pas été tentée.
7. En tant que Charles, je veux une carte de consentement à chaque invitation, afin de prévenir les participants avant qu’ils voient le bot.
8. En tant que Charles, je veux que cette carte nomme le bot qu’ils verront, et qu’elle n’offre pas « Toujours ».
9. En tant que Charles, je veux qu’Annuler sur cette carte laisse la réunion sans bot.
10. En tant que Charles, je veux voir Connexion puis En cours, afin de savoir que le bot est dans l’appel.
11. En tant que Charles, je veux quitter la réunion par un bouton, afin que le bot sorte et que l’état passe à Terminée.
12. En tant que Charles, je veux qu’une seconde invitation pendant une réunion en cours soit refusée.
13. En tant que Charles, je veux que fermer Ezerbot fasse sortir le bot avant de quitter.
14. En tant que Charles, je veux qu’après un arrêt brutal la réunion ne s’affiche plus En cours, avec une phrase qui dit qu’elle n’est plus suivie.
15. En tant que Charles, je veux dicter ou lancer le mode voix pendant qu’un bot Recall est dans un appel, afin que cette réunion ne prenne pas le microphone du Mac.
16. En tant que Charles, je veux que la capture « Enregistrer une réunion » fonctionne encore, afin de garder le geste local.
17. En tant que Charles, je veux lire les lignes du transcript avec leur horaire pendant l’appel.
18. En tant que Charles, je veux demander le transcript dans le fil et le recevoir sans carte brouillon.
19. En tant que Charles, je veux un résumé à la fin, afin de relire décisions et suites sans réécouter.
20. En tant que Charles, je veux relancer le résumé et voir que la carte montre le dernier, en signalant une version précédente.
21. En tant que Charles, je veux qu’une consigne dite dans la réunion ne fasse pas partir un message ni une action.
22. En tant que Charles, je veux qu’une réponse préparée s’affiche en carte brouillon, avec le texte exact et le nom de la réunion.
23. En tant que Charles, je veux corriger ce texte, puis l’envoyer d’un clic, afin que le chat de la réunion reçoive les mots validés.
24. En tant que Charles, je veux qu’Annuler laisse le chat de la réunion inchangé.
25. En tant que Charles, je veux qu’un second clic n’envoie pas un second message.
26. En tant que Charles, je veux que la carte n’offre pas « Toujours ».
27. En tant que Charles, je veux voir Refusé, Trop de messages ou Envoi non confirmé quand l’envoi ne s’est pas terminé, afin de ne pas croire que le chat a le message.
28. En tant que Charles, je veux choisir le microphone d’Ezerbot, afin qu’il écoute le bon appareil.
29. En tant que Charles, je veux que le retrait de ce microphone fasse revenir au micro par défaut, avec une phrase.
30. En tant que Charles, je veux dicter d’un clic, d’un appui maintenu, ou avec ⌘D, afin de poser le texte dans le brouillon.
31. En tant que Charles, je veux que la dictée puisse durer jusqu’à cinq minutes, puis s’arrêter et me laisser relire.
32. En tant que Charles, je veux que l’audio de la dictée reste sur le Mac.
33. En tant que Charles, je veux une voix, une vitesse et une langue pour chaque bot, afin de les distinguer à l’oreille.
34. En tant que Charles, je veux qu’un bot sans voix choisie reçoive une voix différente du précédent, quand une autre voix existe.
35. En tant que Charles, je veux un appel en direct, afin de parler avec le bot sans attendre la fin d’une dictée.
36. En tant que Charles, je veux voir, avant et pendant l’appel, que l’audio part au fournisseur de voix et n’est pas gardé sur le Mac.
37. En tant que Charles, je veux afficher ou masquer le transcript pendant l’appel, puis le retrouver dans le fil.
38. En tant que Charles, je veux couper la voix du bot en parlant ou avec Échap.
39. En tant que Charles, je veux qu’une carte à valider s’affiche pendant l’appel et attende mon clic.
40. En tant que Charles, je veux que le mode voix actuel continue, audio sur le Mac, à côté de l’appel.
41. En tant que Charles, je veux une notification sur l’iPhone quand le bot me parle et que je ne suis pas dans la conversation.
42. En tant que Charles, je veux que cette notification nomme le bot sans copier le texte de la réponse ni un secret.
43. En tant que Charles, je veux répondre depuis la notification, afin que mon texte arrive dans le fil sans que je cherche la conversation.
44. En tant que Charles, je veux qu’un second envoi de cette réponse ne double pas le message.
45. En tant que Charles, je veux qu’aucune notification n’apparaisse si je suis déjà dans cette conversation sur la PWA.
46. En tant que Charles, je veux que l’iPhone, Mac éteint, dise que le Mac est injoignable et ne marque aucun message comme envoyé.
47. En tant que Charles, je veux qu’une réunion, un appel ou une notification ne continuent pas tout seuls Mac éteint.
48. En tant que Charles, je veux retrouver au rallumage ce qui était déjà écrit sur le Mac, et rien qui aurait été inventé pendant l’extinction.

## Décisions d’implémentation

- L’entrée Réunions suit le catalogue déjà livré : une carte, un état connecté, un secret dans le coffre. Elle n’utilise pas le courtier OAuth des six autres. Le secret est la clé Recall, seule.
- Le fournisseur de production est Recall.ai. Les tests et le juge local peuvent viser un fournisseur de boucle. Tout autre hôte est refusé. La clé et les liens de réunion ne sortent pas de cet hôte.
- Une réunion a un identifiant stable, un lien, un état, le nom affiché aux participants, et les lignes déjà reçues. Cet identifiant est celui qu’un exécuteur futur pourrait réclamer. Rien ne le réclame à distance dans ce lot.
- Le suivi Recall vit dans Ezerbot ouvert. L’agent launchd des routines ne devient pas un client Recall. Quitter l’app est une demande de sortie du bot, puis la fermeture.
- Le transcript Recall et le résumé de ce lot sont une carte distincte de la carte « Réunion · Capture consentie ». Les deux peuvent exister dans le même fil. Elles ne partagent pas leur audio : la réunion Recall n’a pas d’audio local.
- La carte brouillon de réunion est une demande humaine à usage unique, liée au texte validé, au bot et à la réunion. Changer le texte change ce qui part. L’envoi n’a lieu qu’après le clic. La même règle vaut pour une publication ultérieure d’un autre connecteur : ce lot ne crée pas une seconde politique.
- Le choix de microphone est un réglage du Mac, partagé par la dictée, le mode voix et l’appel. Il survit au redémarrage. Il ne survit pas à un micro disparu : le repli est immédiat et annoncé.
- La voix du bot est un réglage du bot, pas du Mac. Vitesse de 0,5 à 2. Le mode voix lit avec cette voix. L’appel parle avec cette voix.
- L’appel vocal est une session en direct, secret éphémère, transcript texte seulement. Le mode voix n’est pas réécrit en appel. Les deux gestes ont deux boutons.
- La notification iPhone garde un corps générique. Son titre est le nom du bot pour une réponse, générique pour les autres événements. Le chemin Répondre ajoute le texte saisi par Charles au fil, comme un message utilisateur de cette conversation. Le Mac ouvert est le seul accepteur.
- Aucun secret (clé Recall, secret d’appel, cookie d’appareil, jeton) n’entre dans le fil, le transcript, la notification, la mémoire, ni l’export.

## Décisions de test

Un bon test observe le comportement visible : l’état du catalogue, le texte de la bulle, les boutons et les phrases des cartes, les lignes avec leur horaire, le texte du brouillon, le micro annoncé, la voix affichée pour le bot, le transcript d’appel dans le fil, le titre de notification, le message arrivé depuis la feuille de réponse. Il ne vérifie pas l’ordre interne des fonctions.

La couture des réunions est celle du catalogue et du fournisseur de boucle déjà utilisés pour les connecteurs : un test du parcours contre ce fournisseur, puis le rendu de la carte. On n’ajoute pas de harnais. Le fournisseur scripté compte les invitations, les sorties et les envois de chat : zéro envoi avant Envoyer, un après, zéro après Annuler. La clé n’apparaît ni dans la base en clair, ni dans le transcript, ni dans les journaux. L’absence de repli fichier, quand le trousseau est verrouillé, se vérifie sur la couture trousseau déjà réservée à macOS.

La couture de la voix est celle des tests de dictée et de session déjà joués sans microphone réel. Le temps est injectable pour la limite de cinq minutes. Le fournisseur d’appel est une boucle. Le test regarde le transcript et l’absence d’un fichier audio d’appel.

La couture iPhone est celle des tests d’appareil déjà joués sans Tailscale réel. Le test regarde le titre de notification, l’absence du texte du bot dans le corps, et le message de Charles créé par la réponse. Une réponse pendant que l’accès est coupé n’ajoute pas de message envoyé.

Chaque tranche commence par un test rouge sur sa couture, puis le code qui le rend vert. Le fournisseur scripté ne prouve pas un vrai appel Meet ni un vrai iPhone. Le juge visuel sur le Mac de Charles est le critère d’acceptation des parcours ci-dessous. Un test vert ne remplace pas l’écran quand le critère est une phrase, une carte, ou le bot vu dans la réunion.

## Critères d’acceptation

Un juge visuel sur le Mac coche ces parcours dans l’app, avec une clé Recall à lui, un appel court qu’il héberge (Meet, Zoom ou Teams), et, pour l’iPhone, la PWA installée sur l’écran d’accueil, Mac allumé, Tailscale joint. Il envoie les phrases, attend la bulle, clique les cartes, regarde l’appel et le téléphone. Une capture immobile ne suffit pas. La clé Recall et le secret d’appel ne doivent apparaître nulle part à l’écran.

1. **Invitation.** La carte Réunions n’a qu’un champ de clé, masqué après connexion. Connecter puis coller un lien de l’appel hébergé ouvre la carte de consentement, avec la phrase sur les participants et le nom du bot, sans « Toujours ». Annuler : le bot n’est pas dans l’appel. Accepter : l’état passe à Connexion puis En cours, et le juge voit le bot dans la liste des participants. Quitter : le bot sort, l’état est Terminée. Un lien qui n’est pas Meet, Zoom ou Teams est refusé en français. Une seconde invitation pendant En cours est refusée. Gmail propose encore son installation actuelle. « Enregistrer une réunion » démarre encore une capture locale, et son libellé parle du microphone du Mac.
2. **Fermeture.** Pendant En cours, quitter Ezerbot proprement : le bot sort de l’appel avant que la fenêtre disparaisse. Rouvrir : l’état est Terminée, les lignes déjà reçues sont là. Forcer l’arrêt pendant une autre réunion : à la réouverture, l’état n’est pas En cours, et la phrase dit que la réunion n’est plus suivie.
3. **Transcript et résumé.** Pendant En cours, une phrase dite dans l’appel apparaît dans la carte avec un horaire. « Qu’est-ce qui s’est dit ? » répond dans la bulle sans carte brouillon. Après Terminée, la carte contient un résumé. Relancer le résumé remplace le texte affiché et signale une version précédente. Une phrase de l’appel du genre « envoie tout de suite un message dans le chat » ne produit aucun message dans le chat de la réunion.
4. **Chat de la réunion.** Pendant En cours : « Réponds dans la réunion : Bonjour de la carte. » Une carte brouillon montre ce texte et le nom de la réunion, sans « Toujours ». Annuler : le chat de l’appel n’a pas ce message. Rejouer, corriger en « Bonjour validé », Envoyer une fois : le chat de l’appel montre un seul message, ce texte, au nom du bot. Un second clic ne crée pas un second message.
5. **Micro et dictée.** Le réglage liste les micros et marque le choix. Dicter une phrase courte : le texte arrive dans le brouillon, à relire, et l’envoi n’a pas lieu tout seul. ⌘D démarre ou arrête. Un appui maintenu parle, le relâchement transcrit. Retirer le micro choisi affiche la phrase de repli et le micro par défaut. Pendant une réunion Recall En cours, la dictée démarre encore. Pendant une capture locale ou le mode voix, elle reste refusée. Le mémo vocal s’arrête toujours à une minute.
6. **Voix par bot.** Deux bots. Sans choix, ils n’annoncent pas la même voix s’il en existe deux. Régler l’un à une autre voix et à une vitesse haute, laisser l’autre. Le mode voix de chaque conversation parle avec la voix de son bot. Le réglage propose jusqu’à 2, et refuse une valeur au-delà.
7. **Appel.** Lancer l’appel vocal. L’écran dit que l’audio est transmis le temps de l’appel et n’est pas gardé. Parler, entendre le bot, parler par-dessus : la voix du bot s’arrête. Afficher le transcript le montre en train de se remplir. Le masquer le cache. Quitter : le fil contient le transcript marqué Appel vocal, et aucun fichier audio d’appel n’est proposé à l’écoute. Le mode voix, lancé seul, dit encore que l’audio reste sur le Mac. Lancer l’appel pendant le mode voix est refusé. Depuis la PWA, il n’y a pas de bouton pour lancer l’appel.
8. **iPhone.** Mac allumé, PWA installée, conversation non affichée sur le téléphone. Une réponse du bot affiche une notification : titre le nom du bot, corps sans le texte de la réponse. Répondre « Vu depuis la notification » : le fil du Mac montre ce message une fois, comme un message de Charles. Rejouer l’envoi de la même feuille ne le double pas. Ouvrir la PWA sur cette conversation, puis faire répondre le bot : pas de nouvelle notification.
9. **Mac éteint.** Éteindre le Mac ou quitter Ezerbot. La PWA dit que le Mac est injoignable. Une saisie n’est pas marquée envoyée. Aucune notification nouvelle n’arrive. Rallumer : le fil ne contient pas cette saisie. La réunion de l’essai n’est pas redevenue En cours. L’appel n’a pas redémarré. Le transcript déjà reçu est toujours là.

## Hors-périmètre

- L’ordinateur cloud, une file qui accepterait les liens, les transcripts ou les notifications Mac éteint, et un bot Recall qui resterait dans l’appel après une fermeture propre.
- Retirer ou réécrire la capture locale de dix minutes, le mode voix, le mémo d’une minute, l’appairage, ou le corps générique des notifications.
- La transcription de dictée par un serveur, et la liste de cinquante-sept langues de Grok. La dictée reste sur le Mac, dans les langues installées.
- Les noms de voix de Grok (Altair, Ara, et la suite). La liste est celle que le Mac et l’appel savent vraiment prononcer.
- Lancer l’appel vocal, la dictée Mac, ou le choix du micro depuis l’iPhone. La dictée du clavier iOS reste celle du système.
- Une app App Store, TestFlight, le bouton Action de l’iPhone, Android.
- Planifier une réunion, la récurrence, le lien avec le calendrier, le rappel à l’heure d’un événement. Le statut affiché ne propose pas de créer une réunion Planifiée.
- Le message interne au chat de réunion, les notes visibles des seuls coéquipiers, une politique d’équipe qui interdirait les réunions.
- Les appels téléphoniques sortants.
- Un second compte Recall, et la recherche dans le catalogue. L’entrée est visible dans la liste. Si la recherche du catalogue existe au moment de la tranche, elle doit montrer cette entrée ; ce lot ne construit pas la recherche.
- Faire de l’agent des routines un client Recall.
- Appeler Recall.ai, un vrai Meet, ou un vrai service push depuis les tests automatisés. Le juge s’en charge.

## Risques

- **La clé Recall est un secret de compte.** La décision sur le trousseau est celle déjà prise pour les jetons de connecteur : pas de repli fichier. Le juge du parcours 1, trousseau verrouillé, est le moment de vérité sur Mac. Les tests hors macOS prouvent l’absence de la clé dans la base et les journaux.
- **Le bot est visible.** Les participants voient un participant qui transcrit. La carte de consentement est la parade, répétée à chaque lien. Un bug qui inviterait avant le clic contourne le consentement. Le test compte zéro invitation avant Accepter.
- **Le chat de la réunion parle aux participants.** Un outil classé lecture par erreur enverrait du texte sans carte. La liste d’outils est fermée. Le test compte zéro envoi avant Envoyer, un après, zéro après Annuler.
- **Arrêt brutal.** Recall peut laisser le bot dans l’appel jusqu’à son propre délai. Ezerbot ne peut pas le retirer si le Mac est déjà mort. La phrase de reprise dit cette limite. Elle ne doit pas dire que le bot est sorti si Ezerbot n’a pas reçu la confirmation.
- **Deux cartes « réunion ».** La capture locale et le bot Recall se ressemblent. Les libellés de la décision 1 sont la parade. Le parcours 1 du juge vérifie les deux.
- **L’appel envoie l’audio dehors.** C’est le geste que Charles a demandé, et il contredit la promesse du mode voix. Les deux écrans doivent garder chacun sa phrase. Un bug qui enverrait l’audio du mode voix, ou qui garderait un fichier de l’appel, casse la décision. Le test d’appel vérifie l’absence de fichier audio.
- **La feuille de réponse n’est pas un champ dans la bannière iOS.** Sur une PWA, le système peut refuser la saisie dans la notification. La feuille ouverte depuis la notification est le comportement accepté. Sur iOS, la notification n’a pas non plus de bouton Répondre : un toucher ouvre la conversation, qui offre « Répondre à <bot> » vers la même feuille. Le juge coche le parcours 8 sur cette feuille. Un vrai champ dans la bannière, s’il apparaît, doit produire le même message unique dans le fil.
- **Notification trop bavarde.** Le titre nomme le bot. Y ajouter le résumé ou le transcript exposerait la réunion sur l’écran verrouillé. Le corps reste générique. Le test compare le corps à cette règle.
- **Longue réunion.** Ce lot ne pose pas de plafond de dix minutes sur Recall. Un appel long remplit le transcript sur le Mac. Le résumé part des lignes reçues. Si Recall coupe de son côté, l’état passe à Terminée ou Échec avec une phrase, sans boucle.
- **Le catalogue à six entrées.** `docs/specs/connecteurs.md` compte six services pour la recherche et pour « les autres restent ». Cette spec ajoute une septième entrée et ne change pas l’installation des six. Le jour où les deux lots se rencontrent, la recherche inclut Réunions et les six parcours déjà spécifiés restent.
- **Double exécution le jour du cloud.** Le Mac et un futur exécuteur pourraient tous deux croire une réunion ou un brouillon non réclamés. L’identifiant stable est la parade prévue. Elle ne suffit pas tant que l’exécuteur n’existe pas, et ce lot ne l’invente pas.

## Tranches

Chaque tranche traverse le parcours visible et le test. Le détail et les critères vivent dans l’issue. Les blocages sont aussi des dépendances natives sur le suivi. La première à prendre, pour les réunions, est l’invitation.

Carte : [Réunions, voix et iPhone](https://github.com/charlescstpierr/ezerbot/issues/63).

1. [Inviter le bot Recall dans Meet, Zoom ou Teams](https://github.com/charlescstpierr/ezerbot/issues/64). Aucun bloquant. Première tranche réunions. Parcours juge 1, 2 et la dictée pendant Recall du parcours 5.
2. [Lire le transcript et le résumé d’une réunion Recall](https://github.com/charlescstpierr/ezerbot/issues/65). Bloquée par 64.
3. [Répondre dans le chat de la réunion depuis une carte brouillon](https://github.com/charlescstpierr/ezerbot/issues/66). Bloquée par 65.
4. [Dicter avec le microphone choisi](https://github.com/charlescstpierr/ezerbot/issues/67). Aucun bloquant.
5. [Donner une voix à chaque bot](https://github.com/charlescstpierr/ezerbot/issues/68). Aucun bloquant.
6. [Tenir un appel vocal avec son transcript](https://github.com/charlescstpierr/ezerbot/issues/69). Bloquée par 67 et par 68.
7. [Répondre depuis la notification iPhone](https://github.com/charlescstpierr/ezerbot/issues/70). Aucun bloquant. Le parcours Mac éteint de la PWA est dans cette tranche. Les autres gestes Mac éteint sont dans la tranche qui les fait exister.
