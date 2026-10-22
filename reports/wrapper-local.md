# Wrapper Mac local

Le wrapper ouvre une fenêtre Mac et conserve les conversations dans SQLite. Sans connexion, les réponses sont simulées et identifiées comme telles. L’adaptateur « Sign in with ChatGPT » et les sept capacités ci-dessous sont implémentés localement. Aucun compte Grok, connecteur ou worker cloud n’est fourni. Les comportements locaux ne prouvent pas les mécanismes du serveur fermé de Grok Bot.

## Utiliser et reconstruire

Double-cliquer sur `Ouvrir Grokbot.command`, à la racine, ou sur `app/build/Grokbot.app`. Le lanceur construit le bundle s’il manque. Après une modification :

```sh
bash tools/build-app.sh
open app/build/Grokbot.app
```

La compilation utilise Cargo dans `~/.cargo/bin` et les outils Apple. Le bundle vise macOS 12.3 et l’architecture de la machine; sa signature est ad hoc. Distribution notarifiée et build universel ne sont pas établis. Les données sont dans `~/Library/Application Support/Grokbot/workspace.sqlite`, les journaux dans `logs`. Reconstruire ne remplace pas ces données.

Le moteur reste un processus local, arrêté avec l’hôte natif. Fermer l’app ou éteindre le Mac arrête donc aussi les routines et les enfants. SQLite utilise WAL, un délai d’attente des verrous et des migrations par composant; le wrapper possède les transactions.

## Interface et abonnement ChatGPT

L’interface propose conversations, recherche, favoris, renommage, suppression, copie, export JSON, thèmes et mascotte personnalisable. Les raccourcis comprennent `⌘ N`, `⌘ K` et Entrée. Le [design de la mascotte](mascotte-design.md) décrit les animations. Le fil React (`app/web`) affiche Markdown, code, diagrammes et formules; le composeur Tiptap reste en texte brut. `app.js` gère état et brouillons, `GrokbotPanels` les panneaux spécialisés. Le bundle `app/ui/chat.js`/`chat.css` est versionné pour compiler avec Cargo sans Node. Les fichiers `app/ui/panels/*.js`/`*.css` sont concaténés au build Rust; les cartes viennent de `app/web/src/cards`. Une reconstruction est nécessaire pour embarquer les changements.

Dans les paramètres, « Continuer avec ChatGPT » suit le parcours officiel [Sign in with ChatGPT](https://developers.openai.com/siwc/token-sharing-open-source) : enregistrement dynamique, PKCE, callback sur `127.0.0.1:1455` à `1465`, vérification d’identité JWKS, renouvellement sérialisé et révocation à la déconnexion. Les identifiants sont dans `<nom-de-base>.chatgpt-auth.json` (par exemple `workspace.chatgpt-auth.json`), mode `0600`, à côté de SQLite. Les modèles viennent de `GET /v1/models`, sans nom codé en dur.

Les runs appellent `POST /v1/responses` avec `store:false`, streaming et function calling. Chaque appel renvoie son contexte complet, y compris les appels et résultats d’outils; les événements durables du moteur remplacent l’ancien chemin `wrapper_generation`/`wrapper_remote.reply`, migré à l’ouverture. Le texte partiel reste visible dans une réponse `reply-<message_id>` pour un chat ordinaire. Le titre est recalculé après le premier et le troisième échange, sauf titre choisi par l’utilisateur. Sans connexion, la passerelle simulée conserve les réponses de démonstration; elle ne simule pas une exécution intelligente des sept capacités.

Le parcours fournisseur, les outils et la vision sont testés contre un faux OpenAI. Aucun essai avec un vrai compte ChatGPT n’est revendiqué, notamment pour function calling avec `store:false` et vision. `GROKBOT_FAKE_OPENAI` est réservé aux builds de développement; les builds de production l’ignorent.

## Profils de bot et conversations

Un bot a un nom, une couleur, une forme de mascotte, une description publique et des instructions privées. Chaque conversation a un seul `bot_id`; groupes, mentions et tours de room ne sont pas implémentés. Le bot est relu à chaque réponse. Le contrat de l’application, l’identité, la description puis les consignes privées précèdent les données de contexte.

Le template versionné `DEFAULT_BOT` crée `grokbot` s’il manque et ne met à niveau qu’une copie non modifiée. Il ne peut pas être supprimé. Les autres bots supprimés laissent une pierre tombale : leurs anciennes conversations restent lisibles et ils y répondent encore avec leurs instructions, mais ne peuvent plus déléguer ni demander de formulaire. Aucune nouvelle conversation ne peut être créée avec eux. L’accueil, les paramètres et le menu du fil donnent accès à la création et à l’édition. `/api/bots`, `/update`, `/delete` et `/api/state` exposent les profils; la création avec le même identifiant et le même contenu est idempotente.

Le journal d’envoi conserve les messages immuables et les nonces. « Tester une coupure » perd volontairement la réponse après acceptation; « Vérifier l’envoi » réconcilie avant toute reprise. Un même identifiant ne crée pas un deuxième envoi logique. Les conversations `routine` et `task` ne polluent pas la liste principale; leurs panneaux/cartes ouvrent leurs fils.

## Les sept capacités

### 1. Skills par bot

Dans « Modifier le bot », la section Skills offre liste, création/édition, historique en lecture seule, suppression et import de fichiers ou dossiers. Formats : txt, md, markdown, csv, json, yaml, yml; binaire et UTF-8 invalide sont refusés. L’import reprend la priorité frontmatter → titre Markdown → dossier de `SKILL.md` → nom de fichier et la provenance `origin: upload` de [SR03–SR04](server-orchestration-residual.md). Plafonds : 250 skills par bot et 256 000 unités UTF-16 par texte importé; paquets client de 8 Mio au plus, requêtes serveur de 16 Mio. Le rapport distingue écrits, doublons, refus et résultats incertains; les succès partiels restent acquis.

Les identifiants stables, versions monotones, digest, garde `expected_version` et déduplication sont des **choix locaux**. Autre différence avec SR03 : un nom explicite de frontmatter peut mettre à jour la skill existante, en conservant ses anciennes versions; les autres collisions reçoivent un suffixe. Vérifier l’historique après réimport, même si le rapport dit seulement « Écrite ». Le contexte reçoit un index borné; `list_skills` et `read_skill` permettent au modèle de découvrir puis lire les corps, sans tout injecter. Un dossier importé garde les fichiers à côté de son `SKILL.md` comme annexes de la skill. Pas d’écriture de skill par outil modèle, de Teach→skill ni de publication d’équipe.

### 2. Mémoire par bot

Le panneau Mémoire liste les faits avec identifiant, portée et date; le stockage conserve aussi leur provenance. `remember`, `correct_fact`, `forget_fact` écrivent explicitement la mémoire; aucune extraction automatique de fin de conversation. Corriger crée un fait remplaçant sans écraser l’ancien. « Équipe » signifie tous les bots de cet espace local, pas des comptes distants. La promotion traite au plus 100 IDs par lot, sépare nouveaux/déjà partagés/gardés privés et conserve les lots réussis ([CM09–CM12](goal-context-memory-learning.md), [RC02–RC04](server-context-residual.md)). Les faits sensibles restent privés : choix local.

Oublier efface le texte du fait et de sa lignée de corrections, expurge les traces modèle concernées et invalide les résumés couvrants. **Vos messages utilisateur ne sont pas effacés** : leur historique peut encore rappeler le fait au modèle, qui pourrait le réapprendre. Réinitialiser le contexte (`/api/conversations/reset`) pose une borne d’historique et conserve mémoire, skills et routines; cela annule les runs et demandes en attente appartenant à cette conversation. Ce n’est ni oublier ni supprimer le transcript.

L’oubli est refusé en 409 si une autre réponse concernée est en cours : l’arrêter ou attendre, puis réessayer; le fait reste actif tant que l’oubli n’a pas réussi. L’expurgation parcourt les runs sous verrou d’écriture, potentiellement coûteux sur une grosse base. Elle n’est pas une purge de sauvegardes, des messages utilisateur ou de toutes les copies d’affichage/données déléguées. Le panneau ne montre les tombstones d’oubli que dans la fenêtre qui a fait l’action, jusqu’au rechargement; la base les conserve. Correction, oubli et injection sont notre reconstruction : leur implémentation serveur Grok reste inconnue.

### 3. Runs, contexte et compaction

Les déclencheurs message/tentative, occurrence et délégation sont uniques. Les outils internes committent effet et résultat dans la même transaction; un réveil reprend le même run. Après crash, un run `Running` devient `Interrupted` et peut reprendre au tic suivant sans rejouer un outil déjà committé. Cela ne garantit pas une exécution exactement une fois d’effets externes, qui ne sont pas encore disponibles. Arrêter conserve les événements; relancer une réponse de chat crée une tentative qui supplante l’ancienne.

Un chat finit sans statut de tâche. `begin_task` le transforme en tâche; routines et délégations sont déjà des tâches. Seul `complete_task{summary}` les marque terminées. Fin de tour, résultat d’outil et fin métier sont distincts, comme les vocabulaires relevés en [EA14/EA19](goal-execution-autonomy.md). Une tâche peut donc rester inachevée après la fin du texte ou l’épuisement du budget; la carte affiche « Non terminée » si le run s’est arrêté sans `complete_task`. Ce n’est pas une continuation infinie automatique.

Les **valeurs par défaut locales** sont 12 appels modèle, 24 outils et 10 minutes par run. L’ordre du contexte est contrat → identité/consignes du bot → index skills → mémoire → résumé → données complémentaires et historique depuis reset/résumé → entrée courante. Skills importées, résultats, légendes et contexte délégué sont encadrés comme données, jamais comme permissions. Ces encadrements ne prouvent pas une immunité aux injections de prompt.

La compaction se déclenche à 70 % d’une fenêtre estimée de 100 000 tokens (4 caractères/token); `/api/engine/config` permet de régler la fenêtre estimée. Un résumé chaîné conserve bornes de troncature/reprise et indication des outils inclus, sans modifier le transcript ([RC06](server-context-residual.md), [compaction](compaction-runtimes.md)). Fenêtre, estimation, prompt et politique sont locaux, pas des garanties récupérées du serveur ni la fenêtre réelle annoncée par le modèle. Résumé et titrage ajoutent des appels fournisseur.

### 4. Formulaires, approbations et secrets

`request_form` propose texte, nombre, choix ou booléen avec validation des champs requis; `request_secret` demande une saisie hors chat. La résolution et son réveil sont atomiques. Renvoyer la même soumission, même avec valeurs vides, rend le même reçu sans remplacer les valeurs. La carte sonde « Le bot reprend… » puis « Repris » : `Delivered` exige le premier événement de reprise committé, pas seulement l’acceptation HTTP. Ce reçu est un **ajout local** au retry client retracé en [SR01–SR02](server-orchestration-residual.md).

Le routage des demandes d’approbation vers une conversation du bot principal `grokbot`, où **l’utilisateur** accepte ou refuse, est en place et testé, y compris pour un outil protégé dans un run enfant. Aucun outil livré n’exige encore d’approbation; l’activation de routine utilise une confirmation dans le chat principal, pas une carte. L’hôte préfère le fil principal ancêtre, sinon le plus récent, sinon crée un fil visible. Les formulaires/secrets des fils cachés y sont aussi ramenés. Le reçu réveille toutefois le run d’origine. Supprimer la conversation qui porte la décision annule ses cartes en attente au tic suivant, sans reroutage automatique.

Les décisions sont liées à l’outil et au digest canonique des arguments; `AllowOnce` se consomme avec l’effet. Les règles Toujours autoriser/refuser, persistées et isolées par bot demandeur (la cible est toujours `grokbot`), permettent une évaluation automatique ultérieure sans nouvelle carte. La portée sujet exige les mêmes arguments; une portée globale explicitement accordée couvre l’outil. Un blocage correspondant prévaut. Les paramètres permettent de retirer ces règles, sans annuler un effet déjà autorisé ([PS01–PS06](goal-permissions-secrets.md)). **Le réviseur en langage naturel n’est pas implémenté** : son type et sa table sont prévus dans la crate `forms`, sans exposition par l’API ni l’interface. Seules les règles Toujours sont évaluées automatiquement. Aucun outil externe de production n’exige encore cette gate; elle est exercée par les tests. L’activation de routine utilise la confirmation ci-dessous, pas cette gate.

Le coffre est `workspace.secrets.json` pour la base usuelle, séparé de SQLite, mode `0600`, non chiffré par un trousseau OS. Seuls `SecretRef{name}` et les métadonnées doivent atteindre modèle, état, événements et export; les sorties sont expurgées avec les valeurs connues du coffre. La carte vide sa saisie à l’envoi ([PS07–PS08](goal-permissions-secrets.md)). Ne pas coller de secrets dans le chat. Le fichier et ses sauvegardes restent sensibles; aucun connecteur ne consomme encore ces valeurs. Cette séparation locale est plus forte que ce que les seuls DTO de l’original permettent de prouver.

### 5. Routines

Le panneau Routines, par bot ou dans les paramètres, propose liste, éditeur, aperçu/fuseau, pause/reprise, lancement manuel, fil de résultat et historique incluant les occurrences manquées. Le corps est un prompt libre ou une invocation de skill épinglée par ID/version. Le modèle dispose de `routine_clock`, `create_routine`, `update_routine`, `enable_routine`.

Comme la consigne client [CR19](cron-detail.md), une création modèle naît en pause/non fiable. Une question avec le corps et l’horaire est présentée dans le **chat principal**; seule une confirmation utilisateur ultérieure pour cette révision permet `enable_routine(confirm:true)`. Un « oui » générique vise seulement la dernière question en attente, au premier tour utilisateur qui la suit; en cas d’ambiguïté, nommer l’ID exact. Un tour ne confirme qu’une routine. Une édition modèle demande une nouvelle confirmation. Le modèle ne peut pas fabriquer le consentement ni substituer une carte d’approbation. Les corps proposés par le modèle doivent tenir dans l’aperçu complet de 2 000 caractères. Le panneau permet aussi une activation manuelle explicite.

La grammaire reprend [CR08–CR13](cron-detail.md) : cinq champs, listes/plages/pas, alias `@hourly` à `@annually`, `CRON_TZ=`/`TZ=` avec zone IANA; jour de semaine 7 = dimanche, jour du mois **OU** jour de semaine si les deux sont restreints. `@every <durée>[/<phase>]` accepte ms/s/m/h/d, sans préfixe de fuseau; la phase aligne sur l’époque UTC. Pas de secondes cron, noms MON/JAN, `L`, `W`, `#`, `@reboot`. Une date unique est un vrai `Once`, pas un cron annuel.

Les garanties suivantes sont **locales**, inconnues sur le scheduler Grok : heure inexistante décalée au premier instant valide, heure répétée servie une fois (la première), occurrence unique par routine/version/créneau et nonce de lancement manuel idempotent. Au retour de l’app, un seul rattrapage dans les 60 dernières minutes; les autres créneaux sont manqués. Une pause/confirmation est une coupure, sans rattrapage de la période en pause; pour une date unique dépassée avant confirmation, choisir une nouvelle date. Modifier le planning ne rejoue pas un créneau déjà servi. Chaque routine a sa conversation. Pas de launch agent pour tourner app fermée, ni triggers Slack/GitHub/Linear/email/webhook ou jours d’exclusion. L’historique serveur n’est pas paginé; après une coupure moteur, « Actualiser l’historique » ou recharger la page peut être nécessaire.

### 6. Délégation

`delegate` crée une arête parent/racine/enfant et un fil `task:<id>:<parent>`; `check_delegations` lit les résultats. Plusieurs appels d’un même lot admettent des enfants parallèles et suspendent le parent sur le groupe. L’enfant reçoit uniquement les textes, entrées, faits et skills sélectionnés et autorisés (budget total 32 000 caractères), pas l’historique complet du parent ni sa mémoire privée implicite. Images : ni octets ni légendes ne sont transmis automatiquement à l’enfant, même pour une entrée sélectionnée; seul le texte de l’entrée passe, sauf si le parent recopie la légende dans `texts`. La transmission des légendes sélectionnées reste à implémenter. Le contexte est revérifié à chaque appel; une ressource modifiée peut devenir indisponible ou faire dépasser le budget. Ce n’est pas un snapshot immuable des skills.

Les limites **locales** sont 4 enfants actifs dans l’espace, 3 par racine, profondeur 3 et file 16; cycles et surcharge sont des refus lisibles. Une délégation imbriquée sans slot est refusée plutôt que mise en file derrière ses propres ancêtres. Fin/échec réveille le parent avec les résultats; arrêter le parent annule les descendants, y compris en file. Cette cascade est notre politique, pas une propagation établie par le REA ([DL-S01–S13](delegation-lifecycle-deep.md)).

Les cartes ouvrent le fil enfant et affichent état/résultat; le bandeau du fil parent affiche « en attente de N sous-tâches » sans faire composer le parent (DL-S07). L’interface rend le fil de tâche en lecture seule et renvoie au parent/principal pour les demandes humaines. Un enfant doté d’un run peut être arrêté; un enfant encore en file sans run s’annule via la cascade du parent. Le pilotage par message séparé session mère/taskId de DL-S05/S06 et une annulation individuelle de file attendent des endpoints dédiés. L’API d’envoi n’interdit pas encore tous les suivis sur une tâche terminée : ne pas la traiter comme un chat libre.

### 7. Images et prédescription

Coller, déposer ou choisir une image crée une puce avec miniature et état de description. Types détectés par signature : PNG, JPEG, GIF, WebP, BMP; fichier non vide de 20 Mio au plus, au plus 10 images par message. L’upload binaire va à `/api/attachments`, les octets au stockage adressé par SHA-256 dans `<base-sqlite>.attachments/` (par exemple `workspace.sqlite.attachments/`), et `/api/attachments/<id>` sert l’image. Le composeur accepte un envoi composé seulement d’images.

La prédescription reprend [RC01](server-context-residual.md) : debounce d’une seconde, brouillon et cinq derniers messages humains purs, deadline 150 secondes, recul de dix minutes après `Skipped`. Le cache local dépend du hash, du modèle et de l’empreinte de ce contexte. BMP est conservé/affiché mais n’est pas envoyé directement à Responses. Au tour d’envoi, les images compatibles et légendes disponibles partent au modèle; les tours suivants de cette conversation et la compaction ne reçoivent que les légendes. La délégation ne les reçoit pas encore automatiquement.

Le repli en descriptions seules après rejet d’image ou absence de vision est une reconstruction locale; il est signalé à l’écran, sans inventer de légende. Le recul de prédescription ne retire pas à lui seul les octets d’un envoi. Le catalogue ne prouve pas la capacité vision : certains libellés d’erreur non reconnus peuvent encore faire échouer le run, et une reprise tardive peut retenter un appel sans vision. Un faux serveur valide ces branches, pas un compte réel.

## API locale et sécurité

L’API est liée uniquement à `127.0.0.1` sur un port dynamique. Host, Origin présent et marqueur local des POST sont vérifiés. JSON est requis sauf upload d’image sur `/api/attachments`. Limites : 64 Kio par défaut, 16 Mio pour création/édition/import de skills, 20 Mio pour upload. Ce n’est pas une authentification entre processus locaux du même poste.

La coque [Cocoa/WKWebView](../app/native/main.m) démarre un moteur enfant et n’expose pas de pont JavaScript général vers le Mac. Elle implémente le sélecteur de fichiers et le dialogue d’export; les parcours natifs récents restent à exercer. Pas de shell, de contrôle d’ordinateur, de MCP, de 1Password ou de Messages natifs dans cette livraison.

## Vérifications et limites

```sh
export PATH="$HOME/.cargo/bin:$PATH"
# Dans le dossier principal contenant les archives REA, pas dans une copie isolée :
python3 tools/verify-rust.py
# Ces contrôles fonctionnent aussi dans une copie isolée :
cargo build --locked -p grokbot-wrapper
python3 tools/verify-wrapper.py
# Séparément, avec l’app Mac construite et ouverte :
python3 tools/verify-app.py
```

La passe de clôture du 2026-10-17 a relancé `verify-rust.py`, le build verrouillé et `verify-wrapper.py` dans un clone sans les archives ignorées par Git : **791 tests passants**, doctests compris, fmt, clippy `-D warnings`, scénarios SQLite et **60 contrôles HTTP/SQLite passants** en mode simulé. Les deux contrôles d’archives ne peuvent pas aboutir ici : `saved evidence` signale 1 011 chemins absents (`inputs/`, `artifacts/`, `evidence/rea-bundle.json`), et `static dossier` ne trouve ni l’ASAR sauvegardé ni `/Applications/Grok Bot.app/Contents/Resources/app.asar`. Il faut donc lancer **`verify-rust.py` dans le dossier principal, où ces archives existent**; cette tranche ne l’y a pas exécuté et ne revendique pas de résultat global sur le dossier complet. Les liens de ce rapport ont été vérifiés séparément selon la règle de chemins de `verify-dossier.py`, sans fragments `#` incompatibles avec ce script. Aucun contrôle n’a été contourné. L’absence d’archives dans ce clone ne constitue pas un échec du code Rust. Le contrôle navigateur, sur ce wrapper avec une base jetable, confirme l’import de deux textes et le refus d’un binaire, puis deux `SKILL.md` avec chemins de dossier synthétiques; l’éditeur de routine, son aperçu de fuseau, le lancement manuel, l’historique et la carte « Non terminée » du mode simulé ont aussi été exercés. Cela ne valide ni WKWebView, ni le sélecteur réel de dossier, ni les dialogues macOS. **Contrôle natif dans l’app à faire par le capitaine** : import fichiers/dossiers, collage d’image, cartes et export natif.

Les JSON [Rust](../evidence/rust-verification.json) et [HTTP](../evidence/wrapper-verification.json) du diff livré sont conservés à leur version de `main` : ils ne représentent pas les 791 tests et 60 contrôles de cette passe isolée. Les rapports produits ici restent hors dépôt. Les JSON [natif](../evidence/wrapper-native-verification.json), [mascotte](../evidence/wrapper-mascot-verification.json) et [navigateur historique](../evidence/wrapper-browser-verification.json) restent aussi des instantanés antérieurs. Les tests ciblés sont dans `crates/wrapper/tests/{skills,memory,engine,forms,forms_resume,routines,delegation,attachments,attachments_run,memory_forget}.rs` et les crates de capacité; leurs résultats de livraison sont distincts de ces archives.

Les suites sont recensées dans [MANQUES_COMPLETS.md](../MANQUES_COMPLETS.md). Ni équivalence des 45 familles, ni CI distante, ni service disponible app fermée, ni évaluation métier générale du modèle ne sont revendiqués. Les preuves REA historiques restent séparées du code indépendant et de ses choix.
