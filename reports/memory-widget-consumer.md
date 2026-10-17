# Consommateur des cartes de mémoire : sélection, mutation et reçu

Le consommateur de `Lme` est présent dans le lazy `chunk-view-BnlWmtVC.js` de **Grok Bot 0.70.2**. Il existe deux chemins : l’ancien transmet une décision sur des positions de texte ; le chemin multiplayer promeut d’abord des `factId`, puis transmet un reçu au widget. L’étiquette **shared** peut apparaître avant l’acceptation de ce reçu. Elle ne constitue pas une preuve d’écriture de mémoire.

Cette passe reprend le parent REA `ev_a8b822a62831e30bd01eb4c4488b36f0bd50cb8ccc41d83e2dcff4e00cf82a4d`, son export conservé et le manifeste 0.70.2. Le graphe REA est partiel. Aucun lancement, chargement du JavaScript cible, compte ou réseau n’a été utilisé. La version 0.71 n’était pas nécessaire. CM10 était déjà partiellement résolu par [RC02–RC04](server-context-residual.md) ; MW01 rappelle ce raccordement, les autres constats précisent la sélection, les reçus, les gardes et la reprise.

Le [ledger](../evidence/memory-widget-consumer.json) conserve les sources, hashes et passages LF. Une seule copie LF absente a été ajoutée : `chunk-team-setup-row-C-xk3_Xs.js`, avec Prettier 3.6.2 hors ligne. Les originaux et copies préexistantes restent inchangés. Le [vérificateur dédié](../tools/verify_memory_widget_consumer.py) contrôle le manifeste, les extraits et les raccordements statiques sans exécuter l’application.

## MW01 — Le même slot possède deux consommateurs

La table des cartes charge le lazy pour `event/team-shared`. `On` et `so` distribuent le slot `memories` à `_s`, qui choisit `Cn` si `grok_bot_multiplayer` est actif, sinon `Pn`. L’import `jX as qs` correspond à l’export `Lme as jX` : les deux cartes utilisent donc bien le helper de CM10. Sources : [chargeur](../artifacts/formatted/dist/renderer/assets/index-D6ReTfRE.js:123262), [distribution et gate](../artifacts/formatted/dist/renderer/assets/chunk-view-BnlWmtVC.js:5488), [import](../artifacts/formatted/dist/renderer/assets/chunk-view-BnlWmtVC.js:109), [export](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:81407).

Limite : présence de deux branches livrées ; la valeur effective du gate pour un compte n’a pas été observée.

## MW02 — Il n’existe pas une conversion générale « index → factId »

| Chemin | Sélection initiale et sélection UI | Données envoyées |
|---|---|---|
| Ancien `Pn/Nn` | Tous les indices de `toTeam` sauf `borderline` ; choix individuel ou par thème | `memories:share` si tout est choisi, sinon `memories:share:0,2` ; aucun `factId` |
| Multiplayer `Cn` | Ensemble initial vide ; choix par groupe de faits | IDs des faits chargés, vers la RPC de promotion ; puis reçu distinct |

`qKe`, importé comme `Is`, normalise les espaces de `toTeam` et des faits listés, puis exige une égalité exacte. La première proposition d’un même texte normalisé détermine le thème. Tous les faits correspondants sont conservés : deux faits d’IDs différents et de texte identique peuvent donc être sélectionnés ensemble. Les faits sans correspondance sont exclus. `borderline` n’initialise pas la sélection de `Cn`. Sources : [appariement](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:47179), [sélection par IDs](../artifacts/formatted/dist/renderer/assets/chunk-view-BnlWmtVC.js:3740), [positions anciennes](../artifacts/formatted/dist/renderer/assets/chunk-view-BnlWmtVC.js:5870).

Exemple dérivé : `toTeam:["Paris  bureau"]` et deux faits `{factId:"f1",text:"Paris bureau"}`, `{factId:"f2",text:"Paris\nbureau"}` raccordent les deux IDs. `"paris bureau"` ne correspond pas. Ce n’est pas un appariement sémantique.

Limite : ces règles relient une proposition distante à une liste distante ; elles ne produisent ni les faits ni leur classement privé/équipe.

## MW03 — Une coche verrouillée signifie « traité », y compris gardé privé

Un groupe est coché si au moins un de ses faits appartient à la sélection `E`. Le changement ajoute ou retire **tous** ses IDs. Il est `isDone` si tous appartiennent à `G`, l’ensemble des IDs déjà soumis dans des lots réussis. Or chaque ID soumis entre dans `G`, y compris lorsque le résultat indique des faits gardés privés. Le composant de ligne transforme `isDone` en contrôle ajouté et verrouillé. Sources : [états du groupe](../artifacts/formatted/dist/renderer/assets/chunk-view-BnlWmtVC.js:3820), [IDs traités](../artifacts/formatted/dist/renderer/assets/chunk-view-BnlWmtVC.js:3717), [verrou de ligne](../artifacts/formatted/dist/renderer/assets/chunk-team-setup-row-C-xk3_Xs.js:1203).

`Cn` exige localement une carte non read-only, un agent non nul, `viewerIsOwner !== false` et une ressource chargée pour l’action principale. Il ne remplace pas la condition propriétaire par `viewerCanManage`. L’ancien `Pn` teste read-only et agent non nul dans son handler. Le clic principal déclenche directement le chemin ; aucune seconde confirmation n’apparaît dans ces handlers. Le dialogue de détail affiche les faits, sans mutation. Sources : [gardes](../artifacts/formatted/dist/renderer/assets/chunk-view-BnlWmtVC.js:3467), [détail](../artifacts/formatted/dist/renderer/assets/chunk-view-BnlWmtVC.js:3860), [ancienne action](../artifacts/formatted/dist/renderer/assets/chunk-view-BnlWmtVC.js:5818).

Limite : contrôle et verrou visuels, pas autorisation serveur ni preuve que chaque fait traité a été partagé. L’absence de confirmation reste bornée à ces corps.

## MW04 — `shared` est une projection optimiste conservée en mémoire locale

`Kt.send` enregistre la résolution dans `localAnswers`, referme la carte, puis appelle `respondToWidget`. `Lme` transforme alors `proposed` en `shared` selon le préfixe `memories:share` ou `memories:promoted:` ; il ne consulte pas le résultat de promotion. Les autres résolutions deviennent `kept`. L’ancien `Pn` applique également cette réponse optimiste avant le retour réseau. Sources : [envoi optimiste](../artifacts/formatted/dist/renderer/assets/chunk-view-BnlWmtVC.js:877), [projection](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:47202), [ancien chemin](../artifacts/formatted/dist/renderer/assets/chunk-view-BnlWmtVC.js:5822).

`localAnswers` est une map par agent puis entrée, créée dans le renderer. Le pruning sur les mises à jour du transcript ne reconnaît comme résolues que des entrées `send-message` via `HIe`, pas l’événement `team-shared` lui-même. Reset et clear de l’agent purgent cette map ; une mise à jour de cet événement ne suffit donc pas à démontrer sa purge par ce pruning. Une fois le slot serveur sorti de `proposed`, `Lme` conserve toutefois ce statut serveur. Sources : [store et pruning](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:73217), [consommateur des mises à jour](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:48814).

Limite : état de renderer, sans stockage durable démontré dans cette classe. Ni `shared` ni `kept` ne prouve une écriture ou un oubli de mémoire.

## MW05 — Promotion et reçu sont deux opérations distinctes

`Cn` sélectionne `E − G`, puis appelle la promotion en lots de 100. Le chemin actif va de `teamContext.promoteToTeam` à l’IPC, puis à `GrokBotService.promoteGrokBotMemoriesToTeam({agentId,factIds})`. Les réponses gardent `created`, `alreadyInTeam` et `keptPrivateCount`. Après tous les lots réussis, le reçu vaut `memories:promoted:N`, avec `N = created.length + alreadyInTeam.length`, ou `memories:keep` si ce total est zéro. Ce reçu ne contient ni IDs ni texte des faits. Sources : [boucle et reçu](../artifacts/formatted/dist/renderer/assets/chunk-view-BnlWmtVC.js:3701), [resource](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:13067), [IPC](../artifacts/formatted/dist/electron-main/main-app.cjs:108513), [RPC](../artifacts/formatted/dist/electron-main/main-app.cjs:103051).

L’affichage détaillé sépare les textes nouvellement créés, le nombre déjà connu et le nombre gardé privé. Le total du reçu inclut les faits déjà dans l’équipe, sans compter les privés. [Affichage](../artifacts/formatted/dist/renderer/assets/chunk-view-BnlWmtVC.js:3942).

Limite : le client reçoit un résultat serveur de promotion, mais aucun protocole transactionnel commun avec le reçu du widget. Le classement effectif, l’idempotence et la visibilité ultérieure restent serveur.

## MW06 — L’échec du reçu n’annule pas une promotion réussie

En cas d’échec d’un lot, `Cn` conserve les résultats et IDs des lots réussis ; il n’envoie pas encore le reçu. Si la promotion complète réussit puis que le reçu est rejeté (`accepted:false`) ou que sa promesse échoue, `Kt` retire seulement la résolution locale, mémorise la réponse à retenter et rouvre la carte. `j` et `G` ne sont pas annulés. La sélection a été vidée : sans nouveaux faits admissibles, le prochain clic appelle `R.retry()` avant toute nouvelle promotion. Sources : [lots et branche sans sélection](../artifacts/formatted/dist/renderer/assets/chunk-view-BnlWmtVC.js:3701), [rejet et reprise](../artifacts/formatted/dist/renderer/assets/chunk-view-BnlWmtVC.js:883).

La réponse mémorisée n’est présentée comme échec que si le statut du slot reste celui de l’envoi. Un compteur d’envois empêche une ancienne erreur d’effacer la résolution d’un envoi plus récent. `Kt` possède un fallback optionnel, mais les deux appels mémoire de `Cn` n’en fournissent pas. [État et retry](../artifacts/formatted/dist/renderer/assets/chunk-view-BnlWmtVC.js:869).

Limite : reprise dans cette instance React. Aucun journal durable, aucune annulation de promotion ni garantie contre une réponse perdue suivie d’un remontage n’est établi.

## MW07 — Aucun fait correspondant peut produire automatiquement « garder privé »

Après chargement, si le slot est encore proposé et si `Is(slot,facts)` est vide, `_n` signale ce vide. Lorsque la carte est ouverte et actionnable, son effet appelle `onKeep`. Une ref empêche un nouvel appel automatique dans cette instance ; le résultat envoyé est `memories:keep`, sans promotion. Le vide signifie donc **aucun fait raccordé à la proposition**, pas nécessairement aucune mémoire dans la liste. Sources : [condition et ref](../artifacts/formatted/dist/renderer/assets/chunk-view-BnlWmtVC.js:3480), [effet](../artifacts/formatted/dist/renderer/assets/chunk-view-BnlWmtVC.js:3924).

Un reçu refusé conserve une erreur et un bouton retry dans ce cas. Une ressource indisponible a son rendu propre ; elle n’est pas assimilée à une liste vide par cette branche. [Rendus](../artifacts/formatted/dist/renderer/assets/chunk-view-BnlWmtVC.js:3501).

Limite : chemin statique ; aucune carte réelle vide n’a été manipulée. Ce n’est ni suppression ni promotion des faits non raccordés.

## MW08 — Réouvrir peut promouvoir des faits sans renvoyer un nouveau reçu

Dans la carte multiplayer repliée, la réouverture d’un slot déjà résolu dépend de `im` : gate actif, bot Temporal d’équipe, propriétaire et hors groupe, avec les autres gardes de la carte. `shared` et `kept` ne sont donc pas nécessairement des états terminaux de l’édition. Si la promotion se fait alors que le statut effectif n’est plus `proposed`, la branche `!L` ferme la carte et retourne **avant** `R.send`. Sources : [réouverture](../artifacts/formatted/dist/renderer/assets/chunk-view-BnlWmtVC.js:3586), [condition du bot](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:3496), [retour avant reçu](../artifacts/formatted/dist/renderer/assets/chunk-view-BnlWmtVC.js:3724).

La lecture des faits est activée quand le slot est proposé ou ouvert. La resource utilise `lifetime:"revalidate"`. La promotion déclenche explicitement une relecture **sur erreur**, puis propage cette erreur ; sur succès, ce wrapper rend le résultat sans remplacer ni rafraîchir lui-même la liste. Sources : [chargement](../artifacts/formatted/dist/renderer/assets/chunk-view-BnlWmtVC.js:3464), [resource](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:13067).

Limite : cela ne nie pas d’autres relectures par le lifecycle de resource ou reconnexion. Une coche traitée reste locale et ne constitue pas une relecture de mémoire d’équipe.

## MW09 — Ce qui part au bot est une réponse de widget, sans preuve de réveil

Pour `memories`, la clé locale `jKe(entryId,"memories")` conserve l’ID d’origine ; les autres slots ajoutent un suffixe. L’action traverse `H4e` puis `transcript.respondToWidget`. Le handler main appelle `respondGrokBotWidget` avec **agentId, entryId, value**, et rend `accepted` ou un refus. Son payload ne transmet ni la sélection de faits ni un nouveau prompt. Le protocole partagé autorise aussi `sessionId`, mais ce handler et les arguments de commande examinés ne le renseignent pas. Sources : [clé](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:47199), [action renderer](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:12216), [main](../artifacts/formatted/dist/electron-main/main-app.cjs:110735), [contrat](../artifacts/formatted/dist/electron-main/proto.cjs:56109), [arguments de commande](../artifacts/formatted/dist/node-agent-coordinator/main.cjs:39344).

Limite : un reçu accepté atteste seulement le résultat retourné par cette RPC. Consommation de la réponse par le moteur, réveil, création d’un tour, rechargement de mémoire et texte présenté au modèle restent hors du chemin récupéré.

## Vérification et checklist

Commande hors ligne :

```sh
python3 tools/verify_memory_widget_consumer.py
```

Le vérificateur contrôle l’intégrité des sources et copies LF, les ancres, les bindings import/export et les branches décrites. Il ne lance pas le JavaScript et ne prétend pas tester le serveur.

| Question | Statut | Réponse / frontière |
|---|---|---|
| Consommateur lazy de `Lme` | answered-client-side | Distribution vers Cn/Pn et alias résolus |
| Sélection index / factIds | answered-client-side | Deux formats distincts ; appariement textuel exact pour Cn |
| Promotion ou réponse widget | answered-client-side | Mutation RPC distincte du reçu ; ancien chemin : décision seule |
| Affichage shared / kept / done | answered-client-side | Projection locale, statut serveur et IDs traités séparés |
| Droits et confirmations | partially_answered | Gardes UI et clics retracés ; autorisation serveur absente |
| Erreurs, retry et relecture | answered-client-side | Reprise locale, pas d’atomicité ni de durabilité prouvée |
| Réponse consommée par le bot | partially_answered | Payload et résultat RPC établis ; moteur et tour effectif absents |
| Écriture réelle et rétention | unresolved | Aucun état serveur ni compte consulté |
