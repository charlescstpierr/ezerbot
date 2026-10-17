#!/usr/bin/env python3
"""Build the continuation report as a portable French review page."""
from pathlib import Path
import html
import json
import re
import shutil

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / '.lavish'
DEST = OUT / 'investigation-detail'
TOPICS = [
    ('personality', 'Personnalité', 'bot-personality-detail', 'La description du bot définit son rôle et sa façon de parler dans le prompt vocal retracé.',
     [('Profil','Nom, description, titre et avatar passent par les commandes de création.'),('Personnalité','L’onboarding enrichit la description; des templates apportent des instructions.'),('Contexte','Mémoire, skills et modèle restent des couches distinctes. Le prompt texte complet est distant.')]),
    ('cron', 'Routines et cron', 'cron-detail', 'Le client envoie des routines à la gateway. Le scheduler distant reste absent.',
     [('Configuration','Nom, prompt et déclencheur. Une skill reste optionnelle.'),('Horaires','Cron, @every, zone horaire et fenêtres exactes ou flexibles.'),('Événements','Webhooks et listeners provider. Les règles serveur de livraison restent inconnues.')]),
    ('computer', 'Son ordinateur', 'computer-detail', 'Son bureau cloud arrive par un viewer VNC. Ton Mac utilise une autre voie.',
     [('Connexion','Le backend localise ou prépare la box et fournit URL et tokens.'),('Affichage','La gateway fournit le statut; le client réécrit les URLs VNC main et fork.'),('Contrôle','Le bouton active la saisie du viewer. Ce callback ne démontre aucun verrou serveur.')]),
    ('compaction', 'Compaction', 'compaction-runtimes', 'Les contrats de résumé sont visibles. L’algorithme effectif de compaction reste inconnu.',
     [('Résumé','Configuration spéculative compilée, avec 70 % en valeur de repli.'),('Hook','pre_compact déclare tokens, fenêtre, messages concernés et modèle.'),('Limite','Aucun consommateur Grok établi. Pagination et mémoire ne prouvent pas une compaction.')]),
    ('delegation', 'Délégation', 'other-capabilities-detail', 'Le client suit les sous-agents et les workers cloud. Le moteur qui les lance est distant.',
     [('Sous-agents','Identité, type, statut et transcript. Lecture et suivi client actifs.'),('Bots et rooms','Contrats de messages et de tours, avec identités et corrélations.'),('Workers de code','ListWorkersForManager et suivi de diffs, PR et artifacts. Modèle distinct.')]),
    ('runtime', 'Runtimes', 'compaction-runtimes', 'Le runtime de compte, le coordinator et le daemon ont des cycles séparés.',
     [('Identité','Ports révoqués, version de transition et attente de sortie de l’ancien coordinator.'),('Deux routes','box par gateway; temporal par certaines commandes backend. Types inconnus refusés.'),('Reprise','Backoff, heartbeat, leases et resync. Reprise du run modèle non établie.')]),
    ('native', 'Ton Mac', 'native-helper-detail', 'Le daemon sert shell, fichiers et Messages. Le helper natif possède d’autres indices de capacités.',
     [('Daemon','Quatre exécutants actifs, transferts et Messages. Le shell n’est pas confiné par la racine fichiers.'),('Helper','Socket local, découverte service.json, symboles Messages et déclarations de permissions.'),('Limite','Outils graphiques compilés; aucun handler computer-use enregistré dans oYe.')]),
]

def esc(s):
    return html.escape(str(s), quote=True)

def evidence_href(raw, report):
    raw = raw.strip('<>')
    if raw.startswith(('http://','https://','#')):
        return raw
    match = re.fullmatch(r'(.+):(\d+)',raw)
    path,line = (match.group(1),match.group(2)) if match else (raw,None)
    src = Path(path)
    if not src.is_absolute():
        src = report.parent / src
    src = src.resolve()
    if not src.is_file():
        return '#sources'
    try:
        rel=src.relative_to(ROOT)
    except ValueError:
        return '#sources'
    if src.suffix in {'.json','.md','.py','.txt'} and src.stat().st_size < 2_000_000:
        dst=DEST/rel
        dst.parent.mkdir(parents=True,exist_ok=True)
        shutil.copyfile(src,dst)
        return 'investigation-detail/'+rel.as_posix()
    return '#sources'

def inline(s, report):
    tokens=[]
    def keep(v):
        tokens.append(v)
        return 'ZZINLINE'+str(len(tokens)-1)+'ZZ'
    s=re.sub(r'`([^`]+)`',lambda m:keep('<code>'+esc(m.group(1))+'</code>'),s)
    s=re.sub(r'\[([^\]]+)\]\(([^)]+)\)',lambda m:keep('<a href="'+esc(evidence_href(m.group(2),report))+'" title="'+esc(m.group(2))+'">'+esc(m.group(1))+'</a>'),s)
    s=esc(s)
    s=re.sub(r'\*\*([^*]+)\*\*',r'<strong>\1</strong>',s)
    for i,v in enumerate(tokens):
        s=s.replace('ZZINLINE'+str(i)+'ZZ',v)
    return s

def markdown(report):
    lines=report.read_text().splitlines()
    blocks=[]
    i=0
    while i<len(lines):
        s=lines[i]
        if not s.strip():
            i+=1
            continue
        h=re.match(r'^(#{1,6})\s+(.+)',s)
        if h:
            level=min(6,len(h[1])+2)
            blocks.append(f'<h{level}>'+inline(h[2],report)+f'</h{level}>')
            i+=1
            continue
        if s.startswith('|') and i+1<len(lines) and re.match(r'^\|?[\s:|\-]+$',lines[i+1]):
            def cells(row):
                result=[];current=[];code=False
                for char in row.strip().strip('|'):
                    if char=='`':code=not code
                    if char=='|' and not code:
                        result.append(''.join(current).strip());current=[]
                    else:current.append(char)
                result.append(''.join(current).strip())
                return result
            heads=cells(s)
            i+=2
            rows=[]
            while i<len(lines) and lines[i].startswith('|'):
                rows.append(cells(lines[i])); i+=1
            blocks.append('<div class="table-wrap"><table><thead><tr>'+''.join('<th>'+inline(h,report)+'</th>' for h in heads)+'</tr></thead><tbody>'+''.join('<tr>'+''.join('<td>'+inline(c,report)+'</td>' for c in row)+'</tr>' for row in rows)+'</tbody></table></div>')
            continue
        if s.startswith('```'):
            i+=1
            while i<len(lines) and not lines[i].startswith('```'):
                i+=1
            i+=1
            blocks.append('<p class="muted">Commandes et exemples disponibles dans le rapport source.</p>')
            continue
        if re.match(r'^[-*]\s+',s) or re.match(r'^\d+\.\s+',s):
            ordered=bool(re.match(r'^\d+\.\s+',s)); tag='ol' if ordered else 'ul'
            pattern=r'^\d+\.\s+' if ordered else r'^[-*]\s+'
            items=[]
            while i<len(lines) and re.match(pattern,lines[i]):
                items.append(re.sub(pattern,'',lines[i]));i+=1
            blocks.append('<'+tag+'>'+''.join('<li>'+inline(v,report)+'</li>' for v in items)+'</'+tag+'>')
            continue
        para=[s];i+=1
        while i<len(lines) and lines[i].strip() and not re.match(r'^(#|\||```|[-*]\s|\d+\.\s)',lines[i]):
            para.append(lines[i]);i+=1
        blocks.append('<p>'+inline(' '.join(para),report)+'</p>')
    return '\n'.join(blocks)

def main():
    OUT.mkdir(exist_ok=True)
    panels=[]
    nav=[]
    for index,(ident,label,name,lead,cards) in enumerate(TOPICS):
        report=ROOT/'reports'/(name+'.md')
        assert report.is_file(),report
        evidence_href(str(report),report)
        nav.append('<button role="tab" id="tab-'+ident+'" aria-controls="panel-'+ident+'" aria-selected="'+('true' if index==0 else 'false')+'" tabindex="'+('0' if index==0 else '-1')+'" data-topic="'+ident+'">'+esc(label)+'</button>')
        related={'delegation':'delegation-lifecycle-deep','native':'native-peer-deep'}.get(ident)
        diagram=''
        if ident=='personality':
            diagram='''<figure class="figure" id="persona-flow"><svg viewBox="0 0 900 155" role="img" aria-label="La description devient une instruction du modèle vocal"><g id="persona-description"><title>Description du bot fournie au plan vocal</title><rect class="node" x="10" y="20" width="250" height="85" rx="10"/><text x="135" y="56" text-anchor="middle" font-size="19">Description du bot</text><text x="135" y="81" text-anchor="middle" font-size="13">Rôle, langage et façon de parler</text></g><g id="persona-prompt"><title>Le client construit le prompt Role and Persona</title><rect class="node" x="325" y="20" width="250" height="85" rx="10"/><text x="450" y="56" text-anchor="middle" font-size="19">Prompt vocal local</text><text x="450" y="81" text-anchor="middle" font-size="13">Role &amp; Persona</text></g><g id="persona-model"><title>Les instructions sont transmises dans session.update</title><rect class="remote" x="640" y="20" width="250" height="85" rx="10"/><text x="765" y="56" text-anchor="middle" font-size="19">Modèle vocal distant</text><text x="765" y="81" text-anchor="middle" font-size="13">instructions de session.update</text></g><g id="persona-edge-input"><title>La description entre dans le prompt</title><path class="edge" d="M260 62H325" marker-end="url(#arrow)"/></g><g id="persona-edge-send"><title>Le client transmet les instructions à la session vocale</title><path class="edge" d="M575 62H640" marker-end="url(#arrow)"/></g></svg><figcaption class="caption">Voie vocale package retracée. La voie vocale backend fournit ses propres instructions. Le prompt texte complet reste absent.</figcaption></figure>'''
        extra=''
        if related:
            deep=ROOT/'reports'/(related+'.md')
            assert deep.is_file(),deep
            evidence_href(str(deep),deep)
            extra='<details class="report"><summary>Approfondissement : cycle de vie et contrôles</summary><div class="report-body">'+markdown(deep)+'</div></details>'
        panels.append('<section class="panel" role="tabpanel" id="panel-'+ident+'" aria-labelledby="tab-'+ident+'"'+('' if index==0 else ' hidden')+'><p class="eyebrow">'+esc(label)+'</p><h2>'+esc(lead)+'</h2>'+diagram+'<div class="cards">'+''.join('<article id="'+ident+'-claim-'+str(i)+'"><span class="number">'+str(i+1)+'</span><h3>'+esc(a)+'</h3><p>'+esc(b)+'</p></article>' for i,(a,b) in enumerate(cards))+'</div><details class="report"><summary>Lire l’analyse complète et ses preuves</summary><div class="report-body">'+markdown(report)+'</div></details>'+extra+'<a class="source" href="investigation-detail/reports/'+name+'.md">Rapport source en français</a></section>')
    all_evidence=['bot-personality-detail.json','cron-detail.json','computer-detail.json','other-capabilities-detail.json','compaction-runtime-detail.json','delegation-lifecycle-deep.json','native-deep-disassembly.json','uncovered-search.json','native-helper-macho.json','native-helper-plist.json','native-helper-symbols.json','detail-native-provider-status.json']
    for name in all_evidence:
        src=ROOT/'evidence'/name
        assert src.is_file(),src
        evidence_href(str(src),ROOT/'reports/compaction-runtimes.md')
    template=(ROOT/'tools/investigation-detail-template.html').read_text()
    source_links=''.join('<li><a href="investigation-detail/evidence/'+esc(name)+'">'+esc(name)+'</a></li>' for name in all_evidence)
    output=template.replace('__TABS__',''.join(nav)).replace('__PANELS__','\n'.join(panels)).replace('__EVIDENCE__',source_links)
    dest=OUT/'enquete-grokbot.html'
    dest.write_text(output)
    print(json.dumps({'output':str(dest.relative_to(ROOT)),'topics':len(TOPICS),'reports':len(set(t[2] for t in TOPICS)),'evidence':len(all_evidence)}))

if __name__=='__main__':
    main()
