# Conversations avec plusieurs bots et mentions

Statut : proposition à relire, aucune implémentation de ce lot commencée.

## Résultat voulu

L’utilisateur veut pouvoir accomplir dans son app le travail que permet Grok Bot. Le premier lot rend possible une conversation avec plusieurs de ses bots : ajouter un bot à un fil existant, choisir un destinataire avec `@`, faire répondre plusieurs bots dans ce fil et voir leurs échanges. L’enquête de l’original sert de référence de comportement. Nous construisons notre propre moteur; les internes serveur de Grok Bot ne sont pas disponibles.

La parité complète comprend aussi les outils, la mémoire, les routines, les connecteurs, l’ordinateur cloud et l’iPhone. Ce lot est une dépendance de cette suite et ne sera pas présenté comme une parité complète.

## État vérifié dans notre app

- `crates/wrapper/src/lib.rs` stocke un seul `bot_id` dans `wrapper_conversations`. Le type `Conversation`, la création et `start_generation` utilisent cet unique bot.
- Les bots possèdent déjà une identité, une description et des instructions privées. `compose_instructions` les assemble pour la génération.
- L’historique est durable en SQLite. Le journal d’envoi garde une identité logique stable lors d’une reprise.
- `provider.rs` possède un chemin ChatGPT authentifié et des réponses en streaming. Sans connexion, le wrapper choisit le moteur simulé. Le statut du compte vivant n’a pas été consulté pendant cet audit.
- Le transport modèle actuel transmet des paires rôle/texte et traite les deltas texte. Le code examiné n’expose pas encore de cycle d’appels d’outils pour communiquer entre bots.
- Le composeur de l’interface est dans `app/web/src/Composer.jsx` et l’état de l’application dans `app/ui/app.js`. L’accueil et la création choisissent actuellement un seul bot.

Ces fichiers évoluent en parallèle. L’implémentation devra relire leur version courante et conserver les changements existants.

## Choix d’architecture

| Approche | Conséquence |
|---|---|
| Faire évoluer le moteur Rust et SQLite existant | Réutilise la persistance, les profils, le streaming et le wrapper Mac. Permet de construire et vérifier les groupes avant les services distants. Recommandé. |
| Commencer par un nouveau backend hébergé commun au Mac et à l’iPhone | Prépare immédiatement le multiappareil, mais impose de choisir et d’exploiter auth, stockage, workers et notifications avant ce premier parcours local. |
| Ajouter une orchestration externe et garder l’app comme client | Peut accélérer certains outils; introduit un second propriétaire de l’état et une migration des garanties de reprise. Pas retenu pour ce lot. |

Le premier choix permet ensuite d’extraire les frontières réseau nécessaires au téléphone sans remplacer le domaine des conversations.

## Parcours proposé

1. Depuis une conversation existante, « Ajouter un bot » ouvre le sélecteur de bots actifs. L’ajout conserve l’identifiant du fil et ses messages. Avec plusieurs membres, le fil devient un groupe.
2. La création d’une conversation permet aussi de choisir plusieurs bots d’emblée, jusqu’à six dans ce premier lot de bots personnels.
3. Dans le composeur, `@` propose les membres du fil. Chaque mention conserve le `BotId` et son libellé; renommer un bot ne change pas sa destination.
4. Mentionner un bot absent du groupe propose explicitement de l’ajouter. Une mention seule ne modifie pas silencieusement les membres.
5. Un message adressé à `@A` ouvre un tour pour A. `@A @B` ouvre un tour pour chaque destinataire distinct. `@everyone` cible les membres actifs au moment de l’envoi.
6. Sans mention, le bot coordinateur du groupe reçoit le tour initial. Le premier membre choisi est coordinateur par défaut; il est modifiable dans les détails. S’il est retiré, le membre actif le plus ancien le remplace. Il peut répondre ou déléguer à un autre membre. Ce choix est une règle de notre moteur, pas une affirmation sur le scheduler privé de Grok Bot.
7. Chaque réponse affiche le bot auteur, sa couleur et son état. Les messages d’un bot ne sont pas aplatis en une réponse sans identité.
8. Un bot peut demander un travail à un autre membre avec l’outil interne `message_bot`. L’échange et la réponse restent dans le fil partagé; la livraison et la réponse sont deux étapes différentes.
9. « Arrêter » annule les tours actifs du fil. Les messages déjà produits restent visibles. Les appels en cours passent par leur propre mécanisme d’annulation.

Pour ce premier lot, ajouter l’un de ses propres bots lui donne accès à l’historique visible de ce fil, dans les limites du budget de contexte. Les autres conversations privées et instructions privées des pairs ne rejoignent pas cet historique. Une copie, un résumé et une projection pour le modèle sont identifiés comme tels; ils ne remplacent pas silencieusement les messages sauvegardés.

## Concepts et propriétaires

| Concept | Propriétaire et règle |
|---|---|
| Conversation | Identité stable et type direct/groupe. Une conversation directe a un seul membre actif; un groupe peut en conserver un après retrait. |
| Membership | Persistance Rust; ajout et retrait idempotents. Retrait d’un bot empêche ses nouveaux tours et annule ses tours encore en file. Le retrait du dernier membre actif est refusé. Les messages historiques conservent leur auteur. |
| Message | Auteur humain ou bot, identifiant logique, texte, mentions structurées et réponse citée optionnelle. Un nom affiché n’est jamais une clé d’identité. |
| Tour | Destinataire, cause, état, réponse et corrélation avec le message source. La file durable conserve les états queued/running/completed/failed/cancelled. |
| Chaîne de délégation | RootId commun, parent éventuel, budget restant et destinataires déjà sollicités. Une limite explicite empêche une conversation infinie entre bots. |
| Génération | Adaptateur modèle; reçoit l’identité du bot concerné et le contexte du fil, restitue texte, appel d’outil ou erreur. Aucun secret de connexion n’est ajouté à ce contexte. |

Le moteur possède les règles de destinataire et d’admission. L’interface envoie une commande et affiche l’état; elle ne lance pas elle-même une réponse pour chaque bot.

## Persistance et compatibilité

- Ajouter des relations de membres, des tours et des messages attribués, avec clés de compte et de conversation.
- Migrer chaque ancien `bot_id` vers un membre actif sans modifier le contenu ou les IDs des anciens messages. L’auteur des anciennes réponses est celui du bot déjà associé au fil.
- Les réponses multiples ont chacune leur propre ID. Une réponse n’est plus identifiée par le seul message humain.
- Garder des nonces stables sur l’envoi, l’ajout de membre et la demande de délégation. Une reprise ne crée pas un second tour logique pour la même cause et le même destinataire.
- Lorsqu’un processus s’arrête pendant une génération distante, conserver l’état incertain. Ne pas présenter une nouvelle génération du modèle comme une reprise exactement identique de la précédente.
- Les données du compte réel ne sont pas utilisées par les migrations ou tests automatisés. Les tests emploient des bases temporaires.

## Interfaces proposées

Le contrat exact sera arrêté dans le plan après relecture de cette proposition.

```text
create_conversation(id, initial_member_ids, title?)
add_member(conversation_id, bot_id, command_id)
remove_member(conversation_id, bot_id, command_id)
send_message(conversation_id, message_id, text, mentions, reply_to?)
request_bot_turn(conversation_id, bot_id, cause_id, root_id, parent_turn?)
cancel_conversation_turns(conversation_id)
```

L’outil modèle `message_bot` accepte uniquement un membre actif, du texte et une corrélation créée par le moteur. Il ne peut pas choisir arbitrairement un compte, lire un autre fil privé ou effacer une limite de budget. Les appels d’outils inconnus produisent un résultat d’erreur explicite.

## Contexte et limites

- Les instructions privées du bot actif restent dans ses instructions de génération.
- Le fil partagé fournit les textes et auteurs visibles, les mentions et la citation demandée. Il ne contient pas les instructions privées de chaque pair.
- Un message cité ou mentionné reste une entrée utilisateur ou de pair, sans autorité pour modifier les règles du moteur.
- Une chaîne issue d’un message humain dispose de douze tours au maximum et de trois tours par bot. Ces valeurs sont des choix initiaux de notre moteur, configurables ultérieurement. Une limite atteinte devient visible dans le fil; aucune boucle infinie ne reste silencieuse.
- La file du groupe exécute un tour à la fois. Les destinataires d’un même message suivent l’ordre des mentions, ou l’ordre stable des membres pour everyone. Un nouveau message humain entre avant les délégations encore en file. Un bot reçoit un instantané du fil au début de son tour, et ses instructions sont relues à ce moment.
- Le contexte respecte la limite du fournisseur lorsqu’elle est disponible et une limite locale de 128 Kio de texte UTF-8 du fil. Au-delà, le moteur affiche une erreur et demande un fil plus court. Un rejet de taille par le fournisseur reste une erreur explicite, sans suppression silencieuse des anciens messages. Une compaction automatique mesurée appartient au lot mémoire/contexte.
- Les réponses réseau retardées sont rattachées à leur tour original. Une annulation ou un retrait de membre ne permet pas de les réinterpréter comme une nouvelle demande.
- Les échanges privés directs bot→bot hors du groupe, les équipes et les ACL entre personnes suivent dans un lot distinct. Le premier outil délègue uniquement à un membre du fil.

## Vérification attendue

Le lot sera fini lorsque le parcours réel de l’app passe les scénarios suivants :

1. Ajouter B à une ancienne conversation avec A conserve l’historique et ouvre un groupe relisible après redémarrage.
2. Deux bots portant le même nom restent adressables par leur ID. Un renommage ne casse pas une mention sauvegardée.
3. `@A` déclenche A; `@A @B` et `@everyone` produisent une réponse distincte de chaque destinataire attendu, sans doublon en cas de retry.
4. Chaque génération reçoit les instructions de son auteur et le contexte partagé, sans celles des autres bots ni leurs conversations privées.
5. A demande un avis à B; B répond; A peut reprendre avec ce résultat, avec corrélation visible et budget consommé.
6. La panne du modèle, l’annulation, le retrait d’un membre et la limite de délégation produisent des états terminaux visibles. Le redémarrage ne lance pas aveuglément une nouvelle génération incertaine.
7. Le faux serveur modèle vérifie les requêtes, appels d’outils et résultats. Le navigateur vérifie le menu `@`, les membres et les auteurs. Un essai avec un vrai fournisseur est rapporté séparément lorsqu’une connexion de test est disponible.

## Références de l’enquête

- [Mentions et affichage interbot](../../../reports/interbot-client-paths.md)
- [Messages directs et contrats de tours](../../../reports/interbot-mobile-protocol.md)
- [État et présence des tours](../../../reports/delegation-lifecycle-deep.md)
- [Matrice historique des 45 familles](../../../reports/parity.md)

Les [membres et groupes](../../../reports/multibot-membership-paths.md) et les [tours côté hôte](../../../reports/multibot-turn-paths.md) ont maintenant leurs lectures bornées. Dans l’original, la chaîne de mention ne convertit pas elle-même le chat; un nouveau groupe reçoit un nouvel ID et un brouillon, sans ancien historique dans le payload inspecté. La conversion conservant notre fil et le partage de son historique sont des choix propres à cette proposition. Le scheduler distant reste inconnu. L’utilisateur a demandé de poursuivre avec REA, sans approuver ce design.
