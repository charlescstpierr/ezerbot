# Routines 0.70.2 : run, conversation et transcript

**Aucune jointure run ID→conversation→résultat n’est établie dans les chemins ouverts.** Le client reçoit `runs[]`, sélectionne une routine par automationId et affiche des modifications de configuration. Ces trois mécanismes ne constituent pas un historique navigable d’exécutions.

Cette passe complète [cron-detail.md](cron-detail.md), sans reprendre cron, webhook ou CR16. Les douze hashes des six sources originales/formatées correspondent à ce ledger antérieur. Parent REA réutilisé : `ev_a8b822a62831e30bd01eb4c4488b36f0bd50cb8ccc41d83e2dcff4e00cf82a4d`. Le [ledger nouveau](../evidence/routine-run-conversation.json) conserve provenance, hashes originaux, 35 ancres LF et diagnostics rejouables. Aucune nouvelle analyse globale REA, copie formatée, session, action compte ou réseau.

## RR01 — Identité de run reçue, producteur inconnu

`GrokBotAgentAutomation` transporte `automation_id` et `record_json`. `Upn/Ure` exige seulement un tableau `runs`, sans valider ses éléments, IDs, sessions ou résultats. `u1` trie les routines; il ne reconstruit pas l’ordre des runs. La réponse de `runAgentAutomationNow` est déclarée **`void`**. Le run ID observé par CR16 vient ensuite du premier élément `trigger === "manual"` reçu. Son producteur serveur, sa portée et sa durabilité restent inconnus. Sources : [R01](../artifacts/formatted/dist/electron-main/proto.cjs:49914), [R02](../artifacts/formatted/dist/electron-main/main-app.cjs:111209), [R03](../artifacts/formatted/dist/node-agent-coordinator/main.cjs:42324), [R04](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:52499).

## RR02 — Aucun historique sélectionnable établi

Sur les 600 scripts extraits, `.runs` apparaît dans cette validation, **cinq lectures UI pertinentes**, et trois accès du daemon à des segments de texte Messages. Les cinq lectures UI sont les trois consommateurs CR16 et deux contrôles `runs[0]?.status === "running"`. Aucun ne rend une liste de runs ou ne lit leur résultat/session. La recherche automationRun/routineRun/selectedRun ne donne rien; cette absence lexicale reste bornée.

Le panneau rend le **prompt de la routine**, ses triggers et éventuellement le webhook, puis ses actions. Il n’affiche pas le résultat d’un run. Sources : [R09](../artifacts/formatted/dist/renderer/assets/chunk-automation-detail-panel-DE8HhaGR.js:630), [R10](../artifacts/formatted/dist/renderer/assets/chunk-automation-detail-panel-DE8HhaGR.js:783). `record_json` peut néanmoins contenir des champs supplémentaires non consommés ici.

## RR03 — Sélection de routine et argument session distincts

L’état du panneau reçoit `automationId`; `GFe` retrouve la routine par `id`; le composant lazy reçoit cet objet `automation` et utilise son ID comme clé. Aucun run ID n’intervient. Sources : [R06](../artifacts/formatted/dist/renderer/assets/index-D6ReTfRE.js:44168), [R07](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:13477), [R08](../artifacts/formatted/dist/renderer/assets/index-D6ReTfRE.js:44454).

Le test manuel transmet la session courante si elle diffère du défaut. C’est un argument de commande, sans réponse confirmant l’association du run ni navigation vers une nouvelle conversation. Source : [R05](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:52715). Le blocage local reste celui de CR16.

## RR04 — Session déclarée ≠ association active démontrée

`GrokBotTeamAgentSharedRoutine` déclare `automation_id`, `session_id` et `last_run_at_ms?`, dans l’état partagé administratif. Aucun consommateur renderer qui ouvrirait cette session n’est établi ici; aucun run ID n’est dans cette déclaration. Sources : [R11](../artifacts/formatted/dist/electron-main/proto.cjs:63212), [R34](../artifacts/formatted/dist/electron-main/proto.cjs:61772), [R35](../artifacts/formatted/dist/electron-main/proto.cjs:63165).

Le transcript possède ses propres clés `{agent_id,session_id,generation,after_updated_seq}`; les sessions possèdent `agent_id/session_id/kind`. La clé client `transcript:<agent>[\0<session>]` ne prend pas de run ID. Ces corps n’établissent ni une session créée par run ni une session unique réutilisée. Sources : [R12](../artifacts/formatted/dist/electron-main/proto.cjs:48144), [R13](../artifacts/formatted/dist/electron-main/proto.cjs:48665), [R14](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:7918).

## RR05 — Le transcript montre des changements de routine

`routine-event-group` regroupe au moins deux entrées adjacentes `automation-changed`, conserve les entrées originales et forme son ID depuis la première. `NWe` synthétise par **automationId**, conserve la dernière action/le dernier nom et mémorise une création antérieure. Les actions affichées sont `created/updated/enabled/disabled/deleted`; une suppression finale l’emporte. C’est une agrégation visuelle des modifications, pas une déduplication d’exécutions. Sources : [R16](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:36842), [R17](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:38756), [R18](../artifacts/formatted/dist/renderer/assets/index-D6ReTfRE.js:61246).

Les cartes/chips appellent `openAutomations()` sans run ID ni demande de transcript de run. Sources : [R19](../artifacts/formatted/dist/renderer/assets/index-D6ReTfRE.js:78855), [R20](../artifacts/formatted/dist/renderer/assets/index-D6ReTfRE.js:86942).

## RR06 — Relectures et invalidations

Le watch `agent_state_changed` demande un refresh des familles. Pour `automations`, le coordinateur relit `getAgentAutomations` puis émet `agents-automation`. Il déduplique les familles et regroupe le travail par agent/famille, avec second passage demandé si nécessaire; les erreurs sont journalisées. Sources : [R24](../artifacts/formatted/dist/node-agent-coordinator/main.cjs:42439), [R25](../artifacts/formatted/dist/node-agent-coordinator/main.cjs:45984).

Le renderer installe la liste, invalide les lectures concurrencées par compteur de tentative/génération, conserve les données précédentes en cas d’échec et revalide à la reconnexion. Le gateway peut aussi fournir ces listes. Cela ne demande pas les entrées du run X. Sources : [R21](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:52541), [R22](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:52622), [R23](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:52776), [R26](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:76330), [R33](../artifacts/formatted/dist/node-agent-coordinator/main.cjs:45723).

## RR07 — Erreurs humaines, résultat du bot non relié au run

Le formatter actif `Rve` rend une tray `titleKind:"automation_failed"` à partir du nom; l’UI appelle ce formatter. `gw` sait aussi construire ce titre, mais ses appels directs ouverts concernent `bot_failed_to_respond`. Aucun producteur de tray lié à `runs[i].id` n’est établi. Sources : [R27](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:51663), [R28](../artifacts/formatted/dist/renderer/assets/index-D6ReTfRE.js:134640), [R29](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:73304).

Le watch d’échec général fournit **agent/session/turnId**, code, résumé et date. Le renderer déduplique par turnId dans l’instance, accepte une version plus riche et retire les trays lorsqu’un transcript plus récent est publié/effacé. Le flux host upsert par tray ID. Ces identités ne sont pas reliées au run ID de routine. Sources : [R15](../artifacts/formatted/dist/electron-main/proto.cjs:49570), [R25](../artifacts/formatted/dist/node-agent-coordinator/main.cjs:45984), [R30](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:73569), [R31](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:73495), [R32](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:73362).

Quel texte d’un run devient une entrée assistant, et ce que le bot reçoit dans son contexte, restent inconnus. Les capacités du transcript général ne suffisent pas à attribuer une sortie à une routine.

## RR08 — Checklist et vérification

| Question | Statut |
|---|---|
| Contrat/producteur du run ID; schéma de l’historique | partial : JSON reçu, producteur distant inconnu |
| Sélection de run; jointure run→session→résultat | unresolved |
| Cartes de modifications; invalidations/relectures | answered statiquement |
| Erreur humaine et contexte reçu par le bot | partial : affichage possible, attribution au run inconnue |
| Déduplication durable ou multi-client de runs | unresolved |

```sh
python3 tools/verify-routine-run-conversation.py
```

Le [vérificateur](../tools/verify-routine-run-conversation.py) contrôle les douze hashes, 35 ancres, recherches et preuves; [résultat](../evidence/routine-run-conversation-verification.json). Aucun JavaScript évalué ni target lancé. Aucune extension à 0.71.2, aucun ancien rapport/index/code Rust modifié.
