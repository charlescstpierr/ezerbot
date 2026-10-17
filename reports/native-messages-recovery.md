# Messages 0.71.2 : envoi natif et reprise après panne

**Le circuit natif étudié ne fournit pas de garantie de déduplication après panne.** Le daemon possède des caches de replay en mémoire. Le handler Swift extrait quatre champs de l’opération, lance un AppleScript via `osascript` et construit son résultat à partir du statut du sous-processus. Il ne reçoit ni `sendKey` ni nonce dans cette projection et ne retourne pas d’ID du message Messages. Une panne après l’effet et avant le résultat reste donc ambiguë dans les preuves disponibles.

Ces constats portent sur des corps **x86_64 statiques** de la distribution 0.71.2, sans exécution ni émulation. Le [ledger](../evidence/native-messages-recovery.json) conserve 20 désassemblages, leurs hashes et plages d’octets, les ancres et la table d’arguments. Aucun fichier de conversations, compte ou profil n’a été lu.

## MS01 — Le module Messages est dans le helper déjà identifié

La cible est la copie `inputs/releases/0.71.2/helper/Grok Bot Computer Use.app/Contents/MacOS/CUGrokBotService`, SHA-256 `c337d70faba610cbc78f437b9e19be40d8dbd68fb72c59643daf8263ad2a76b3`, 6 486 368 octets. Malgré son nom produit Computer Use, l’exécutable contient les corps `CUMessages.MessagesRPCHandler` et `MessagesSend` examinés ici. Son plist déclare version `1.0.0`, build `1`; 0.71.2 désigne la distribution.

La slice x64 commence à l’offset universel 16 384 et mesure 3 320 240 octets; SHA-256 `090e5852366f1da64991cfa6a40cb75d587941b81a6c61de433b6a7e893f3ae6`. Métadonnées REA réutilisées : `ev_220ccf89be1b5be1559b4b2acbe6f2dac6e73596a29ea8fd4ae83d3d8da92bf6`, [bundle existant](../evidence/releases/0.71.2/native-mcp/rea-macho-bundle.json). Aucune session REA n’est ouverte, fermée ou modifiée pour cette passe; aucun nouveau montage.

## MS02 — Le sendKey reste dans les caches du daemon

Dans le daemon 0.71.2, `finishedSends`, `sentByKey` et `sendKeyTail` sont des `Map`. `replayedSend` compare `JSON.stringify(e.op)` et recherche par requestId, puis par sendKey. `rememberSend` conserve jusqu’à 64 résultats par requestId; seuls les succès `messages-result` alimentent le cache par sendKey. L’éviction supprime la première clé de l’ordre de Map. `withSendKey` sérialise la clé et retire son chaînage en `finally`. Sources : [J01](../artifacts/releases/0.71.2/formatted/dist/local-exec-daemon/main.cjs:128123), [J03](../artifacts/releases/0.71.2/formatted/dist/local-exec-daemon/main.cjs:128301), [J04](../artifacts/releases/0.71.2/formatted/dist/local-exec-daemon/main.cjs:128080), [J05](../artifacts/releases/0.71.2/formatted/dist/local-exec-daemon/main.cjs:128460).

L’appel effectif est **`this.messages.run(e.op, this.maxFileBytes)`**, sans transmission séparée de `e.sendKey` ou du requestId. Ces clés protègent la vie de cette instance du daemon et sa fenêtre de cache; ce sont des états en mémoire, sans restauration durable dans les corps cités. Source : [J02](../artifacts/releases/0.71.2/formatted/dist/local-exec-daemon/main.cjs:128251). Cette observation complète la frontière déjà décrite dans [autonomie et reprise](goal-execution-autonomy.md), sans l’étendre au serveur distant.

## MS03 — Le handler natif projette exactement quatre champs

Le service RPC sélectionne un handler par préfixe de méthode, puis lui remet méthode, dictionnaire d’arguments et identité de signature du pair. `MessagesRPCHandler` reconnaît `messages/send`. La table à VM `0x10019d110` contient les couples suivants, relus directement dans les octets du binaire :

| Champ JSON natif | Argument construit |
|---|---|
| `to` | `--to` |
| `service` | `--service` |
| `chatId` | `--chat-id` |
| `textFile` | `--text-file` |

La boucle avance de `0x20` et s’arrête à `0x80`; seules les valeurs JSON de type chaîne donnent une paire flag/valeur. Elle appelle `MessagesSend.parse`, puis programme la closure d’envoi sur la queue globale. **Aucun nonce/sendKey n’est propagé par cette projection**, même si un dictionnaire entrant contenait d’autres propriétés. Sources : [S02](../evidence/releases/0.71.2/native-messages/rpc_method_handler.asm.txt:39), [S03](../evidence/releases/0.71.2/native-messages/rpc_dispatch.asm.txt:1247), [S04](../evidence/releases/0.71.2/native-messages/rpc_dispatch.asm.txt:1543), [S05](../evidence/releases/0.71.2/native-messages/handler_dispatch.asm.txt:145), [S06](../evidence/releases/0.71.2/native-messages/handler_dispatch.asm.txt:179), [S07](../evidence/releases/0.71.2/native-messages/handler_dispatch.asm.txt:313), [S08](../evidence/releases/0.71.2/native-messages/handler_dispatch.asm.txt:371); table et hash dans le ledger.

## MS04 — L’effet est délégué à AppleScript

La closure d’envoi appelle `MessagesSend.run(address:textFile:)` à VM `0x10013f5f9`. `sendScript` construit un script qui lit `textFile` en UTF-8, entre dans `tell application "Messages"`, puis utilise soit `send targetText to chat id …`, soit le premier service du type choisi et `send targetText to participant … of targetService`. Ces fragments sont relus aux adresses VM `0x100163120–0x100163220`; les annotations de chaînes LLVM ne suffisent pas car les représentations Swift peuvent introduire un décalage de 32 octets. Sources : [S09](../evidence/releases/0.71.2/native-messages/handler_c4.asm.txt:11), [S10](../evidence/releases/0.71.2/native-messages/send_script.asm.txt:15), [S11](../evidence/releases/0.71.2/native-messages/send_script.asm.txt:47), [S12](../evidence/releases/0.71.2/native-messages/send_script.asm.txt:76).

`run` crée `NSTask`, choisit **`/usr/bin/osascript`**, configure les pipes, puis appelle `launchAndReturnError:`. Le script est encodé en `Data`, écrit sur stdin et le handle est fermé. stdout est envoyé au périphérique nul; stderr est lu jusqu’à EOF. Aucune commande n’a été exécutée pendant l’enquête. Sources : [S13](../evidence/releases/0.71.2/native-messages/send_run.asm.txt:43), [S14](../evidence/releases/0.71.2/native-messages/send_run.asm.txt:89), [S15](../evidence/releases/0.71.2/native-messages/send_run.asm.txt:126), [S16](../evidence/releases/0.71.2/native-messages/send_run.asm.txt:156), [S17](../evidence/releases/0.71.2/native-messages/send_run.asm.txt:182).

## MS05 — Le succès natif est le succès du sous-processus

Après `waitUntilExit`, `terminationStatus == 0` prend la branche de succès. Le résultat `MessagesSendResult` encode **exactement `version`, `verbs`, `via`**. `via` vaut `chat_id` ou `participant` selon l’adresse choisie. Il n’encode ni nonce, sendKey, identifiant du message, instant d’envoi, état de remise ni reçu du destinataire. Sources : [S18](../evidence/releases/0.71.2/native-messages/send_run.asm.txt:308), [S20](../evidence/releases/0.71.2/native-messages/send_run.asm.txt:408), [S21](../evidence/releases/0.71.2/native-messages/send_result_key.asm.txt:4), [S22](../evidence/releases/0.71.2/native-messages/send_result_encode.asm.txt:49), [S23](../evidence/releases/0.71.2/native-messages/send_result_encode.asm.txt:73).

La closure encode ensuite ce résultat en payload JSON et la continuation retourne au service RPC. Le statut nul d’osascript constitue donc le critère local observé dans le code; il n’établit pas une remise au destinataire ni un reçu durable pour réconcilier un replay. Sources : [S24](../evidence/releases/0.71.2/native-messages/handler_c4.asm.txt:18), [S25](../evidence/releases/0.71.2/native-messages/handler_return.asm.txt:21), [S26](../evidence/releases/0.71.2/native-messages/rpc_handler_result.asm.txt:19).

## MS06 — Les erreurs ne certifient pas l’absence d’effet

Un échec de lancement convertit l’`NSError` en erreur Swift. Un statut de sortie non nul construit `osascript_exit_<status>` avec le stderr nettoyé, puis jette `MessagesError`. La closure propage l’erreur au lieu de produire `MessagesSendResult`. Sources : [S27](../evidence/releases/0.71.2/native-messages/send_run.asm.txt:267), [S18](../evidence/releases/0.71.2/native-messages/send_run.asm.txt:308), [S19](../evidence/releases/0.71.2/native-messages/send_run.asm.txt:377), [S09](../evidence/releases/0.71.2/native-messages/handler_c4.asm.txt:11).

Les corps cités ne montrent ni compensation, ni recherche d’un message précédemment envoyé, ni boucle de retry de l’effet. Ils ne permettent pas de conclure qu’un message n’a pas été envoyé lorsqu’une réponse est perdue, qu’une étape ultérieure échoue ou que le processus est interrompu. L’erreur de transport et l’absence d’effet restent deux propositions distinctes.

## MS07 — Frontière de reprise après panne

| Point de panne hypothétique | Ce que les corps établissent | Ce qui reste inconnu |
|---|---|---|
| Avant le lancement d’osascript | Il n’a pas encore été lancé par cet appel | État d’un éventuel appel antérieur |
| Après l’instruction AppleScript `send`, avant le résultat natif | L’effet est confié à Messages; le résultat est produit plus tard | Effet réellement appliqué, remise et état récupérable |
| Après résultat, avant mémorisation JS | Le cache JS n’est alimenté qu’au retour | Reprise atomique effet + cache |
| Après redémarrage du daemon ou éviction | Les `Map` ne constituent pas un journal persistant | Réconciliation garantie avec l’historique Messages |

C’est une **inférence de l’ordre des appels**, pas un test de crash. Sur ce circuit tracé, aucune transaction ne relie l’effet AppleScript à une entrée durable de déduplication. Le constructeur `MessagesRPCHandler` instancie son `PromptGate` et son préfixe `messages/`; il ne restaure pas de store de résultats. Cette preuve bornée n’est pas un inventaire exhaustif de tout le processus ni des mécanismes internes de Messages. Source : [S01](../evidence/releases/0.71.2/native-messages/handler_init.asm.txt:6), avec les chemins d’effet et de résultat ci-dessus.

## MS08 — Vérification et questions restantes

```sh
python3 tools/inspect-native-messages.py verify
```

Le [vérificateur](../tools/inspect-native-messages.py) relit la copie du binaire, le hash x64, les octets des 20 corps sélectionnés, les littéraux et les quatre paires de la table, les ancres et les hashes des preuves. Le [résultat](../evidence/releases/0.71.2/native-messages/verification.json) est conservé. Il ne télécharge, ne monte, ne lance et n’émule rien. `capture` rejoue seulement LLVM sur ces symboles.

Les recherches lexicales bornées pour nonce/sendKey/dedup/replay/SQLite sont conservées comme diagnostics secondaires; leurs résultats n’établissent pas seuls une absence sémantique de persistance.

| Question | Statut |
|---|---|
| Champs natifs et propagation de sendKey/nonce | answered pour la projection citée |
| Point de délégation de l’effet et critère de résultat | answered, statique x64 |
| Caches JS : constructeur, clés, sérialisation et éviction | answered pour les corps cités |
| Journal/transaction native de déduplication après panne | unresolved; aucun établi dans ce circuit |
| État réellement envoyé/livré après perte de réponse | unresolved |
| Équivalence arm64 et comportement macOS réel | unresolved |

Aucun accès compte/profil, aucune permission macOS et aucune exécution du helper, d’osascript ou de Messages. Aucun rapport antérieur, index ou code Rust n’est modifié.
