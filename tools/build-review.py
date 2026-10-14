#!/usr/bin/env python3
"""Render the investigation as a local, repeatable Lavish review artifact."""
from pathlib import Path
import collections
import html
import json
import re
import shutil

root = Path(__file__).resolve().parents[1]
out = root / 'vue'
out.mkdir(parents=True, exist_ok=True)
parity = json.loads((root/'evidence/parity.json').read_text())
counts = json.loads((root/'evidence/protocol-contracts.json').read_text())['counts']
summary = json.loads((root/'evidence/rea-summary.json').read_text())
states = collections.Counter(row['status'] for row in parity['features'])
labels = {'static':'Trace client', 'contract':'Contrat partiel', 'documented':'Documentation', 'unknown':'Inconnu'}

md = ['# Parité et vérification de la reconstruction Rust', '',
  f"La matrice couvre {len(parity['features'])} familles. {states['static']} ont un chemin client retracé, {states['contract']} un contrat partiel, {states['documented']} un comportement documenté, et {states['unknown']} une implémentation inconnue.", '',
  'Ces statuts qualifient la preuve disponible. Ils ne mesurent pas un pourcentage de produit terminé. Aucun code Rust produit ni scénario authentifié exécuté ne valide la reconstruction à ce stade.', '',
  '| Fonction | Preuve disponible | Composant Rust proposé | Scénario de validation future |',
  '|---|---|---|---|']
for row in parity['features']:
    md.append('| '+' | '.join((row['feature'], labels[row['status']]+'; '+row['evidence'], row['rust'], row['test']))+' |')
md += ['', '## Phases de construction et critères de sortie', '',
  '| Phase | Périmètre | Preuve nécessaire avant la suivante |', '|---|---|---|',
  '| 1 | Identités, protocole, account runtime, journal et répliques | Replays et pannes déterministes; aucune contamination de compte; nonce stable |',
  '| 2 | Client natif et services de bots/conversations | Composeur riche, IME, accessibilité, profils, rooms, notifications et reprises |',
  '| 3 | Runs, modèle, ordinateur cloud, outils et approvals | Client fermé, panne worker, résultat externe incertain, leases d’écran et consentement |',
  '| 4 | Mémoire, connecteurs, skills, démonstrations et routines | Routines prompt-only, triggers provider, DST, révocation OAuth, save/discard |',
  '| 5 | Voix, meetings, fichiers, dépenses et hosted apps | Corpus de formats, audio repris, challenge de dépense, publication/version attendue |',
  '| 6 | Équipes, administration, egress, updates et autres clients | ACL serveur, budgets, tunnel à propriétaire unique, migrations et OS réels |', '',
  'Cette séquence répartit la livraison de toutes les familles. Les délais et coûts demandent des preuves d’implémentation et une capacité d’infrastructure réelle.', '',
  '## Ce qui reste à observer dans le produit original', '',
  'Les parcours après connexion et l’activation réelle des feature gates restent à capturer. Les internes du backend, de la fleet, des modèles, des helpers natifs et des clients mobiles restent absents ou non établis. La déduplication du client ne démontre pas une transaction serveur.', '',
  '## Référence commune de preuve', '',
  '`'+summary['parent_evidence_id']+'`', '',
  'Le bundle REA portable est `evidence/rea-bundle.json`. La couverture sémantique est partielle; le rapport conserve cette limite. Les tests ci-dessus sont des obligations futures, pas des tests passés.']
(root/'reports/parity.md').write_text('\n'.join(md)+'\n')

code_files = {}
excerpts = {}

def code_file(contents, name, language, description):
    key = 'code-'+str(len(code_files)+1)
    code_files[key] = {'name':name, 'contents':contents}
    return '<p class="code-caption">'+html.escape(description)+' · '+html.escape(language or 'texte')+'</p><div class="code-surface" data-code-file="'+key+'"><p class="muted">Le code se charge à l’ouverture.</p></div>'

def evidence_link(target):
    match = re.fullmatch(r'(.+):(\d+)', target)
    path, line = (match.group(1), int(match.group(2))) if match else (target, None)
    p = Path(path)
    if p.is_absolute() and str(p).startswith(str(root/'artifacts/formatted')+'/') and line and p.is_file():
        key = (str(p), line)
        if key not in excerpts:
            lines = p.read_text().split('\n')
            first, last = max(1,line-4), min(len(lines),line+17)
            ident = 'source-'+str(len(excerpts)+1)
            rel = str(p.relative_to(root))
            snippets = '\n'.join(lines[first-1:last])+'\n'
            excerpts[key] = {'id':ident,'path':rel,'line':line,'first':first,'last':last,'code':snippets}
        return '#'+excerpts[key]['id']
    if p.is_absolute() and str(p).startswith(str(root)+'/') and p.is_file():
        if p.suffix in ('.json','.md') and p.stat().st_size < 8*1024*1024:
            rel = p.relative_to(root)
            dest=out/rel
            dest.parent.mkdir(parents=True,exist_ok=True)
            shutil.copyfile(p,dest)
            return str(rel)
        return None
    if not p.is_absolute() and p.name in ('desktop.md','protocol.md','execution.md','architecture-rust.md','parity.md'):
        return '#report-'+p.stem
    if target.startswith(('https://','http://')):
        return target
    return None

def inline(text):
    tokens=[]
    def reserve(value):
        key='ZZTOKEN'+str(len(tokens))+'ZZ'
        tokens.append(value)
        return key
    text=re.sub(r'`([^`]+)`',lambda m:reserve('<code>'+html.escape(m.group(1))+'</code>'),text)
    def link(m):
        target=m.group(2).strip('<>')
        href=evidence_link(target)
        label=html.escape(m.group(1))
        return reserve('<a href="'+html.escape(href,quote=True)+'">'+label+'</a>' if href else '<span class="source-label" title="'+html.escape(target,quote=True)+'">'+label+'</span>')
    text=re.sub(r'\[([^\]]+)\]\(([^)]+)\)',link,text)
    text=html.escape(text)
    text=re.sub(r'\*\*([^*]+)\*\*',r'<strong>\1</strong>',text)
    for i,value in enumerate(tokens): text=text.replace('ZZTOKEN'+str(i)+'ZZ',value)
    return text

def markdown(text, filename):
    lines=text.split('\n'); blocks=[]; i=0
    while i<len(lines):
        line=lines[i]
        if not line.strip(): i+=1; continue
        if line.startswith('```'):
            lang=line[3:].strip(); i+=1; code=[]
            while i<len(lines) and not lines[i].startswith('```'): code.append(lines[i]);i+=1
            i+=1
            blocks.append(code_file('\n'.join(code)+'\n',filename+' / esquisse.'+('rs' if lang=='rust' else 'txt'),lang,'Bloc de '+filename+', esquisse ou référence'))
            continue
        heading=re.match(r'^(#{1,6})\s+(.+)',line)
        if heading:
            level=min(6,len(heading.group(1))+1)
            blocks.append(f'<h{level}>'+inline(heading.group(2))+f'</h{level}>'); i+=1;continue
        if line.startswith('|') and i+1<len(lines) and re.match(r'^\|?[\s:|\-]+$',lines[i+1]):
            def cells(s):return [c.strip() for c in s.strip().strip('|').split('|')]
            heads=cells(line);i+=2;rows=[]
            while i<len(lines) and lines[i].startswith('|'):rows.append(cells(lines[i]));i+=1
            blocks.append('<div class="table-wrap"><table><thead><tr>'+''.join('<th>'+inline(h)+'</th>' for h in heads)+'</tr></thead><tbody>'+''.join('<tr>'+''.join('<td data-label="'+html.escape(heads[min(j,len(heads)-1)],quote=True)+'">'+inline(c)+'</td>' for j,c in enumerate(row))+'</tr>' for row in rows)+'</tbody></table></div>')
            continue
        if re.match(r'^[-*]\s+',line) or re.match(r'^\d+\.\s+',line):
            ordered=bool(re.match(r'^\d+\.\s+',line));tag='ol' if ordered else 'ul';items=[]
            pattern=r'^\d+\.\s+' if ordered else r'^[-*]\s+'
            while i<len(lines) and re.match(pattern,lines[i]):items.append(re.sub(pattern,'',lines[i]));i+=1
            blocks.append('<'+tag+'>'+''.join('<li>'+inline(x)+'</li>' for x in items)+'</'+tag+'>');continue
        para=[line];i+=1
        while i<len(lines) and lines[i].strip() and not re.match(r'^(#|\||```|[-*]\s|\d+\.\s)',lines[i]):para.append(lines[i]);i+=1
        blocks.append('<p>'+inline(' '.join(para))+'</p>')
    return '\n'.join(blocks)

reports=[]
for name,label in [('architecture-rust','Architecture Rust détaillée'),('desktop','Client Mac et identité'),('protocol','Contrats et conversations'),('execution','Outils et ordinateur'),('parity','Parité et vérifications futures')]:
    p=root/'reports'/f'{name}.md'
    rendered=markdown(p.read_text(),f'reports/{name}.md')
    (out/'reports').mkdir(exist_ok=True)
    shutil.copyfile(p,out/'reports'/p.name)
    reports.append('<details class="report" id="report-'+name+'"><summary>'+label+'<span>Lire le rapport</span></summary><div class="report-body"><a class="download" href="reports/'+p.name+'" download>Télécharger le Markdown</a>'+rendered+'</div></details>')

excerpt_html=[]
for value in excerpts.values():
    p=value['path']
    description=f"{p} · ancre {value['line']} · contexte {value['first']} à {value['last']}. La numérotation du bloc est relative à cet extrait."
    excerpt_html.append('<details class="report" id="'+value['id']+'"><summary>Source '+str(value['line'])+'<span>'+html.escape(Path(p).name)+'</span></summary><div class="report-body">'+code_file(value['code'],p+' / extrait.cjs','JavaScript',description)+'</div></details>')

rows=''.join('<tr data-proof="'+r['status']+'" id="feature-'+r['id']+'"><td>'+html.escape(r['feature'])+'</td><td><span class="status '+r['status']+'">'+labels[r['status']]+'</span></td><td>'+html.escape(r['rust'])+'<small>'+html.escape(r['evidence'])+'</small></td><td>'+html.escape(r['test'])+'</td></tr>' for r in parity['features'])
template=(root/'tools/review-template.html').read_text()
replacements={'@@FEATURE_COUNT@@':str(len(parity['features'])),'@@RPC_COUNT@@':str(next(s for s in json.loads((root/'evidence/protocol-contracts.json').read_text())['services'] if s['typeName']=='aiserver.v1.GrokBotService')['methods'].__len__()),'@@MESSAGE_COUNT@@':f"{counts['messages']:,}".replace(',',' '),'@@ENUM_COUNT@@':str(counts['enums']),'@@ROWS@@':rows,'@@REPORTS@@':'\n'.join(reports),'@@EXCERPTS@@':'\n'.join(excerpt_html),'@@CODE_JSON@@':json.dumps(code_files,ensure_ascii=False).replace('<','\\u003c')}
for key,value in replacements.items():template=template.replace(key,value)
(out/'architecture.html').write_text(template)
print(json.dumps({'html':str(out/'architecture.html'),'feature_families':len(parity['features']),'code_blocks':len(code_files),'source_excerpts':len(excerpts),'proof_states':dict(states)},indent=2))
