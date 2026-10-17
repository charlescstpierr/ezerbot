# Grok Bot 0.71.2 : distribution officielle et comparaison ciblée

La distribution publique 0.71.2 ajoute un worker local pour Messages et transmet désormais le fuseau de l’appareil dans le chemin `sendPrompt` inspecté. **Elle ne fournit pas, dans les composants et corps examinés ici, le moteur serveur interbot, de compaction ou de mémoire, ni l’application iOS.** Les inconnues correspondantes restent ouvertes. Cette conclusion est bornée à l’inventaire et aux chemins décrits ci-dessous; aucune analyse sémantique globale des 596 scripts n’est revendiquée.

Cette version est conservée séparément de **0.70.2**. Aucun résultat 0.71.2 n’est rétroattribué à l’ancien paquet. Le [ledger](../evidence/release-0712-discovery.json) contient les hashes, extraits, comparaisons, références REA et limites. Les livrables précédents et `inputs/Grok Bot.app` restent inchangés.

## R71-01 — Acquisition et intégrité observée

Le GET public du **17 octobre 2026, 04:25:53–04:25:55 UTC** vise uniquement l’[URL officielle du DMG](https://downloads.cursor.com/grokbot/stable/darwin-x64/0.71.2/Grok_Bot_0.71.2_x64.dmg), découverte par la route de distribution du paquet précédent. Aucun cookie, compte ou redirection n’est utilisé pendant ce GET.

| Élément | Valeur |
|---|---|
| Fichier acquis | `inputs/releases/0.71.2/Grok_Bot_0.71.2_x64.dmg` |
| Taille | `156486357` octets |
| SHA-256 DMG | `0187b33b8ec21cb7fbaf380d476b57462dc3a5bb512f919ed9b5b0d25718d4aa` |
| SHA-256 ASAR | `05a48b6088fcb89c00b29cce6bbb6b46b377506cd5a394e51285ca12c55c12d2` |
| Info.plist lu dans le volume | `com.anysphere.sand`, version/build `0.71.2` |

REA `open_binary` reconnaît le DMG et son hash. Le premier `inspect_artifact` refuse une contradiction ASAR. La reprise explicite `integrity_policy=record-and-continue` conserve **six contradictions**, toutes sur des natifs unpacked : cursor-proclist, notification-settings, tree-sitter-bash, tree-sitter, sand-op-launcher et sand-webauthn-signer. Les hashes déclarés et observés sont conservés; ces membres restent `observed-untrusted`. Une signature appliquée après l’ASAR est une explication possible, pas une cause démontrée.

Preuve d’inspection : `ev_87e734ee1e33c5a658dfe8d3861d7e99a0eeba507bdcadc54ad521305bca7bf8`; inventaire : `ev_899fd0c224155306982f15219634bb0ad958dd57df86d099317ad9ea02d02bf4`; manifeste : `agm_3f81763ba91abfa8dcd1941fb2aef04938385ca346eec5c8d7202b5387c8f04c`. Le [bundle REA versionné](../evidence/releases/0.71.2/rea-dmg-bundle.json) conserve 947 nœuds et 1 404 occurrences. Le hash du DMG est une identité observée, pas une vérification de signature de l’éditeur.

Le montage manuel est `hdiutil attach -readonly -nobrowse -noautoopen`; seules Info.plist, l’ASAR et ses compagnons unpacked sont copiés. Le volume est démonté avec succès et le point temporaire supprimé. Aucun framework complet n’est recopié, aucune installation ou exécution du produit n’a lieu.

## R71-02 — Inventaire : quelques nouveaux noms, aucune source map

L’[inventaire ASAR](../artifacts/releases/0.71.2/asar-inventory.json) porte sur **668 fichiers**, contre **673** pour la baseline extraite. Le nombre de scripts JS/CJS/MJS passe de **600 à 596**. Parmi les chemins exactement communs, 218 fichiers sont identiques et 56 changent. Les 394 chemins ajoutés et 399 retirés incluent beaucoup de renommages de chunks par hash; ce ne sont pas autant de fonctionnalités ajoutées ou supprimées.

Après retrait heuristique du suffixe de build, trois familles de noms apparaissent :

```text
dist/local-exec-daemon/messages-worker.cjs
dist/renderer/assets/chunk-auto-review-approval-CkDg2elb.js
dist/renderer/assets/chunk-provider-computer-use-_8Rdmtlu.js
```

Cette correspondance ne suffit pas à identifier tous les modules : plusieurs familles ont des collisions, notamment `chunk-view` qui passe de 60 à 62 fichiers. Le [delta d’inventaire](../artifacts/releases/0.71.2/inventory-comparison.json) garde ces groupes au lieu de les présenter comme équivalents. Les noms `server`, `harness`, `scheduler`, `room` ou `interbot` ne révèlent pas de nouveau fichier dédié; cela ne prouve pas l’absence de code inliné dans un bundle modifié.

Aucun membre `.map`, `.ipa`, `.apk` ou `.aab` n’est trouvé dans les deux ASAR comparés. Aucun `sourceMappingURL=` n’est trouvé dans les scripts des deux versions. Le reste du paquet Mac est inventorié séparément; aucun paquet mobile ni source map n’y est découvert. Ces résultats n’excluent pas un artefact extérieur au DMG.

## R71-03 — Le nouveau worker sert les Messages du Mac

L’ancien daemon exécute `fre(Ere({installDir:...}),op,maxFileBytes)` dans son propre chemin Messages. Le nouveau daemon installe `oZe`, crée un `node:worker_threads.Worker` vers `messages-worker.cjs`, maintient une map de requêtes par ID et transmet `{id,op,maxFileBytes,expiresAtMs}`. Les erreurs ou sorties du worker rejettent les demandes en attente. Sources : L70:128112–128130; L71:128040–128108,128113–128131.

Le worker vérifie `parentPort`, reçoit ces messages, rejette une lecture déjà expirée et renvoie soit un résultat soit une erreur. Ses corps effectifs traitent les opérations Messages, puis joignent le sidecar local par son socket de service; les verbes comprennent snapshot, pièce jointe, envoi et permissions Messages. Sources W71:2372–2440,2576–2602. La lecture du code ne lit aucun profil Messages réel et ne lance aucun helper. C’est une séparation d’exécution locale macOS; elle ne révèle pas un ordonnanceur cloud ni le transport iOS.

Les deux nouveaux chunks nommés sont aussi lus : `provider-computer-use` expose les ressources de permissions, refresh, ouverture des réglages et révocation d’app autorisée; `auto-review-approval` prépare une décision utilisateur et appelle le résolveur. Le résolveur rejoint `resolveGrokBotAutoReviewApproval`, sans livrer la politique de décision serveur. Sources U71:29–73; R71:545–577; M71:113430–113448. Leur nouveau découpage en fichiers n’établit pas que ces capacités sont nouvelles.

## R71-04 — Un delta concret du caller `sendPrompt` : le fuseau

Le corps 0.70.2 appelle `sendGrokBotUserMessage` avec bot, nonce, texte, richText, références et options de timeout. Le corps 0.71.2 conserve ces champs, détecte aussi le fuseau système, le passe à `sendableDeviceTimeZone`, puis ajoute **`x-cursor-timezone`** aux headers lorsque le helper retourne une valeur. Le constructeur de l’adapter raccorde effectivement `detectDeviceTimeZone ?? detectTimeZone`. Sources M70:110631–110671; M71:112768–112777,113145–113184,115068–115083.

Le helper utilise `Intl.DateTimeFormat` : absence ou fuseau invalide donne `undefined`; un fuseau dont la résolution est `UTC` est aussi omis; sinon la chaîne d’entrée est retournée. Ce n’est pas une canonisation générale du nom. Sources K71:20556–20578,45464–45473; M71:64–68,7468–7471 établissent le raccordement au registre `sand.main-core.modules`.

Le nom de header existait déjà dans le chemin vocal 0.70.2. La nouveauté constatée est donc limitée à ce caller texte. L’utilisation serveur de ce header pour dates, contexte ou routines reste inconnue. Aucun envoi de message n’est effectué.

## R71-05 — Les frontières interbot restent distantes

Les descripteurs `SendGrokBotAgentMessageRequest` et `Response` ont les mêmes champs sérialisés dans les deux versions : `from_agent_id`, `to_agent_id`, `message_id`, `text`, `sent_at_ms`, `reply_to_id?`, puis livraison, destinataire, nom et workflow optionnel. Les extraits de schéma sont comparés textuellement, sans évaluer le bundle protobuf. Sources P70:56672–56698; P71:58581–58607.

La recherche de `sendGrokBotAgentMessage` retrouve toujours exactement les trois descripteurs partagés, dans proto, daemon et coordinator. Les corps vérifiés du client continuent à parser un événement gateway `{channel,payload}` et à séparer les transcriptions box/temporal. Le coordinator construit effectivement un POST JSON vers `${baseUrl}/api/<méthode>` et l’autre branche passe par `performServerAgentAction`; une réponse serveur non implémentée conserve ses gardes et erreurs. Sources C70:41997–42038; C71:41813–41825,42465–42474,40732–40747,44358–44372.

Le transport, les schémas et ces consommateurs sont donc visibles; le choix du bot destinataire, le contenu exact délégué à son modèle et l’ordonnanceur distant ne sont pas récupérés. L’absence de nom d’appel direct ne couvre pas les accès calculés.

## R71-06 — La promotion de mémoire garde le même corps utile

Le wrapper renderer conserve `listGrokBotUserBotMemories`, `agentId`, `factIds` et une relecture après erreur. Le main transmet toujours `agentId` et une copie de `factIds`, attend 120 secondes et rend `created`, `alreadyInTeam`, `keptPrivateCount`. Les deux fragments sont égaux après substitutions explicites des seuls identifiants minifiés, conservées dans le ledger. Sources A70:13067–13088, A71:13482–13503; M70:103055–103067, M71:103703–103715; constantes M70:102889–102894, M71:103537–103542.

Le descripteur `GrokBotTeamContextSummary` conserve aussi exactement ses champs. Le nom `getGrokBotTeamContextSummary` reste limité aux trois descripteurs dans le corpus recherché. Aucun nouvel extracteur, correcteur, oubli durable ou assembleur de mémoire modèle n’est établi dans ces chemins.

## R71-07 — La compaction ne gagne pas de producteur retracé

`ConversationSummary` conserve exactement résumé, bornes de troncature/envoi, résumé précédent, inclusion des outils et stratégie. `SummarizeAction` reste un contrat vide. La configuration spéculative conserve son schéma et ses replis `70,5,5,5`; ses deux occurrences sont toujours déclaration et repli. Sources P70:27826,28463; P71:29195,29832; M70:57999–58005,59880–59889; M71:58200–58206,60062–60071.

La lecture de contexte vocal existante reste une requête gateway `readMainAgentContext`, avec son repli en cas d’indisponibilité. Sources M71:41174–41188,48755–48768. Cela ne devient pas le constructeur du prompt texte. Le seuil effectif par modèle, le prompt de résumé, le choix des messages gardés, les checkpoints et la relation avec la mémoire durable restent inconnus.

## Checklist et reproduction

| Question | Statut | Résultat |
|---|---|---|
| Acquisition officielle et identité 0.71.2 | answered | DMG/ASAR hashés, version lue |
| Membres nouveaux et sources maps | answered | Inventaires et limites des correspondances conservés |
| Rôle des trois nouvelles familles nommées | answered | Worker Messages local; ressources UI/permissions |
| Delta des appels texte inspectés | answered | Header timezone ajouté au chemin sendPrompt |
| Delta interbot, mémoire, compaction | partial | Schémas et fragments comparés; moteurs absents de ces chemins |
| Code iOS natif / APNs / fond | unresolved | Aucun artefact mobile acquis |
| Moteur serveur, scheduler et assemblage contexte | unresolved | Aucun corps d’implémentation établi dans cette passe bornée |

Le [script de collecte et comparaison](../tools/release-0712-discover.py) fournit `acquire`, `copy-package`, `extract-asar`, `compare`, `format-selected` et `verify`. `acquire`/`copy-package` réutilisent seulement des fichiers dont les hashes correspondent; l’extraction exige une destination neuve. Il parse l’ASAR en Python, copie ses bytes, contrôle l’intégrité et n’évalue aucun JavaScript cible. Les neuf copies de lecture versionnées sont formatées par Prettier 3.6.2, pas exécutées.

La commande suivante vérifie la preuve **hors ligne**, sans acquisition, montage, reformatage ou évaluation de code cible :

```sh
python3 tools/release-0712-discover.py verify
```

Elle recalcule les identités du DMG et de l’ASAR, les 668 membres, les six contradictions, les hashes des sources/extraits et les six comparaisons conservées, puis écrit [verification.json](../evidence/releases/0.71.2/verification.json).

Les alias `M/P/C/L/A` désignent respectivement main-app, proto, coordinator, daemon et eager-app; `K71` main-core, `W71` messages-worker, `R71` auto-review-approval et `U71` provider-computer-use. Le suffixe 70 vise `artifacts/formatted`, le suffixe 71 `artifacts/releases/0.71.2/formatted`. Tous les chemins originaux exacts, lignes LF, anchors et SHA-256 figurent dans le ledger. Les 668 hashes de membres extraits sont recoupés avec le manifeste REA; les six contradictions natives restent explicites. Aucune exécution, installation, lecture de compte ou requête RPC n’a lieu pendant cette comparaison.
