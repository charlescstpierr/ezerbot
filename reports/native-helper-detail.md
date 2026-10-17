# Le helper natif du Mac

La [seconde passe sur le socket](native-peer-deep.md) suit désormais huit corps de fonctions x86_64 au désassemblage. Elle établit l'admission du peer, la comparaison d'utilisateur et les appels de contrôle de signature. Les conclusions ci-dessous gardent la portée de la première inspection de métadonnées.

Le client Grok Bot possède un programme séparé nommé `CUGrokBotService`. Cette passe inspecte enfin son format, son Info.plist, ses imports, ses exports Swift et des chaînes de ses octets. Elle ne décompile pas les corps de fonctions et ne lance pas le helper.

La copie analysée est dans `inputs/Grok Bot.app/Contents/Helpers/Grok Bot Computer Use.app`. Son exécutable garde l'empreinte SHA-256 `94a4a3d6965dd9f759c2fb811450fbe2a2194d14f538a351b945d2142d628b31`. Ce helper est hors ASAR. Les six contradictions d'intégrité d'autres modules restent inchangées.

## Preuves nouvelles

| Inspection REA | Identifiant de preuve | Résultat |
|---|---|---|
| Mach-O | `ev_ff21f12aab0af609bc80c2db09125c9d84f17ffbb60a75d8dfa4310101126050` | Deux slices x86_64 et arm64; inventaires de métadonnées et symboles |
| Info.plist | `ev_b788f933954a888b13cbb89ea3f8f454040af1df6c7593753c65c1e7b6c21f9d` | Déclarations de permissions, identifiant, macOS minimal et mode d'app |
| Démanglage Swift | `ev_931391772961b63e7a84240966e4279d714dfeb832c677607b66b0ad99f84e65` | Neuf signatures lisibles issues des exports observés |

Les records complets sont [native-helper-macho.json](../evidence/native-helper-macho.json), [native-helper-plist.json](../evidence/native-helper-plist.json) et [native-helper-symbols.json](../evidence/native-helper-symbols.json). L'inventaire Mach-O normalise 1 001 entrées d'imports, 367 entrées d'exports, cinq segments et 38 entrées de dépendances. Ces nombres comptent les entrées normalisées, pas des fonctions uniques du produit. L'inspection note que des symboles masqués ou supprimés peuvent manquer.

## Pourquoi le helper peut utiliser les services du Mac

Le plist déclare l'accessibilité, la capture d'écran, les Apple Events et les Contacts. `LSUIElement` vaut `true`, ce qui indique une app auxiliaire. Le minimum macOS déclaré est 14.0. Ces descriptions établissent l'intention du programme; elles ne prouvent aucune permission déjà accordée sur le Mac de l'utilisateur.

Le wrapper JavaScript actif pour Messages trouve le bundle prod ou lab dans `Contents/Helpers`, ou une variante dev. Il peut copier une version à installer par comparaison des fichiers et remplacement atomique, puis l'ouvrir par `/usr/bin/open -g`. Il lit `service.json` et envoie du JSON terminé par une nouvelle ligne au socket `rpcSocketPath`. Sources : [sélection du helper](../artifacts/formatted/dist/local-exec-daemon/main.cjs:83896), [installation](../artifacts/formatted/dist/local-exec-daemon/main.cjs:83966), [connexion et lancement](../artifacts/formatted/dist/local-exec-daemon/main.cjs:84065).

Les méthodes de ce chemin sont `messages/snapshot`, `messages/copy-attachment`, `messages/send`, `messages/check-permissions`, `messages/find-contacts` et `messages/resolve-handles`. Le wrapper vérifie que le helper renvoie une version et les verbes Messages attendus. Source : [méthodes et validation](../artifacts/formatted/dist/local-exec-daemon/main.cjs:84093).

Les exports démanglés apportent une preuve native supplémentaire. On y trouve `MessagesSnapshot.snapshot`, `MessagesSnapshot.copyAttachment`, `MessagesContacts.find`, `MessagesSend.sendScript` et `MessagesPermissions.check`. Le handler Messages reçoit `peerSigningIdentifier` optionnel. Cela corrobore le contrat du wrapper. Cela ne donne ni le texte du script d'envoi ni la règle exacte qui accepte un peer.

## Les outils de capture et de clic ont des indices natifs

La recherche d'octets conserve des chaînes `computer_use_screenshot`, `computer_use_click`, `computer_use_typing`, `computer_use_press_key`, `computer_use_apps_list`, `computer_use_select_app` et `computer_use_app_state`. On trouve aussi des textes sur les identifiants d'éléments et les snapshots périmés. Ces chaînes sont des indices de capacités dans le binaire; aucune exécution de ces outils n'est observée.

Exemples d'offsets dans le fichier universel : `computer_use_screenshot` à 1 391 184, `computer_use_app_state` à 1 391 632, `permissions/status` à 1 394 336 et `peer_not_permitted` à 1 460 848. Les deux architectures peuvent répéter une chaîne. [uncovered-search.json](../evidence/uncovered-search.json) conserve chaque occurrence; [inspect-uncovered.py](../tools/inspect-uncovered.py) les relit dans les octets.

Le daemon compile également une classe `zs` de RPC pour captures, clics, scroll, mouvements, drag, saisie et clavier. Elle lit un fichier d'état, ouvre un socket local et distingue l'échec avant livraison de l'échec après livraison. Elle limite une réponse à 32 MiB et compare l'ID attendu. Ses paramètres par défaut désignent `Cursor Computer Use` et `CUCursorService`, pas directement `CUGrokBotService`. Source : [adaptateur générique](../artifacts/formatted/dist/local-exec-daemon/main.cjs:120270), [méthodes](../artifacts/formatted/dist/local-exec-daemon/main.cjs:120405), [transport](../artifacts/formatted/dist/local-exec-daemon/main.cjs:120614).

Le constructeur actif `oYe` n'enregistre pas `computerUseExecutor`. L'accès graphique complet du Mac par le daemon Grok n'est donc pas démontré par cette classe générique. La voie Messages, elle, possède un dispatch actif dans `P4`. Sources : [enregistrements actifs](../artifacts/formatted/dist/local-exec-daemon/main.cjs:128042) et [dispatch](../artifacts/formatted/dist/local-exec-daemon/main.cjs:128147).

## Les contrôles de peer restent à décompiler

Le binaire importe `getpeereid`, `proc_pidpath`, des fonctions Security et des fonctions de sockets. Ses chaînes mentionnent une politique de peer et des erreurs de permissions. Les signatures Swift indiquent un passage d'identité de signature jusqu'au handler Messages. L'inférence raisonnable est qu'il existe une voie de contrôle du processus appelant. La règle exacte et sa couverture restent inconnues tant que les corps et leurs appels ne sont pas inspectés.

La session REA ne dispose d'aucun moteur de décompilation natif configuré : Hopper est absent, Ghidra n'est pas configuré et IDA n'est pas configuré. Les outils Apple d'inspection restent disponibles et ont produit les trois preuves ci-dessus. Aucun moteur n'a été installé pour cette passe.

| Question | Statut |
|---|---|
| Quel helper est livré et quelles permissions déclare-t-il ? | Répondue statiquement |
| Comment le wrapper Messages le trouve et lui parle ? | Répondue statiquement |
| Quels symboles natifs corroborent les opérations Messages ? | Répondue statiquement pour neuf signatures sélectionnées |
| Quels indices existent pour capture, clics et état d'app ? | Partiellement répondue par chaînes et adaptateur compilé |
| Quelle identité peut appeler le socket et avec quels droits ? | Non résolue au niveau des corps natifs |
| Le daemon Grok active-t-il un contrôle graphique du Mac ? | Aucun enregistrement actif observé dans le constructeur inspecté |

La session native a servi uniquement à des inspections inertes. Les permissions déclarées et les capacités compilées ne sont pas des actions réalisées sur l'ordinateur.
