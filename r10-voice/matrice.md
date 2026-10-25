# Correction voix de la classe brouillon

Le code produit est dans le worktree ez-i294. Aucun commit, push, cargo, navigateur réel, verifier ou pret n'a été lancé par cette aide. Les fixtures utilisent uniquement des identifiants et données synthétiques.

Le principe principle-fix-root-causes a conduit à supprimer la création et le refresh privés des panneaux voix. L'hôte crée et transfère désormais le même brouillon avec ensureDraftConversation. Le principe principle-model-the-domain a conduit à transférer explicitement session.key et scopeKey avant onState, sans annuler la capture du même brouillon. Les deux fiches ont été lues dans ~/.codex/skills.

| Cas | Qualification et résultat | Test | Rouge | Vert |
| --- | --- | --- | --- | --- |
| R23, R19, T17 | Déclencheur synthétisé. Fil créé absent du refresh, sauvegarde refusée puis onState. Le résultat reste réessayable et une seule création est nécessaire. | voice.test.cjs, `T17 R23 memo survives…` | red.log, échec du bouton Réessayer | all-green.log |
| R19, T17 | Conservation avant et après. Création refusée, onState puis nouvel essai sans réenregistrement. | voice.test.cjs, `T17 creation refusal…` | red.log, déjà vert | all-green.log |
| R23, T17 | La sauvegarde du mémo utilise la réponse de création et ne dépend plus du refresh qui peut échouer. | voice.test.cjs, `T17 created memo…` | red.log, aucune puce | all-green.log |
| N05, R19, R32, T24 | Une capture récente suit le transfert automatique de new:bot au fil du même brouillon. Elle se termine dans ce fil sans seconde capture. | voice.test.cjs, `N05 T24 recent capture…` | red.log, aucune puce | all-green.log |
| R19, T14 | Conservation avant et après. Deux mémos préparés résistent au snapshot ancien. L'accusé du premier depuis un autre fil laisse le second disponible au retour. | voice.test.cjs, `R19 T14 prepared memos…` | red.log, déjà vert | all-green.log |
| R19, T17 | Une navigation réelle abandonnant un mémo encore en sauvegarde annonce qu'il n'a pas été conservé. | voice.test.cjs, `R19 T17 a context change…` | red.log, seule erreur précédente | all-green.log |
| R19, T16 | discard annonce l'abandon du mémo pending avant retrait de Réessayer. | voice.test.cjs, `R19 T16 discard…` | red.log, seule erreur précédente | all-green.log |
| R19, T09 | Un mémo préparé masqué par l'absence du fil est annoncé et redevient accessible quand son fil revient. localMemos n'est pas effacé. | voice.test.cjs, `R19 T09 prepared memo hidden…` | red.log, aucune notification | all-green.log |
| R32, T24 | Le mode voix en écoute suit lui aussi la création automatique par un autre producteur du brouillon. | voice-conversation-ui.test.js, `R32 voice session follows…` | conversation-red.log, aucun envoi | all-green.log |
| R32, T24 | La première phrase du mode voix utilise le fil créé même si un refresh serait indisponible. | voice-conversation-ui.test.js, `R32 welcome voice sends…` | conversation-red.log, aucun envoi | all-green.log |

Les notifications de récupération passent par notifyDraft. La conservation de leur affichage après notify(error) est une responsabilité de l'hôte, en cours de correction par l'owner. Les tests locaux prouvent l'appel et son texte, sans prétendre vérifier le toast final du shell.

La commande finale était `/Users/cstpierrecharles/.treehouse/pm/_orch/bin/test-gate node --test app/ui/panels/voice.test.cjs app/web/src/voice-ui.test.js app/web/src/voice-conversation-ui.test.js`. Elle a passé 85 tests, sans test ignoré. `git diff --check` est passé. Les rouges ont été exécutés avant les modifications produit correspondantes. Les mocks historiques ont été migrés au contrat d'hôte convenu, sans fallback dans la production.

Les tests ciblés exécutent les vrais scripts des panneaux dans un DOM simulé. Ils ne constituent pas une preuve Mac ou une preuve de citation Pages transférée par le shell réel. L'intégration de ensureDraftConversation, le toast final et la citation Pages restent à vérifier par l'owner avec ses tests du shell et sa preuve live.

Meetings reste à l'owner. Son endpoint /api/meetings/start crée aussi un fil mais porte une sémantique de suppression du fil créé uniquement pour la réunion. Précréer aveuglément ce fil par /api/conversations changerait cette sémantique. L'adoption du fil retourné doit partager le transfert sans perdre la propriété de suppression.
