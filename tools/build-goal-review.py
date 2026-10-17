#!/usr/bin/env python3
"""Consolidate the nine requested topics and build an offline review page."""
from pathlib import Path
import json,html,re,importlib.util,hashlib
ROOT=Path(__file__).resolve().parents[1]
GROUPS=[('context-memory-learning','Contexte, mémoire et apprentissage'),('execution-autonomy','Exécution et autonomie'),('permissions-secrets','Permissions et secrets'),('resources-traceability','Ressources et traçabilité')]
TOPICS=[
 (1,'context','Contexte et instructions','context-memory-learning','Le client compose un prompt vocal et filtre ce qu’il rappelle.',
  'La voie vocale package possède un registre d’outils réel, un rappel de huit textes et des filtres de session/auteur. Les fichiers du message partent comme références. Une priorité est formulée dans le prompt vocal.',
  'Le prompt texte complet, la sélection des faits et skills, l’extraction des fichiers et l’ordre effectif des instructions restent distants.'),
 (2,'memory','Mémoire et oubli','context-memory-learning','Recommencer une conversation conserve les ressources durables.',
  'Le client liste et promeut des faits par bot, conserve les résultats partiels et différencie la décision de carte de la mutation. Reset et oubli durable ont des portées différentes.',
  'Extraction des faits, correction, suppression, propagation de l’oubli, rétention et droits privés/équipe ne sont pas entièrement établis.'),
 (3,'uncertain','Actions incertaines','execution-autonomy','Une réponse perdue peut laisser une action déjà exécutée.',
  'Les reprises d’envoi utilisent nonce et statut. Certains handlers MCP avertissent explicitement qu’un timeout peut arriver après l’effet externe. Les caches de déduplication locaux ont leurs propres portées.',
  'Aucune garantie universelle d’exécution unique des effets shell, fichiers et connecteurs n’est prouvée. Les transactions serveur restent absentes.'),
 (4,'loop','Boucle outils et fin','execution-autonomy','La boucle vocale est livrée et reliée à ses consommateurs.',
  'Un appel de fonction passe au harness, son résultat retourne comme function_call_output, puis le client arbitre la réponse parlée. La fin d’un appel et la fin d’un travail sont des événements différents.',
  'Choix des outils, replanification et critère de réussite métier du moteur texte restent inconnus.'),
 (5,'permissions','Permissions et secrets','permissions-secrets','Les autorisations et les magasins de secrets suivent des routes distinctes.',
  'Les cartes relient demande, décision et règle persistée. Les échecs de sauvegarde peuvent retomber sur une autorisation unique. Les secrets de bot sont envoyés au backend; le magasin local chiffré est un autre circuit.',
  'Révoquer une demande ne prouve pas l’arrêt d’un effet déjà lancé. La visibilité finale des secrets par le modèle n’est pas établie.'),
 (6,'budgets','Budgets et limites','execution-autonomy','Des délais et plafonds locaux existent pour les outils et le transport.',
  'Le MCP backend distingue un délai de 3 minutes et deux exceptions à 45 minutes. Le provider utilisateur borne ses requêtes actives et son backlog. Les tailles de sorties et transferts ont d’autres limites.',
  'Tours, tokens, coût, durée totale d’un bot et concurrence des sous-agents distants ne se déduisent pas de ces plafonds locaux.'),
 (7,'resources','Ressources partagées','resources-traceability','Le helper Mac lie le contrôle à une session et au processus titulaire.',
  'Le bail natif vérifie identité et activation, surveille la sortie du titulaire et passe les entrées maintenues au nettoyage. Le tunnel réseau négocie une possession avec une génération d’attache.',
  'Le partage des fichiers, des profils navigateur et des écrans entre bots cloud reste incomplet.'),
 (8,'learning','Démonstrations et skills','context-memory-learning','Teach, publication et synchronisation sont plusieurs mécanismes.',
  'Le client suit start, save, discard et réconciliation de la démonstration. La bibliothèque publie puis relit les workflows. Les fichiers optionnels de skill disparaissent dans un adapter protobuf retracé.',
  'La capture, recording→skill, validation, historique de versions, rollback et sélection de la skill au tour restent serveur.'),
 (9,'traceability','Traces et audit','resources-traceability','Les traces relient des étapes, avec pertes et échantillonnage possibles.',
  'Nonce et traceparent relient envoi, ack et affichage. Le client compare deux sources de transcript et signale les corps introuvables. Certaines files de télémétrie sont bornées et peuvent abandonner des rapports.',
  'Le contexte exact du modèle, la cause complète d’une décision et la rétention des traces serveur ne sont pas récupérés.'),
]
def esc(s):return html.escape(str(s),quote=True)
def main():
 ledgers=[]
 for name,label in GROUPS:
  p=ROOT/'evidence'/('goal-'+name+'.json');value=json.loads(p.read_text())
  ledgers.append({'name':name,'label':label,'report':value['report'],'ledger':str(p.relative_to(ROOT)),
    'sha256':hashlib.sha256(p.read_bytes()).hexdigest(),'findings':len(value['findings']),'anchors':len(value['anchors'])})
 topics=[{'number':n,'id':i,'title_fr':title,'group':g,'answer_fr':lead,'established_fr':known,'unresolved_fr':unknown,'status':'partially_answered','report':'reports/goal-'+g+'.md','ledger':'evidence/goal-'+g+'.json'} for n,i,title,g,lead,known,unknown in TOPICS]
 aggregate={'schema':1,'target':'Grok Bot 0.70.2 saved macOS artifact','scope':'Nine requested client-side investigations with explicit server/runtime unknowns',
  'target_executed':False,'authenticated_runtime_verified':False,'rust_changes_performed':False,'topics':topics,'ledgers':ledgers,
  'counts':{'topics':len(topics),'reports':len(ledgers),'findings':sum(a['findings'] for a in ledgers),'anchors':sum(a['anchors'] for a in ledgers),'new_native_function_bodies':8},
  'native_evidence':'evidence/native-lease-disassembly.json'}
 (ROOT/'evidence/goal-nine-topics.json').write_text(json.dumps(aggregate,ensure_ascii=False,indent=2)+'\n')
 counts=aggregate['counts']
 parts=['# Les neuf sujets importants — résultats de l’approfondissement',
  f"Les neuf sujets ont été investigués dans Grok Bot 0.70.2. Cette passe conserve {counts['findings']} constats et {counts['anchors']} extraits hashés dans quatre rapports, avec huit nouveaux corps de fonctions natives désassemblés. Elle retrace les chemins client accessibles et nomme les règles encore inconnues côté serveur.",
  'Cette enquête utilise la copie sauvegardée et les preuves REA antérieures. Aucun compte, modèle ou action réelle de Grok n’a été exécuté. Les citations vérifient les artefacts livrés. « Chemin actif » signifie relié statiquement à un constructeur ou handler; cela ne signifie pas exécuté pendant cette session.',
  '## Ce qui change la compréhension du système',
  'Une réponse réseau perdue peut suivre une action déjà exécutée. Le client sait reprendre certains envois, mais cette reprise ne garantit pas l’unicité de tous les effets externes. Les limites locales des outils ne sont pas un budget global d’autonomie. La boucle vocale est visible; le moteur texte qui choisit, replanifie et juge le travail reste distant.',
  'Le rappel vocal sélectionne huit textes admissibles dans le transcript. Il ne constitue pas la mémoire durable. Reset conserve mémoire, fichiers et routines selon le message livré. Teach possède son propre cycle de sauvegarde; la fabrication puis la validation d’une skill ne sont pas récupérées dans le client.',
  'Le helper Mac possède un bail de contrôle de session et un nettoyage lors de la disparition du titulaire. Les traces de transport relient les étapes et signalent certains écarts, mais leurs bornes et leur échantillonnage empêchent de les prendre pour un audit exhaustif.',
  '## Réponses par sujet']
 for t in topics:
  parts.extend([f"### {t['number']}. {t['title_fr']}",t['answer_fr'],f"**Établi côté client.** {t['established_fr']}",
   f"**Encore inconnu.** {t['unresolved_fr']}",f"Preuves et détails : [{t['title_fr']}](goal-{t['group']}.md). Statut de connaissance : `partially_answered`."])
 parts.extend(['## Rapports et preuves',
  '| Rapport | Constats | Extraits | Ledger |\n|---|---:|---:|---|\n'+'\n'.join(f"| [{a['label']}](../{a['report']}) | {a['findings']} | {a['anchors']} | [JSON](../{a['ledger']}) |" for a in ledgers),
  '[Vue navigable des neuf sujets](../.lavish/goal-neuf-sujets.html). La [passe précédente](investigation-continuation.md) conserve cronjobs, ordinateurs, compaction, délégation, runtimes et personnalité. Les résultats se complètent; la nouvelle passe ne remplace pas ces preuves.',
  '## Reproduire la vérification',
  '```sh\npython3 tools/evidence_ledger.py evidence/goal-context-memory-learning.json evidence/goal-execution-autonomy.json evidence/goal-permissions-secrets.json evidence/goal-resources-traceability.json\npython3 tools/verify-goal-investigation.py\npython3 tools/verify-dossier.py\n```',
  'Le vérificateur de goal contrôle les quatre ledgers, leurs empreintes, les corps natifs, les offsets des champs et la couverture exacte des neuf sujets. Le vérificateur du dossier relit les 673 fichiers expédiés. Ces commandes ne testent pas un runtime authentifié.',
  '## Limite de clôture',
  'Le travail demandé est réalisé comme enquête sur les artefacts accessibles, avec rapports et preuves vérifiables. Il ne constitue pas une récupération des serveurs privés ni une preuve d’équivalence du produit Rust. L’algorithme réel de compaction, le moteur texte, les garanties de transaction et le chemin complet démonstration→skill restent explicitement ouverts.'])
 (ROOT/'reports/goal-nine-topics.md').write_text('\n\n'.join(parts)+'\n')
 spec=importlib.util.spec_from_file_location('detail',ROOT/'tools/build-investigation-detail.py')
 detail=importlib.util.module_from_spec(spec);spec.loader.exec_module(detail)
 css=re.search(r'<style>([\s\S]*?)</style>',(ROOT/'tools/investigation-detail-template.html').read_text()).group(1)
 nav=[];panels=[]
 for idx,t in enumerate(topics):
  nav.append(f"<button role='tab' id='tab-{t['id']}' aria-controls='panel-{t['id']}' data-topic='{t['id']}' aria-selected='{str(idx==0).lower()}' tabindex='{0 if idx==0 else -1}'>{t['number']} · {esc(t['title_fr'])}</button>")
  report=ROOT/t['report'];detail.evidence_href(str(report),report)
  panels.append(f"<section class='panel' role='tabpanel' id='panel-{t['id']}' aria-labelledby='tab-{t['id']}'{'' if idx==0 else ' hidden'}><p class='eyebrow'>Sujet {t['number']} · connaissance partielle</p><h2>{esc(t['answer_fr'])}</h2><div class='boundary'><article id='{t['id']}-known'><h3>Établi dans le client</h3><p>{esc(t['established_fr'])}</p></article><article id='{t['id']}-unknown'><h3>Ce que le package ne révèle pas</h3><p>{esc(t['unresolved_fr'])}</p></article></div><a href='investigation-detail/{t['report']}'>Lire le rapport et ses citations</a></section>")
 details=[];sources=[]
 for a in ledgers:
  report=ROOT/a['report'];detail.evidence_href(str(report),report);detail.evidence_href(str(ROOT/a['ledger']),report)
  details.append(f"<details class='report' id='report-{a['name']}'><summary>{esc(a['label'])} · {a['findings']} constats · {a['anchors']} extraits</summary><div class='report-body'>{detail.markdown(report)}</div></details>")
  sources.append(f"<li><a href='investigation-detail/{a['ledger']}'>{esc(a['label'])} — ledger</a></li>")
 for path in ['reports/goal-nine-topics.md','evidence/goal-nine-topics.json','evidence/native-lease-disassembly.json','evidence/native-lease-field-offsets.txt','tools/inspect-native-lease.py','tools/evidence_ledger.py','tools/verify-goal-investigation.py']:
  assert (ROOT/path).is_file(),path
  detail.evidence_href(str(ROOT/path),ROOT/'reports/goal-nine-topics.md')
  sources.append(f"<li><a href='investigation-detail/{path}'>{esc(path)}</a></li>")
 body=f"""<!doctype html><html lang='fr' data-theme='light'><head><meta charset='utf-8'><meta name='viewport' content='width=device-width,initial-scale=1'><title>Grok Bot — les neuf sujets approfondis</title><style>{css}</style></head><body><main>
 <header><span class='brand'>Grok Bot · approfondissement</span><div class='head-actions'><span class='badge'>0.70.2 sauvegardé</span><button id='theme' aria-label='Passer au thème sombre'>Thème sombre</button></div></header>
 <p class='eyebrow'>Neuf questions, preuves conservées</p><h1>Ce qui se passe entre une demande et le travail du bot.</h1><p class='lead'>Contexte, mémoire, effets incertains, outils, permissions, budgets, ressources, démonstrations et traces. Chaque sujet montre ce que le client livre et ce que le serveur garde.</p>
 <div class='note' id='goal-proof'><strong>{counts['findings']} constats · {counts['anchors']} extraits · 8 nouveaux corps natifs.</strong> Analyse statique vérifiable. Aucun scénario authentifié de l’application originale n’a été exécuté.</div>
 <figure class='figure' id='uncertain-effect'><svg viewBox='0 0 740 215' role='img' aria-label='Une action peut réussir et sa réponse être perdue'><defs><marker id='arrow' viewBox='0 0 10 10' refX='9' refY='5' markerWidth='6' markerHeight='6' orient='auto'><path d='M0 0L10 5L0 10z' fill='var(--accent)'/></marker></defs>
 <g id='send'><title>Le client transmet la demande au connecteur</title><rect class='node' x='10' y='35' width='205' height='90' rx='12'/><text x='112' y='73' text-anchor='middle' font-size='21'>Demande envoyée</text><text x='112' y='100' text-anchor='middle' font-size='15'>Identité de requête</text></g>
 <g id='external-effect'><title>Le connecteur peut avoir appliqué l’action avant la réponse</title><rect class='remote' x='267' y='35' width='205' height='90' rx='12'/><text x='369' y='73' text-anchor='middle' font-size='21'>Action appliquée</text><text x='369' y='100' text-anchor='middle' font-size='15'>Chez le fournisseur</text></g>
 <g id='timeout'><title>Le handler rend un timeout et avertit que l’action peut avoir eu lieu</title><rect class='unknown' x='525' y='35' width='205' height='90' rx='12'/><text x='627' y='73' text-anchor='middle' font-size='21'>Réponse perdue</text><text x='627' y='100' text-anchor='middle' font-size='15'>Résultat incertain</text></g>
 <g id='request-edge'><title>Envoi de la demande</title><path class='edge' d='M215 80H267' marker-end='url(#arrow)'/></g><g id='response-edge'><title>La réponse peut ne pas parvenir avant le délai</title><path class='edge' d='M472 80H525' stroke-dasharray='5 5' marker-end='url(#arrow)'/></g><text x='369' y='181' text-anchor='middle' font-size='18'>Un timeout ne prouve pas l’absence d’effet.</text></svg><figcaption class='caption'>Scénario possible explicitement signalé par certains handlers MCP du client. Les garanties dépendent du chemin et du fournisseur.</figcaption></figure>
 <nav class='tabs' role='tablist' aria-label='Neuf sujets approfondis'>{''.join(nav)}</nav>{''.join(panels)}
 <section id='full-reports'><p class='eyebrow'>Détails et preuves</p><h2>Quatre rapports pour relire chaque constat.</h2>{''.join(details)}</section>
 <footer id='sources'><details><summary>Ledgers et scripts reproductibles</summary><ul>{''.join(sources)}</ul></details><p><a href='investigation-detail/reports/goal-nine-topics.md'>Synthèse source</a> · <a href='enquete-grokbot.html'>Cron, ordinateurs, compaction, délégation, runtimes et personnalité</a></p><p>Design repris des tokens CSS du client Grok Bot : fonds neutres, typographie système et accent bleu. Sources volumineuses conservées dans le dossier, sans embarquer les bundles dans la page.</p></footer>
 </main><script>const tabs=[...document.querySelectorAll('[data-topic]')];function selectTopic(id,focus=false){{for(const tab of tabs){{const on=tab.dataset.topic===id;tab.setAttribute('aria-selected',String(on));tab.tabIndex=on?0:-1;document.getElementById('panel-'+tab.dataset.topic).hidden=!on;if(on&&focus)tab.focus()}}history.replaceState(null,'','#'+id)}}for(const tab of tabs){{tab.addEventListener('click',()=>selectTopic(tab.dataset.topic));tab.addEventListener('keydown',e=>{{if(!['ArrowLeft','ArrowRight','Home','End'].includes(e.key))return;e.preventDefault();let i=tabs.indexOf(tab);i=e.key==='Home'?0:e.key==='End'?tabs.length-1:(i+(e.key==='ArrowRight'?1:-1)+tabs.length)%tabs.length;selectTopic(tabs[i].dataset.topic,true)}})}}const initial=location.hash.slice(1);if(tabs.some(t=>t.dataset.topic===initial))selectTopic(initial);document.getElementById('theme').addEventListener('click',e=>{{const dark=document.documentElement.dataset.theme!=='dark';document.documentElement.dataset.theme=dark?'dark':'light';e.currentTarget.textContent=dark?'Thème clair':'Thème sombre';e.currentTarget.setAttribute('aria-label',dark?'Passer au thème clair':'Passer au thème sombre')}});</script></body></html>"""
 (ROOT/'.lavish/goal-neuf-sujets.html').write_text(body)
 print(json.dumps(aggregate['counts']))
if __name__=='__main__':main()
