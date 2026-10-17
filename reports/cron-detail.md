# Configuration des routines et des horaires dans Grok Bot 0.70.2

Le client configure des routines par des commandes de gateway et par quelques RPC serveur. Les chemins retracés ne créent pas de crontab sur le Mac. Ils envoient un nom, un prompt et un déclencheur, puis reçoivent des records JSON. Le champ `filePath` appartient à ces réponses; le client inspecté ne révèle ni le chemin réel du fichier de routine, ni son format de sauvegarde, ni le moteur qui décide des départs. Une routine peut contenir son propre prompt sans référencer une skill. [CR01, CR02, CR03]

Cette passe approfondit la section routines de [protocol.md](protocol.md). Aucun processus Grok Bot, daemon de la cible, compte, routine réelle ou endpoint n'a été lancé ou interrogé. Les exemples et les conséquences du code sont des lectures statiques. Les garanties du scheduler distant restent séparées.

## Provenance des références

Le parent REA réutilisé est `ev_a8b822a62831e30bd01eb4c4488b36f0bd50cb8ccc41d83e2dcff4e00cf82a4d`, conservé dans `evidence/rea-bundle.json`. Son ASAR a le SHA-256 `9fe94a66942f8511281f6d7c80a42432d749284c46e1e26f5b6d2f99b87652b0`. L'identité et les hashes des six fichiers ci-dessous ont été relus dans ce bundle et comparés aux octets locaux. Le graphe global est `partial`; l'AST de M est aussi `partial`. Les conclusions nouvelles reposent sur les passages du JavaScript relus directement, et pas sur une absence dans ce graphe.

Les lignes utilisent les copies formatées sous `artifacts/formatted`. Chaque chemin relatif ci-dessous existe aussi sous `artifacts/client`, dont le hash est celui des octets originaux. [cron-detail.json](../evidence/cron-detail.json) conserve les hashes des copies, les intervalles, un hash de chaque extrait et les limites de chaque constatation.

| Alias | Chemin relatif exact | SHA-256 original |
|---|---|---|
| P | `dist/electron-main/proto.cjs` | `856b35e5e9f93037b4f90ca9d27fd4ea4120c4175ce95d32ceeaf6ca0bbd2b29` |
| M | `dist/electron-main/main-app.cjs` | `06756304c69a22e75aee48892fb0ac42eb7eb22feec239f1deee47e79c60131a` |
| C | `dist/node-agent-coordinator/main.cjs` | `c6bad29e64073f4fbebe5a9d6818e53381efd7aa1c7909c84172e3da9b2de9f0` |
| A | `dist/renderer/assets/index.eager-app-BfVhIMYf.js` | `09f8a360a138c4680956c797c8a0fc25fbf619c9421327be75da0935a9aa9e80` |
| U | `dist/renderer/assets/index-D6ReTfRE.js` | `f3c242161efb08158161e259960b0ae91c150f6891983f6a70faf80c6f5ee697` |
| D | `dist/renderer/assets/chunk-automation-detail-panel-DE8HhaGR.js` | `4f878e203b8b14acb61fb663dc55408641d5dabc6f5fb405d6ce2d5b7e42d77e` |

D est une nouvelle copie dérivée créée avec `prettier@3.6.2`, parser `babel`, entrée standard et `--ignore-path /dev/null`. Les originaux et le manifest de formatage historique n'ont pas été modifiés.

## Ce que le client envoie et où

Le schéma partagé du coordinateur est `aU = {name:string, prompt:string, trigger:$H, isEnabled?:boolean}`. Il n'impose aucun ID de skill, timezone séparé, liste d'exclusions ou champ de fichier. Les commandes passent par la validation `wx`; ce schéma est donc une frontière d'appel utilisée par le client. `Y9` reconstruit l'objet à partir des seules propriétés déclarées, en retirant les champs supplémentaires. Une chaîne `schedule` n'y reçoit aucune validation cron, et `firesAt` reste une chaîne opaque. `earlyMinutes` et `lateMinutes` sont des nombres finis, sans contrainte locale de signe ou d'intégralité dans ce schéma. Sources C:37193-37223, 38865-38891, 39220-39232, 42378-42408. [CR01, CR05]

| Intention | Arguments de domaine | Chemin retracé |
|---|---|---|
| Créer | `{id, spec}` | A:52671-52679 vers `createAgentAutomation`; C:43662-43683 retombe sur le gateway |
| Modifier | `{id, automationId, spec}` | A:52684-52685 vers `updateAgentAutomation`; même gateway |
| Tester maintenant | `{id, automationId, sessionId?}` | A:52715-52738 vers `runAgentAutomationNow`; même gateway, résultat déclaré `void` à C:42335 |
| Lire pour un bot | `{id}` | Backend serveur pour une route Temporal prise en charge; gateway pour la route box |
| Pause/reprise | `{id, automationId, isEnabled}` | `SetGrokBotAgentAutomationEnabled` sur la route serveur; gateway sur la route box |
| Supprimer | `{id, automationId}` | `DeleteGrokBotAgentAutomation` sur la route serveur; gateway sur la route box |
| Lire la credential webhook | `{id, automationId}` | Serveur, puis `DashboardService.CreateAutomationWebhookApiKey`, ou gateway selon la route |

La distinction est explicite. `Yh` contient lecture, activation, suppression et credential, mais pas création, modification ou run manuel. Dans `IJ`, ces trois derniers appels aboutissent à `me()`, donc `gateway.dispatchCommand`. Le performer M:112638-112676 n'expose lui aussi que la partie lecture/activation/suppression/credential/liste de compte. On ne peut pas inventer un RPC `CreateGrokBotAgentAutomation` ou un endpoint HTTP pour ces opérations. Sources C:39474-39535 et 43619-43683; M:112638-112676. [CR02]

Les écrans retracés proposent de configurer une routine par le chat. Le bouton Edit prépare le texte traduit `Edit your routine: <name>` via `onSeedPrompt`; il n'ouvre pas un formulaire cron dans le panneau D. Le panneau affiche le prompt en Markdown et la liste des déclencheurs. Test est masqué quand `isServerOwned` vaut vrai. Cette restriction d'interface ne suffit pas à prouver un refus du serveur. Sources D:630-678, 783-840, 910-946; U:42031-42045. [CR04]

Le client connaît aussi le nom d'outil `UpdateRoutine`. Pour l'import de template, le prompt embarqué demande l'action `create`, désigne la voie sous-jacente `createAutomation`, demande `enabled=false`, puis une confirmation avant `resume`. Ce sont des instructions au modèle présentes dans le client. Leur application par l'agent et la sauvegarde effective ne sont pas observées. A:32930-33012. [CR19]

## Le record reçu n'est pas le spec de création

P:49934 ne transporte que `GrokBotAgentAutomation {automation_id:string, record_json:string}`. M `Ure` parse ce JSON et `Upn` demande les propriétés suivantes. [CR03]

| Propriété JSON | Contrôle effectué dans M:111209-111228 |
|---|---|
| `id`, `name`, `prompt` | Chaînes |
| `trigger` | Objet, sans vérification ici de sa variante |
| `schedule`, `triggerDescription` | Chaînes, même pour un déclencheur événementiel |
| `isEnabled` | Booléen |
| `createdAt` | Nombre |
| `filePath` | Chaîne, sans existence ou lecture de fichier |
| `runs` | Tableau, sans validation ici des éléments |

`nextRunAt` est optionnel en pratique dans le tri. `triggerPresentation.version === 1` permet de reformuler une liste de triggers, sinon l'affichage emploie le trigger cron ou `triggerDescription`. Ces propriétés sont consommées sans constituer un schéma complet de stockage. Un record qui échoue `Upn` est ignoré par `u1`; il ne provoque pas automatiquement une erreur de toute la liste. `u1` trie d'abord `nextRunAt`, les valeurs absentes à l'infini, puis `createdAt`. L'aperçu de routines U:41942-41944 les retrie par `createdAt`. M:111224-111240; A:31650-31674. [CR03, CR12]

L'ID JSON `id` et le `automation_id` protobuf sont distincts dans le code. La récupération d'une clé cherche la routine par l'ID JSON, puis utilise l'ID protobuf pour générer l'URL et demander la clé. Leur égalité n'est pas garantie par ce chemin. M:111313-111358. [CR14]

Aucune constatation nouvelle n'établit un fichier `AUTOMATION.md`, un répertoire `cron/`, un contenu YAML, une écriture de crontab, un lancement de `cron` ou un service launchd de routine. Les recherches de mots et les fonctions de routine ont été bornées aux bundles cités et au JavaScript extrait; les chaînes `k8s.cronjob.*` trouvées appartiennent au catalogue de télémétrie. Cela n'exclut pas un fichier ou un cron distant dont l'implémentation n'est pas livrée. [CR20]

## Déclencheurs et filtres déclarés

Le schéma C:38806-38891 admet dix types simples et un groupe. Un groupe a au moins deux listeners simples; le schéma ne permet pas les groupes imbriqués. L'affichage utilise une disjonction entre les listeners. Le contrat indique plusieurs sources d'événements, mais ne démontre ni leur enregistrement réel, ni une politique serveur de déduplication. [CR05, CR06]

| Type | Champs du schéma partagé |
|---|---|
| `cron` | `schedule`; optionnels `firesAt`, `window:{earlyMinutes,lateMinutes}`, `timing:"exact"|"flexible"` |
| `slack` | `channel`; `match.kind` parmi `mention`, `keyword` avec `keyword`, `message`, `reaction` avec `emoji?` et `bySelf?` |
| `github` | `repo`, `events[]`; `pr?`, `userAllowlist[]?`, `ciBranch?` |
| `origin` | `repo`, `events[]`; `pr?`, `userAllowlist[]?` |
| `microsoftTeams` | `tenantId`, `teamIds[]`, `channelIds[]`, `messageContains`, `messageContainsIsRegex`, `blockUnauthenticatedTeamsUsers` |
| `linear` | `event.case`: `issueCreated`, `statusChanged` avec `statusIds[]`, ou `endOfCycle` avec `cycleIds[]`; `projectIds[]`, `teamIds[]` |
| `sentry` | `event.case`: `issueCreated`, `issueResolved`, `issueAssigned`, `issueArchived`, `issueUnresolved`, `issueAny`; `projectIds[]` |
| `pagerduty` | `event.case`: `incidentTriggered`, `incidentAcknowledged`, `incidentResolved`, `incidentEscalated`, `incidentAny`; `serviceIds[]` |
| `email` | `inbox`; `from[]?`, `requireAuthPass?` |
| `webhook` | Le seul discriminant `type` |
| `group` | `listeners` contenant au moins deux des types simples ci-dessus |

GitHub accepte `pr-opened`, `pr-pushed`, `pr-merged`, `pr-closed`, `review-requested`, `review-approved`, `review-changes-requested`, `review-commented`, `pr-comment`, `inline-review-comment`, `review-thread-resolved`, `review-thread-unresolved`, `issue-assigned`, `ci-passed`, `ci-failed`. Origin a la même liste sauf `pr-closed` et `issue-assigned`. C:38419-38450. [CR06]

L'affichage Slack connaît `channel="*"` pour "anywhere on Slack". L'affichage GitHub ajoute la branche pour les événements CI et l'allowlist d'utilisateurs. Pour email, l'absence de `requireAuthPass` signifie authentification attendue dans la description; seul `false` décrit l'inclusion des expéditeurs non authentifiés. Ce sont des conventions de présentation, pas des preuves d'enforcement du provider. A:906-908, 31531-31602. [CR07]

Les intégrations de listeners ont une ressource distincte. Le client lit `getListenerIntegrations`; les chemins de connexion passent soit par le lien Slack desktop, soit par les connexions SCM desktop derrière `sand_desktop_scm_connect`, soit par `getListenerConnectUrl`. La lecture périodique nominale est de 5 secondes ou 15 secondes selon cette gate, avec fraîcheur de 30 secondes et suspension de certains ticks quand l'écran est caché. Ce polling observe des intégrations, il n'exécute pas le cron. A:60343-60559. [CR18]

## Syntaxe cron, intervalle et timezone

Le parseur client `i0/lne/Fi` normalise les espaces et sépare un préfixe `CRON_TZ=<zone>` ou `TZ=<zone>`. Il reconnaît les aliases `@hourly`, `@daily`, `@midnight`, `@weekly`, `@monthly`, `@yearly`, `@annually`, insensibles à la casse. Le cron ordinaire a cinq champs, minute 0-59, heure 0-23, jour du mois 1-31, mois 1-12, jour de semaine 0-7; 7 devient dimanche 0. Listes avec virgules, plages et pas sont reconnus. Le code inspecté ne prend pas en charge un champ de secondes, les noms `MON/JAN`, `L`, `W`, `#` ou `@reboot`. Sources A:30583-30670. [CR08]

Si jour du mois et jour de semaine sont tous deux restreints, `pR` les combine par `OR`. Le mois reste obligatoire. Le calcul ne traite donc pas `0 9 1-7 * 1` comme "premier lundi seulement": il accepte les sept premiers jours ou les lundis. Pourtant la description `Ane` peut reconnaître ce motif comme `nth-weekday`. La divergence entre le texte descriptif et le prédicat est visible dans la copie; elle n'établit pas un bug du scheduler serveur. A:30657-30667 et 30913-30925. [CR09]

`@every <durée>[/<phase>]` est une autre syntaxe. Le parseur accepte des unités contiguës `ms`, `s`, `m`, `h`, `d` et des composantes décimales, par exemple `1h30m` ou `0.5s`. La durée totale doit être positive. Une phase peut être zéro, mais doit rester inférieure à l'intervalle. Ce chemin ne prend pas le préfixe timezone. Sources A:30583-30587 et 30744-30766. [CR10]

Le calcul candidat est précis:

- Sans phase, `next = reference + interval`.
- Avec phase, `next = (floor((reference-phase)/interval)+1)*interval+phase`, alignement à l'époque UTC.
- Pour cron, il démarre à la minute strictement suivante, à `floor(reference/60000)*60000+60000`, puis cherche au plus `366*24*60` minutes.
- Il projette chaque instant via `Intl.DateTimeFormat` pour une timezone reconnue. Sans zone utilisable, il emploie les getters locaux de `Date`. Il accélère les jours non admissibles par un saut avec correction de changement de date.

Sources A:30682-30743 et 30787-30802. [CR11]

Il faut corriger la portée de la conclusion antérieure sur le "calcul next run côté client". `mne` n'a qu'un appel direct dans A:31243, pour décrire l'heure d'un `@every` horaire avec phase. Ce cas emprunte la branche intervalle. Aucun appel actif de la branche cron de `yne/pne` n'a été retrouvé dans les fichiers extraits examinés. Le calcul cron est du code statiquement présent; son utilisation pour choisir une occurrence de routine n'est pas établie. Le `nextRunAt` utilisé pour trier arrive dans le record serveur. [CR11, CR12]

La conséquence DST du calcul candidat vient de son parcours en instants réels. Dans une zone IANA, une heure locale supprimée au printemps ne correspond à aucune minute visitée; une heure répétée à l'automne peut correspondre à deux instants. Il n'ajoute ni clé d'occurrence, ni suppression explicite de la seconde occurrence. La borne de 366 jours peut retourner `null` pour un prochain 29 février situé plus loin. Ces conséquences sont déduites du code, sans exécuter la cible. Elles ne sont pas les garanties du scheduler. [CR11]

La zone envoyée aux RPC serveur est l'override utilisateur, à défaut la zone détectée. Lecture, liste de compte, activation et suppression l'ajoutent sous `timeZone` si le resolver renvoie une valeur. La route box reçoit aussi `userTimeZone` et `userTimeZoneOverride` dans les réglages hôte. Une zone inscrite dans la chaîne cron a priorité dans le calcul candidat. Sources M:107298-107307, 111249-111251, 111275-111291, 111360-111374, 98784-98793; A:30798-30802. [CR13]

Le label de timezone cron n'apparaît que si le préfixe existe, que la zone est valide et qu'elle diffère de la zone du lecteur après canonicalisation Intl. La description d'une fenêtre remplace "at" par "around" lorsque `firesAt` ou `window` existe; elle ne consulte pas `timing` ici. Les minutes early/late ne sont pas appliquées par ce code d'affichage. A:31137-31147 et 31604-31634. [CR05, CR13]

## Exécution unique, exclusions et historique

`firesAt` est admis dans le trigger cron, mais aucun parse ISO, choix de date, départ unique ou suppression après départ n'est retracé. Le label "Once" de `AR/Rne` apparaît pour un cron avec un seul mois, jour, horaire, sans année ni jour de semaine restreint. Il ne suffit pas à prouver une exécution unique: le prédicat cron reste annuel. A:31085-31132. [CR15]

Il n'existe aucun champ d'exclusion dans `aU/qH`, ni dans les schémas de trigger partagé retracés. Aucune règle locale de fêtes, jours exclus, dates exceptées, délai de rattrapage ou concurrence serveur n'a été trouvée dans ces fonctions. Cette absence est bornée au spec accepté par le client; le document JSON ou le serveur peuvent avoir des champs et règles supplémentaires. [CR05, CR20]

Les runs ne sont pas un contrat protobuf détaillé. Dans les chemins observés, chaque élément utile contient au moins `id`, `trigger` et `status`. `trigger=="manual"` sélectionne le dernier run manuel d'après l'ordre reçu. `status=="running"` maintient le blocage du bouton; toute autre valeur termine ce blocage local. Le code ne valide pas ici une liste exhaustive `success/failed/cancelled`, la durée ou les timestamps. A:52501-52524 et D:678. [CR16]

Le run manuel conserve les phases locales `awaiting`, `running` et `cooldown`. Il compare le nouvel ID manuel à l'ancien et attend le changement de statut quand il voit `running`. La réarmement est de 3 secondes. Après succès de la commande, il relit la liste; si la liste n'a pas encore montré de nouvel ID, il passe aussi en cooldown. Une erreur de commande retire le blocage, une erreur de relecture déclenche le cooldown puis remonte l'erreur. Le blocage n'est donc ni une transaction durable, ni une garantie contre deux desktops. Il n'y a pas de nonce de run dans les arguments partagés. A:52306-52315, 52482-52524, 52715-52766. [CR16]

Le `sessionId` est passé pour la session affichée si elle diffère de la constante de session par défaut. Le bouton est aussi bloqué si `runs[0]` est `running`, même si ce run n'est pas manuel. La portée des runs automatiques par session et les autorisations serveur correspondantes restent inconnues. A:52731-52738; D:648-717. [CR16]

## Clés webhook et suppression

Le panneau montre une URL "POST to", une clé et le header `Authorization: Bearer <key>`. C'est une convention présentée à l'utilisateur; aucun POST webhook n'a été observé, donc format de corps, signature, réponses, quotas et idempotence restent inconnus. D:436-487. [CR14]

La route serveur relit la liste, vérifie qu'un webhook simple ou membre du groupe existe, cherche l'ID serveur non vide et appelle `CreateAutomationWebhookApiKey` avec un timeout de 20 secondes. Elle construit `/automations/webhook/<automation_id encodé>` sous le backend courant. Le périmètre de compte vient du token et chaque nouvelle lecture du token doit rester dans ce périmètre, sinon l'accès est annulé. M:111252-111273 et 111313-111358. [CR14]

La clé est conservée dans `userData/automation-webhook-keys/<sha256>.bin`. Le nom est le SHA-256 du JSON `[backendUrl, accountScope, agentId, automationId]`. Le contenu déchiffré a `{version:1, createdAt, key}`. `safeStorage` chiffre le JSON; le dossier est créé en `0700` et l'écriture atomique en `0600`. `createdAt` invalide une clé pour une routine recréée. La file d'accès exclusif est par dossier/backend/agent/routine. En cas d'échec de l'écriture, le cache de clé permet de retenter la sauvegarde sans demander immédiatement une autre clé. La suppression locale enlève ce fichier avant le RPC de suppression; une suppression serveur échouée peut donc laisser la routine active avec une credential locale à récupérer de nouveau. M:111123-111207 et 111360-111367. Aucun fichier de clé réel n'a été lu. [CR14]

Le renderer attend une clé prête pendant au plus 15 lectures nominalement espacées de 2 secondes. Une clé `null` garde l'URL visible; à la limite il produit `automations/webhook-key-timed-out`. Une erreur de lecture produit `automations/webhook-credential-read-failed` et arrête le polling; une commande client permet de réessayer. Le polling s'arrête après le dernier abonnement. Un changement de `createdAt` invalide la ressource. A:52306-52473 et 52694-52713. [CR14]

## Activation, synchronisation et échecs

L'activation affiche un état optimiste. Si la commande échoue, le panneau retire cet état et annonce l'échec pause/reprise. Un compteur évite qu'une réponse plus ancienne annule une intention plus récente. D:675-696; U:42152-42173. Le client conserve les précédentes données quand une relecture échoue, revalide après reconnexion et ignore les résultats d'une génération de travail abandonnée. A:52332-52346, 52539-52568, 52626-52647, 52777-52812. [CR17]

La liste de compte est une fusion explicite. Les agents Temporal sont demandés par groupes de 200; si le backend demande `readIndividually`, le coordinateur utilise quatre lecteurs concurrents. Il vérifie la correspondance exacte des IDs normalisés de la réponse. Les lignes serveur remplacent les lignes box pour les agents servis. En cas d'indisponibilité serveur pour un agent, il peut conserver ses lignes box dans cette vue globale; si le gateway échoue et que des lignes serveur existent, il retourne ces seules lignes. C:43320-43346, 43429-43530, 43586-43614; constante C:42512. Cette tolérance de liste globale ne transforme pas un refus de mutation Temporal en fallback. [CR17]

Le rafraîchissement de la famille `automations` après un tour relit les routines et émet `agents-automation`. Les lectures simultanées pour un même agent/famille se regroupent et peuvent demander un second passage; un échec est journalisé par `<family> refresh for <agent> after a server turn failed`. Les événements gateway transmettent aussi les lignes `agents-automation`. C:42439-42507, 45727-45730, 45915-45930; A:52806-52812. [CR17]

Le renderer traite `automation-run-now/refused` comme un échec de frontière hôte sans retry. Il sait afficher `automation_failed`, `routine_sync_failed`, `automation_sync_failed`, `routines_paused_while_away` et `automations_paused_while_away`. Les textes embarqués annoncent des routines mises en pause après des résultats non lus et une synchronisation susceptible de retarder les horaires, ainsi que des choix spend-guard `keep/pause/never-ask/resume/stay-paused`. Ces textes et choix ne donnent pas les seuils, le budget, les règles de retries d'exécution ou l'algorithme distant. Sources A:32708-32712, 40139-40154, 51656-51670, 73306-73315, 77429-77430 et catalogue A:551. [CR21]

## Questions couvertes et frontières restantes

`answered` signifie que la question sur le client est établie statiquement. `partial` signifie qu'une frontière est retrouvée, avec un mécanisme distant absent. Aucun état ne signifie validation en production.

| Question | État | Réponse / limite |
|---|---|---|
| Arguments create/update/enable/delete/run-now | answered | Schémas et points d'appel retracés |
| Route création/mise à jour/test | answered | Gateway; ces trois opérations ne sont pas des RPC protobuf ajoutés |
| Routine sans skill | answered | Nom, prompt et trigger autonomes |
| Champs minimum `record_json` | answered | Validation et consommation relues; schéma complet absent |
| Fichier de sauvegarde et cron OS | partial | `filePath` reçu; écriture réelle, fichier et scheduler distants inconnus |
| Grammaire cron et intervalle du client | answered | Cinq champs, aliases, préfixe timezone, durées et phase |
| Next-run client actif | partial | Algorithme cron présent, appel actif de cette branche non établi; `nextRunAt` reçu |
| DST et rattrapage distant | unresolved | Déductions du calcul candidat, aucune garantie d'exécution serveur |
| Exclusions et jours fériés | unresolved | Absents du spec partagé; règles serveur inconnues |
| One-shot | partial | `firesAt` et labels présents; année/suppression/départ unique non établis |
| Providers, filtres et groupes | answered | Schéma partagé et présentation; delivery/enforcement distants inconnus |
| Webhook et stockage local de clé | answered | URL, convention Bearer, scope et chiffrement retracés |
| Webhook delivery / idempotence | unresolved | Corps, signatures, réponses, retries et déduplication non observés |
| Runs et anti-double-clic | partial | États et ID locaux; garanties multi-client et durabilité inconnues |
| Logs, sync failures et pause spend-guard | partial | Événements, textes et handlers client; seuils et retry moteur absents |

Pour poursuivre sans mélanger hypothèse et preuve, il faudrait des sources du scheduler, une capture passive déjà disponible de records/runs, ou une autorisation distincte d'observation runtime contrôlée. Cette passe ne réclame pas ces actions et ne les a pas effectuées.
