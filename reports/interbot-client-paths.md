# Comment les échanges entre bots arrivent dans le client

Le client sait afficher « bot A écrit à bot B », regrouper ces échanges et ouvrir une vue du pair. Il conserve les IDs dans les mentions riches, lie les réponses à leur session et renvoie certains résultats vers la voix. Le moteur distant qui choisit le destinataire et émet réellement le message n’est pas contenu dans ces consommateurs.

Cette passe réutilise [les contrats](protocol.md), [la première enquête de délégation](other-capabilities-detail.md), [les cycles de session](delegation-lifecycle-deep.md) et [la boucle vocale](goal-execution-autonomy.md). Elle ajoute les transformations de transcript, le panneau d’échange, le composeur lazy, les références sand-msg et les contrôles de provenance. Elle ne répète pas les seuls DTO SendGrokBotAgentMessage ou tours de room.

La cible est la copie sauvegardée de Grok Bot 0.70.2 dans `inputs/Grok Bot.app`. Le parent REA réutilisé est `ev_a8b822a62831e30bd01eb4c4488b36f0bd50cb8ccc41d83e2dcff4e00cf82a4d`; son graphe est partiel. Les lignes citées sont celles des copies LF. Le [ledger](../evidence/interbot-client-paths.json) garde les empreintes des originaux, les extraits et les recherches bornées. Les lectures shell ne sont pas de nouveaux Evidence IDs REA.

Aucun lancement cible, compte, secret vivant, outil de l’application ou appel réseau n’a été utilisé. Cette portée ajoute seulement deux copies LF manquantes, chunk-prompt-editor et chunk-reference-menu, avec Prettier 3.6.2 offline et ignore-path /dev/null. Aucune copie déjà citée n’a été modifiée. Le chunk transcript-markdown préparé par l’enquête parallèle est lu sans reformatage.

## 1. Le transcript conserve les métadonnées interbot jusqu’à l’interface

Le tail serveur décode le corps JSON, le normalise avec Oh puis ajoute seq. Pour kind=message, le normaliseur exige notamment content:string et conserve les autres propriétés. Le consommateur émet appended ou updated avec cette entrée. Le renderer reconnaît comme échange interbot un message doté de fromAgent ou toAgent. Il reçoit ainsi le contenu par la réplique de transcript, sans refaire un envoi au pair.

Observation statique. Sources [A1 main.cjs:35899](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/node-agent-coordinator/main.cjs:35899), [A3 main.cjs:35164](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/node-agent-coordinator/main.cjs:35164), [A2 main.cjs:36031](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/node-agent-coordinator/main.cjs:36031), [A4 index.eager-app-BfVhIMYf.js:1706](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:1706).

Limite. Chaîne statique côté client. La production serveur des deux représentations, les droits de lecture et le contexte modèle transmis restent inconnus.

## 2. Les échanges sont regroupés avec leur direction

xle interprète fromAgent comme inbound, sinon toAgent comme outbound. MWe déduit single, fanout si chaque message sort vers un pair distinct, ou thread dans les autres cas. R_e regroupe les lignes consécutives en agent-comm-group. Le fold actif Nv appelle cette transformation; le rendu choisit v7 et les contrôles des pairs.

Observation statique. Sources [A5 index.eager-app-BfVhIMYf.js:36779](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:36779), [A6 index-D6ReTfRE.js:61291](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index-D6ReTfRE.js:61291), [A7 index-D6ReTfRE.js:60930](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index-D6ReTfRE.js:60930), [A8 index-D6ReTfRE.js:89019](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index-D6ReTfRE.js:89019).

Limite. Ce regroupement d’affichage ne crée ni session ni sous-agent. Si les deux champs existent, la priorité fromAgent appartient à ce code client.

## 3. Le panneau filtre un pair et lit séparément une room

Le clic sur un pair appelle openAgentExchange. Ap filtre les message dont fromAgent.id ou toAgent.id correspond au pair. Il les transforme pour l’affichage en send-message text, avec auteur self pour un envoi au pair et auteur fromAgent pour un retour. tZ utilise ce résultat pour un bot ordinaire. Si le pair est une room, il charge son transcript avec sa session et expose entriesAgentId égal à son ID. Les workers cloud ont une fusion distincte avec leur conversation distante, corrélée par turn et texte.

Observation statique. Sources [A9 index-D6ReTfRE.js:78577](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index-D6ReTfRE.js:78577), [A10 index-D6ReTfRE.js:92862](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index-D6ReTfRE.js:92862), [A11 index-D6ReTfRE.js:121723](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index-D6ReTfRE.js:121723), [A12 index-D6ReTfRE.js:92947](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index-D6ReTfRE.js:92947), [A13 index-D6ReTfRE.js:121890](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index-D6ReTfRE.js:121890).

Limite. Le panneau d’un pair ordinaire montre les échanges du transcript consulté, pas son historique complet ni son contexte modèle. Il dépend des données autorisées remises au client.

## 4. Coéquipier humain, canal et bot ont des auteurs différents

Xr classe un message user selon author et viewerAuthId. Le propriétaire peut être représenté séparément pour un lecteur non propriétaire. Bl construit des clés de groupe distinctes pour member, channel, agent et human. Le nom accessible d’un échange entrant vient de fromAgent.name; un sortant utilise toAgent. channelSender porte la provenance de canal. Le client ne traite donc pas chaque role=user comme un texte écrit par le lecteur.

Observation statique. Sources [A14 index-D6ReTfRE.js:61500](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index-D6ReTfRE.js:61500), [A15 index-D6ReTfRE.js:61535](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index-D6ReTfRE.js:61535), [A16 index-D6ReTfRE.js:61626](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index-D6ReTfRE.js:61626).

Limite. Ces règles attribuent l’affichage au lecteur courant. Elles ne décident pas qui reçoit une copie et ne constituent pas un contrôle d’accès serveur.

## 5. Le menu @ suit les membres et personnes du contexte courant

Dans une room, CKe prend les memberIds résolus dans le roster. Hors room, il exclut le bot courant, conserve d’autres bots et certaines rooms contenant ce bot, avec un filtre de bots d’équipe selon gate. Le menu lazy ajoute everyone quand l’ensemble bots et personnes contient au moins deux candidats. Les personnes portent `user:<id>` ou `user:name:<nom>`. Le choix d’une ligne insère un nœud mention avec id et label, puis un espace.

Observation statique. Sources [A18 index-D6ReTfRE.js:118068](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index-D6ReTfRE.js:118068), [A17 index.eager-app-BfVhIMYf.js:38183](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:38183), [A19 chunk-prompt-editor-DisVTqqh.js:2774](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/chunk-prompt-editor-DisVTqqh.js:2774), [A20 chunk-prompt-editor-DisVTqqh.js:2862](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/chunk-prompt-editor-DisVTqqh.js:2862).

Limite. La présence dans le menu ne prouve ni réveil automatique ni autorisation d’écrire à tous. Le roster et les gates effectifs n’ont pas été observés.

## 6. Le composeur transmet @nom et garde l’ID dans richText

Le chunk prompt-editor est importé dynamiquement puis monté comme PromptEditor avec getMentionMembers et getMentionPeople. Son extension mention sérialise le texte en @label, avec ID comme repli. Au submit, texte du document et JSON riche sont transmis séparément. j.sendPrompt envoie prompt et richText; le performer les traduit en SendGrokBotUserMessage.text et rich_text pour l’agent et la session courants.

Observation statique. Sources [A21 index-D6ReTfRE.js:108083](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index-D6ReTfRE.js:108083), [A22 index-D6ReTfRE.js:96930](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index-D6ReTfRE.js:96930), [A23 chunk-prompt-editor-DisVTqqh.js:4450](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/chunk-prompt-editor-DisVTqqh.js:4450), [A24 chunk-prompt-editor-DisVTqqh.js:4224](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/chunk-prompt-editor-DisVTqqh.js:4224), [A25 chunk-prompt-editor-DisVTqqh.js:5022](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/chunk-prompt-editor-DisVTqqh.js:5022), [A26 index-D6ReTfRE.js:119014](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index-D6ReTfRE.js:119014), [A27 main-app.cjs:110631](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/main-app.cjs:110631).

Limite. Dans cette chaîne, cliquer une mention n’appelle pas de RPC interbot distinct. La lecture distante du document riche, le réveil du destinataire et l’arbitrage des tours restent inconnus.

## 7. Une mention dessinée depuis du texte ne prouve pas un routage

Le rendu de prose d’une room construit des handles à partir des noms des membres. GHe garde un lien handle→ID seulement si la meilleure correspondance est unique. everyone et all sont retirés de cette map. VHe recherche les @handles avec des bornes de caractères et préfère les correspondances longues. Le consommateur dessine les mentions comme pills isRoomScoped, éventuellement sans ID. Leur résolution refuse un repli global par nom.

Observation statique. Sources [A28 index.eager-app-BfVhIMYf.js:38248](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:38248), [A29 index-D6ReTfRE.js:79198](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index-D6ReTfRE.js:79198), [A30 index-D6ReTfRE.js:77494](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index-D6ReTfRE.js:77494), [A31 index-D6ReTfRE.js:77255](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index-D6ReTfRE.js:77255).

Limite. Enrichissement visuel seulement. Une pill sans ID ne prouve aucun envoi et une correspondance ne dit pas quel bot le serveur a réveillé.

## 8. Le reply target est lié à la session de l’envoi

Le bouton de réponse place replyToId et replySessionId dans le draft. Avant l’envoi, le composeur garde la cible seulement si sa session mémorisée correspond à celle de la commande. Le service de drafts retrouve la session de l’entrée dans le transcript; un baseline complet qui ne contient plus la cible la retire. Le performer transmet replyToId et la session de la commande.

Observation statique. Sources [A32 index-D6ReTfRE.js:121329](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index-D6ReTfRE.js:121329), [A33 index-D6ReTfRE.js:118962](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index-D6ReTfRE.js:118962), [A34 index.eager-app-BfVhIMYf.js:72311](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:72311), [A35 index.eager-app-BfVhIMYf.js:11254](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:11254), [A27 main-app.cjs:110631](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/main-app.cjs:110631).

Limite. Guards client contre un mélange de fils. Ils ne révèlent pas la corrélation distante reply_to_id des messages émis par un bot vers un autre, décrite dans les contrats.

## 9. sand-msg référence une entrée et peut ouvrir son fil de tâche

Le parseur reconnaît sand-msg: et accepte seulement une adresse d’entrée de sa grammaire. Le chunk transcript-markdown est réellement importé. Il résout l’entrée dans le contexte de transcript fourni, puis dessine une pill ou garde un texte. L’aperçu prend fromAgent.name pour un message entrant de bot. Le clic scrolle ou révèle l’entrée. Pour une adresse de tâche, il ouvre openThread avec agentId, taskId et focusEntryId.

Observation statique. Sources [A36 index.eager-app-BfVhIMYf.js:47997](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:47997), [A37 index-D6ReTfRE.js:76120](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index-D6ReTfRE.js:76120), [A38 chunk-transcript-markdown-CQqFEjjt.js:619](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/chunk-transcript-markdown-CQqFEjjt.js:619), [A39 chunk-transcript-markdown-CQqFEjjt.js:352](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/chunk-transcript-markdown-CQqFEjjt.js:352), [A40 chunk-transcript-markdown-CQqFEjjt.js:471](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/chunk-transcript-markdown-CQqFEjjt.js:471), [A41 chunk-transcript-markdown-CQqFEjjt.js:543](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/chunk-transcript-markdown-CQqFEjjt.js:543).

Limite. Référence d’interface, pas transfert de contexte modèle. La navigation peut demander de l’historique affichable; le partage de ce contenu avec un autre bot n’est pas établi ici.

## 10. Deux chemins actifs renvoient les résultats vers la voix

La gateway parse une enveloppe channel et payload. Le registre renderer relie voice-call-nudge à ingestInboundNudge, qui exige le bon agentId, le bon callId et un appel encore actif. Il crée un nudge to-voice. Pour une thread teammate, une autre branche lit les send-message text dont message.channel vaut `voice:<callId>`. Elle exige aussi la session du fil, compare callId et déduplique par entry.id avant de produire ce résultat vocal.

Observation statique. Sources [A42 main.cjs:41997](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/node-agent-coordinator/main.cjs:41997), [A43 index.eager-app-BfVhIMYf.js:76337](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:76337), [A44 index.eager-app-BfVhIMYf.js:41758](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:41758), [A45 index.eager-app-BfVhIMYf.js:40577](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:40577), [A46 index.eager-app-BfVhIMYf.js:41807](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:41807).

Limite. La déduplication retracée du chemin transcript porte sur entry.id. L’émission serveur, le choix du canal et les garanties de remise restent distants.

## 11. Un message de bot ne devient pas un écrit du caller

Le rail written-user accepte seulement les message user non vides attribués au lecteur par j0. Il exclut les entrées avec channel ou fromAgent, puis channelSender et fromUser sans author correspondant. Les écrits acceptés passent à forwardWrittenTurn. Sur le protocole vocal async-update, Yn encode user_message et borne la copie à 2 000 unités de longueur JavaScript. Les résultats utilisent le nudge outcome et le topic work_landed.

Observation statique. Sources [A47 index.eager-app-BfVhIMYf.js:40842](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:40842), [A46 index.eager-app-BfVhIMYf.js:41807](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:41807), [A49 index.eager-app-BfVhIMYf.js:80015](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:80015), [A48 index.eager-platform-DUN5VkaE.js:11077](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index.eager-platform-DUN5VkaE.js:11077), [A50 index.eager-platform-DUN5VkaE.js:11140](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index.eager-platform-DUN5VkaE.js:11140), [A51 index.eager-app-BfVhIMYf.js:41180](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:41180).

Limite. Séparation de provenance côté client. Elle ne prouve pas un filtrage équivalent du contexte modèle distant. La borne concerne cette copie vocale de l’écrit.

## 12. Aperçu et notification gardent le nom du pair

Le renderer formate sent_agent_message avec recipient et received_agent_message avec sender, plus le texte trim. Le main possède le même consommateur. aut l’utilise pour composer une notification depuis lastMessagePreviewSource; le corps est ensuite nettoyé et borné à 140 unités par le formateur de notification.

Observation statique. Sources [A52 index.eager-app-BfVhIMYf.js:29353](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:29353), [A54 main-app.cjs:116801](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/main-app.cjs:116801), [A53 main-app.cjs:117179](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/main-app.cjs:117179).

Limite. L’aperçu n’est pas le message complet. Cette fonction ne détermine pas quel lecteur reçoit la notification; destinataire, mute et autorisation appartiennent à d’autres étapes.

## 13. L’émetteur interbot et l’ordonnanceur restent distants

La recherche dans les 600 JavaScript originaux retrouve sendGrokBotAgentMessage dans trois descripteurs et aucun appel direct sous ce nom. Aucun littéral sourceChannel, targetChannel, source_channel, target_channel ou replyChannel n’a été trouvé. Les consommateurs actifs retracés lisent fromAgent, toAgent, channel et author. Le channel de l’enveloppe gateway et message.channel dans le transcript ont des consommateurs différents.

Frontière inconnue. Sources [A55 index.eager-app-BfVhIMYf.js:1731](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:1731), [A42 main.cjs:41997](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/node-agent-coordinator/main.cjs:41997), [A27 main-app.cjs:110631](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/main-app.cjs:110631).

Limite. Absences bornées aux motifs du ledger. Un appel dynamique ou un autre artefact reste possible. Choix du pair, contexte d’entrée et propagation de sa réponse à un tiers demeurent unresolved.

## Exemples dérivés du code

Ces exemples suivent les branches statiques. Ils n’ont pas été exécutés.

| Entrée | Ce que le client en fait | Ce qui reste inconnu |
|---|---|---|
| Dans le transcript de A, une demande avec toAgent=B puis une réponse avec fromAgent=B. | La carte résume deux échanges avec B. Le panneau de B donne A comme auteur de la demande et B comme auteur du retour. | Comment B reçoit la demande et quel historique consulte son modèle. |
| Un coéquipier sélectionne B dans le menu @ d’une room et envoie une demande. | richText garde mention{id:B,label:B}; prompt contient @B. sendPrompt vise la room et sa session. | Quel membre reçoit un tour et si B est effectivement réveillé. |
| Un send-message text de la session teammate porte channel=voice:call-123. | Si agent, session et callId correspondent, son texte devient un résultat vocal, une seule fois par entry.id. | Tout message entrant fromAgent ne suit pas ce chemin. |
| Un draft répond à une entrée de dm:42 mais l’envoi vise dm:43. | Le composeur ne conserve pas cette cible de réponse. | Contrôle d’accès serveur et sort d’une commande déjà envoyée. |

## Checklist

| Question | Statut | Limite |
|---|---|---|
| Reconnaissance des pairs, direction et regroupement | answered-client-side | Production serveur des métadonnées non observée. |
| Vue du pair et transcript d’une room | answered-client-side | Historique visible, pas contexte modèle complet. |
| Mentions proposées, sérialisées et envoyées | answered-client-side | Réveil et arbitrage distants inconnus. |
| Réponses scoped et navigation des citations | answered-client-side | Corrélation interbot distante traitée dans les contrats. |
| Retour des résultats vers la voix | answered-client-side | Garanties de remise serveur inconnues. |
| Destinataire et tour effectif après mention ou demande interbot | partially_answered | Émetteur et ordonnanceur distants absents. |
| Contexte modèle délégué à un autre bot | unresolved | Le transcript affiché ne constitue pas ce contexte. |
