# Routines : runs, conversation et transcript dans Grok Bot 0.70.2

**Les preuves relient les routines à leur panneau et à des événements de configuration, mais pas un run ID à une conversation ou à son résultat.** `runs` arrive dans un JSON serveur peu validé; ses consommateurs retrouvés servent au blocage local déjà documenté dans CR16. La sélection UI ouverte ici est celle d’une routine. Aucun producteur client de run ID, écran d’historique sélectionnable par run, ni jointure run→session→entrées n’est établi.

Cette passe statique complète [cron-detail.md](cron-detail.md), sans refaire parser cron, webhook ou anti-double-clic. Elle utilise les mêmes six sources 0.70.2, originaux et copies formatées inchangés; leurs douze hashes correspondent au ledger précédent. Le parent REA `ev_a8b822a62831e30bd01eb4c4488b36f0bd50cb8ccc41d83e2dcff4e00cf82a4d` est réutilisé, sans nouvelle analyse globale ni session. Le [ledger](../evidence/routine-run-conversation.json) conserve hashes originaux, lignes LF, extraits, recherches bornées et limites. Aucune action compte, routine ou réseau.

## RR01 — Le run ID reste un champ reçu, sans contrat détaillé

`GrokBotAgentAutomation` ne contient que `automation_id` et `record_json`. Le parseur `Upn/Ure` exige seulement que `runs` soit un tableau, puis retourne l’objet complet; il ne valide pas ses éléments, leur ID, leur session ou leurs résultats. L’ordre interne de `runs` n’est pas reconstruit par `u1`, qui trie les routines elles-mêmes. Sources : [R01](../artifacts/formatted/dist/electron-main/proto.cjs:49914), [R02](../artifacts/formatted/dist/electron-main/main-app.cjs:111209).

La commande `runAgentAutomationNow` a une réponse déclarée **`void`**, pas `{runId,…}`. Le code client ouvert ne produit donc pas cet ID à cette frontière. L’ID observé par CR16 est lu ultérieurement dans le premier run dont `trigger === "manual"`, selon l’ordre reçu. Le producteur serveur, l’unicité et la durabilité de cet ID restent inconnus. Sources : [R03](../artifacts/formatted/dist/node-agent-coordinator/main.cjs:42324), [R04](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:52499).

## RR02 — Le tableau reçu n’est pas un historique de runs affiché

La recherche littérale `.runs` sur les 600 scripts extraits donne cinq accès pertinents : trois dans l’état CR16, un `runs[0]?.status === "running"` dans la ligne de routine et le même contrôle dans le panneau. Trois autres occurrences du daemon concernent des segments de texte Messages. Aucun de ces cinq consommateurs ne rend une liste de runs, ne lit un résultat, un timestamp de run, un sessionId de run, ni un lien de transcript. La recherche des noms explicites automationRun/routineRun/selectedRun n’a aucun résultat; ce diagnostic lexical ne prouve pas une absence sémantique générale. Les contextes exacts et hashes des 600 fichiers sont conservés et rejouables dans le ledger.

Le panneau rend le prompt de la **routine**, ses triggers et éventuellement le webhook, puis ses actions. Ce prompt n’est pas le résultat d’un run. Sources : [R09](../artifacts/formatted/dist/renderer/assets/chunk-automation-detail-panel-DE8HhaGR.js:630), [R10](../artifacts/formatted/dist/renderer/assets/chunk-automation-detail-panel-DE8HhaGR.js:783). Un serveur peut enrichir `record_json`; aucun de ces enrichissements ne devient un historique visible par les corps cités.

## RR03 — La sélection ouvre une routine, pas une exécution

La requête de panneau contient `agentId`, éventuellement `automationId` et une ancre de focus. L’état UI conserve cet automationId; `GFe` recherche `routine.id === automationId` dans les snapshots du bot. Le composant lazy reçoit ensuite `agentId` et l’objet `automation`, et sa clé React est `je.id`. Aucun run ID n’intervient dans ce chemin. Sources : [R06](../artifacts/formatted/dist/renderer/assets/index-D6ReTfRE.js:44168), [R07](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:13477), [R08](../artifacts/formatted/dist/renderer/assets/index-D6ReTfRE.js:44454).

Pour un test manuel, le client transmet la session courante si elle n’est pas la session par défaut. C’est **un argument de destination de la commande**, pas une réponse qui confirme l’association du run ou ouvre une autre conversation. Le handler ne sélectionne aucune conversation au retour. Source : [R05](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:52715); mécanisme de blocage déjà décrit par CR16.

## RR04 — Les champs session retrouvés ne créent pas la jointure manquante

Le contrat compilé `GrokBotTeamAgentSharedRoutine` déclare `automation_id`, `session_id` et `last_run_at_ms?`. Il appartient à l’état partagé administratif des bots. Cette passe n’établit pas de consommateur renderer de ce contrat qui ouvrirait la session d’une routine, ni de run ID dans cette déclaration. Il s’agit donc d’une **association déclarée routine→session**, pas d’une preuve d’affichage actif ou de jointure run→conversation. Source : [R11](../artifacts/formatted/dist/electron-main/proto.cjs:63212).

Le transport de transcript possède ses propres clés : `{agent_id, session_id, generation, after_updated_seq}`; les sessions ont `agent_id/session_id/kind`. Le constructeur client de clé utilise `transcript:<agent>` ou `transcript:<agent>\0<session>`. Le run ID de routine n’est pas un de ses arguments. Sources : [R12](../artifacts/formatted/dist/electron-main/proto.cjs:48144), [R13](../artifacts/formatted/dist/electron-main/proto.cjs:48665), [R14](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:7918). Les corps cités ne permettent pas de déduire que chaque run crée une session ou réutilise toujours la même.

## RR05 — Les cartes « routine » du transcript décrivent des modifications

Le groupe visuel `routine-event-group` est construit à partir d’entrées `event` dont `event.type === "automation-changed"`. À partir de deux entrées adjacentes, il conserve les entrées d’origine dans `entries`, forme un ID de groupe à partir de la première entrée et calcule une synthèse. Ce n’est pas un nouvel enregistrement d’exécution. Source : [R18](../artifacts/formatted/dist/renderer/assets/index-D6ReTfRE.js:61246).

La synthèse `NWe` agrège par **automationId**, mémorise si une création apparaît et garde la dernière action/le dernier nom. Les actions reconnues sont `created`, `updated`, `enabled`, `disabled`, `deleted`; une création suivie de modifications peut rester résumée comme création, tandis qu’une suppression finale l’emporte. Cette réduction est une présentation des changements, pas une déduplication de runs ni une suppression des entrées sources. Sources : [R16](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:36842), [R17](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:38756).

Les cartes et chips appellent `openAutomations()`; le clic ne transmet pas de run ID et ne demande pas un transcript de run. Sources : [R19](../artifacts/formatted/dist/renderer/assets/index-D6ReTfRE.js:78855), [R20](../artifacts/formatted/dist/renderer/assets/index-D6ReTfRE.js:86942). Les noms « routine-events » ou « run » dans d’autres fonctions UI ne suffisent pas à établir une exécution du scheduler.

## RR06 — La relecture des routines est indépendante de celle du transcript

Un `agent_state_changed` reçu du watch appelle `refresh({agentId,families})`. Pour `automations`, le coordinateur relit `getAgentAutomations`, puis émet `agents-automation`. Les familles sont dédupliquées par `Set`; les lectures concurrentes d’une même paire agent/famille se regroupent, avec un second passage demandé si besoin. Une erreur est journalisée et la clé de travail est libérée. Sources : [R24](../artifacts/formatted/dist/node-agent-coordinator/main.cjs:42439), [R25](../artifacts/formatted/dist/node-agent-coordinator/main.cjs:45984).

Le renderer installe la nouvelle liste via `ingestAutomationsEvent`; il maintient des compteurs de tentative/génération, invalide une lecture concurrencée et conserve les données précédentes en cas d’échec. Reconnexion et abonnements déclenchent les relectures prévues. Le flux gateway peut aussi transmettre `agents-automation`. Sources : [R21](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:52541), [R22](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:52622), [R23](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:52776), [R26](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:76330), [R33](../artifacts/formatted/dist/node-agent-coordinator/main.cjs:45723). Aucun de ces rafraîchissements ne demande « les entrées du run X » : le transcript et la liste de routines sont deux consommateurs distincts.

## RR07 — Erreurs visibles : chemins génériques et titre de routine

Le formatter actif `Rve` sait rendre `titleKind === "automation_failed"` avec `titleParams.name`; le composant de tray appelle ce formatter pour son titre. C’est une capacité d’affichage humaine si une telle tray arrive. Le helper `gw` possède aussi une branche de construction du titre de routine, mais ses appels directs ouverts concernent `bot_failed_to_respond`; aucun producteur d’échec de routine relié à `runs[i].id` n’est établi. Sources : [R27](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:51663), [R28](../artifacts/formatted/dist/renderer/assets/index-D6ReTfRE.js:134640), [R29](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:73304).

Le watch distant possède un échec de **tour**, avec `agent_id`, `session_id`, `turn_id`, code, résumé et date; le coordinateur transmet cette notification au renderer. Le renderer déduplique ces échecs génériques par `turnId` dans l’instance, conserve une version plus riche lorsqu’elle arrive et retire les trays du thread quand un transcript plus récent est publié/effacé. Le chemin host upsert une tray par son `id`. Ce ne sont pas des garanties de déduplication par run ID. Sources : [R15](../artifacts/formatted/dist/electron-main/proto.cjs:49570), [R25](../artifacts/formatted/dist/node-agent-coordinator/main.cjs:45984), [R30](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:73569), [R31](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:73495), [R32](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:73362).

Aucun corps cité n’établit quel texte de succès ou d’erreur d’un run devient une entrée assistant, ni ce que le bot reçoit dans son contexte de modèle. Une sortie normale peut emprunter le transcript général, mais **son attribution à un run est ici inconnue**.

## RR08 — Ce qui est répondu, et ce qui reste distant

| Question | Statut | Réponse précise |
|---|---|---|
| Contrat et provenance du run ID | partial | Champ JSON reçu/consommé; producteur et portée non livrés |
| Historique reçu | partial | `runs[]` opaque; ordre et schéma complet inconnus |
| Sélection/ouverture d’un run | unresolved | Chemin actif trouvé seulement pour la routine |
| Run→session/conversation | unresolved | Session de commande manuelle et contrat admin distincts |
| Cartes de routine dans le transcript | answered | Modifications de configuration agrégées, pas historique de runs |
| Résultat/erreur au bot et à l’humain | partial | Formatters et échecs génériques actifs; jointure au run absente |
| Invalidations/relectures | answered | Ressource routines, événements et générations retracés |
| Déduplication de runs après panne/multi-client | unresolved | Agrégation visuelle, fetch coalescing et turnId ne la prouvent pas |

```sh
python3 tools/verify-routine-run-conversation.py
```

Le [vérificateur hors ligne](../tools/verify-routine-run-conversation.py) contrôle les douze hashes source, les ancres LF, les recherches lexicales et les preuves; [résultat](../evidence/routine-run-conversation-verification.json). Il ne formate ni n’évalue le JavaScript, n’ouvre aucune session REA et ne lance aucun target. Cette question n’a pas nécessité d’attribuer des corps 0.71.2 à la version 0.70.2.
