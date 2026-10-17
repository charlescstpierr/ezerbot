#!/usr/bin/env python3
"""Build a reviewable ledger of shared resources and incident trace paths."""
from pathlib import Path
from evidence_ledger import Ledger
import json
ROOT=Path(__file__).resolve().parents[1]
REPORT='reports/goal-resources-traceability.md'
M='dist/electron-main/main-app.cjs'
C='dist/node-agent-coordinator/main.cjs'
D='dist/local-exec-daemon/main.cjs'
TARGET='inputs/Grok Bot.app/Contents/Helpers/Grok Bot Computer Use.app/Contents/MacOS/CUGrokBotService'
HASH='94a4a3d6965dd9f759c2fb811450fbe2a2194d14f538a351b945d2142d628b31'
def main():
 l=Ledger(REPORT,'Sujets 7 et 9 : partage de ressources, leases natives, corrélation des opérations et limites des traces.')
 rows=[]
 def js(path,start,end,literal):return l.excerpt(path,start,end,literal)
 def native(name,literal,margin=5):
  path='evidence/native-lease-'+name+'.asm.txt'
  lines=(ROOT/path).read_text().split('\n')
  idx=next(i for i,s in enumerate(lines) if literal in s)
  return l.artifact_excerpt(path,max(1,idx-margin+1),min(len(lines),idx+margin+1),literal,TARGET,HASH)
 def add(ident,title,claim,refs,limit,classification='observation'):
  l.finding(ident,claim,refs,limit,classification)
  rows.append((ident,title,claim,refs,limit,classification))
 add('R1','Un seul état de contrôle natif actif',
  'Le démarrage du bail de contrôle prend un NSLock, consulte un unique état active, puis compare le requestID. Si le même identifiant est déjà actif, il rend la session et la tâche existantes; un identifiant différent rend le tuple optionnel vide. Un démarrage neuf fabrique un UUID et installe la session.',
  [native('start_entry','Objc selector ref: lock',18),native('start_locked','100093eb0',45),native('start_locked','UUIDV10uuidString',9),native('start_locked','10009407a',16)],
  'Le refus décrit cette closure optimisée; le résultat public async et son texte d’erreur ne sont pas reconstruits intégralement. Ce bail appartient au helper Mac, pas au bureau cloud.', 'inference')
 add('R2','L’identité de session et l’activation conditionnent l’admission',
  'La closure admit compare le sessionID aux identifiants conservés, vérifie la présence de l’état actif et un drapeau de disponibilité, puis ne rend le permit actif que pour la session correspondante. La closure de fin d’activation marque ce drapeau ou appelle revokeLocked sur la branche d’échec.',
  [native('admit_locked','100094755',27),native('start_ready','10009446c',24)],
  'Les valeurs numériques de l’enum d’admission restent non nommées. Les branches sont visibles; aucun clic réel ni appel agent vers ce helper n’est observé.', 'inference')
 add('R3','Une libération vise sa session',
  'release compare le sessionID à la session stoppée puis à la session active. L’appel à revokeLocked n’est présent que sur la branche où la session active correspond; les autres branches rendent des variantes distinctes.',
  [native('release_locked','1000947ee',24),native('release_locked','1000948e3',12)],
  'Les noms exacts des variantes de retour ne sont pas récupérés; cette comparaison n’établit aucune politique de partage du navigateur cloud.', 'inference')
 add('R4','La disparition du titulaire révoque le bail',
  'watchHolderExit crée une Dispatch process source pour l’événement exit du PID fourni. Son callback prend le verrou, compare la session encore active à la session capturée, consigne un événement diagnostic, puis révoque seulement si elles correspondent.',
  [native('watch_holder','ProcessEventV4exit',11),native('holder_exit','10009499f',30)],
  'Le suivi est optionnel lorsque holderProcessID est absent. Le comportement du noyau et les courses réelles entre sortie et nouvelle session n’ont pas été testés.', 'inference')
 add('R5','La révocation nettoie aussi les entrées maintenues',
  'revokeLocked annule la source de surveillance, vide l’état actif, appelle CURemoteControlPermit.revoke qui rend un Set<CUHeldInput>, puis transmet cet ensemble à la closure releaseHeldInput et appelle onStopped. Les noms des champs conservés dans le Mach-O correspondent aux offsets utilisés.',
  [native('revoke_locked','100094643',25),native('revoke_locked','CURemoteControlPermitC6revoke',14),l.artifact_excerpt('evidence/native-lease-field-offsets.txt',1,7,'releaseHeldInput',TARGET,HASH)],
  'Cette séquence prouve le passage à un callback de nettoyage. L’émission effective des événements clavier ou souris par ce callback n’est pas reconstruite ici.', 'inference')
 add('R6','Le tunnel réseau négocie sa possession',
  'Le tunnel egress démarre en mode if-free. claim peut demander une reprise explicite. La connexion transmet l’identité client et la génération d’attache à reconquérir; elle ne se déclare connectée qu’après un ping, pong ou message reçu. Si un autre ordinateur détient le tunnel, le client détruit ses flux et passe en attente avant de recontrôler.',
  [js(M,93357,93379,'this.connect("if-free")'),js(M,93434,93465,'reclaimAttachGen'),js(M,93471,93488,'l.confirmed = !0'),js(M,93529,93570,'another computer holds the tunnel')],
  'Le serveur qui décide le titulaire et incrémente la génération n’est pas livré. Ce verrou réseau ne démontre aucun verrou de fichier ou d’écran.')
 add('R7','Les contrats de conflits fichiers ne garantissent pas leur activation',
  'Le SDK contient AgentStoreConflict avec cursor, journal_epoch, seq, événements de conflits et gap. Mais le constructeur actif oYe n’enregistre que shell, shell en arrière-plan, lecture et listing. Le notice executor partagé Coe rend ack/released/noted avec count 0. Aucun verrou global de fichier n’est établi par ces seuls contrats.',
  [js(D,95716,95848,'AgentStoreConflictSuccess'),js(D,97692,97720,'not-applicable'),js(D,128042,128105,'d.register(Sg')],
  'Le périmètre est la construction locale oYe et ses types compilés. Les uploads, Messages, services distants et autres builds peuvent avoir des politiques différentes.', 'inference')
 add('R8','Les uploads locaux sérialisent l’écriture par chemin',
  'L’upload actif P4 résout le chemin puis appelle R2. R2 prend une file P9t indexée par chemin absolu, attend son tour et la libère en finally. L’écriture passe par un fichier temporaire, fsync puis rename. Deux uploads du même chemin dans cette instance passent donc successivement par ce writer.',
  [js(D,128370,128385,'await R2'),js(D,78990,79010,'await i.ready'),js(D,79047,79064,'Mr.rename'),js(D,79193,79208,'hX.get')],
  'Cette file est une Map en mémoire du processus; le shell arbitraire ne passe pas nécessairement par ce writer. Elle ne fournit pas une version attendue, un historique de conflits ou un verrou global entre machines.')
 add('T1','Les opérations réseau héritent d’un identifiant de trace',
  'La gateway valide un traceparent au format version 00, rejette les identifiants nuls et crée un span enfant aléatoire de 8 octets. request l’ajoute aux headers HTTP POST et rend compte du succès ou de l’erreur du command span.',
  [js(C,40318,40365,'traceparent'),js(C,41341,41445,'this.reportCommandSpan')],
  'Une trace valide permet la corrélation. Elle ne contient pas à elle seule les instructions sélectionnées ni la raison sémantique d’une action.')
 add('T2','Les étapes d’envoi sont bornées et terminées une fois',
  'beginSend indexe les corrélations par accountSlot et clientNonce. Sans nonce, il renvoie un enregistreur inerte. beginStage utilise l’horloge monotone pour la durée et un drapeau V pour ne publier qu’une seule terminaison complete ou fail.',
  [js(C,45404,45414,'echo-coordinator-sse'),js(C,45473,45514,'if (C == null || C === "")')],
  'L’idempotence de la terminaison concerne ce rapport de mesure, pas l’effet externe de l’action.')
 add('T3','Une trace peut être perdue sous charge',
  'Le collecteur laisse au plus 64 rapports simultanément en vol et 64 corrélations echo. Les corrélations anciennes sont nettoyées après 120 000 ms lors de l’ajout suivant; au plafond, la plus ancienne est évincée. Les abandons in_flight_cap, dispatch_failed et echo_evicted sont comptés et journalisés avec une fenêtre de 60 000 ms.',
  [js(C,45405,45471,'in_flight_cap')],
  'Ces bornes sont celles de la télémétrie du coordinator. Elles ne constituent ni un budget de bot ni une garantie de conservation complète.')
 add('T4','L’échantillonnage dépend de la trace parente',
  'Le tracer sand-desktop-send est construit avec un AlwaysOffSampler racine enveloppé par un ParentBasedSampler. Les spans importent un contexte distant; le provider utilise BatchSpanProcessor et un exporteur muni d’un accès au token backend.',
  [js(M,107150,107187,'AlwaysOffSampler'),js(M,107189,107229,'isRemote: !0')],
  'L’export final dépend de l’échantillonnage, du token, de la connexion et du traitement backend. Le bundle n’établit pas leur rétention en production.')
 add('T5','Accusé de réception, rendu et résultat client sont distincts',
  'sand.send.ack consigne outcome, durée, fork, nombre de pièces jointes et identifiants. La télémétrie séparée renderTtfr signale le premier chunk affiché; turnClientOutcome inclut erreur, retryable, user_action, settle_basis et reconnected. Ces étapes évitent de confondre envoi accepté, interface mise à jour et résultat de travail.',
  [js(M,107250,107296,'sand.send.ack'),js(M,122650,122678,'settle_basis'),js(M,122723,122791,'render-first-chunk')],
  'L’issue du tour côté client n’est pas une vérification métier universelle de chaque action de bot.', 'inference')
 add('T6','Deux sources du transcript sont comparées',
  'bJ compare les entrées gateway et serveur avec une représentation JSON aux clés ordonnées. Il mesure matched, mismatched, missing, extra et staleTail; les messages streaming et tool-call pending sont traités comme transitoires. Le call site Et envoie reportTranscriptParity et le main classe les écarts en avertissement.',
  [js(C,43900,43964,'staleTail'),js(C,46020,46028,'reportTranscriptParity'),js(M,99547,99582,'transcript_store_parity')],
  'La comparaison porte sur les pages disponibles. Elle ne prouve ni l’intégralité des conversations ni l’égalité du contexte réellement envoyé au modèle.')
 add('T7','Un corps de transcript introuvable produit un signal explicite',
  'Le lecteur cherche les bodies omis, relit les lignes et récupère les blobs disponibles. Après la limite de cinq tentatives, il retire le pending body et émet onRowLost avec agentId, generation, seq, bodySource et attempts. Le signal est dédupliqué selon updatedSeq puis transféré à reportTranscriptRowLoss.',
  [js(C,35603,35612,'xs = 5'),js(C,35955,36028,'r.onRowLost'),js(C,45971,45977,'reportTranscriptRowLoss'),js(M,99889,99899,'transcriptTailRowLost')],
  'Le signal décrit une perte constatée à la lecture; le client ne démontre pas une récupération automatique du contenu définitivement absent.')
 add('T8','Le diagnostic de mise à jour possède un nettoyage spécifique',
  'WPn lit une queue du log ShipIt limitée à 5 000 000 octets, la passe à Gft, puis joint uniquement ShipIt_stderr.sanitized.log au message diagnostic. Gft retire chemins personnels, credentials URL, en-têtes auth, clés privées et familles de tokens; il conserve le hash SHA-256 de la queue brute, sa taille et le nombre de remplacements.',
  [js(M,123340,123413,'REDACTED:PRIVATE_KEY'),js(M,123414,123480,'rawTailSha256'),js(M,125350,125415,'ShipIt_stderr.sanitized.log')],
  'Ce nettoyage concerne le diagnostic ShipIt retracé. Il n’est pas une preuve de nettoyage de tous les logs, des réponses outils ou des contextes modèle.')
 checklist=[{'topic':7,'question_fr':'Comment les bots partagent-ils fichiers, navigateur et écran ?','status':'partially_answered','answered_fr':'Bail natif de session, sortie du titulaire, nettoyage des entrées et possession du tunnel retracés. Construction locale et contrat de conflits distingués.','unresolved_fr':'Arbitrage du bureau cloud, isolation des profils browser et conflits de fichiers entre bots distants.'},
 {'topic':9,'question_fr':'Peut-on reconstruire pourquoi une action a été prise ?','status':'partially_answered','answered_fr':'Corrélations réseau, étapes client, limites de livraison, parité et pertes de transcript retracées.','unresolved_fr':'Audit causal complet des instructions et de la sélection de contexte, décision du moteur texte et rétention serveur.'}]
 result=l.write('evidence/goal-resources-traceability.json',checklist)
 anchors={a['id']:a for a in l.anchors}
 def links(refs):
  out=[]
  for aid in refs:
   a=anchors[aid];source=l.sources[a['source']];path=Path(source['formatted_path']).relative_to(ROOT)
   out.append(f"[{aid}](../{path}:{a['formatted_start_line']})")
  return ', '.join(out)
 parts=['# Ressources partagées et traçabilité — approfondissement',
  'Le helper Mac possède un bail de contrôle attaché à une session. Le client sait corréler des étapes de transport et détecter certains écarts de transcript. Le partage du bureau cloud et l’audit des décisions du moteur texte restent partiellement inconnus.',
  'Cible : Grok Bot macOS 0.70.2 sauvegardé. Analyse statique des bundles et de huit corps de fonctions x86_64 du helper, sans exécuter l’application. Les branches Swift sont interprétées avec prudence; aucune équivalence arm64 n’est revendiquée. Les propositions Rust des rapports antérieurs ne sont pas des preuves du produit.',
  '## 7. Qui possède une ressource quand plusieurs acteurs travaillent ?']
 for ident,title,claim,refs,limit,cls in rows:
  if ident=='T1':parts.append('## 9. Quelles traces permettent de comprendre un incident ?')
  label='Interprétation statique' if cls=='inference' else 'Observation statique'
  parts.extend([f'### {ident}. {title}',f'**{label}.** {claim} Preuves : {links(refs)}.',f'**Limite.** {limit}'])
 parts.extend(['## Scénarios concrets',
  'Si un contrôleur Mac demande deux fois le même requestID alors que son bail est actif, la closure de démarrage réutilise sa session. Une demande concurrente différente prend une autre branche. Si le titulaire disparaît, le callback vérifie l’identité de session avant de révoquer, ce qui évite de révoquer aveuglément une session remplacée.',
  'Si un utilisateur cherche un message absent après reconnexion, une trace d’envoi ne suffit pas. Il faut relier clientNonce, résultat ack, affichage, génération du transcript et éventuel transcriptTailRowLost. Les plafonds et l’échantillonnage expliquent pourquoi une étape peut manquer même sans preuve d’échec de l’action.',
  '## Vérification et inconnues',
  '| Sujet | Couvert dans l’artefact | Encore absent |\n|---|---|---|\n| Ressources partagées | Bail helper, session, activation, décès du titulaire, nettoyage et tunnel egress | Arbitrage cloud, browser et fichiers entre bots |\n| Traçabilité | Nonce/traceparent, étapes, pertes, comparaison de pages et log de mise à jour nettoyé | Décision modèle, contexte exact et rétention serveur |',
  'Les corps natifs et leurs commandes sont conservés dans [native-lease-disassembly.json](../evidence/native-lease-disassembly.json). Le ledger [goal-resources-traceability.json](../evidence/goal-resources-traceability.json) relie chaque conclusion à un extrait hashé. Les scripts [inspect-native-lease.py](../tools/inspect-native-lease.py) et [build-goal-resource-evidence.py](../tools/build-goal-resource-evidence.py) permettent de reproduire ces preuves.',
  'La vérification vérifie les empreintes et citations de cette investigation. Elle n’est pas un test du service Grok en production.'])
 (ROOT/REPORT).write_text('\n\n'.join(parts)+'\n')
 print(json.dumps(result))
if __name__=='__main__':main()
