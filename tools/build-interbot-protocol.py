#!/usr/bin/env python3
"""Project saved message contracts and preserve the iPhone/Mac findings."""
from pathlib import Path
import json,hashlib
from evidence_ledger import Ledger
ROOT=Path(__file__).resolve().parents[1]
P='dist/electron-main/proto.cjs';M='dist/electron-main/main-app.cjs'
C='dist/node-agent-coordinator/main.cjs';A='dist/renderer/assets/index.eager-app-BfVhIMYf.js'
REPORT='reports/interbot-mobile-protocol.md'
NAMES=['SendGrokBotAgentMessageRequest','SendGrokBotAgentMessageResponse','GrokBotRoomMemberTurnRoom','GrokBotRoomMemberTurnPeer','GrokBotRoomMemberTurnMessage','GrokBotRoomMemberTurnMessage.ReplyTarget','RequestGrokBotRoomMemberTurnRequest','RequestGrokBotRoomMemberTurnResponse','CancelGrokBotRoomMemberTurnRequest','CancelGrokBotRoomMemberTurnResponse','GrokBotRoomPost','DeliverGrokBotRoomMemberTurnResultRequest','DeliverGrokBotRoomMemberTurnResultResponse','RegisterSandMachineRequest','RegisterSandMachineResponse','SandMachine','ListSandMachinesResponse','SendGrokBotUserMessageRequest','GrokBotTranscriptCursor','WatchGrokBotTranscriptsRequest','ReportGrokBotNotificationPermissionRequest','ReportGrokBotDesktopActivityRequest']
def main():
 catalog=json.loads((ROOT/'evidence/protocol-contracts.json').read_text())
 messages=[next(m for m in catalog['messages'] if m['typeName']=='aiserver.v1.'+n) for n in NAMES]
 enums={f['type'] for m in messages for f in m['fields'] if f['kind']=='enum'}
 enum_names=['GrokBotRoomMemberTurnDispatch','GrokBotRoomMemberTurnOutcome','GrokBotRoomMemberTurnResultIntake','GrokBotNotificationPlatform']
 enums.update('aiserver.v1.'+n for n in enum_names)
 selected_enums=[e for e in catalog['enums'] if e['typeName'] in enums]
 methods=[{'service':s['typeName'],**m} for s in catalog['services'] for m in s['methods'] if m['rpcName'] in ['SendGrokBotAgentMessage','RequestGrokBotRoomMemberTurn','CancelGrokBotRoomMemberTurn','DeliverGrokBotRoomMemberTurnResult','RegisterSandMachine','ListSandMachines','WatchGrokBotTranscripts','SendGrokBotUserMessage']]
 pretty=(ROOT/'artifacts/formatted'/P).read_text()
 for m in messages:assert m['compactDescriptor'] in pretty
 projection={'schema':1,'method':'Projection of retained descriptor inventory, corroborated against unchanged formatted descriptor literals','target':'Grok Bot macOS 0.70.2','target_executed':False,'native_ios_artifact_available':False,
  'catalog_path':'evidence/protocol-contracts.json','catalog_sha256':hashlib.sha256((ROOT/'evidence/protocol-contracts.json').read_bytes()).hexdigest(),
  'inherited_inventory_method':catalog['descriptorRecovery'],'messages':messages,'enums':selected_enums,'methods':methods,
  'limits':['Contracts are not backend implementations or proof of active interbot calls.','Examples below are invented illustrations, not captured messages.','Native iPhone authentication, push and execution bodies are absent from this saved macOS target.'],
  'examples':{
   'direct':{'fromAgentId':'bot_analyste','toAgentId':'bot_relecteur','messageId':'exemple_msg_1','text':'Vérifie les trois chiffres du rapport.','sentAtMs':'1700000000000'},
   'reply':{'fromAgentId':'bot_relecteur','toAgentId':'bot_analyste','messageId':'exemple_msg_2','text':'Les trois chiffres concordent.','sentAtMs':'1700000005000','replyToId':'exemple_msg_1'},
   'room':{'nonce':'exemple_tour_1','room':{'id':'room_equipe','name':'Rapport','description':'Préparer le rapport ensemble'},'memberAgentId':'bot_relecteur','peers':[{'id':'bot_analyste','name':'Analyste','description':'Prépare les chiffres'}],'newMessages':[{'speakerKind':'AGENT','speakerName':'Analyste','isSelf':False,'text':'Vérifie les trois chiffres.'}],'isWindingDown':False,'deadlineMs':'1700000060000'}}}
 (ROOT/'evidence/interbot-message-contracts.json').write_text(json.dumps(projection,ensure_ascii=False,indent=2)+'\n')
 l=Ledger(REPORT,'Messages interbot, tours de room et connexion iPhone↔Mac accessibles dans le client sauvegardé.')
 rows=[]
 def ex(p,a,b,s):return l.excerpt(p,a,b,s)
 def add(i,title,claim,refs,limit,kind='observation'):
  l.finding(i,claim,refs,limit,kind);rows.append((i,title,claim,refs,limit,kind))
 add('W1','Un message direct est une enveloppe adressée',
  'Le request SendGrokBotAgentMessage contient from_agent_id, to_agent_id, message_id, text, sent_at_ms et reply_to_id optionnel. Il déclare un appel Unary de GrokBotService. Ce request ne porte ni historique complet, ni fichier joint, ni session_id, ni prompt système.',
  [ex(P,56648,56704,'SendGrokBotAgentMessageRequest'),ex(P,64761,64770,'sendGrokBotAgentMessage')],
  'Les champs absents de cette enveloppe peuvent être résolus ou enrichis côté serveur. Aucun initiateur client de cet RPC n’est retrouvé par la recherche littérale.', 'contract')
 add('W2','Livraison et réponse du bot sont deux résultats distincts',
  'La réponse annonce delivery, identité et nom de la cible, avec workflow_id optionnel. Les issues nommées sont DELIVERED_TEMPORAL, DELIVERED_BOX, DUPLICATE, TARGET_NOT_FOUND, FORBIDDEN, BOX_UNREACHABLE, TEMPORAL_UNAVAILABLE et INVALID_TARGET. Aucun texte de réponse du destinataire n’est dans ce DTO; reply_to_id peut relier un autre message.',
  [ex(P,46221,46242,'GrokBotAgentMessageDelivery'),ex(P,56677,56703,'workflow_id')],
  'Le stockage de déduplication, la clé exacte de DUPLICATE, les ACL et les garanties d’ordre ne sont pas récupérés. Une livraison nommée ne prouve pas une tâche accomplie.', 'contract')
 add('W3','Un tour de groupe transporte un contexte explicite',
  'Le tour désigne un membre par member_agent_id. Il reçoit room avec nom/description, peers avec ID/nom/description et new_messages. Chaque message décrit HUMAN ou AGENT, speaker_name, is_self, text et éventuellement une citation reply_to. La citation transporte à son tour type/nom du locuteur, is_self et quote. Le membre destinataire et les pairs possèdent des IDs; la projection de chaque locuteur porte surtout nom et rôle.',
  [ex(P,59072,59162,'GrokBotRoomMemberTurnMessage'),ex(P,59166,59200,'new_messages')],
  'Ce paquet ne révèle pas la mémoire ajoutée ensuite au modèle, ni la validation des noms, ni quels nouveaux messages le serveur sélectionne.', 'contract')
 add('W4','Le groupe possède trois phases corrélées',
  'Request demande un tour avec nonce, deadline_ms, is_winding_down, parent_request_id et root_parent_request_id. Dispatch distingue ACCEPTED/DUPLICATE et refus. Cancel vise nonce+membre avec raison et répond delivered. Deliver rend room+nonce+membre, outcome, messages texte, erreur et posts {text,message_json}; intake distingue ACCEPTED, UNKNOWN_NONCE et HOST_UNAVAILABLE. Les outcomes incluent SENT, PASS, SKIPPED, TIMEOUT, CANCELLED et ERROR.',
  [ex(P,46396,46446,'GrokBotRoomMemberTurnDispatch'),ex(P,59166,59199,'root_parent_request_id'),ex(P,59202,59271,'CancelGrokBotRoomMemberTurnRequest'),ex(P,59273,59383,'DeliverGrokBotRoomMemberTurnResultRequest')],
  'Le protocole prévoit ces phases. Il ne montre pas l’ordonnanceur, l’arbitrage des tours, l’application de la deadline ni la propagation de cancel. Les quatre noms RPC n’ont aucun appel direct littéral dans les 600 JS.', 'contract')
 add('I1','Le Mac reconnaît les appareils iOS du même compte',
  'Uf construit DashboardService avec accès au token, équipe et machine courante. Zu vérifie que le sujet/email du token reste celui du compte attendu. N5 lit ListSandMachines. NF conserve machineId, label, isCurrent, kind ios/android/desktop, connected et dates; le roster UI se lie à l’identité authentifiée et se réinitialise quand elle change.',
  [ex(M,48621,48650,'function NF'),ex(M,48690,48710,'listSandMachines'),ex(A,54150,54179,'bindAuth')],
  'Cela établit une liste de machines côté compte, pas la connexion native de l’iPhone ni une paire physique entre un téléphone et ce Mac.')
 add('I2','Enregistrement de machine et connexion du compte sont séparés',
  'RegisterSandMachineRequest accepte label, local_tool_permission, messages_enabled et kind. La réponse prévoit credential, expires_at_ms et credential_held. Le caller Mac B5 fournit son identité machine au client puis envoie label et permission sous contrôle du compte. NF ne recopie pas mobile_local_tools alors que ce tableau est dans SandMachine.',
  [ex(P,36688,36736,'RegisterSandMachineRequest'),ex(P,36753,36790,'mobile_local_tools'),ex(M,48639,48676,'registerSandMachine')],
  'Le caller Mac n’implémente pas l’enregistrement iOS. Ni l’obtention native du kind ios, ni le renouvellement de son credential ne sont visibles ici. Les outils mobiles ne peuvent pas être déduits de cette seule projection du roster.')
 add('I3','Les messages partagent des identités de conversation',
  'Le request utilisateur transporte agent_id, message_id, session_id, reply_to_id, task_id et machine_id optionnel. source distingue desktop/mobile; send_source distingue CHAT, PUSH, APP_INTENT et ACTION_BUTTON_DICTATION. Le performer z8e convertit ces champs en appel backend et traite ACCEPTED_BOX, ACCEPTED_TEMPORAL ou DUPLICATE comme une acceptation.',
  [ex(P,55724,55733,'SendGrokBotUserMessageRequest'),ex(M,110278,110320,'GrokBotClientSurface.MOBILE'),ex(M,110633,110684,'sendGrokBotUserMessage')],
  'Ce chemin actif est dans le Mac. Les variantes mobiles sont supportées par le contrat; le code iOS qui les produit n’a pas été inspecté. machine_id n’est pas une preuve de connexion directe au téléphone.')
 add('I4','La synchronisation Mac lit un flux authentifié du serveur',
  'resolveServerTranscriptTail refuse l’accès lorsque le compte n’est pas logged-in puis fournit baseUrl et headers. ND construit le transport depuis cette URL et ajoute les headers actualisés à chaque demande. Son watch appelle WatchGrokBotTranscripts. Le curseur porte agent_id, session_id, generation et after_updated_seq.',
  [ex(M,107309,107321,'logged-in'),ex(C,35004,35050,'r.resolveAccess'),ex(C,35043,35080,'watchGrokBotTranscripts'),ex(P,48147,48198,'after_updated_seq')],
  'Ce flux établit le chemin serveur→Mac. Son usage identique par l’application iPhone n’est pas démontré sans artefact iOS ou capture passive autorisée. Les agents box possèdent aussi leurs chemins gateway.')
 add('I5','La reprise utilise lignes, générations et relecture',
  'Le consommateur reçoit rows, cleared, cursorTooOld, agentState et rosterChanged. Il normalise la session, protège les retours contre une génération locale remplacée et réhydrate après cursorTooOld. Les curseurs de reconnexion repartent de streamAckUpdatedSeq, ou de zéro si l’état demande une réhydratation; les bodies en attente peuvent reculer ce curseur.',
  [ex(C,36336,36448,'cursorTooOld')],
  'Ces mécanismes synchronisent une projection de transcript côté Mac. Ils ne copient pas nécessairement tous les fichiers, réglages locaux, cookies ou états natifs entre appareils.')
 add('I6','La carte de permission mobile reste informative sur le Mac',
  'Le loader relie mobile-tool-permission au chunk DhxNwitm. Ce composant affiche titre, texte et une note otherDeviceNote ou otherDeviceGrantedNote selon permission.status. Il ne contient aucun handler local pour accorder cette permission; l’état granted modifie seulement sa note.',
  [ex('dist/renderer/assets/index-D6ReTfRE.js',123388,123396,'mobile-tool-permission'),ex('dist/renderer/assets/chunk-view-DhxNwitm.js',35,115,'otherDeviceGrantedNote')],
  'La permission OS sur iPhone et l’outil qui la demande restent côté application mobile. Une carte reçue sur Mac ne constitue pas une autorisation iOS exécutée.')
 add('I7','Notification, présence et transcript ont des circuits distincts',
  'Le protocole de notification porte plateforme/device_id/permission OS et connaît IOS. L’implémentation Hdt du Mac sélectionne uniquement MACOS/WINDOWS/LINUX, déduplique les rapports pendant 24 heures par compte/appareil/empreinte et appelle ReportGrokBotNotificationPermission. Tdt publie séparément l’activité desktop selon connexion, gate, verrouillage, idle, fenêtre, feed et bannières; le serveur retourne une durée de lease.',
  [ex(P,45683,45700,'GrokBotNotificationPlatform'),ex(M,116216,116306,'reportGrokBotNotificationPermission'),ex(M,115384,115445,'leaseMs / 2'),ex(M,115484,115497,'reportGrokBotDesktopActivity'),ex(M,115581,115615,'feedLive')],
  'Aucun motif d’enregistrement APNs spécifique n’est retrouvé par la recherche retenue. La décision serveur de notifier l’iPhone et la priorité entre appareils restent inconnues.')
 add('I8','Le Mac exécute les demandes qui lui sont adressées par le backend',
  'Le contrat user-computer utilise machine_id+credential pour watch/poll, ack_ids pour les livraisons et frames pour les résultats. Un appel d’outil sur le Mac suit donc un autre circuit que la simple lecture de conversation. Ces contrats sont raccordés au provider local décrit dans le rapport d’exécution précédent.',
  [ex(P,58338,58362,'WatchGrokBotUserComputerRequestsRequest'),ex(P,58448,58473,'ack_ids'),ex(P,58519,58546,'SubmitGrokBotUserComputerResponsesRequest')],
  'Cela ne prouve pas qu’un iPhone donné peut lancer n’importe quelle action sur ce Mac. L’ordonnanceur, les droits et le choix de la machine distante restent serveur.', 'contract')
 add('I9','Les termes pairing et Bluetooth trouvés ont d’autres consommateurs',
  'Les recherches de 600 JS donnent zéro appel direct des RPC interbot ciblés, zéro motif d’inscription APNs spécifique et zéro appel mobile bridge selon les motifs retenus. Les hits pairing renvoient surtout à 1Password ou au code de recherche; Bluetooth est une liste de routes audio; qrcode se trouve dans le worker PDF. Aucun de ces hits n’établit un appairage iPhone↔Mac.',
  [ex(M,40832,40853,'bluetooth_hfp'),ex(M,117756,117777,'OnePasswordBrokerConnectionState.PAIRING')],
  'C’est une recherche littérale bornée avec faux positifs examinés. Un appel calculé, une bibliothèque native, un autre artefact mobile ou un service distant peut porter le mécanisme manquant.', 'unknown')
 checklist=[{'question_fr':'Message direct entre bots','status':'partially_answered','known_fr':'Enveloppe, réponse, reply_to et affichage client retrouvés.','unknown_fr':'Initiateur et traitement backend, ACL et déduplication effectives.'},
 {'question_fr':'Dialogue de groupe','status':'partially_answered','known_fr':'Contexte de tour, corrélations, outcomes et cycle client.','unknown_fr':'Ordonnanceur, sélection et enrichissement modèle.'},
 {'question_fr':'Connexion iPhone↔Mac','status':'partially_answered','known_fr':'Compte/roster, identités, flux transcript Mac, permissions et contrats iOS.','unknown_fr':'App native iOS, auth/push/capture/exécution et éventuel autre transport.'}]
 search=json.loads((ROOT/'evidence/interbot-mobile-search.json').read_text())
 l.write('evidence/interbot-mobile-protocol.json',checklist,[{'path':'evidence/interbot-mobile-search.json','file_count':search['file_count'],'counts':{n:s['count'] for n,s in search['searches'].items()},'limitation_fr':'Motifs littéraux seulement, faux positifs explicités; aucune conclusion d’absence serveur.'}])
 anchors={a['id']:a for a in l.anchors}
 def links(refs):return ', '.join(f"[{a}](../artifacts/formatted/{anchors[a]['source']}:{anchors[a]['formatted_start_line']})" for a in refs)
 parts=['# Messages entre bots et connexion iPhone↔Mac',
  'Les échanges prévus entre bots sont des messages texte adressés et des tours de groupe corrélés. Le client sait les afficher et les relier. Pour iPhone↔Mac, les preuves côté Mac montrent surtout un compte, une liste d’appareils et des données servies par le backend. La partie native iOS n’est pas livrée dans la cible macOS sauvegardée.',
  'Cible : Grok Bot 0.70.2 sauvegardé. L’utilisateur a précisé iPhone. Aucun accès à son téléphone, compte ou secret et aucune exécution de la cible n’ont été effectués. Les contrats ont été projetés depuis l’inventaire antérieur, dont la récupération évaluait un module protobuf contrôlé; cette passe ne le réévalue pas. Les littéraux sont recorroborés dans les copies LF et les originaux restent hashés.',
  'Le parent REA portable a été réimporté sans nouvelle analyse globale. La trace ciblée [ev_c3c38ce2965b11faa9c65a2f220209937ce842f10376661ad1fc07d32c8650cd](../evidence/interbot-rea-trace.json) n’a pas de match sur son graphe partiel; elle n’est pas une preuve d’absence. Les observations ci-dessous suivent donc les artefacts et leurs consommateurs.',
  '## Comment les bots se parlent']
 for i,title,claim,refs,limit,kind in rows:
  if i=='I1':parts.append('## Comment le Mac et l’iPhone retrouvent les mêmes objets')
  label={'contract':'Contrat compilé','unknown':'Recherche bornée','observation':'Observation statique'}[kind]
  parts.extend([f'### {i}. {title}',f'**{label}.** {claim} Sources : {links(refs)}.',f'**Limite.** {limit}'])
 parts.extend(['## Exemple de dialogue',
  'Exemple inventé à partir des champs récupérés, sans appel réel : le bot Analyste envoie « Vérifie les trois chiffres du rapport » au bot Relecteur, avec un messageId. Relecteur peut envoyer un autre message vers Analyste avec replyToId égal au premier ID. Le retour de livraison du premier message ne contient pas cette réponse du Relecteur. Dans un groupe, le serveur demande séparément un tour au membre choisi et attend un résultat corrélé par nonce.',
  'Les paquets illustratifs et leurs types sont dans [interbot-message-contracts.json](../evidence/interbot-message-contracts.json). Ils ne sont ni une capture réseau ni un moteur serveur reconstruit.',
  '## Lecture pratique iPhone↔Mac',
  'L’hypothèse la mieux soutenue est une continuité par les services du compte : chaque client retrouve agents et sessions côté serveur, puis maintient sa projection locale. Le Mac n’a pas besoin de recevoir directement le texte depuis le téléphone pour afficher une modification serveur. La connexion propre de l’iPhone, son transport de fond et son enregistrement push demandent encore l’artefact iOS ou une observation passive ciblée.',
  'Un ordre donné depuis un appareil peut aussi déclencher un outil sur une autre machine, mais ce serait une chaîne distincte : conversation → décision serveur → demande user-computer adressée → exécuteur Mac → résultat. Les permissions, la disponibilité et le choix du Mac interviennent dans cette chaîne; la synchronisation du texte ne les remplace pas.',
  '## Checklist et preuves manquantes',
  '| Question | Établi | Encore manquant |\n|---|---|---|\n| Messages directs | DTO, livraison, réponses liées et consommateurs d’affichage | Initiateur backend, ACL, ordre et déduplication |\n| Groupes | Membres, contexte de tour, corrélations et résultats | Ordonnanceur et contexte modèle enrichi |\n| iPhone↔Mac | Roster du compte, source mobile, sessions et flux Mac | App native iOS, authentification, APNs et éventuel transport supplémentaire |',
  'Les trois sujets sont `partially_answered`. Pour résoudre les frontières restantes, il faut les corps du service distant ou des captures passives de scénarios explicitement délimités; changer de décompilateur ne peut pas recréer des corps serveur absents de l’app.',
  'Voir aussi [consommateurs interbot](interbot-client-paths.md), [contexte restant](server-context-residual.md), [orchestration restante](server-orchestration-residual.md), le [ledger de cette lecture](../evidence/interbot-mobile-protocol.json) et les [recherches sur 600 scripts](../evidence/interbot-mobile-search.json).'])
 (ROOT/REPORT).write_text('\n\n'.join(parts)+'\n')
 print(json.dumps({'findings':len(l.findings),'anchors':len(l.anchors),'messages':len(messages),'enums':len(selected_enums)}))
if __name__=='__main__':main()
