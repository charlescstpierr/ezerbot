# Identité, personnalité et configuration des bots

La personnalité visible du client passe surtout par une **description libre du bot**, son nom, les instructions d'un template et des ressources durables comme les mémoires et les skills. La preuve la plus directe se trouve dans l'appel vocal: le client insère la description dans un prompt "Role & Persona" et lui donne priorité pour le rôle et la manière de parler. Pour les conversations écrites, le client transmet les données de profil, mais l'assemblage du prompt système du moteur distant n'est pas livré. [BP01, BP06, BP08, BP12]

Cette passe porte sur Grok Bot 0.70.2, cible conservée dans `inputs/Grok Bot.app` et JavaScript extrait sous `artifacts/client`. Elle réutilise le parent REA `ev_a8b822a62831e30bd01eb4c4488b36f0bd50cb8ccc41d83e2dcff4e00cf82a4d` dans `evidence/rea-bundle.json`. Aucun compte, endpoint, bot réel ou processus de la cible n'a été utilisé. Les observations sont statiques; le comportement réel du modèle n'est pas testé.

Les références utilisent les lignes des copies existantes sous `artifacts/formatted`. [bot-personality-detail.json](../evidence/bot-personality-detail.json) conserve chaque passage, son hash, le hash original et le node REA. Le graphe global est `partial`; l'AST de M est aussi `partial`, ce qui empêche de transformer une absence de graphe en preuve d'absence.

| Alias | Chemin relatif exact sous `artifacts/client` et `artifacts/formatted` | SHA-256 original |
|---|---|---|
| P | `dist/electron-main/proto.cjs` | `856b35e5e9f93037b4f90ca9d27fd4ea4120c4175ce95d32ceeaf6ca0bbd2b29` |
| M | `dist/electron-main/main-app.cjs` | `06756304c69a22e75aee48892fb0ac42eb7eb22feec239f1deee47e79c60131a` |
| C | `dist/node-agent-coordinator/main.cjs` | `c6bad29e64073f4fbebe5a9d6818e53381efd7aa1c7909c84172e3da9b2de9f0` |
| A | `dist/renderer/assets/index.eager-app-BfVhIMYf.js` | `09f8a360a138c4680956c797c8a0fc25fbf619c9421327be75da0935a9aa9e80` |
| U | `dist/renderer/assets/index-D6ReTfRE.js` | `f3c242161efb08158161e259960b0ae91c150f6891983f6a70faf80c6f5ee697` |
| F | `dist/renderer/assets/index.eager-platform-DUN5VkaE.js` | `560912e771f4989101d04ad06dca024cd1c0513cbdcad122222d9a6bd1df1183` |

## Douze constatations

### BP01. Le profil offre du texte libre, pas un vecteur de traits

Le schéma client `cU/cH` a deux champs requis `name:string` et `description:string`, puis `title?`, `avatarShape?`, `avatarColor?`. La création ajoute `avatarPngBase64?`, `origin?`, `purpose?`, `language?`, `templateId?`, `isIntroductionSuppressed?`, `isKickstartRequested?`, `isAutomatic?`, `clientNonce?`, `creationRoute?`, `supportsTemporalHarness?` et `harness?`. `origin` vaut `user` ou `dev`; les purposes explicitement autorisés ici sont `disk-saver` et `plugin-auth`. C:38628-38655, 5109-5131.

Le protobuf `CreateGrokBotAgentRequest` récupère nom, description, titre, avatar, identité, harness, kickstart, suppression d'introduction et langue. Le DTO de réponse `GrokBotAgent` ajoute role, visibilité, droits du lecteur et préférences vocales. Ces contrats n'ont pas de champ `personality`, `systemPrompt`, température ou modèle de conversation par bot. `role` dans le DTO ne suffit pas à prouver une instruction de rôle envoyée au modèle. P:52127, 51706, 53011.

Exemple construit d'après ce schéma, sans création réelle:

```json
{
  "name": "Archiviste",
  "description": "Tu classes mes notes. Réponds en français, avec des phrases courtes et les sources utiles.",
  "avatarShape": "blob",
  "avatarColor": "black",
  "isKickstartRequested": true
}
```

Cet exemple montre où une personnalité peut être décrite. Il n'est pas un record capturé et ne prouve pas comment le moteur écrit traite ce texte.

### BP02. Le launcher enrichit certaines descriptions avec le contexte utilisateur

`jOe` prend le nom et la description saisis, construit `description:SOe(n.description,l)`, puis appelle `roster.createAgent` avec un nonce, `origin`, kickstart et les choix d'avatar. Le contexte du launcher peut fournir le rôle professionnel et les outils quotidiens de l'utilisateur. Le helper `p7e`, exporté sous `kA` et importé comme `Gre`, produit les phrases persistées qui demandent d'adapter les suggestions au domaine de travail et de commencer par ses outils habituels. U:93778-93825; A:28369-28392, 81449.

Cette personnalisation modifie le texte de description envoyé. Elle n'est pas un changement de poids du modèle. La création d'un chat ordinaire peut aussi envoyer une description vide et démarrer directement avec le premier message utilisateur. U:124094-124142.

### BP03. La création garde une identité et une route séparées du profil

Le domaine renderer injecte `language:e.language()`. Une création Temporal persiste sa préparation par compte dans `agent-creations`, schéma 1, avant l'appel. La route provient de `resolveAgentCreation`; les helpers `dev` ou avec `purpose` passent par la box. A:64170-64332; C:43748-43804.

La branche serveur `zYe.create` appelle `CreateGrokBotTemporalAgent`. Le nonce devient `agentId`; nom, description, titre et avatar sont transmis, et le harness est forcé à Temporal. `introductionSuppressed` vaut vrai si demandé ou si kickstart n'est pas demandé. L'appel vérifie le scope backend/compte et exige que l'agent retourné confirme l'ID et le harness. M:100587-100687. Le rôle et les actions d'introduction du moteur distant restent inconnus.

### BP04. L'édition du profil est une mutation explicite avec état optimiste

Le panneau offre une description en `textarea`. Il trim le texte et commit à la perte de focus; Escape restaure le texte précédent. Une description non vide peut être effacée, tandis qu'un nom vide est refusé par ce composant. `FYe` fabrique une mutation `{name:nomActuel,description:nouveauTexte}` ou conserve la description lors d'un changement de nom. U:37225-37405, 37523-37540, 128297-128361.

Le chemin ordinaire appelle `source.updateAgent({id,profile})`, qui suit le gateway dans le routeur inspecté. Le domaine tient une modification optimiste; il confirme après succès, garde l'incertitude pour une panne transport ou rollback après un autre échec. Pour un bot dont le lecteur n'est pas propriétaire et qui possède un `serverId`, `yCe` utilise la voie desktop `updateGrokBotAgent`. Elle rejoint M `team.updateIdentity`, puis `UpdateGrokBotAgent` avec `partialUpdate:true`; les champs d'avatar sont séparés. A:65271-65340, 66621-66633, 67292-67330; M:108404-108405, 104120-104137; C:39474-39535, 43662-43683. Les droits sont vérifiés ailleurs; cette route ne rend pas tous les profils modifiables par tous les lecteurs.

### BP05. Le premier bot est issu d'un template versionné

Les constantes expédiées donnent un bot "Grok Bot", description d'aperçu "A teammate with its own computer that takes real work off your plate.", avatar `blob/black`, template `sbc4f60e296bbd316b2cb`, version 1. F:34813-34821.

`OCe` réserve un ID par compte, puis appelle `roster.createAgentFromTemplate` avec ces constantes, `isAutomatic:true` et le `creatorContext` non vide. Il surveille le changement de compte. Après création, le flow marque l'onboarding et attend le premier message; une expérience peut créer d'autres bots adaptés au rôle utilisateur. A:67864-67929, 67423-67473. Le contenu actuel du template n'a pas été téléchargé; sa description d'aperçu n'est pas son prompt complet.

### BP06. Le template distingue description publique et instructions du bot

La recette a `profile:{name,description,avatarColor?,avatarShape?}`, `memory[]`, `skills[]`, `routines[]`, `plugins[]`, et `gettingStarted?:{skill}`. Le parser exige que `gettingStarted.skill` référence une skill de la recette. `R$e` place `recipe.profile.description.trim()` dans `instructions`, alors que `view.description` emploie la description publique de la listing. `gGe` reprend ensuite ces instructions comme description du bot dans la vue d'import. M:87172-87250, 87345-87369; A:32907-32909.

L'import serveur reçoit `shareId`, `agentId`, `expectedActiveVersion`, `creatorContext?`, `language?`, `setupDelegationSupported:true`. Il vérifie l'identité Temporal retournée. `setupHandledByServer`, `conversationalSetupEnabled`, `gettingStartedTrusted` et `updateStateSplit` changent le setup client. M:112480-112524. Cela explique comment plusieurs bots peuvent partir de profils et comportements initiaux différents, sans supposer que leurs prompts sont présents intégralement dans l'ASAR.

### BP07. Une partie du premier échange est un prompt de setup embarqué

`mGe/yGe` produit deux textes: un message visible bref et un `modelText` avec `<bot_template_setup_context>` et `<bot_template_setup_instructions>`. Il y sérialise routines, plugins, mémoires et skills. Il demande d'écrire les mémoires, d'installer les plugins confirmés, d'écrire les skills et de créer les routines en pause. La variante conversational demande un setup discret et peut lancer la skill getting-started comme première conversation. A:32935-33084.

La recette mémoire accepte un `kind` `profile` ou `log`, mais `UR` ne recopie ici que contenu et date; l'instruction `qR` demande le tier `log`. Ce détail décrit ce setup client précis, pas tous les modes d'import. M:87160, 87201-87213; A:32958-32995. Le call site envoie le `modelText` via `sendPrompt`, avec `initiator:"template_import"` et une provenance de routine choisie selon la confiance du setup. U:47030-47058. Des instructions présentes dans un prompt ne prouvent pas que le moteur distant les a toutes appliquées.

### BP08. La description commande explicitement la persona vocale du package

`Au.run` transmet à `harness.session` `agentId`, `agentName`, `userName`, `description` si disponible et `spokenLanguage`. Le plan local `rU.session` appelle `iI.sessionPlan`. A:41870-41889, 41996-42029; M:41003-41035.

Le prompt local commence par "Role & Persona", pose le nom et insère la description. Il décrit cette dernière comme une contrainte de qui est le bot, de son travail et de sa manière de parler. Si elle nomme une voix ou une construction de phrase, le bot doit la garder dans le greeting et chaque réponse; sinon le registre par défaut est celui d'un ami chaleureux et vif. La description est répétée dans "CRITICAL INSTRUCTIONS", avec une consigne de première personne. M:40377-40458.

Le prompt encadre aussi les tools, les interruptions et la véracité des résultats. Le frame WebSocket `session.update` envoie ensuite `instructions:t.instructions.text`, les tools et les réglages audio. A:80420-80447. Ceci prouve une construction et un envoi client; le respect de la persona à chaque phrase reste un comportement de modèle à mesurer.

### BP09. La voix et le modèle vocal sont des paramètres distincts

Le bot peut avoir `voiceId`, `voiceSpeed`, `voiceLanguage`. `SetGrokBotAgentVoiceRequest` et M `setVoice` transmettent ces valeurs indépendamment du profil. `session.update` applique une voix et une vitesse; l'URL realtime prend le modèle dans la credential fournie. P:53054; M:104106-104119; A:80405-80444.

La route vocale est choisie séparément: une thread teammate force `backend`; autrement la gate `sand_voice_server_harness`, ou l'override de dev, choisit backend ou package. Le package utilise le prompt local BP08. Le backend retourne lui-même instructions/tools/greeting via `VoiceCallHarnessSession`. A:75644-75669; M:48478-48509, 41310-41324; P:63423, 63539. On ne peut donc pas attribuer le prompt package à tous les appels ni au moteur texte.

### BP10. Skills, plugins et instructions MCP ajoutent des ressources de comportement

Les skills d'un bot portent nom, description et contenu. Le domaine client possède `addSkill`, `updateSkill`, `removeSkill`, `setPlugins`, `setPluginVariables`, puis les adapters appellent les RPC dédiés. Les plugins peuvent embarquer skills et configuration MCP. A:13089-13139; M:102997-103049; P:54172, 54201, 54466, 54517.

Les instructions personnalisées MCP appartiennent aux réglages de serveurs, identifiés par `server_id` ou par nom. Leur schéma n'est pas le profil de bot. P:50809, 50861. Le client fournit ces ressources au système, mais le choix de skills pour un tour, leur priorité face à la description et la composition du contexte texte restent dans le moteur distant.

### BP11. La mémoire et le partage ont des scopes propres

`GrokBotMemoryFolder` possède un texte `profile` et une map `logs`. La définition admin expose des shards avec `scope`, `scope_key`, `version`, `folder`; ces noms ne révèlent pas un fichier profil local ni la politique complète de consolidation. Le résumé d'équipe possède `memory_version`, `generated_at_ms`, `prose`, `summary_model`, et ses réponses joignent skills, faits learned et nombre de mémoires. P:46805, 61325, 54642, 54689.

L'UI liste les mémoires utilisateur d'un bot et promeut une sélection de `factIds` vers l'équipe. La réponse distingue `created`, `alreadyInTeam`, `keptPrivateCount`, donc le client ne présume pas que tout devient partagé. A:13067-13087; M:103051-103068. Les changements de visibilité et copies vers l'équipe sont des mutations séparées, avec les noms de secrets personnels à traiter. Les rows conservent `viewerIsOwner`, `viewerCanManage`, `viewerSessionId` et `teamId`. M:103922-103954, 103998-104047. Les règles d'extraction, de confidentialité et d'injection de mémoire dans le prompt ne sont pas récupérées ici.

### BP12. Le prompt texte complet et les réglages de modèle par bot restent inconnus

La recherche ne trouve pas de système de sliders de personnalité ou de champ modèle dans les schémas de création/édition retracés. Il existe bien un nom de configuration `grok_bot_system_prompt_override` avec `{basePrompt:string}` et fallback vide, mais ses occurrences relues sont la déclaration et le catalogue; aucun consommateur effectif de cette option n'est établi. M:57945-57947, 59826-59829.

De même, le champ `systemPrompt` de M:66996-67008 appartient à `sampling/createMessage` du SDK MCP. `model_override` de P:12257 appartient à `TaskParams`, et `computerUseModel` dans C:38718 est un réglage hôte de Computer Use. Ces champs partagés ne démontrent pas un modèle propre à chaque personnalité de bot. L'absence d'un champ modèle dans le profil ne prouve pas que le produit n'offre aucun réglage global de modèle; les niveaux global, outil, voix et bot doivent rester distincts. Aucune preuve de cette passe n'établit des poids adaptés pour un bot, le prompt système du moteur texte, son ordre de priorité description/mémoires/skills ou l'évolution automatique de la personnalité. La limite est le moteur absent, pas la lecture du profil client.

## Questions couvertes

| Question | État | Limite |
|---|---|---|
| Champs de profil et création | answered | Schéma client et DTO relus |
| Création UI vers gateway/backend | answered | Route, nonce, langue et introduction |
| Édition du profil et droits du lecteur | partial | Mutation/état client et route trouvés; enforcement serveur non testé |
| Personnalisation rôle/outils utilisateur | answered | Texte ajouté à la description dans ce launcher |
| Onboarding et templates | partial | Identité/version/setup établis; contenu distant non téléchargé |
| Persona vocale du package | answered | Prompt construit et voie d'envoi retracés, sans test du modèle |
| Persona vocale backend | partial | Contrat instructions retournées; contenu serveur inconnu |
| Skills/plugins/mémoires | partial | Ressources et scopes clients établis; sélection/injection distantes inconnues |
| Prompt système texte, modèle par bot et adaptation des poids | unresolved | Aucun mécanisme effectif récupéré dans cette passe |

La reconstruction peut reprendre les champs et les flows client documentés. Reproduire la personnalité écrite demande encore de choisir et de vérifier un moteur qui interprète la description, la mémoire et les skills, puis de mesurer son comportement sur des conversations réelles.
