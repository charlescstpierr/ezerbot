# Images et documents

Statut : variante du grill du 21 octobre 2026, gardée à côté de la spec retenue [`images-documents.md`](images-documents.md) (carte [#79](https://github.com/charlescstpierr/ezerbot/issues/79)). Elle apporte le verdict d’accès vérifié le même jour (génération par la route Images du jeton Codex, repli par clé d’API, galerie par bot, accent suivi dans PDF et DOCX) ; là où elle diffère de la spec retenue, la spec retenue fait foi. Les tranches #33 à #43 qu’elle cite sont fermées au profit de celles de la carte #79. La vidéo, génération comme lecture, est hors périmètre. Le lot garde les images, les documents et la bibliothèque par bot. Aucune implémentation n’est commencée. Les tranches sont des issues enfants de la carte [Images et documents](https://github.com/charlescstpierr/ezerbot/issues/31).

## Contexte

Charles utilise Ezerbot, l’app Mac, pour faire produire à un bot des images et des documents, puis les revoir dans la conversation. Le tableau de parité laisse ces écarts ouverts : la génération d’images est absente (IMG-2) ; il n’y a pas de galerie (IMG-3, pour sa part images) ; le PDF généré est en Courier et refuse les emoji (DOC-2) ; aucun bot n’a de bibliothèque de fichiers (DOC-3).

Aujourd’hui, le bot peut déjà créer un fichier local (PDF, DOCX, PPTX, Markdown, CSV, XLSX, SVG) et l’afficher comme une carte dans la conversation. Le PDF est une police standard Courier encodée en WinAnsi : le français passe, un emoji fait échouer la génération. Le DOCX met en forme les titres et les puces en Arial, sans tableau et sans les couleurs de l’app. L’aperçu dans la conversation est une vignette ; l’agrandissement actuel est une fenêtre bornée, et l’ouverture se fait hors de l’app. Les images que Charles joint sont montrées ; aucune image n’est produite par le bot. Une pièce audio peut déjà se lire dans la conversation.

Le fournisseur a deux sessions distinctes, toutes deux au nom du compte que Charles a connecté dans Ezerbot :

- **Codex.** Jeton OAuth du flux Codex, portée `openid profile email offline_access`, envoyé en `Authorization: Bearer` avec l’en-tête `chatgpt-account-id` vers `https://chatgpt.com/backend-api/codex`. C’est la session utilisée quand Codex est sélectionné.
- **Partage d’abonnement** (« Sign in with ChatGPT »). Jeton destiné à `https://api.openai.com/v1`, avec la portée de plan `chatgpt.tokens.use.direct`. OpenAI documente cette voie comme des requêtes Responses éligibles au plan ChatGPT.

Le bot décide déjà seul d’appeler un outil local, comme pour créer un document. Il n’existe pas de bouton « générer une image ».

## Verdict d’accès

### Images : le jeton Codex suffit, par la route Images du client officiel

Le jeton Codex qu’Ezerbot garde peut demander une image. La voie qui marche est celle du client Codex officiel, pas l’outil hébergé `image_generation` posé sur `POST /backend-api/codex/responses`.

Preuves, lues le 21 octobre 2026 :

- Le client officiel (`openai/codex`, client Images typé, outil de génération) envoie `POST {base}/images/generations` et `POST {base}/images/edits` avec le jeton ChatGPT et l’identifiant de compte. Le corps de génération porte `prompt`, `model`, `background`, `quality`, `size`. Le modèle envoyé par cet outil est `gpt-image-2`. La réponse contient `data[].b64_json`.
- Une sonde publiée le 10 septembre 2026, avec un vrai jeton OAuth ChatGPT/Codex, a obtenu HTTP 200 et un PNG décodable sur `https://chatgpt.com/backend-api/codex/images/generations`, pour `gpt-image-2.5-flare` comme pour un nom de modèle volontairement invalide. Cette sonde ne prouve pas quel poids a réellement dessiné, ni que tous les comptes se comportent pareil. Elle prouve que la route répond une image à ce type de jeton.
- Une demande sans authentification sur la même route reçoit HTTP 403. La route existe et exige le jeton.
- L’outil Responses `image_generation` est documenté pour `https://api.openai.com/v1/responses` avec une clé d’API (`Authorization: Bearer $OPENAI_API_KEY`). Les modèles d’image cités par le guide sont `gpt-image-2.5-sunburst`, `gpt-image-2.5-flare`, `gpt-image-2`, `gpt-image-1.5`, `gpt-image-1`, `gpt-image-1-mini`. Forcer `tool_choice` à `{ "type": "image_generation" }` est le contrat de cette API publique.
- Sur le backend Codex, ce `tool_choice` est rejeté (HTTP 400, l’outil n’est pas trouvé dans `tools`). Sans `tool_choice`, des comptes ont reçu du texte seul, sans événement `image_generation_call`. Le champ `experimental_supported_tools` a été observé vide. D’autres récits signalent un succès. Ce contrat est trop instable pour s’y fier. Le préparateur de corps Codex d’Ezerbot force en plus `tool_choice` à `auto`, ce qui ne déclenche pas l’outil hébergé.
- Le partage d’abonnement exclut explicitement la génération d’images. La page « Preview limitations » de Sign in with ChatGPT liste, parmi les outils non pris en charge : génération d’images, recherche de fichiers, interpréteur de code, usage d’ordinateur natif, MCP hébergé. Un jeton de partage présenté à `api.openai.com` ne produit donc pas une image, même si on ajoute `image_generation` au corps Responses.

**Ce que Charles fournit pour les images.** Rien, tant que la session sélectionnée est Codex et que la route Images accepte le compte. Si cette route refuse le compte (plan, droit ou rejet du client), la voie de repli est une clé d’API OpenAI collée une fois dans le trousseau du Mac, le même coffre que les secrets déjà gardés par l’app. Charles la crée dans les clés d’API de `platform.openai.com`. Elle est facturée sur le compte API, à part de l’abonnement ChatGPT. Il ne la fournit pas à l’avance.

## Décisions

1. **Le bot génère l’image sans bouton.** Quand Charles décrit une image à créer ou à modifier, le modèle appelle un outil local, comme il appelle déjà la création de document. L’outil parle à la route Images Codex (`images/generations` pour créer, `images/edits` pour modifier) avec le jeton et l’identifiant de compte de la session Codex, modèle demandé `gpt-image-2`. Le PNG revient dans la bulle du bot. Il n’y a pas de bouton dédié dans le composeur.
2. **Le partage d’abonnement ne génère pas d’images.** Si la session sélectionnée est le partage, ou si la route Images Codex refuse le compte, l’app utilise la clé d’API du trousseau contre l’API Images publique. Sans cette clé, la bulle explique en français quoi connecter (Codex, ou une clé dans le trousseau) et n’affiche pas une image inventée ni un simple texte à la place.
3. **Charles regarde, agrandit, télécharge et modifie en parlant.** Un clic agrandit l’image dans l’app. Le téléchargement écrit un PNG neuf sur le Mac après le même accord que l’enregistrement d’un document. Une phrase suivante (« mets le fond en rouge ») renvoie l’image précédente à `images/edits` et affiche la nouvelle image dans une nouvelle bulle. L’ancienne reste.
4. **La galerie est par bot.** Les images générées dans les conversations d’un bot se retrouvent dans sa galerie. Un autre bot ne les voit pas.
5. **Les documents générés deviennent des pages lisibles.** PDF et DOCX utilisent une vraie police proportionnelle, gardent les emoji visibles, traitent les titres comme des titres et les tableaux comme des tableaux, et portent les couleurs d’accent déjà choisies dans l’app. L’aperçu plein écran se fait dans Ezerbot. La page reste claire : l’accent colore le bandeau ou le titre, le texte reste encre foncée. Le thème sombre de l’app ne renverse pas la page.
6. **L’accent suivi est celui de l’app.** L’accent par défaut est le chartreuse de l’interface, `rgb(230 255 61)`, encre `#0f1012`, accent profond `#5b6b00`. Si Charles choisit un autre accent (orange, ambre, menthe, cyan, magenta, vermillon, violet, blanc), les documents générés ensuite prennent cet accent. Un document déjà produit ne change pas.
7. **La bibliothèque est une, avec deux rôles.** Elle appartient au bot. Les fichiers de contexte sont relus, de façon bornée, à chaque conversation de ce bot. Les fichiers générés (images et documents) y sont rangés et retrouvables ; ils ne sont pas recopiés en entier dans chaque prompt. La recherche, au besoin, couvre les deux rôles et reste locale, sur le modèle du rappel de mémoire : lexicale, sans embeddings, limitée au bot.
8. **Les octets restent sur le Mac.** Images, documents et fichiers de bibliothèque vivent dans la base locale, comme les documents générés aujourd’hui. Le prompt et les images envoyés à OpenAI partent parce que c’est le générateur. Aucun galerie en ligne n’est ajoutée. Le téléchargement est le passage vers le disque, et il demande un accord.

## Récits

1. En tant que Charles, je veux décrire une image dans la conversation, afin d’obtenir cette image dans la bulle sans chercher un bouton.
2. En tant que Charles, je veux que le bot choisisse seul de générer, afin de parler normalement (« dessine un cercle bleu sur fond blanc »).
3. En tant que Charles, je veux qu’une question ordinaire ne produise pas d’image, afin que le fil ne se remplisse pas d’illustrations non demandées.
4. En tant que Charles, je veux voir l’image dans la bulle du bot qui l’a produite, afin de la distinguer d’un fichier que j’aurais joint moi-même.
5. En tant que Charles, je veux un message français clair si la génération échoue, afin de savoir si je dois me reconnecter, attendre le quota, ou ajouter une clé.
6. En tant que Charles, je veux que l’échec ne montre pas une image cassée ni le corps brut du fournisseur, afin de ne pas prendre un diagnostic technique pour une image.
7. En tant que Charles, je veux cliquer l’image pour l’agrandir dans l’app, afin de juger le dessin sans quitter Ezerbot.
8. En tant que Charles, je veux fermer l’agrandissement et retrouver la conversation au même endroit, afin de continuer à parler.
9. En tant que Charles, je veux télécharger l’image en PNG, afin de la réutiliser hors de l’app.
10. En tant que Charles, je veux confirmer avant l’écriture sur le disque, afin qu’aucun fichier n’apparaisse sans mon accord.
11. En tant que Charles, je veux que le PNG téléchargé soit celui que je vois, afin de ne pas récupérer une autre image.
12. En tant que Charles, je veux modifier l’image en décrivant le changement, afin de ne pas recommencer le prompt depuis zéro.
13. En tant que Charles, je veux que l’image modifiée apparaisse à la suite, et que l’originale reste visible, afin de comparer.
14. En tant que Charles, je veux une galerie pour le bot courant, afin de retrouver les images de ses conversations.
15. En tant que Charles, je veux que la galerie d’un autre bot soit distincte, afin de ne pas mélanger les univers des bots.
16. En tant que Charles, je veux ouvrir une image de la galerie en grand, afin de la revoir sans chercher le message d’origine.
17. En tant que Charles, je veux demander un PDF avec un titre, un tableau et un emoji, afin de lire une page propre dans l’app.
18. En tant que Charles, je veux une police proportionnelle dans ce PDF, afin de ne plus voir de Courier.
19. En tant que Charles, je veux voir l’emoji comme un glyphe, afin que « 😀 » ne fasse pas échouer le fichier ni disparaisse.
20. En tant que Charles, je veux un titre visuellement distinct du corps, afin de parcourir la page.
21. En tant que Charles, je veux un tableau aligné, avec une ligne d’en-tête reconnaissable, afin de lire des chiffres en colonnes.
22. En tant que Charles, je veux que le bandeau ou le titre porte l’accent choisi dans l’app, afin que le document ressemble à Ezerbot.
23. En tant que Charles, je veux une page claire même si l’app est en thème sombre, afin de pouvoir imprimer ou relire sans inverser les couleurs.
24. En tant que Charles, je veux ouvrir ce PDF en plein écran dans l’app, afin de le lire sans passer par Aperçu.
25. En tant que Charles, je veux le même traitement pour un DOCX (police, emoji, titre, tableau, accent, plein écran), afin qu’un document Word ne soit pas le parent pauvre du PDF.
26. En tant que Charles, je veux déposer un fichier de contexte sur un bot, afin que ce bot le connaisse dans chaque nouvelle conversation.
27. En tant que Charles, je veux que ce fichier n’apparaisse pas chez un autre bot, afin de garder les contextes séparés.
28. En tant que Charles, je veux que seule une part bornée du fichier entre dans le prompt, afin qu’une grosse bibliothèque ne fasse pas échouer la conversation.
29. En tant que Charles, je veux que le bot cherche dans la bibliothèque quand il a besoin d’un passage précis, afin de ne pas tout charger d’avance.
30. En tant que Charles, je veux que la recherche reste sur ce Mac et sur ce bot, afin qu’un fait d’un autre bot ne ressorte pas.
31. En tant que Charles, je veux que l’image ou le document que le bot vient de créer se range tout seul dans la bibliothèque, afin de le retrouver plus tard.
32. En tant que Charles, je veux que les fichiers générés ne soient pas relus en entier à chaque tour, afin que le contexte reste celui des fichiers que j’ai désignés comme contexte.

## Décisions d’implémentation

- L’outil d’image est un outil du bot, au même titre que la création de document. Son nom et son schéma vivent avec les autres outils. La description dit de l’appeler quand l’utilisateur décrit une image à créer ou à modifier, et de ne pas l’appeler pour une question ordinaire. Le modèle reste libre de cet appel : l’app ne scanne pas le texte pour forcer une image.
- Création : `POST` sur la route Images de la base Codex déjà configurée, chemin `images/generations`, en-têtes déjà utilisés pour Codex (`Authorization`, `chatgpt-account-id`). Corps minimal aligné sur le client officiel : `prompt`, `model` valant `gpt-image-2`, `n` valant 1, `background` `opaque`, `quality` `auto`, `size` `auto`. La réussite est un PNG décodable dans `b64_json`. Le nom de modèle renvoyé peut manquer : l’app ne prétend pas connaître le poids réel.
- Modification : `POST images/edits` avec l’image précédente et le nouveau prompt. L’outil reçoit l’identifiant local de l’image à modifier. Sans image identifiable, l’outil échoue en français et n’en crée pas une de toutes pièces en silence.
- Repli clé d’API : uniquement après un refus d’habilitation de la route Codex, ou quand la session sélectionnée est le partage. La clé se lit dans le trousseau, jamais dans le prompt, les journaux ou la mémoire. L’appel public suit le guide Images / Responses d’OpenAI, authentifié par `Authorization: Bearer` et cette clé. Une clé absente produit l’explication française, pas une deuxième tentative muette.
- Le préparateur de corps Responses n’est pas le canal d’image. On n’y ajoute pas `image_generation`.
- L’image est enregistrée comme les documents générés : octets et métadonnées locaux, liés au message et au bot. La bulle affiche ces octets. La galerie liste les images de ce bot.
- L’agrandissement et le plein écran d’un document restent dans la fenêtre de l’app. Le téléchargement et l’enregistrement réutilisent l’accord déjà exigé avant d’écrire un fichier neuf. Aucun remplacement silencieux.
- PDF et DOCX acceptent le même contenu structuré : titres Markdown (`#`, `##`, `###`) et tableau Markdown. L’emoji n’est plus une erreur. La police du PDF n’est plus Courier. Sur Mac, le dessin passe par l’API PDF du système, afin qu’Aperçu et l’aperçu in-app montrent le glyphe emoji sans embarquer une police Apple dans le dépôt. Le DOCX continue d’embarquer des caractères Unicode ; il gagne le tableau et l’accent.
- Les couleurs du modèle sont l’accent courant de l’app au moment de la génération, plus l’encre foncée pour le corps. Elles sont copiées dans le fichier. Changer l’accent plus tard ne réécrit pas les fichiers.
- La bibliothèque a deux rôles, `contexte` et `généré`. Seul `contexte` est injecté, borné, dans chaque conversation du bot, avec un décompte du reste et l’invitation à chercher, sur le modèle du rappel de mémoire. `généré` est rangé à la création (image ou document) et ressort par la recherche ou par la galerie. La recherche est lexicale, locale, insensible aux accents français, limitée au bot, avec un budget de résultat. Elle ne fait pas d’appel réseau.
- Les plafonds déjà en place pour les pièces jointes et les documents générés s’appliquent. Ce lot n’en invente pas de plus larges.
- Aucun secret de connexion n’entre dans le contexte, la mémoire, ni l’export de conversation.

## Décisions de test

Un bon test observe le comportement visible : le message produit, l’image ou le fichier rendu, le contexte reçu par le tour suivant, le refus français. Il ne vérifie pas l’ordre interne des fonctions.

La couture unique est celle qui existe déjà pour les outils : un test du wrapper qui scripte le serveur HTTP du fournisseur, puis le test de présentation web qui rend la charge de ce message dans la bulle. La génération de document garde sa couture actuelle (générateur et test d’outil), et le juge visuel regarde l’aperçu produit. On n’ajoute pas de nouveau harnais.

Chaque tranche commence par un test rouge sur cette couture, puis le code qui le rend vert. Le fournisseur scripté ne prouve pas le compte réel. Le juge visuel sur Mac, avec la session Codex de Charles, est le critère d’acceptation des pixels, du glyphe emoji et du geste (clic, lecture, plein écran).

Le juge visuel travaille dans l’app Mac, fenêtre assez grande pour lire une page, thème clair puis thème sombre, accent par défaut puis un accent modifié. Il ne se contente pas d’une capture statique : il envoie le message, attend la bulle, clique, télécharge, change de bot, rouvre une conversation neuve.

## Critères d’acceptation

Un juge visuel sur Mac coche ces parcours. Chacun échoue si le résultat est seulement décrit en texte, ou si l’image ou la page est illisible.

1. **Image dans la bulle.** Session Codex connectée, aucun bouton d’image ajouté au composeur. Phrase : « Dessine un cercle bleu centré sur un fond blanc, sans texte. » La bulle du bot montre une image. Un décodeur y voit un PNG. Une question suivante sans demande d’image (« Quelle heure est-il à Montréal ? ») ne fait pas apparaître d’image.
2. **Refus lisible.** Avec une session de partage seule et aucune clé dans le trousseau, la même phrase d’image produit une explication française dans la bulle, sans image et sans JSON brut. Après avoir collé une clé d’API de test dans le trousseau, la même phrase produit une image.
3. **Agrandir et télécharger.** Cliquer l’image l’ouvre en grand dans l’app. Fermer ramène au message. Télécharger demande un accord, écrit un PNG neuf, et ce PNG est la même image (mêmes dimensions, même aperçu).
4. **Modifier en parlant.** Phrase : « Mets le fond en rouge et garde le cercle bleu. » Une nouvelle bulle montre la nouvelle image. L’image précédente est encore dans le fil. Les deux PNG diffèrent.
5. **Galerie.** L’image apparaît dans la galerie de ce bot. Après changement de bot, la galerie ne la montre pas. Revenir au premier bot la montre encore, y compris après avoir rouvert l’app.
6. **PDF.** Phrase demandant un PDF avec le titre « Bilan », un emoji 😀 dans le titre ou juste dessous, et un tableau à deux colonnes (Nom, Valeur) et deux lignes. L’aperçu dans la bulle, puis le plein écran dans l’app, montrent une police proportionnelle, le glyphe emoji, un titre plus grand que le corps, un tableau aligné dont l’en-tête se distingue, et un bandeau ou un titre de la couleur d’accent. Le texte reste foncé sur fond clair, y compris en thème sombre. Aucun message du type « non pris en charge par Courier ».
7. **Accent.** Changer l’accent de l’app pour le violet, demander un nouveau PDF : le bandeau ou le titre est violet. Le PDF précédent reste à l’accent qu’il avait.
8. **DOCX.** Le même contenu en DOCX, ouvert en plein écran dans l’app, montre titre, emoji, tableau et accent. Fermer le plein écran rend la conversation.
9. **Contexte.** Déposer un fichier de contexte sur le bot A contenant un mot rare. Ouvrir une conversation neuve avec A et demander ce mot : la réponse s’en sert. La même demande au bot B ne le connaît pas.
10. **Recherche.** Avec assez de fichiers de contexte pour dépasser le budget injecté, demander un fait présent seulement dans un fichier non injecté en entier. Le bot le retrouve. Un fait privé d’un autre bot ne ressort pas.
11. **Rangement.** L’image du parcours 1 et le PDF du parcours 6 figurent dans la bibliothèque du bot, rôle généré. Une conversation neuve ne réinjecte pas leur contenu intégral ; elle peut les retrouver par une question de recherche.

## Hors-périmètre

- Réécrire PPTX, XLSX, CSV, Markdown et SVG. Ils continuent de se générer comme aujourd’hui et ne doivent pas régresser. Ils ne reçoivent pas le modèle couleur dans ce lot.
- La vidéo, en entier. Charles la laisse de côté pour l’instant : ni génération, ni lecture d’une vidéo jointe, ni lecteur dans la bulle. Les issues ouvertes pour cela sont fermées. La galerie d’images, elle, reste dans le lot.
- Un bouton « générer une image », un éditeur de dessin, des calques, ou un choix manuel de taille et de qualité. `quality` et `size` restent `auto`.
- Prétendre que le poids `gpt-image-2` a bien été celui qui a dessiné, lorsque la route ne le confirme pas.
- Embarquer une police Apple dans le dépôt ou dans un fichier redistribué.
- Une galerie synchronisée en ligne, un partage d’images entre Mac, un compte d’équipe multi-comptes.
- Relire en entier, à chaque tour, les fichiers générés ou une bibliothèque qui dépasse le budget déjà pratiqué pour la mémoire.
- La recherche sémantique, les embeddings, et une garantie que le modèle citera le bon passage.
- Modifier une image jointe par Charles qui n’a pas été générée dans le fil, dans ce premier parcours. La modification vise l’image que le bot vient de produire.
- La génération d’avatars de bot comme produit séparé. Une image demandée dans la conversation suit le parcours général.

## Risques

- **La route Images peut refuser le client Ezerbot.** L’origine envoyée aujourd’hui est celle du flux pi, pas celle du client Codex officiel. Une sonde tierce a réussi avec une autre origine, ce qui rend le succès plausible, pas garanti pour ce compte. Mitigation : le repli par clé d’API dans le trousseau, et un message français si les deux voies manquent. Le juge visuel du parcours 1 est le moment de vérité sur le compte de Charles.
- **Le modèle peut ne pas appeler l’outil.** Les tests scriptent l’appel. Seul le juge, avec le vrai compte, vérifie qu’une description d’image déclenche bien une image. Si le modèle décrit l’image en texte, on resserre la description de l’outil ; on n’ajoute pas un détecteur de mots-clés.
- **Le nom de modèle peut être ignoré.** La sonde de septembre 2026 a reçu un PNG même pour un nom invalide, sans champ de modèle effectif. L’acceptation est le PNG dans la bulle, pas une affirmation sur `gpt-image-2.5-sunburst` ou un autre poids.
- **Quota et droits.** Une image consomme le quota d’images du compte ChatGPT/Codex, ou le crédit de la clé d’API. Un refus de quota réutilise le type d’échec déjà montré pour le chat (`limit` ouvre les réglages d’utilisation), sans inventer un pourcentage local.
- **Emoji et PDF hors de ce Mac.** L’API PDF du système montre l’emoji dans Aperçu et dans l’app. Un lecteur Linux du fichier peut ne pas dessiner l’emoji couleur. Les tests hors Mac vérifient que la génération accepte l’emoji et que la police n’est plus Courier ; le glyphe est jugé sur Mac.
- **Couleur d’accent peu lisible.** L’accent « blanc » ou un accent clair sur page claire disparaîtrait. Le bandeau utilise l’accent profond quand l’accent clair ne contraste pas avec le fond de page. Le juge le vérifie avec l’accent par défaut et avec le violet.
- **Taille des images dans la base.** Les PNG s’accumulent par bot. Les plafonds existants s’appliquent ; un dépassement échoue en français avant d’écrire. Ce lot ne migre pas l’historique.
- **Clé d’API.** Elle est un secret de facturation. Elle reste dans le trousseau, absente des prompts, des journaux, de la mémoire et des exports. Les tests utilisent un coffre jetable, comme les tests de trousseau déjà présents.

## Tranches

Carte : [Images et documents](https://github.com/charlescstpierr/ezerbot/issues/31). Chaque tranche traverse l’outil, la persistance, la bulle et le test. Le détail et les critères vivent dans l’issue. Les blocages sont aussi des dépendances natives sur le suivi.

1. [Générer une image et l’afficher dans la bulle](https://github.com/charlescstpierr/ezerbot/issues/33). Aucun bloquant. Première tranche à prendre.
2. [Agrandir et télécharger l’image de la bulle](https://github.com/charlescstpierr/ezerbot/issues/38). Bloquée par 33.
3. [Modifier une image en conversant](https://github.com/charlescstpierr/ezerbot/issues/39). Bloquée par 33.
4. [Galerie des images du bot](https://github.com/charlescstpierr/ezerbot/issues/40). Bloquée par 33.
5. [PDF lisible en plein écran, aux couleurs de Charles](https://github.com/charlescstpierr/ezerbot/issues/34). Aucun bloquant.
6. [DOCX au même modèle que le PDF](https://github.com/charlescstpierr/ezerbot/issues/41). Bloquée par 34.
7. [Fichiers de contexte lus à chaque conversation](https://github.com/charlescstpierr/ezerbot/issues/35). Aucun bloquant.
8. [Recherche dans la bibliothèque du bot](https://github.com/charlescstpierr/ezerbot/issues/42). Bloquée par 35.
9. [Ranger images et documents générés dans la bibliothèque](https://github.com/charlescstpierr/ezerbot/issues/43). Bloquée par 40 et 35.

La vidéo est hors périmètre. [Lecture d’une vidéo jointe hors périmètre](https://github.com/charlescstpierr/ezerbot/issues/36) et [Génération vidéo hors périmètre](https://github.com/charlescstpierr/ezerbot/issues/37) sont fermées. Aucune tranche vidéo ne reste ouverte.
