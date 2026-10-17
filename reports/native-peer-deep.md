# Admission au socket du helper natif

Cette seconde passe suit huit fonctions x86_64 de `CUGrokBotService` dans leur désassemblage LLVM. Elle établit des branches de contrôle, au-delà des chaînes et des imports de la première inspection. Elle ne lance pas le helper et ne récupère aucun code source Swift original.

L'empreinte du fichier universel reste `94a4a3d6965dd9f759c2fb811450fbe2a2194d14f538a351b945d2142d628b31`. Les adresses ci-dessous sont des adresses virtuelles de sa slice x86_64, pas des offsets du fichier universel. [native-deep-disassembly.json](../evidence/native-deep-disassembly.json) conserve symboles, commandes, empreintes et limites. [inspect-native-peer.py](../tools/inspect-native-peer.py) reproduit ces lectures avec les outils Apple déjà présents.

## La connexion passe par une admission avant le handler

Le listener appelle `accept` à `0x10005621d`. Il configure le FD, puis construit `CULocalRPCPeerIdentity(fileDescriptor:)` à `0x10005630a`. Une identité absente conduit à la fermeture. Pour une identité présente, il appelle `CULocalRPCPeerPolicy.allows` à `0x100056365`.

Si ce prédicat renvoie faux, la branche à `0x10005636c` va à `0x100056492`, puis à la fermeture à `0x10005620e`. La préparation de la tâche du handler arrive après la branche d'acceptation. Il existe donc un contrôle d'admission dans ce chemin natif. Source : [accept loop](../evidence/native-deep-accept_loop.asm.txt:115).

Ce résultat reste statique. Il ne prouve pas que toutes les autres entrées du helper passent par ce listener ni que chaque outil possède la même autorisation.

## L'identité vient du socket, pas d'un champ JSON fourni par l'appelant

Le constructeur `CULocalRPCPeerIdentity(fileDescriptor:)` appelle `getpeereid` à `0x100059cde`. Il refuse de produire cette identité si l'appel échoue. Il appelle ensuite `getsockopt` et vérifie une taille de 32 octets pour la première structure. Il lit aussi une donnée de quatre octets pour le PID.

La voie retenue passe ensuite par `CULocalRPCCodeSigningIdentity(auditToken:)` à `0x100059dfe` et par la récupération du chemin du processus à `0x100059e24`. L'interprétation de la structure comme audit token est corroborée par le symbole de cette fonction. Source : [identité du socket](../evidence/native-deep-socket_peer.asm.txt:18).

La représentation Swift optimisée demande encore de la prudence. Les tags d'optionnels et les valeurs de retour ne sont pas des noms de variables récupérés.

## La politique vérifie d'abord le même utilisateur

`CULocalRPCPeerPolicy.allows` appelle `geteuid` à `0x10005895c` puis compare cette valeur au champ UID du peer. Si les valeurs diffèrent, la branche retourne faux avant le dispatch des autres variantes. Source : [politique de peer](../evidence/native-deep-peer_policy.asm.txt:31).

Le code possède ensuite plusieurs variantes : comparaison d'une identité de signature, appartenance à un ensemble, exigence de bundle, et variante limitée au même utilisateur. La création du service RPC construit son ensemble à partir de la configuration produit et injecte un tag de variante dans le listener. Le symbole public `localRPCTrustedTeamIdentifiers` donne le sens de cette configuration. La correspondance des branches aux champs de team et de signature est une interprétation du désassemblage, pas une exécution observée. Sources : [configuration du service](../evidence/native-deep-application_rpc_setup.asm.txt:136), [symboles de configuration](../evidence/native-helper-x64-symbol-table.txt:3158).

Les valeurs effectives de toutes les variantes prod, lab et dev ne sont pas établies dans cette passe. La présence d'une variante même-utilisateur ne prouve pas qu'elle est celle retenue pour le bundle prod.

## La signature est contrôlée et ses métadonnées sont lues

La voie `guestAttributes` appelle `SecCodeCopyGuestWithAttributes`, puis le constructeur d'identité de signature. Ce constructeur crée une exigence avec `SecRequirementCreateWithString`. Lorsque la création réussit et donne une exigence, il appelle `SecCodeCheckValidity` et garde le résultat de succès dans l'identité.

Il récupère ensuite le code statique et ses informations de signature. Les clés `kSecCodeInfoIdentifier` et `kSecCodeInfoTeamIdentifier` sont lues. Cette voie établit un contrôle réel dans le code natif, pas seulement l'import de l'API Security. Sources : [guest](../evidence/native-deep-signing_guest.asm.txt:15), [validité et information de signature](../evidence/native-deep-signing_identity.asm.txt:26).

La chaîne de l'exigence a été résolue dans les octets sauvegardés : `anchor apple generic`, à l'offset universel 1 393 904 et à l'adresse virtuelle x86_64 `0x1001504f0`. Le label de literal-pool affiché par LLVM est décalé de 32 octets pour cette représentation Swift. On ne reprend donc pas ce label comme si c'était le texte de l'exigence.

Ce contrôle n'autorise pas à conclure qu'un paquet signé par Apple possède tous les droits de Messages. Les comparaisons de politique et le handler restent des étapes distinctes.

## La variante bundle traite aussi le chemin réel

`CULocalRPCBundledPeerRequirement.allows` prépare un ensemble de chemins exécutables autorisés. Il standardise le chemin du peer, résout les symlinks, en extrait le chemin puis demande son appartenance à l'ensemble. Le résultat conditionne les comparaisons suivantes d'identité et de signature. Source : [exigence de bundle](../evidence/native-deep-bundled_policy.asm.txt:57).

Cette variante est bien appelée depuis une branche de la politique générique. Son utilisation effective pour toutes les connexions de Grok Bot n'est pas établie. Le chemin par défaut du service étudié doit être interprété avec la configuration produit.

## Ce que cette passe ferme et laisse ouvert

| Question | Résultat |
|---|---|
| Une connexion est-elle admise avant le dispatch ? | Oui, dans le listener x86_64 cité |
| L'identité du peer provient-elle seulement du JSON ? | Non; le constructeur lit les données du socket et du processus |
| Le même utilisateur macOS est-il vérifié ? | Oui, avant le choix de variante de la politique citée |
| Des API de validité de signature sont-elles effectivement appelées ? | Oui, dans la voie d'identité de signature citée |
| Existe-t-il une variante qui vérifie les chemins et symlinks ? | Oui; son application universelle n'est pas établie |
| Les droits de chaque outil et de chaque variante sont-ils reconstruits ? | Partiellement; handlers, paramètres effectifs et arm64 restent à approfondir |

Les corps inspectés sont des instructions machine. Les règles sémantiques qui dépassent les appels et branches cités restent des inférences. Les outils de décompilation profonde restent indisponibles; aucun prérequis n'a été installé pour lire ces instructions.
