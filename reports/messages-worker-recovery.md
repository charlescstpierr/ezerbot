# Messages 0.71.2 : reprise après panne du worker

Le nouveau worker isole l’exécution Messages, mais ne rend pas l’envoi transactionnel. **Si le helper a envoyé le texte puis que sa réponse ou celle du worker se perd, le daemon ne peut pas établir si l’effet a eu lieu.** Il peut rejouer un résultat déjà mémorisé ; les corps examinés ne montrent pas de journal durable des envois commencés. Une nouvelle demande peut donc rejoindre le helper après un résultat incertain, selon ses IDs et l’état des caches.

Analyse statique de deux fichiers de la distribution sauvegardée **0.71.2**, sans exécution du produit, worker, helper, RPC, profil ou Messages réel. D désigne [le daemon LF](../artifacts/releases/0.71.2/formatted/dist/local-exec-daemon/main.cjs), W [le worker LF](../artifacts/releases/0.71.2/formatted/dist/local-exec-daemon/messages-worker.cjs). Les numéros A renvoient aux extraits hashés du [ledger](../evidence/messages-worker-recovery.json). Il est lié au manifeste ASAR versionné et aux deux membres du bundle REA déjà conservé, sans nouvelle analyse REA.

## Ce qui est nouveau

Le [rapport .70, §5](goal-execution-autonomy.md) établissait les deux caches de 64 entrées, leur caractère volatil et l’absence de signal d’annulation vers Messages. [R71-03](release-0712-discovery.md) repérait le nouveau worker. Cette passe suit ses corps : recréation paresseuse sans reprise, séparation des quatre identifiants, expiration des seules lectures, absence de timeout d’envoi dans les deux couches examinées, transmission au helper et fenêtres de perte après effet. Elle confirme séparément les caches dans .71 ; elle ne rétroattribue pas le worker à .70.

## MWR-01 — Le worker est partagé, les caches appartiennent à chaque B4

Le module construit `lsr = oZe(...)` vers `messages-worker.cjs`. Le constructeur B4 prend cet objet par défaut. Le chemin serveur crée un B4 par provider de compte sans lui injecter un autre objet Messages. Les maps `finishedSends`, `sentByKey`, `sendKeyTail` et `inflight` sont des champs d’instance B4, hors du worker. Un remplacement du worker conserve donc les caches d’un B4 encore vivant ; remplacer ce B4 ou le daemon les perd.

Preuve statique : **A1–A3**, D:128094–128141,157680–157704. Limite : ce montage démontre les propriétaires en JavaScript, pas une durée de vie observée sur un compte ni une persistance native éventuelle.

## MWR-02 — Une panne rejette les attentes ; le prochain appel crée un worker

`oZe` crée le worker au premier `run`, l’unref et maintient une map de promesses par ID. À `error` ou `exit`, il rejette toutes les promesses de ce worker et vide la map. Il remet la référence courante à zéro seulement si elle désigne ce worker : la sortie tardive de l’ancien ne supprime pas le remplaçant. Le prochain `run` en crée un autre. Aucun corps de ce wrapper ne renvoie automatiquement les opérations interrompues et aucun timer n’y appelle `terminate`.

Preuve statique : **A4**, D:128040–128078. Limite : l’OS peut avoir déjà réalisé l’effet avant l’événement ; ce rejet est un échec de la promesse, pas une preuve d’absence d’envoi. Une absence de réponse sans error/exit laisse l’attente ouverte dans ce wrapper.

## MWR-03 — Le compteur du worker sert à corréler, pas à dédupliquer l’effet

Le décodeur serveur conserve `requestId` et un `sendKey` non vide facultatif. B4 transmet seulement `op` et la taille maximale à `run`. Ce dernier ajoute un compteur `id = n++`, puis le worker répond avec ce même ID ; le wrapper retire la promesse correspondante. Ni `requestId` ni `sendKey` ne figurent dans l’enveloppe vers le worker. Le compteur reste dans `oZe` lorsque son worker est remplacé et repart avec un nouveau daemon.

Preuve statique : **A4–A8, A11**, D:128040–128078,157212–157218,157257–157271,128256–128263 ; W:2580–2589. Limite : ces IDs établissent des corrélations dans les couches retracées. Ils ne prouvent aucune garantie du helper natif ou du serveur.

## MWR-04 — L’expiration de lecture est une admission, pas une interruption

`usr` retourne `undefined` pour un send, sinon `Date.now() + _Pe(op)`, soit 30 secondes. `ns` ne vérifie ce temps qu’une fois, à l’entrée, avant `We`. Une lecture déjà périmée reçoit `messages_read_expired` ; une lecture qui a commencé n’est pas interrompue par cette garde. `_Pe` déclare aussi 180 secondes pour un send, mais son seul appel dans ce daemon est derrière la branche qui exclut les sends : cette valeur ne crée pas de timeout d’envoi dans ce chemin.

Preuve statique : **A9–A11**, D:96023–96031,128080–128082 ; W:2580–2589. La vérification rejouable contrôle aussi les deux seules occurrences de `_Pe(`. Limite : ceci borne l’expiration locale examinée ; une deadline distante ou une politique native n’est pas établie ici.

## MWR-05 — Pas de file globale explicite ni d’annulation du Messages déjà parti

Le worker appelle `ns(e).then(rs)` pour chaque événement `message`, sans chaîne de promesses qui sérialise toutes les opérations. B4 ignore un `requestId` déjà actif et sérialise les appels qui portent le même `sendKey`. Il vérifie `cancelled` avant `messages.run`. En revanche, le callback `cancel` initial demeure vide dans la branche Messages ; aucun signal n’atteint `run` ou le worker. L’annulation reçue après le départ ne coupe donc pas ce corps.

Preuve statique : **A12–A15**, W:2600–2602 ; D:128207–128218,128252–128342,128378–128389. Limite : l’absence de file explicite n’implique pas un parallélisme garanti : le code synchrone, les caches de snapshots et le helper peuvent sérialiser certains travaux. Le cancel peut encore empêcher un envoi en attente de son admission.

## MWR-06 — Deux caches de 64 entrées, avec égalité stricte de l’opération

`replayedSend` consulte d’abord `finishedSends[requestId]`, puis `sentByKey[sendKey]`, et exige `JSON.stringify(op)` identique. `rememberSend` retient pour les sends les réponses résultat, erreur et consentement par requestId. Il remplit `sentByKey` seulement pour `messages-result`. Chaque map est bornée à 64 entrées ; l’éviction retire la première clé de la Map, sans expiration temporelle ni rafraîchissement par lecture. Le replay adapte le requestId de la réponse au demandeur courant.

Preuve statique : **A16–A18**, D:128083–128084,128301–128322,128457–128466. Limite : la fenêtre est bornée aux entrées encore présentes dans le même B4. La comparaison n’est pas une équivalence sémantique de destinataires ou de texte. Les opérations de lecture ne sont pas mémorisées par `rememberSend`.

## MWR-07 — Après une erreur incertaine, requestId et sendKey ne protègent pas pareil

Le catch de `handleMessagesOp` passe son erreur à `rememberSend`. Si une panne du worker a rejeté un send, le même requestId et la même opération retrouvent donc l’erreur mémorisée. Une nouvelle demande avec **un autre requestId mais le même sendKey** ne trouve pas ce seul échec dans `sentByKey` et peut appeler le worker. À l’inverse, un résultat réussi déjà mémorisé peut être rejoué par sendKey sans nouvel envoi, même avec un nouveau requestId. La mémoire est écrite avant le callback de transmission du résultat.

Inférence à partir des corps : **A19–A21**, D:128219–128253,128256–128263,128301–128322. Limite : il faut que le résultat ou l’erreur ait atteint B4 et reste dans sa map. Un crash du daemon avant `rememberSend`, une éviction ou la recréation du B4 supprime cette protection. Un blocage local retourne directement son erreur sans passer par `rememberSend`.

## MWR-08 — Le helper reçoit un UUID de transport nouveau, pas le sendKey

L’opération send rejoint `Pe(..., confirm:true)`, écrit le texte dans un fichier temporaire et appelle `sendMessage`. `Hr` ne conserve que `{to,service,textFile}` ou `{chatId,textFile}`. `Ur` construit une ligne JSON portant `id:1`, `method` et un `requestID` UUID créé pour cet appel. Cette ligne est construite avant sa boucle de reconnexion : elle reste identique pendant ces essais, mais un nouvel appel à `Ur` génère un nouvel UUID. L’ID entier du worker n’est pas transmis non plus.

Preuve statique : **A22–A26**, W:2044–2048,1937–1963,2448–2452,2485–2488,2374–2393. Limite : l’UUID pourrait être utilisé par le helper pour ses propres besoins ; ces corps ne livrent ni son implémentation de déduplication ni une correspondance durable avec requestId/sendKey.

## MWR-09 — Reconnexion bornée au démarrage, mais send sans timeout socket

`Ur` réessaie seulement `ENOENT` et `ECONNREFUSED`, lance le helper via son callback au premier de ces échecs puis attend par pas de 200 ms, avec une limite de 8 secondes après ce lancement. Il ne réessaie pas dans ce corps une fermeture de socket, une réponse invalide ou un timeout. `Vr` ne donne aucun délai à `send-message` ; `kr` fait alors `socket.setTimeout(0)`. Les autres verbes ordinaires reçoivent 10 secondes, `resolve-handles` 4 secondes ; la présence de `prompt` enlève aussi le délai.

Preuve statique : **A27–A30**, W:2227–2228,2336–2369,2374–2393,2395–2426. Limite : les 8 secondes bornent le traitement des échecs de disponibilité après lancement ; elles ne bornent pas un send dont le socket reste ouvert sans réponse ni un callback de lancement bloqué. Aucun lancement n’a été effectué.

## MWR-10 — Le repli iMessage→SMS n’est pas un retry général de transport

Pour une cible participant en service `auto`, `lr` propose iMessage puis SMS. `dr` ne passe à l’option suivante que pour l’erreur exacte `messages_addressing_failed`. La normalisation associe notamment les codes natifs `-1728` et `-1700` à cette erreur. Une fermeture du socket ou une panne du worker ne déclenche pas ce repli dans ces corps.

Preuve statique : **A31–A32**, W:1937–1972,2408–2422. Limite : le sens natif précis de ces échecs n’est pas démontré par la table de traduction ; elle ne constitue pas une transaction qui annule un éventuel premier effet.

## MWR-11 — `verified` est un contrôle après envoi, pas un reçu durable

`Pe` attend d’abord `dr`, puis vérifie un snapshot de la base : message `fromMe`, date au moins égale au début de l’appel et texte égal après retrait des U+FFFC et trim, dans le chat choisi. Toute erreur de cette vérification donne `verified:false` ; le résultat reste de type sent/send. Le retour ne transporte pas un GUID d’envoi natif qui servirait de journal ou de clé de reprise.

Preuve statique : **A33–A35**, W:1901–1921,1973–2002,2044–2048. Limite : deux textes identiques proches peuvent satisfaire le contrôle ; un snapshot retardé peut échouer. La livraison au destinataire et l’atomicité native restent inconnues. `verified:false` n’est pas traité comme un échec à réessayer par `We`.

## MWR-12 — Plusieurs frontières peuvent perdre le résultat après l’effet

Après `We`, le worker tente `parentPort.postMessage`. Si cet appel lève, il tente une erreur `messages_worker_reply_failed` ; cette seconde transmission n’a pas son propre catch. Après réception, B4 valide encore le résultat avec `gPe`, qui peut lever ; son catch mémorise alors une erreur. Plus loin, le provider retire son batch de réponses de l’outbox avant submit et peut l’abandonner après échec des submits. Réessayer ces mêmes frames de réponse ne rappelle pas Messages. Ces frontières séparent donc effet, résultat local et accusé distant.

Preuve statique : **A36–A39**, W:2583–2602 ; D:96018–96022,128256–128263,157111–157164. Limite : ce sont des fenêtres possibles établies par l’ordre des appels, pas des crashes observés. Le helper natif peut posséder des protections supplémentaires ; aucun corps examiné ici n’établit une reprise durable de bout en bout.

## Scénarios déduits des corps

Les scénarios suivants sont des inférences statiques, sous réserve d’admission par les permissions. Ils n’ont pas été exécutés contre l’application.

| Situation au moment de la nouvelle demande | Comportement client établi | Limite pratique |
|---|---|---|
| Send réussi, réponse B4 mémorisée, remise distante perdue ; même ID ou autre ID avec même sendKey et op identique | Replay du succès depuis B4, aucun nouvel appel au worker dans cette branche (MWR-06/07) | Même B4, entrée non évincée |
| Helper a peut-être envoyé ; worker sort avant retour ; B4 mémorise son erreur ; même requestId/op | Replay de l’erreur (MWR-02/07) | Ne réconcilie pas l’effet natif |
| Même panne, puis nouveau requestId avec même sendKey ; aucun succès antérieur en cache | Nouveau passage possible vers worker/helper (MWR-07/08) | Un doublon natif est possible, pas démontré ; comportement interne helper inconnu |
| Worker remplacé après erreur, B4 encore vivant | Anciennes attentes rejetées ; worker recréé au prochain appel ; caches B4 conservés (MWR-01/02) | Aucune reprise automatique de l’opération interrompue |
| Daemon/B4 remplacé après send, ou entrée évincée | Le replay local décrit ici n’est plus disponible (MWR-01/06) | Aucun journal durable de send montré dans cette chaîne |
| Lecture livrée au worker plus de 30 secondes après `run` | `messages_read_expired` avant `We` (MWR-04) | Le temps avant `run`, notamment dans une file sendKey, n’est pas inclus |
| Lecture déjà commencée ; son expiration arrive, ou cancel reçu après départ d’un send | Pas d’interruption par ces gardes (MWR-04/05) | Une erreur/fermeture ailleurs peut encore terminer l’appel |
| Send dont le socket demeure ouvert et silencieux | Aucun timer d’envoi n’est installé par oZe/Ur/Vr/kr (MWR-02/04/09) | La durée effective dépend d’autres couches non établies |

## Vérification et checklist

```sh
PYTHONDONTWRITEBYTECODE=1 python3 tools/verify-messages-worker-recovery.py
```

Cette commande hors ligne vérifie les hashes du rapport, du manifeste, des deux originaux et des copies LF, les extraits du ledger, leur raccordement aux occurrences REA sauvegardées, et quelques assertions bornées sur les corps retracés. Elle n’importe ni n’évalue aucun JavaScript cible et n’écrit pas dans les profils. Elle ne simule pas un envoi. Les sondes à fixtures éventuelles du parent sont des preuves distinctes, non revendiquées ici.

| Question | Statut | Résultat |
|---|---|---|
| IDs requestId/sendKey/worker.id/requestID et transmission au helper | answered-client-side | Portées et pertes de correspondance retracées |
| Rejet des pending, recréation du worker et reprise des opérations | answered-client-side | Recréation paresseuse ; aucune remise en file automatique dans oZe |
| Expiration, file, cancel et timeout du send | answered-client-side | Admission des lectures ; cancel après départ non propagé ; send sans timer dans les corps examinés |
| Caches de résultats et survie au restart | answered-client-side | Deux maps B4 de 64 entrées ; survivent au seul remplacement du worker |
| Effet natif réussi puis réponse perdue | partially_answered | Fenêtres et conséquences client établies ; état de l’effet non réconcilié |
| Déduplication durable, transaction native et garantie de livraison au destinataire | unresolved | Corps natifs/serveur et observation de panne non établis dans cette passe |
