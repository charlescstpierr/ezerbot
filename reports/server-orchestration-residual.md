# Orchestration serveur et démonstration vers skill: frontières restantes

Cette passe cherche des consommateurs supplémentaires, après EA, CM et PS. Elle apporte une réparation active de réveil après formulaire et un chemin complet d’import de fichiers texte en skills. Elle ne récupère pas le moteur serveur qui décide du plan, choisit un bot, délègue, vérifie la réussite ou transforme une démonstration enregistrée en skill.

Les acquis de [EA](goal-execution-autonomy.md), [CM](goal-context-memory-learning.md), [PS](goal-permissions-secrets.md), [délégation](delegation-lifecycle-deep.md) et [capacités](other-capabilities-detail.md) ne sont pas recomptés. Notamment, sélection de moniteur/subagent, liste des enfants, parent de tâche, contrats de room et goals SDK étaient déjà documentés.

La cible reste la copie sauvegardée `inputs/Grok Bot.app`, Grok Bot 0.70.2. Les 600 scripts réellement livrés, dont 551 .js, 48 .cjs et un .mjs, ont été recherchés avec vérification de leurs SHA-256 contre le manifeste. Les 532 chunks lazy constituent une partie de ce corpus. Les seuls nouveaux corps formatés par cette passe sont chunk-view-CP4eA5UX.js, chunk-team-setup-new-skill-dialog-BvGAp4Wb.js et chunk-use-team-bot-access-CSgf1Fus.js, via Prettier 3.6.2 déjà en cache, sans réseau et sans modifier une copie existante. Le dialogue contexte déjà présent a été réutilisé. Les SHA-256 originaux/formatés et ancres LF figurent au [ledger](../evidence/server-orchestration-residual.json).

Parent REA réutilisé: `ev_a8b822a62831e30bd01eb4c4488b36f0bd50cb8ccc41d83e2dcff4e00cf82a4d`. Aucune analyse globale répétée, cible exécutée, requête externe, profil, compte ou secret vivant consulté. Les chemins dits actifs sont reliés statiquement, jamais observés dans une session vivante.

## Nouveaux consommateurs

### SR01. Un formulaire réglé peut encore attendre le réveil de l’agent

Un nouveau consommateur lazy distingue formResolution et formSubmissionEffect; si wakeDelivery reste pending, il réessaie submitUserForm avec les mêmes agentId/entryId et un objet values vide. Classification: `observation`.

Le composant de carte `Rs` passe `entry.formResolution` et `entry.formSubmissionEffect` à `bt → Ve`. La continuation démarre seulement si une résolution existe, si submissionEffect.kind vaut settled, si wakeDelivery vaut pending et si agentId existe. Le hook crée un AbortController et une politique `user-form-wake-reconciliation`, mode until-signal. Il attend d’abord 1000 ms, puis appelle `runWithRetry(() => submitUserForm({entryId,values:{},agentId}))`; le backoff déclaré démarre à 5000 ms, plafonne à 15000 ms et multiplie par trois. Le nettoyage du hook annule le signal et le timer.

Cela découvre une réparation de remise d’un réveil, déclenchée par l’état d’une carte déjà réglée. Le premier submit utilisateur collecte les champs non vides; la réconciliation n’expédie pas de nouveau ces champs. Ce chemin n’était pas décrit par EA, CM ou PS.

Limite: Le hook ne prouve pas le réveil effectif, sa déduplication serveur ou la persistence de la demande après fermeture du client. Il ne choisit pas un nouveau plan métier.

Artefact nécessaire pour aller plus loin: Handler SubmitGrokBotUserForm et stockage des submission effects/pending wakes, avec trace reliant entryId à la reprise du run.

Preuves: [A1: F:2568](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/chunk-view-CP4eA5UX.js:2568), [A2: F:2132](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/chunk-view-CP4eA5UX.js:2132), [A3: F:1880](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/chunk-view-CP4eA5UX.js:1880), [A4: F:2262](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/chunk-view-CP4eA5UX.js:2262).

### SR02. Le contrat de réparation s’arrête à accepted

La réconciliation rejoint le même RPC SubmitGrokBotUserForm que la soumission initiale; son adapter ne restitue qu’un succès nul si accepted est vrai, pas un reçu de reprise du moteur. Classification: `observation`.

Le validator du coordinator conserve entryId, values, agentId, plateforme optionnelle et saveToVault. Son parser accepte ces données, puis le router classe submitUserForm parmi les actions de scope temporal sans miroir gateway. Selon le harness de l’agent, le dispatch appelle le performer serveur ou la gateway; le comportement général de routage était connu, mais son consommateur de réparation est maintenant relié.

Le performer main clone values et appelle `submitGrokBotUserForm`. Un refusal devient un refus client; accepted vrai devient `{status:"ok",value:null}`. Le callback de carte ne reçoit ni run ID, ni workflow ID, ni confirmation que les outils ont repris. Ce payload précis utilise entryId comme rattachement; il ne comporte pas de sourceRequestId ni de sessionId. L’objet vide de SR01 n’est donc interprétable correctement que par le service qui connaît déjà cette entrée.

Limite: Une réponse accepted ne permet pas d’inférer la transaction entre enregistrement des valeurs et livraison du wake. Le client ne montre pas comment le backend traite un values vide ou une entrée déjà réglée.

Artefact nécessaire pour aller plus loin: Implémentation serveur SubmitGrokBotUserForm et journal de remise du wake; trace de la transition accepted → run repris ou erreur.

Preuves: [A5: C:39018](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/node-agent-coordinator/main.cjs:39018), [A6: C:42981](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/node-agent-coordinator/main.cjs:42981), [A7: C:39550](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/node-agent-coordinator/main.cjs:39550), [A8: C:43812](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/node-agent-coordinator/main.cjs:43812), [A9: A:110789](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/main-app.cjs:110789).

### SR03. Un autre chemin fabrique réellement des skills à partir de fichiers texte

Le lazy d’import convertit des fichiers UTF-8 en couples name/content et ajoute des métadonnées origin:upload; cette conversion locale ne consomme pas une démonstration Teach. Classification: `observation`.

`Ol/Ge` du dialogue contexte branche les inputs de fichiers/dossiers sur `Tt`, avec readListing et addSkills de l’accès au bot. `xt` lit les bytes des fichiers, accepte txt/md/markdown/csv/json/yaml/yml, refuse le binaire et les séquences UTF-8 invalides, puis décode le texte. La limite de contenu intervient après lecture et décodage.

`mt → ft` choisit le nom depuis le frontmatter, le titre Markdown, le dossier parent de SKILL.md ou le nom de fichier. Les noms sont normalisés et les collisions reçoivent des suffixes. Chaque texte utile devient une skill distincte. La limite déclarée est 256000 unités de longueur JavaScript par texte et 250 skills après décompte des entrées connues. `Xze`, importé comme je, conserve le corps et inscrit dans son frontmatter `origin: upload`, upload-extension et, selon le cas, upload-name. Ces métadonnées donnent une origine exploitable à la bibliothèque, sans résultat de modèle dans cette conversion.

Limite: Ces limites concernent ce formulaire d’import, pas le stockage serveur ou toutes les voies de skills. Il ne lit pas de vidéo/audio Teach et n’extrait pas de procédure depuis une recording.

Artefact nécessaire pour aller plus loin: Pour Teach: média sauvegardé, job d’extraction et résultat contenant skill ID, contenu, provenance et validation. Le convertisseur upload livré ne remplit pas ce rôle.

Preuves: [A10: X:106](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/chunk-team-bot-context-dialog-BSwm3Kpp.js:106), [A11: S:48](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/chunk-team-setup-new-skill-dialog-BvGAp4Wb.js:48), [A12: S:59](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/chunk-team-setup-new-skill-dialog-BvGAp4Wb.js:59), [A13: S:125](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/chunk-team-setup-new-skill-dialog-BvGAp4Wb.js:125), [A14: S:278](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/chunk-team-setup-new-skill-dialog-BvGAp4Wb.js:278), [A15: S:381](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/chunk-team-setup-new-skill-dialog-BvGAp4Wb.js:381), [A16: S:444](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/chunk-team-setup-new-skill-dialog-BvGAp4Wb.js:444), [A17: R:49015](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:49015), [A18: R:81195](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:81195).

### SR04. L’import écrit en série et conserve les succès partiels

Le callback addSkills de l’accès au bot appelle addGrokBotAgentSkill pour chaque élément; il retourne un compte de succès sans rollback de ceux déjà écrits. Classification: `observation`.

`Ce.contextAccess.addSkills` boucle sur les couples name/content. Pour chaque élément, le wrapper r met l’UI en pending, exécute l’écriture, transforme une erreur en false et poursuit le lot. Le compteur s augmente pour chaque succès; s inférieur au nombre demandé signale writeFailed. Le hook p est l’export bL, qui retourne le domaine grokBotPlugins.addSkill. Ce domaine appelle desktop.addGrokBotAgentSkill, puis remplace le listing par celui de la réponse. Le handler main conduit au RPC AddGrokBotAgentSkill avec agentId, nom éventuel, description éventuelle et content.

Dans l’importer, un nombre positif donne le statut local done, même si le lot est partiel. La liste `written` ne contient les entrées que lorsque le nombre ajouté égale la taille du lot. Il faut lire ces deux sorties séparément: done signifie ici que des skills ont été ajoutées, pas qu’un apprentissage a réussi ou que tout le lot est atomique.

Limite: Le client ne prouve ni la validation sémantique des skills par le serveur, ni la sélection de la skill au prochain tour. Le wrapper ne fournit pas de transaction ou de rollback global du lot.

Artefact nécessaire pour aller plus loin: Handler AddGrokBotAgentSkill et son stockage/versionnement, puis moteur de sélection/invocation des skills pour établir leur usage effectif.

Preuves: [A19: U:102](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/chunk-use-team-bot-access-CSgf1Fus.js:102), [A20: U:190](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/chunk-use-team-bot-access-CSgf1Fus.js:190), [A21: U:237](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/chunk-use-team-bot-access-CSgf1Fus.js:237), [A22: R:80874](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:80874), [A23: R:13229](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:13229), [A24: R:13114](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:13114), [A25: A:108485](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/main-app.cjs:108485), [A26: A:103017](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/main-app.cjs:103017), [A27: S:381](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/chunk-team-setup-new-skill-dialog-BvGAp4Wb.js:381).

### SR05. Le nombre 32 reste une déclaration de configuration

local_subagent_limits.maxRunning a un repli de 32 mais ne possède que ses déclarations dans les 600 scripts livrés; aucun consommateur de ce nom n’est retrouvé. Classification: `configuration`.

Le schéma main exige un maxRunning entier positif. Le catalogue annonce local_subagent_limits côté client avec fallbackValues.maxRunning=32. La recherche exacte sur les 600 fichiers .js/.cjs/.mjs retrouve deux occurrences de local_subagent_limits et deux de maxRunning, toutes dans ces passages. Aucun nouveau chunk lazy ne lit ces noms.

Un autre résultat de recherche budgetExhausted appartient à GrokBotHarnessMigrationPassStatus, qui décrit un passage de migration de harness avec workflow/run, agents/rooms et resultAvailable. Ce champ ne fournit pas le budget d’un tour conversationnel. Les bornes locales actives d’outils et les goals du SDK étaient déjà distingués dans EA; cette passe ne les requalifie pas en ordonnanceur de bots.

Limite: 32 n’est ni une valeur effective observée, ni une borne prouvée de concurrence Grok. Une lecture calculée/dynamique ou un consommateur distant peut échapper aux noms littéraux. Le budget de coût/tokens/tours global reste inconnu.

Artefact nécessaire pour aller plus loin: Consommateur d’admission des sous-agents dans la box ou le workflow Temporal, configuration effective et décision de refus/attente reliée au compteur.

Preuves: [A28: A:58295](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/main-app.cjs:58295), [A29: A:60148](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/main-app.cjs:60148), [A30: PR:60378](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/electron-main/proto.cjs:60378).

## SR06. Ce que les ressources livrées ne ferment toujours pas

Le nouveau chemin de réveil donne une commande consommée, et l’import donne des skills réellement écrites. Ils ne donnent ni l’ordonnanceur ni un générateur de skills par démonstration. Les événements `teach-recording`, `agents-workflow`, `agents-bot-skills` et les commandes de capture restent distincts. Le raccordement entre eux n’est pas fourni par leur coexistence. Le stop Teach relu applique le snapshot retourné; son contrat transporte agentId/save, sans sortie de skill dans ce call site. Ce dernier point est un acquis CM réutilisé comme frontière, pas une nouvelle découverte.

Preuves de frontière: [A31: stop snapshot](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:57572), [A32: contrat](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/node-agent-coordinator/main.cjs:39235), [A33: familles d’événements](/Users/cstpierrecharles/Desktop/grokbot-rust/artifacts/formatted/dist/node-agent-coordinator/main.cjs:42164).

Les recherches nouvelles sont précises. local_subagent_limits et maxRunning restent limités aux deux déclarations main; tokenBudget/token_budget, createGoal/updateGoal/getGoal, sourceRequestId/source_request_id, taskSource/task_source, learnFromDemonstration/learn_from_demonstration, recordingId/recording_id, learnedSkill/learned_skill et skill_created n’ont pas de correspondance exacte dans ces 600 scripts. wakeDelivery et user-form-wake-reconciliation aboutissent au consommateur SR01. Les catalogues de traduction et grammaires de langages ont été distingués des call sites.

Chaque résultat négatif est limité à ces mots et fichiers. Des noms différents, une lecture calculée, une ressource téléchargée à l’exécution ou un autre artefact peuvent modifier la réponse. L’absence du code serveur reste une inconnue; aucun planificateur n’est inventé à partir du SDK.

| Question restante | Dernière frontière vérifiable | Artefact exact à obtenir |
|---|---|---|
| Comment choisir un pair/sous-agent et lui donner une tâche? | Lecture/présentation d’enfants et contrats distants déjà retracés; aucun nouvel initiateur dans ces lazy | Handler de délégation et politique de choix dans la box ou le workflow Temporal; input/output de décision et IDs parent/enfant |
| Quel contexte reçoit l’enfant et quel source request le relie? | Payloads client et IDs de conversation déjà connus; le nouveau wake utilise entryId | Constructeur de requête modèle déléguée et trace reliant task, source request, parent et contexte choisi |
| Quand replanifier ou reprendre? | Le client peut réparer une remise de wake, sans choisir l’étape suivante | Handler wake et état durable du run, avec cause et prochain appel de modèle |
| Quand dire terminé et arrêter les enfants? | Accusé accepted ou états UI ne sont pas un prédicat de réussite | Fonction de transition terminale, politique de drain/annulation des enfants et résultat de vérification métier |
| Quels tours/tokens/coûts/concurrence sont autorisés? | Config maxRunning déclarée, bornes locales déjà documentées | Compteurs effectifs et lecteur de politique du moteur, décision d’admission/épuisement et trace de fin |
| Comment une démonstration devient-elle une skill utile? | Capture/save/status d’un côté; import/édition de texte et RPC skill de l’autre | Média Teach + job d’extraction + résultat recording-ID → skill-ID/version; critères de validation et mécanisme de sélection de skill |

## Checklist finale

| Question | Statut | Limite |
|---|---|---|
| Réparation du réveil après formulaire | `answered-client-side` | Retry et rattachement à entryId retracés; livraison/persistence serveur inconnues. |
| Conversion upload texte → skills et résultat de lot | `answered-client-side` | Conversion/écritures/compte de succès prouvés; usage modèle non établi. |
| Démo Teach → procédure → skill validée | `unresolved` | Aucun job/résultat recording-ID vers skill-ID dans la frontière inspectée. |
| Planification, choix du pair ou enfant, replanification et fin métier | `unresolved` | Pas de nouveau consommateur moteur dans les chunks examinés; box/Temporal nécessaires. |
| Budgets effectifs et admission des sous-agents | `partially_answered` | Nouveau repli maxRunning 32 identifié sans consommateur; compteurs et politique moteur absents. |
| Task/source request, goals et reprise du run | `partially_answered` | EntryId de formulaire consommé; noms sourceRequestId et appels goals exacts absents, sans exclure des équivalents distants. |
