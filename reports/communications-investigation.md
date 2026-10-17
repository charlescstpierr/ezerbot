# Comment les bots se parlent et comment l’iPhone rejoint le Mac

Cette passe conserve 39 constats dans quatre rapports et 159 extraits hashés. Les messages interbot, leur présentation et les contrats de tours de groupe sont mieux établis. Le volet iPhone retrace les interfaces communes et les consommateurs Mac; il ne contient pas le binaire natif iOS ni les services privés.

## Entre deux bots

Le protocole prévoit du texte avec expéditeur, destinataire, messageId, date et replyToId éventuel. Le retour immédiat renseigne la livraison; le destinataire peut répondre par un autre message lié au premier. Dans le client, fromAgent/toAgent servent à regrouper les échanges et reconstruire une conversation avec le pair. Les cartes envoyées/reçues ont des consommateurs actifs.

Un @bot dans le composeur devient une mention structurée dans richText et un @nom dans le texte. Ce chemin ne déclenche pas lui-même un appel direct SendGrokBotAgentMessage. Le serveur interprète la demande et les mentions; son choix de destinataire n’est pas récupéré.

Les groupes possèdent un autre protocole : un membre reçoit un tour, une liste de pairs et de nouveaux messages, une deadline et un nonce. Le retour peut publier, passer, expirer ou échouer. Une délégation à un sous-agent possède encore d’autres contrats. La présence de ces trois circuits n’établit pas un unique moteur commun.

Détails : [consommateurs et conversations](interbot-client-paths.md), [enveloppes et tours](interbot-mobile-protocol.md). Les [paquets illustratifs](../evidence/interbot-message-contracts.json) sont inventés à partir des champs récupérés; aucune capture réelle n’est revendiquée.

## Entre ton iPhone et ton Mac

Le Mac lit une liste d’appareils du compte qui reconnaît ios, avec identité et état connecté. Les messages transportent bot, session et identité de message; les contrats distinguent desktop/mobile et plusieurs origines mobiles. Le Mac reçoit ensuite des lignes et états depuis un flux serveur authentifié, avec curseur et génération pour reprendre après déconnexion.

Cela soutient une architecture de continuité par les services du compte. Une connexion directe iPhone↔Mac n’est pas établie dans les chemins inspectés. Le code de login iOS, de synchronisation en arrière-plan et d’inscription push n’est pas livré dans cet artefact Mac. Les hits Bluetooth concernent des routes audio, et pairing vise principalement 1Password.

La carte mobile de permission sur Mac affiche un état et une note pour l’autre appareil. Le Mac ne possède pas le bouton ni le handler d’autorisation iOS dans ce composant. Une demande d’outil sur le Mac passe par une autre route backend→machine ciblée→exécuteur local, avec ses permissions. Synchroniser le texte ne suffit pas à autoriser cet outil.

Détails : [connexion iPhone↔Mac](interbot-mobile-protocol.md).

## Ce que la recherche supplémentaire a trouvé

Le partage de mémoire possède un consommateur lazy réel : sélection par factIds, lots de 100 et conservation locale des lots déjà réussis après un échec ultérieur. La préparation d’image envoie le brouillon et jusqu’à cinq textes humains filtrés; elle exclut les messages entre bots dans cette projection. Cela ne reconstitue pas le prompt texte général.

Une carte de formulaire déjà réglée peut retenter la remise du réveil du bot via submitUserForm avec values vide. Le retour accepted ne prouve pas encore la reprise du moteur. L’import de fichiers texte produit et écrit de vraies skills; ce circuit est distinct de l’apprentissage d’une démonstration. Le repli maxRunning=32 reste une configuration sans consommateur Grok retrouvé.

Détails : [contexte et mémoire](server-context-residual.md), [réveil et skills](server-orchestration-residual.md).

## Les frontières encore ouvertes

| Question | Statut | Preuve encore nécessaire |
|---|---|---|
| Messages directs et réponses entre bots | partially_answered | Initiateur/service distant, ACL, ordre et déduplication réelle. |
| Tours de groupe et sous-agents | partially_answered | Ordonnanceur et contexte délégué effectif; ces mécanismes restent distincts. |
| Contexte texte et compaction | partially_answered | Constructeur de prompt et consommateur des bornes de résumé dans le moteur. |
| Mémoire durable et oubli | partially_answered | Correction, suppression et injection; partage client mieux retracé. |
| Démonstration vers skill | unresolved | Capture mobile/cloud, job d’extraction, recording→skill et validation. |
| Budgets et fin métier | unresolved | Compteurs/politique effectifs et prédicat terminal du moteur. |
| Connexion iPhone↔Mac | partially_answered | App native iOS, auth native, inscription APNs et transport de fond. |

Ces frontières ne sont pas résolues par un nom de SDK ou une valeur de repli. Les corps du moteur box/Temporal et l’app iOS, ou des observations passives ciblées, sont nécessaires pour les trancher. La passe complète les preuves client et ne prétend pas avoir récupéré les serveurs.

## Rapports et vérification

| Rapport | Constats | Extraits | Ledger |
|---|---:|---:|---|
| [Les conversations entre bots](../reports/interbot-client-paths.md) | 13 | 55 | [JSON](../evidence/interbot-client-paths.json) |
| [Les paquets et l’iPhone](../reports/interbot-mobile-protocol.md) | 13 | 36 | [JSON](../evidence/interbot-mobile-protocol.json) |
| [Contexte et mémoire](../reports/server-context-residual.md) | 7 | 35 | [JSON](../evidence/server-context-residual.json) |
| [Réveil, skills et frontières serveur](../reports/server-orchestration-residual.md) | 6 | 33 | [JSON](../evidence/server-orchestration-residual.json) |

La [vue navigable](../.lavish/communications-grokbot.html) complète la [synthèse précédente des neuf sujets](goal-nine-topics.md). Le [vérificateur](../tools/verify-communications.py) contrôle les ledgers, le corpus des recherches, les extraits et les contrats projetés. Aucune implémentation Rust n’a été modifiée par cette enquête.
