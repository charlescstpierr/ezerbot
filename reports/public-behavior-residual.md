# Inconnues serveur : ce que les sources publiques primaires ajoutent

Les sources officielles expliquent plusieurs comportements absents du package : séparation du contexte de conversation et de l’état durable, réveil d’un bot destinataire, scopes de mémoire et production d’un brouillon après démonstration. Elles ne livrent pas l’implémentation du serveur.

Consultation : **17 octobre 2026 UTC** (17 octobre au Québec). Cette passe documentaire complète les rapports statiques existants, sans réexaminer leurs chemins. Les documentations décrivent le produit à leur date de consultation ; le forum et le guide d’ingénierie sont des déclarations datées. Aucun de ces textes n’atteste le comportement exact du client Mac **0.70.2**. Aucun compte, RPC privé ou lancement de la cible n’a été utilisé dans cette passe.

Le [ledger](../evidence/public-behavior-residual.json) conserve les URLs, dates, types de preuve, empreintes des réponses publiques lues et courtes ancres. Les corps complets ne sont pas conservés. Les métadonnées publiques du forum identifient l’auteur de PB01 comme administrateur, modérateur et membre `CursorStaff`.

## PB01 — Contexte et compaction : nouvelle conversation de groupe, état durable préservé

Le **23 septembre 2026**, Dean Rie explique qu’un nouveau groupe avec le même bot ne charge pas son ancien transcript individuel dans le contexte de travail. Profil, mémoire, routines, skills, connecteurs et fichiers durables restent disponibles. Il distingue ce contournement du résumé automatique à la limite de contexte et indique que compaction manuelle, nouvelle session du même bot avec archivage et compteur visible ne sont alors pas livrés. [Déclaration du staff, message 33](https://forum.cursor.com/t/grok-bot-prune-compact-an-agent-s-context-without-creating-a-new-bot/168333/33).

Limite : déclaration datée, sans seuil de tokens, algorithme du résumé, contenu du prompt ni garantie sur les versions ultérieures. Un contexte de travail neuf ne signifie pas l’effacement de la mémoire durable.

## PB02 — Mémoire et oubli : deux scopes et une commande conversationnelle

La documentation Team Bots, mise à jour le **15 octobre**, décrit des faits d’équipe lus dans les conversations des membres, enregistrés quand leur partage avec toute l’équipe est explicitement demandé. Les notes par personne restent privées et la suivent entre son chat et ses messages Slack individuels. Consulter, corriger ou retirer un souvenir d’équipe se demande au bot dans le chat. Les conversations des autres membres ne sont pas mises en commun. [Team Bots, Memory](https://docs.x.ai/grok-bot/team-bots#memory).

Limite : cela décrit l’interface et les scopes promis, sans prouver la transaction d’édition, l’invalidation des caches, le retrait des résumés déjà produits ou l’effacement des sauvegardes.

## PB03 — Moteur de tâches : orchestration distante et priorité utilisateur

Cursor décrit une requête passant par ses serveurs, le travail sur un ordinateur hébergé et les prompts envoyés aux fournisseurs de modèles. [Architecture officielle](https://cursor.com/docs/grok-bot/security#architecture). Un message direct de l’utilisateur peut réorienter le tour et prend priorité sur le travail d’arrière-plan ; arrêter ne défait pas les actions terminées. [Documentation du 13 octobre](https://docs.x.ai/grok-bot/chat-and-collaboration#redirect-work-in-progress).

Limite : aucune de ces pages ne spécifie la machine d’états, la persistance des étapes, l’ordre de la file, les reprises après panne ou une technologie de moteur. Les attribuer au serveur serait une extrapolation.

## PB04 — Échanges entre bots : message asynchrone et réveil

Le message d’un bot vers un autre est asynchrone : le destinataire se réveille, traite la demande et peut répondre plus tard. Le passage de relais apparaît dans la conversation. L’option administrateur **Messaging Between Bots**, activée par défaut selon la documentation, bloque les communications directes dans les deux sens lorsqu’elle est désactivée. [Let Bots hand work off](https://docs.x.ai/grok-bot/chat-and-collaboration#let-bots-hand-work-off).

Limite : le texte ne garantit ni livraison unique, ni délai, ni ordre, ni transport du transcript complet ou de la mémoire de l’émetteur. Le blocage est une politique documentée, pas un contrôle testé ici.

## PB05 — Délégation : les agents de code sont une autre destination

Un guide d’ingénierie officiel du **10 septembre** décrit des bots lançant des **Cursor cloud agents** avec un prompt détaillé et des skills, lisant leurs transcripts et preuves, puis envoyant des relances en file ou interrompant le travail. [Grok Bot for Engineering](https://x.ai/bot/guides/grok-bot-for-engineering). Pour les passages de relais vers un groupe de bots, la documentation actuelle précise que les messages sont limités au texte. [Groupes](https://docs.x.ai/grok-bot/chat-and-collaboration#start-a-group-chat).

Limite : le guide décrit un usage de l’équipe produit. Il ne publie pas les messages système de l’enfant, les champs de contexte hérités, les scopes de notes ni une règle universelle de création d’agents.

## PB06 — Scheduler : échéance, résultat et reprise sont distincts

La FAQ officielle indique qu’une routine suit le fuseau configuré, attend sa prochaine échéance après création et continue dans le cloud. Un webhook peut transmettre son JSON avec l’instruction ; une réponse HTTP 200 signifie acceptation et démarrage, sans attester l’achèvement. La page signale aussi une erreur connue de limite commune aux routines et précise que **cet échec particulier** n’est pas réessayé automatiquement. [Routines](https://cursor.com/help/grok-bot/routines).

Limite : page sans date de mise à jour visible consultée le 17 octobre. Ce défaut documenté n’établit pas la politique générale de retry. Ordonnancement concurrent, rattrapage d’échéances et garanties d’exécution restent indéterminés.

## PB07 — Démonstration vers skill : un brouillon à réviser

La documentation du **12 octobre** décrit Teach dans une conversation individuelle avec la vue ordinateur : annoncer le résultat, démontrer, arrêter, relire puis tester le skill. Elle annonce jusqu’à dix minutes d’interactions visibles capturées, sans son du microphone. Le résultat est un **brouillon**, à compléter avec décisions, erreurs et limites d’approbation ; la fonction peut être déployée progressivement. [Teach a workflow by demonstration](https://docs.x.ai/grok-bot/skills-routines-and-automations#teach-a-workflow-by-demonstration).

Limite : ni conversion serveur de la démonstration, ni prompt de génération, ni versionnement transactionnel des skills n’est divulgué. La durée annoncée ne prouve pas un arrêt local automatique dans 0.70.2.

## Checklist des inconnues

Les statuts portent sur la résolution de l’enquête du client 0.70.2. Une réponse documentaire au comportement utilisateur ne devient pas `answered-client-side`.

| Sujet | Statut | Apport public | Frontière restante |
|---|---|---|---|
| Contexte / compaction | partially_answered | Nouvelle conversation de groupe et état durable distingués par le staff | Assemblage du prompt, seuil et stratégie de résumé |
| Mémoire / oubli | partially_answered | Privé / équipe, correction et retrait par chat | Écriture effective, propagation et effacement complet |
| Moteur de tâches | unresolved | Orchestration distante et priorité utilisateur documentées | Machine d’états, durabilité, file et reprise |
| Échanges entre bots | partially_answered | Asynchronisme, réveil et option administrateur | Garanties de livraison et contexte transmis |
| Délégation | partially_answered | Agents de code pilotés par prompt, transcript et relance | Héritage exact et protocole serveur |
| Scheduler | partially_answered | Fuseau, prochaine échéance, webhook et échec particulier sans retry | Concurrence, rattrapage et politique générale |
| Démonstration → skill | partially_answered | Capture limitée et brouillon révisable | Conversion et versionnement serveur |

La documentation de compaction de l’API xAI décrit un autre produit ; aucune liaison établie ne permet d’en attribuer le mécanisme à Grok Bot. Les articles tiers sur une architecture interne ne sont pas retenus comme preuves primaires.
