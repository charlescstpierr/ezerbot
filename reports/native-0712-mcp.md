# Helper natif 0.71.2 : façade MCP, socket interne et lease de contrôle

Le helper possède un mode **MCP sur stdin/stdout** relié au même code produit que son mode application. Il contrôle son processus père, traduit les outils `computer_*`, puis délègue des opérations au service local par `CUServiceConnector` et `CULocalRPCClient`. Le socket existe donc toujours derrière cette façade. **Le `conversation-id` n’est pas le sessionID du lease de contrôle.** Ces résultats proviennent de corps x86_64 désassemblés, sans exécution ni émulation du helper.

Le [ledger](../evidence/native-0712-mcp.json) conserve symboles, adresses, plages de bytes, jump tables, chaînes résolues et ancres. Le [rapport JavaScript complémentaire](local-cua-path-0712.md) traite l’activation du broker par le daemon. Les présents constats ne corrigent pas rétroactivement les anciennes frontières socket, qui étudiaient une autre entrée.

## NM01 — Provenance et architecture

Seul `Contents/Helpers/Grok Bot Computer Use.app` est copié depuis le DMG officiel 0.71.2 déjà acquis et hashé, vers `inputs/releases/0.71.2/helper/`. Le montage est `readonly/nobrowse/noautoopen`; le démontage et la suppression du point temporaire réussissent. Les cinq fichiers copiés correspondent aux hashes de l’inventaire du DMG. Le paquet original 0.70.2 est préservé.

| Identité | Valeur |
|---|---|
| Exécutable | `Grok Bot Computer Use.app/Contents/MacOS/CUGrokBotService` |
| Taille universelle | `6486368` octets |
| SHA-256 universel | `c337d70faba610cbc78f437b9e19be40d8dbd68fb72c59643daf8263ad2a76b3` |
| Slice x86_64 | offset fichier `16384`, taille `3320240` |
| Slice arm64 livrée | offset `3342336`, taille `3144032`, non étudiée ici |
| Plist du helper | `co.anysphere.grok-bot-computer-use`, version `1.0.0`, build `1`, macOS minimum `14.0` |

« 0.71.2 » identifie la distribution contenant le helper; ce n’est pas la version déclarée par son propre plist. REA `inspect_macho` produit `ev_220ccf89be1b5be1559b4b2acbe6f2dac6e73596a29ea8fd4ae83d3d8da92bf6`, conservé dans le [bundle versionné](../evidence/releases/0.71.2/native-mcp/rea-macho-bundle.json). Aucun moteur de décompilation ou autre prérequis n’est installé. Les adresses suivantes sont des **VM addresses x86_64**, pas des offsets universels.

## NM02 — L’entrée bifurque avant la boucle application

`_main` initialise `CUProductContext`, lit les arguments et appelle `CUServiceApplication.main` à `0x1001314ca`. Ce dernier cherche `--mcp-stdio`; sa branche à `0x100011366` rejoint l’appel `CUMcpStdioMode.run` à `0x1000115dc`. Le mode MCP et le mode accessoire RPC appartiennent au même exécutable, mais sont deux chemins d’entrée.

`StdioMode.parse` lit `GROK_SESSION_ID`, reconnaît `--conversation-id` et remplace la valeur par l’argument suivant. Il accepte aussi `--log stderr`; une valeur de conversation absente ou vide conduit à un refus de parsing. Les preuves incluent les comparaisons de chaîne et les branches, pas seulement les chaînes présentes dans le fichier. Sources : [entrée](../evidence/releases/0.71.2/native-mcp/entry_main.asm.txt), [sélection](../evidence/releases/0.71.2/native-mcp/application_main.asm.txt), [parser](../evidence/releases/0.71.2/native-mcp/stdio_parse.asm.txt), notamment `0x100077c58`, `0x100077d4a`, `0x100077dba`, `0x100077ef8`.

Le run stocke séparément cette identité de conversation dans `CUMcpSessionState`. `callCompanion` l’ajoute à ses arguments sous `session_id`, à `0x10007584f–0x100075872`. Cela fournit du contexte à cette voie; ce n’est pas une preuve d’autorisation de contrôle par la seule connaissance d’un ID.

## NM03 — Le processus père est contrôlé et un refus termine avec 77

Avant le parsing et l’ouverture du serveur MCP, le run appelle `checkCurrentParent` à `0x100078669`. Ce corps lit `getppid`, construit `CULocalRPCPeerIdentity(processID:)`, lit `geteuid`, traite le père PID 1 comme refus, exige une identité exploitable, compare l’UID et appelle `CULocalRPCPeerPolicy.allows` à `0x10006e9e3`. Les branches de refus sont visibles dans le même corps.

Le run transforme ce refus en journal puis **`exit(77)`**, à `0x1000789c2–0x1000789c7`. Sources : [contrôle du père](../evidence/releases/0.71.2/native-mcp/parent_check.asm.txt), [run](../evidence/releases/0.71.2/native-mcp/stdio_run.asm.txt). L’ensemble d’identités utilisé dépend de `CUProductConfiguration`; cette passe ne reconstitue pas toutes ses valeurs prod/dev ni toutes les variantes de politique. Le contrôle MCP porte sur le processus père; l’admission ultérieure au socket reste une étape distincte.

## NM04 — Le transport est JSON par lignes et les calls entrent dans une file

Le run obtient `fileHandleWithStandardInput` et `fileHandleWithStandardOutput`, puis construit `CUMcpServer`. Son lecteur appelle `read`, accumule les bytes, cherche `0x0a` et remet chaque ligne à `CUMcpServer.receive`. Ce dernier utilise `JSONDecoder`, construit les requêtes et une `CUMcpCancellation` par ID, puis passe les appels outils à une continuation `AsyncStream`. Le handler de requête rejoint ensuite `CUMcpToolset.call`.

Sources : run `0x100078fa8–0x100078fd5`; [lecture](../evidence/releases/0.71.2/native-mcp/reader_loop.asm.txt), `0x10006ff61–0x10006fff9`; [réception](../evidence/releases/0.71.2/native-mcp/server_receive.asm.txt), `0x1000706a5–0x1000706e8` et `0x10007104d`; [handler](../evidence/releases/0.71.2/native-mcp/handle_tool.asm.txt), `0x10006f395`. Cette preuve ne mesure ni concurrence effective ni débit et ne remplace pas les règles de queue du broker JavaScript.

## NM05 — Les noms MCP sont adaptés aux méthodes internes

Les getters `CUMcpToolName.rawValue` et `wireName` dispatchent sur deux tables de 16 entrées, à `0x100079df0` et `0x10007b0c4`. Le ledger conserve les offsets relatifs, destinations de branche et bytes de ces tables. Les longs littéraux Swift sont relus dans le fichier à leur adresse effective : les annotations LLVM peuvent désigner le texte situé 32 octets avant la chaîne réellement retournée. Le tableau suivant ne repose donc pas sur les annotations seules.

| Nom MCP | Nom interne `wireName` |
|---|---|
| `computer_screenshot` | `computer_use_screenshot` |
| `computer_click` | `computer_use_click` |
| `computer_move` | `computer_use_mouse_move` |
| `computer_drag` | `computer_use_drag` |
| `computer_type` | `computer_use_typing` |
| `computer_key` | `computer_use_press_key` |
| `computer_scroll` | `computer_use_scroll` |
| `computer_wait` | aucun : branche locale d’attente |
| `computer_check_permissions` | `computer_use_check_permissions` |
| `computer_apps` | `computer_use_apps_list` |
| `computer_resolve_app` | `computer_use_select_app` |
| `computer_app_state` | `computer_use_app_state` |
| `computer_set_value` | `computer_use_set_value` |
| `computer_app_action` | `computer_use_perform_secondary_action` |

L’enum natif comprend aussi `computer_start_control` et `computer_release_control`, sans wireName. `Toolset.call` les branche directement vers `startControl` et `releaseControl`; ils ne doivent pas être ajoutés aux 14 outils annoncés par le daemon sur cette seule preuve. Un nom inconnu produit une réponse de refus. Sources : [nom MCP](../evidence/releases/0.71.2/native-mcp/tool_raw_name.asm.txt), [nom interne](../evidence/releases/0.71.2/native-mcp/tool_wire_name.asm.txt), [dispatch](../evidence/releases/0.71.2/native-mcp/tool_call.asm.txt), `0x100079eb6–0x100079ec3`, `0x100079fde–0x10007a090`.

## NM06 — La façade rejoint le companion et son lease, pas directement CGEvent

Le run construit `CUServiceConnector` après `CUServiceDiscovery.serviceAppURL(containing:)`, puis `CUMcpSessionState` et `CUMcpToolset`. `Toolset.plan` produit un `CUMcpServiceCall`; `perform` distingue une voie companion et, pour les noms reconnus par `CULocalRPCService.isRemoteInputTool`, une voie `callRemoteInput`. Le classificateur compare notamment les méthodes internes de clic, scroll et mouvement. Sources : run `0x100078dc0–0x100078efb`; [plan](../evidence/releases/0.71.2/native-mcp/tool_plan.asm.txt), `0x10007fbd6`; [perform](../evidence/releases/0.71.2/native-mcp/tool_perform.asm.txt), `0x10007c1a4–0x10007c233`; [classement](../evidence/releases/0.71.2/native-mcp/is_remote_input.asm.txt).

`startControl` crée ou réutilise un **UUID de requestID**, à `0x100073c79–0x100073cfb`, puis appelle `CUServiceConnector.startControl`. Au succès, le sessionID retourné est stocké comme lease détenu; ce n’est pas l’identité de conversation stockée ailleurs. `callRemoteInput` lit ce lease, refuse le chemin lorsqu’il manque et le transmet à `callTool`. `releaseHeld` emploie `withRunningClient` et `releaseControl(sessionID:timeoutSeconds:)`. Le corps cité établit cette route de libération; ses effets sur le cycle de vie du service ne sont pas observés. Sources : [acquisition](../evidence/releases/0.71.2/native-mcp/session_start_connect.asm.txt), [stockage](../evidence/releases/0.71.2/native-mcp/session_start_store.asm.txt), [input avec lease](../evidence/releases/0.71.2/native-mcp/session_remote_input.asm.txt), [libération](../evidence/releases/0.71.2/native-mcp/session_release_held.asm.txt).

Les deux voies utilisent ensuite `CUServiceConnector.withClient` et un `CULocalRPCClient` possédant une `socketURL`. La closure `callTool` rejoint finalement **`CULocalRPCClient.send`** à `0x1000bd2fc`. Sources : [connector](../evidence/releases/0.71.2/native-mcp/connector_call.asm.txt), `0x1000b7a3b–0x1000b7aab`; [envoi](../evidence/releases/0.71.2/native-mcp/connector_rpc_call.asm.txt). La séparation établie est donc : **daemon → enfant MCP par conversation sur stdio → connector local → service companion par RPC/socket**. La découverte du service dépend du produit/app, pas d’un socket arbitraire fourni dans l’outil.

Ce raccordement expose la frontière d’admission et de lease avant les opérations graphiques. Il ne prouve pas, à lui seul, qu’un `CGEvent` a été posté. Les corps finaux CGEvent/AX de chaque outil 0.71.2, l’unicité du companion en exécution et les courses entre conversations ne sont pas reconstruits ici.

## NM07 — Annulation, EOF et SIGTERM ont des effets précis

Le récepteur retrouve une annulation par `requestId` puis appelle `CUMcpCancellation.cancel` à `0x1000712b1`. Celui-ci pose un flag sous verrou et annule la tâche Swift enregistrée, si elle existe, à `0x10007af33`. `cancelAll` applique ces opérations aux demandes connues. Cela établit une annulation coopérative des tâches, pas une transaction annulant des actions déjà effectuées par le companion.

Lorsque `read` retourne zéro, le lecteur rejoint son chemin de fermeture. Il programme une libération anticipée, appelle `cancelAll` et termine le stream entrant à `0x10007048b–0x1000704b0`. Le run configure aussi un `DispatchSource` pour **SIGTERM 15**, après avoir ignoré le traitement Unix par défaut; son handler effectue la même séquence de libération/annulation/fin de stream. Sources : [EOF](../evidence/releases/0.71.2/native-mcp/reader_loop.asm.txt), [setup SIGTERM](../evidence/releases/0.71.2/native-mcp/stdio_run.asm.txt), `0x1000792ff–0x10007945a`; [handler SIGTERM](../evidence/releases/0.71.2/native-mcp/stdio_sigterm.asm.txt), `0x100077265–0x100077353`.

La tâche de consommation atteint `releaseControlBestEffort`, puis une branche `exit(0)` à `0x100077a16–0x100077a18`. La libération reste **best effort** : le code existe, mais sa réussite et son délai réel ne sont pas observés. La fermeture de la façade n’est pas un ordre démontré de terminer le processus companion. Aucun signal ni EOF n’a été provoqué contre le helper pendant l’enquête.

## NM08 — Limites de version et vérification

Le tableau de symboles 0.70.2 conservé contient déjà `CUMcpStdioMode`, `CUMcpParentCheck` et `CUMcpToolset`. Leur présence native n’est donc pas une nouveauté attribuée à 0.71.2. Cette passe décrit précisément les corps du helper fourni par 0.71.2; le rapport JavaScript établit séparément les consommateurs actifs du daemon. Les garanties de l’ancienne voie socket ne sont pas automatiquement celles de l’entrée stdio, ni l’inverse.

```sh
python3 tools/inspect-native-0712-mcp.py verify
```

La [vérification hors ligne](../evidence/releases/0.71.2/native-mcp/verification.json) relit le hash universel et celui de la slice x64, compare les bytes imprimés des désassemblages aux offsets Mach-O, vérifie les plages, littéraux, tables et ancres et contrôle les hashes des preuves. Elle ne lance, n’émule, ne monte ni ne télécharge la cible. `capture` permet de rejouer les seuls désassemblages sélectionnés avec LLVM déjà présent. Les éventuels bytes de jump tables imprimés comme mauvais opcodes ne sont pas interprétés comme instructions.

| Question | Statut |
|---|---|
| Entrée stdio, conversation et contrôle du père | answered, statique x64 |
| Adaptation des 14 noms annoncés | answered, tables et corps |
| Connector/socket et identités de lease distinctes | answered pour les chemins cités |
| Annulation, EOF, SIGTERM et sortie | answered au niveau des branches; effets réels non observés |
| CGEvent/AX final de chaque outil et concurrence du companion | partial |
| Équivalence arm64 et permissions OS effectives | unresolved |

Session REA fermée; montage démonté et point temporaire supprimé. Aucune installation, permission macOS ou lecture de compte/profil n’a été réalisée.
