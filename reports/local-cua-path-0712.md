# Contrôle graphique local dans Grok Bot 0.71.2

Le paquet 0.71.2 contient un chemin JavaScript effectivement composé pour faire agir le Mac. Le daemon annonce des outils graphiques au service, reçoit une demande `localTool`, contrôle l’app ciblée, puis lance une façade MCP native par conversation. Cette façade rejoint ensuite le companion par son RPC interne. La présence d’un mode MCP dans le binaire 0.70.2 ne suffisait pas à établir cette composition.

```text
Service distant
  → watch / poll authentifiés du provider Mac
  → décodeur localTool → exécuteur B4
  → broker partagé, gate + consentement d’app, file par conversation
  → enfant CUGrokBotService --mcp-stdio --conversation-id ...
  → CUServiceConnector / CULocalRPCClient → companion natif
  → session de contrôle et actions natives autorisées
```

Ce rapport analyse les sources distribuées, sans lancer la version 0.71.2, se connecter à un compte ou effectuer d’action sur le bureau. Il ne fournit pas la boucle serveur qui choisit les outils. Le mot « mobile » dans `mobileLocalTools` est le nom d’un champ de protocole employé ici par le daemon Mac.

Les faits suivants sont liés aux extraits LF de copies formatées séparément. Le [ledger](../evidence/local-cua-path-0712.json) lie aussi les sources originales à leurs empreintes dans le manifeste du nouvel ASAR. Les résultats .71 restent distincts de ceux de l’original .70.

## LCP-01

Le daemon compose réellement un broker sN partagé et le fournit aux exécuteurs B4 des providers Gj de comptes. Ce chemin n’est pas déduit de la seule présence de symboles natifs.

Composition statique .71; activation effective sur un compte et comportement après connexion non observés. Ne rétroattribue pas ce montage à .70.

Preuves : [A1, local-exec-daemon:157670](../artifacts/releases/0.71.2/formatted/dist/local-exec-daemon/main.cjs:157670), [A2, local-exec-daemon:157730](../artifacts/releases/0.71.2/formatted/dist/local-exec-daemon/main.cjs:157730), [A3, local-exec-daemon:128135](../artifacts/releases/0.71.2/formatted/dist/local-exec-daemon/main.cjs:128135), [A4, local-exec-daemon:157823](../artifacts/releases/0.71.2/formatted/dist/local-exec-daemon/main.cjs:157823).

## LCP-02

Le main publie la valeur du flag sand_enable_local_cua dans un marqueur local. Le daemon n’annonce le helper que si sa lecture vaut exactement true. La valeur compilée par défaut est false.

Le défaut compilé ne donne pas la configuration distante du compte. Lire false empêche une nouvelle admission par ce broker; aucun arrêt immédiat des appels admis n’est montré par ce seul test.

Preuves : [A5, electron-main:56364](../artifacts/releases/0.71.2/formatted/dist/electron-main/main-app.cjs:56364), [A6, electron-main:103187](../artifacts/releases/0.71.2/formatted/dist/electron-main/main-app.cjs:103187), [A7, electron-main:32569](../artifacts/releases/0.71.2/formatted/dist/electron-main/main-core.cjs:32569), [A8, electron-main:32785](../artifacts/releases/0.71.2/formatted/dist/electron-main/main-core.cjs:32785), [A9, local-exec-daemon:78560](../artifacts/releases/0.71.2/formatted/dist/local-exec-daemon/main.cjs:78560), [A10, local-exec-daemon:79524](../artifacts/releases/0.71.2/formatted/dist/local-exec-daemon/main.cjs:79524), [A11, local-exec-daemon:79601](../artifacts/releases/0.71.2/formatted/dist/local-exec-daemon/main.cjs:79601), [A12, local-exec-daemon:128529](../artifacts/releases/0.71.2/formatted/dist/local-exec-daemon/main.cjs:128529), [A13, local-exec-daemon:128590](../artifacts/releases/0.71.2/formatted/dist/local-exec-daemon/main.cjs:128590).

## LCP-03

Le broker annonce 14 outils graphiques, dont capture, clic, frappe, applications et actions d’accessibilité. Le hello du provider Mac les publie sous mobileLocalTools et annonce le consentement par app.

Le nom du champ partagé mobileLocalTools ne prouve aucun exécuteur iPhone. Cette composition est celle du daemon Mac.

Preuves : [A14, local-exec-daemon:82831](../artifacts/releases/0.71.2/formatted/dist/local-exec-daemon/main.cjs:82831), [A15, local-exec-daemon:128135](../artifacts/releases/0.71.2/formatted/dist/local-exec-daemon/main.cjs:128135), [A16, local-exec-daemon:156683](../artifacts/releases/0.71.2/formatted/dist/local-exec-daemon/main.cjs:156683).

## LCP-04

Le provider surveille puis récupère les demandes du service avec machineId et credential. Le décodeur localTool transporte toolId, inputJson, conversationId et approvalId vers B4, puis les résultats repartent sous les frames locales dédiées.

Ce sont des corps client et leur branchement. Ils ne donnent ni le choix d’outil par le modèle, ni la décision serveur, ni une trace réseau authentifiée.

Preuves : [A17, local-exec-daemon:156895](../artifacts/releases/0.71.2/formatted/dist/local-exec-daemon/main.cjs:156895), [A18, local-exec-daemon:156985](../artifacts/releases/0.71.2/formatted/dist/local-exec-daemon/main.cjs:156985), [A19, local-exec-daemon:157080](../artifacts/releases/0.71.2/formatted/dist/local-exec-daemon/main.cjs:157080), [A20, local-exec-daemon:157272](../artifacts/releases/0.71.2/formatted/dist/local-exec-daemon/main.cjs:157272), [A21, local-exec-daemon:157126](../artifacts/releases/0.71.2/formatted/dist/local-exec-daemon/main.cjs:157126), [A22, local-exec-daemon:157382](../artifacts/releases/0.71.2/formatted/dist/local-exec-daemon/main.cjs:157382).

## LCP-05

B4 émet accepted avant l’appel du broker, puis permission-pending, une nouvelle acceptation après permission ou pendant la file, et enfin le résultat. Sa décision use-app force authorization:none et décrit le bundle destinataire.

Accepted accuse réception côté client, pas la réussite de l’action. La politique détaillée et les scopes de grants sont traités dans le rapport de consentement séparé.

Preuves : [A23, local-exec-daemon:128265](../artifacts/releases/0.71.2/formatted/dist/local-exec-daemon/main.cjs:128265).

## LCP-06

Les appels sont mis en file par conversationId. Le broker réutilise une session enfant par conversation et un compteur de leases maintient son processus pendant les appels. Une libération programme son arrêt après dix minutes d’inactivité.

La clé de file n’est pas globale au bureau ni isolée par compte dans ce broker partagé. Le compteur JS de leases ne constitue pas le permis natif de contrôle. L’admission concurrente du companion doit être examinée séparément.

Preuves : [A24, local-exec-daemon:128500](../artifacts/releases/0.71.2/formatted/dist/local-exec-daemon/main.cjs:128500), [A25, local-exec-daemon:128532](../artifacts/releases/0.71.2/formatted/dist/local-exec-daemon/main.cjs:128532), [A26, local-exec-daemon:128568](../artifacts/releases/0.71.2/formatted/dist/local-exec-daemon/main.cjs:128568), [A27, local-exec-daemon:128593](../artifacts/releases/0.71.2/formatted/dist/local-exec-daemon/main.cjs:128593), [A28, local-exec-daemon:128806](../artifacts/releases/0.71.2/formatted/dist/local-exec-daemon/main.cjs:128806).

## LCP-07

Le broker prépare le helper embarqué, lance son exécutable avec --mcp-stdio et --conversation-id, puis utilise JSON-RPC ligne par ligne sur stdin/stdout. Il initialise MCP avec la version 2025-06-18 avant notifications/initialized.

Le stdio décrit seulement la frontière daemon→façade native. Le binaire utilise ensuite CUServiceConnector et CULocalRPCClient vers le companion; le socket interne ne disparaît pas. Le mode MCP existe déjà parmi les symboles du helper .70.

Preuves : [A29, local-exec-daemon:128607](../artifacts/releases/0.71.2/formatted/dist/local-exec-daemon/main.cjs:128607), [A30, local-exec-daemon:128505](../artifacts/releases/0.71.2/formatted/dist/local-exec-daemon/main.cjs:128505), [A31, local-exec-daemon:128858](../artifacts/releases/0.71.2/formatted/dist/local-exec-daemon/main.cjs:128858), [A32, local-exec-daemon:128917](../artifacts/releases/0.71.2/formatted/dist/local-exec-daemon/main.cjs:128917).

## LCP-08

Le consentement d’app précède le premier tools/call. Si le résultat signale permission_required, le broker attend les permissions macOS par sondage d’une seconde, pendant cinq minutes au maximum, puis réessaie une fois après vérification de la gate et du signal d’annulation.

Le retry interne ne repasse pas par qAe/decideAppUse. Une révocation de grant ultérieure ne signifie donc pas une interruption rétroactive de cet appel déjà admis. Aucun prompt macOS n’a été déclenché dans cette analyse.

Preuves : [A33, local-exec-daemon:128568](../artifacts/releases/0.71.2/formatted/dist/local-exec-daemon/main.cjs:128568), [A34, local-exec-daemon:128500](../artifacts/releases/0.71.2/formatted/dist/local-exec-daemon/main.cjs:128500), [A35, local-exec-daemon:128620](../artifacts/releases/0.71.2/formatted/dist/local-exec-daemon/main.cjs:128620), [A36, local-exec-daemon:128728](../artifacts/releases/0.71.2/formatted/dist/local-exec-daemon/main.cjs:128728).

## LCP-09

Chaque requête MCP porte un délai de deux minutes. Son expiration règle la promesse puis arrête l’enfant par SIGTERM et termine les autres requêtes en attente. La file émet un keepalive chaque minute lorsqu’elle attend.

Les délais ne sont pas une durée maximale garantie de l’effet OS. L’attente des permissions est une boucle distincte de plusieurs requêtes MCP. Aucun scénario de latence ou de crash n’est exécuté.

Preuves : [A37, local-exec-daemon:128500](../artifacts/releases/0.71.2/formatted/dist/local-exec-daemon/main.cjs:128500), [A38, local-exec-daemon:128514](../artifacts/releases/0.71.2/formatted/dist/local-exec-daemon/main.cjs:128514), [A39, local-exec-daemon:128545](../artifacts/releases/0.71.2/formatted/dist/local-exec-daemon/main.cjs:128545), [A40, local-exec-daemon:128830](../artifacts/releases/0.71.2/formatted/dist/local-exec-daemon/main.cjs:128830), [A41, local-exec-daemon:128923](../artifacts/releases/0.71.2/formatted/dist/local-exec-daemon/main.cjs:128923).

## LCP-10

L’annulation B4 déclenche AbortController. iN annule la requête locale; la session règle sa promesse puis envoie notifications/cancelled. Une annulation simple n’appelle pas directement kill. Le texte client reconnaît que l’action a pu avoir lieu.

Notification d’annulation, arrêt de processus, relâchement des entrées et annulation d’un effet déjà livré sont des opérations différentes. Ce chemin ne promet pas de rollback ni une action unique après redémarrage.

Preuves : [A42, local-exec-daemon:128265](../artifacts/releases/0.71.2/formatted/dist/local-exec-daemon/main.cjs:128265), [A43, local-exec-daemon:128508](../artifacts/releases/0.71.2/formatted/dist/local-exec-daemon/main.cjs:128508), [A44, local-exec-daemon:128743](../artifacts/releases/0.71.2/formatted/dist/local-exec-daemon/main.cjs:128743), [A45, local-exec-daemon:128847](../artifacts/releases/0.71.2/formatted/dist/local-exec-daemon/main.cjs:128847).

## LCP-11

Le statut externe done sérialise le résultat MCP. Hormis le cas spécial permission_required, runUntilGranted peut produire done avec un objet interne isError:true. Done ne signifie donc pas réussite de l’action ni fin du travail du bot.

Le consommateur serveur peut interpréter cette erreur interne, mais son handler n’est pas présent. La fin de tâche choisie par le modèle reste inconnue.

Preuves : [A46, local-exec-daemon:128620](../artifacts/releases/0.71.2/formatted/dist/local-exec-daemon/main.cjs:128620), [A47, local-exec-daemon:128709](../artifacts/releases/0.71.2/formatted/dist/local-exec-daemon/main.cjs:128709), [A48, local-exec-daemon:128728](../artifacts/releases/0.71.2/formatted/dist/local-exec-daemon/main.cjs:128728).

## Portée de la comparaison et suite

Le vérificateur rejoue quatre recherches de chaînes exactes dans les fichiers JavaScript de chacun des deux manifestes. `--mcp-stdio`, `local-computer-use-gate.json`, `sand_enable_local_cua` et la chaîne JSON `"local-tool"` sont absents des 600 fichiers .70 recherchés et présents parmi les 596 fichiers .71. Les offsets sont des positions d’octets UTF-8 à partir de zéro. Une absence de chaîne ne prouve pas une absence sémantique; des noms, routes ou encodages différents restent possibles. Le [premier rapport .71](release-0712-discovery.md) portait sur d’autres chemins et ne prétendait pas comparer tout le paquet.

Le [rapport de consentement .71](local-cua-consent-0712.md) précise les grants par compte, action et bundle ainsi que la portée du retrait d’une app. Le [contrôle arm64 de l’original .70](native-arm64-residual.md) confirme seulement neuf corps et six propriétés sélectionnées; il ne constitue pas une équivalence complète de toutes les architectures ou versions.

Le [rapport natif MCP .71](native-0712-mcp.md) établit les contrôles du père, le mapping des noms d’outils, le raccord au socket, les identités distinctes et la libération coopérative après EOF/SIGTERM. Il ne prouve ni unicité du companion en exécution, ni livraison de tous les événements graphiques.

Reproduire les vérifications avec `python3 tools/build-local-cua-path-0712.py --verify`. Sans option, ce script régénère seulement le rapport et son ledger à partir des artefacts sauvegardés. Il ne lance aucun code de la cible et n’utilise pas le réseau. Les questions serveur, iOS, APNs et de garanties de reprise restent ouvertes.
