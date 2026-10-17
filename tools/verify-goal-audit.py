#!/usr/bin/env python3
"""Verify research-audit artifacts. This does not prove hidden product behavior."""
from pathlib import Path
import argparse
import datetime
import hashlib
import json
import os
import re

ROOT = Path(__file__).resolve().parents[1]
AUDIT = ROOT / 'evidence/goal-evidence-audit.json'
REPORT = ROOT / 'reports/goal-evidence-audit.md'
IDS = {'server_engine', 'context_compaction', 'memory_forgetting', 'interbot', 'crash_effects',
       'iphone_native', 'teaching', 'scheduler', 'native_helpers'}


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def render(audit, access):
    def link(path):
        return f'[{Path(path).name}]({os.path.relpath(ROOT / path, REPORT.parent)})'

    lines = ['# Bilan de l’investigation REA et preuves encore manquantes', '',
        'L’investigation a réduit les inconnues sur les neuf sujets et conserve leurs preuves. '
        'Les exigences de recherche sont remplies ; la connaissance du produit reste partielle. '
        'Cette conclusion ne prétend ni récupérer le moteur fermé, ni terminer la parité de notre app.', '',
        'Les preuves statiques, les déclarations documentaires, le démarrage observé et les tranches '
        'originales exécutées dans un test sont distingués. Une hypothèse de backend ne remplace jamais un corps ou une trace.', '',
        '## Audit de l’objectif', '', audit['objective'], '',
        'Chaque exigence de travail a été confrontée aux fichiers actuels, aux inventaires et aux revues indépendantes :', '']
    for criterion in audit['research_requirements']:
        lines.append(f"- {criterion['requirement']}")
    lines += ['', 'Les correspondances détaillées sont conservées dans ' + link('evidence/goal-evidence-audit.json')
        + '. Les trois revues sont des analyses, pas de nouvelles observations de la cible : '
        + link('evidence/goal-scope-review.json') + '.', '',
        '## État des neuf questions', '']
    for question in audit['questions']:
        status = 'Non résolu' if question['knowledge_status'] == 'unresolved' else 'Partiel'
        lines += [f"### {question['label']} : {status}", '',
            question['established'] + ' Sources : ' + ', '.join(link(p) for p in question['reports']) + '.', '',
            question['unproven'], '', 'Preuve nécessaire : ' + question['required_external_evidence'], '']
    lines += ['## Accès réellement vérifié', '',
        'La vérification datée ' + access['recorded_at_utc'] + ' ne trouve aucun fichier IPA/APK/HAR/mitm/flow '
        'dans les deux racines sélectionnées. Les six outils iPhone cherchés ne sont pas disponibles. '
        'La requête USB ne trouve aucun produit nommé iPhone. Ces résultats sont bornés aux chemins, suffixes, '
        'outils et noms interrogés ; ils ne décrivent pas tous les dossiers ou appareils possibles.', '',
        'Aucun numéro de série, profil, jeton ou compte n’a été lu. L’état de connexion actuel du compte '
        'reste inconnu. Le corpus conserve un démarrage original sur écran de connexion, sans scénario '
        'authentifié observé. Sources : ' + link('evidence/goal-access-audit.json') + ', '
        + link('evidence/original-runtime-verification.json') + '.', '',
        'Le contrôle d’accès se rejoue explicitement avec `python3 tools/inspect-goal-access.py --capture`. '
        'Une nouvelle capture change les preuves et exige un nouvel audit ; la vérification ci-dessous '
        'ne lance pas cette commande.', '', '## Portée de la conclusion', '']
    lines += ['- ' + limitation for limitation in [
        'L’analyse ne prétend pas couvrir toutes les fonctions du paquet ou toutes les versions.',
        'Les six contradictions d’intégrité ASAR de chaque distribution restent conservées. Le helper Computer Use hors ASAR n’en fait pas partie.',
        'Le sous-ensemble arm64 .70 ne prouve pas l’équivalence arm64 .71.',
        'Les compteurs fictifs de Messages et les instants candidats cron ne sont pas des effets externes observés.',
        'La signature, le retour d’un outil et un état de carte ne prouvent pas la réussite métier ou la livraison.',
        'Une observation authentifiée peut préciser un scénario sans révéler le prompt ou l’algorithme privé.',
        'Les artefacts externes listés ci-dessus permettraient une nouvelle investigation. Ils ne sont pas remplacés par des suppositions.']]
    lines += ['', '## Vérification', '',
        '`python3 tools/verify-goal-audit.py` contrôle les identités, les empreintes des fichiers, les neuf rubriques, '
        'les sept correspondances et les classes de preuves. Il ne relance ni cible, ni tranches JavaScript, '
        'ni réseau, ni inspection de téléphone. Ce contrôle garantit la cohérence du dossier ; il ne '
        'vérifie pas les mécanismes serveur encore inconnus.', '']
    return '\n'.join(lines)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--build', action='store_true')
    parser.add_argument('--write-result', action='store_true')
    args = parser.parse_args()
    audit = json.loads(AUDIT.read_bytes())
    goal_source = json.loads((ROOT / 'evidence/goal-objective-source.json').read_bytes())
    assert audit['objective'] == goal_source['objective']
    assert audit['objective_sha256'] == hashlib.sha256(audit['objective'].encode()).hexdigest()
    assert audit['decision'] == 'research_requirements_satisfied_with_explicit_unknowns'
    assert len(audit['questions']) == 9 and {q['id'] for q in audit['questions']} == IDS
    assert len(audit['research_requirements']) == 7
    assert {c['id'] for c in audit['research_requirements']} == {
        'reuse', 'accessible', 'new_paths', 'preserve', 'reports', 'boundaries', 'inaccessible'}
    paths = {item['path'] for item in audit['artifacts']}
    assert len(paths) == len(audit['artifacts'])
    for item in audit['artifacts']:
        assert sha(ROOT / item['path']) == item['sha256'], item['path']
    for criterion in audit['research_requirements']:
        assert criterion['assessment'] == 'satisfied'
        assert criterion.get('support') or criterion.get('support_question_ids')
        for path in criterion.get('support', []):
            assert path in paths or path == 'evidence/remaining-evidence-progress.json'
        assert set(criterion.get('support_question_ids', [])) <= IDS
    for question in audit['questions']:
        assert question['knowledge_status'] in {'partial', 'unresolved'}
        assert question['unproven'] and question['required_external_evidence']
        assert question['reports'] and question['evidence']
        assert set(question['reports'] + question['evidence']) <= paths
    for key in ['question_knowledge_complete', 'product_parity_proven', 'server_implementation_recovered',
                'native_ios_implementation_recovered', 'authenticated_scenario_observed']:
        assert audit[key] is False
    manifest = json.loads((ROOT / 'artifacts/manifest.json').read_bytes())
    release = json.loads((ROOT / 'artifacts/releases/0.71.2/asar-inventory.json').read_bytes())
    assert len(manifest['entries']) == 673 and len(release['entries']) == 668
    assert sum(e.get('integrity') == 'observed-untrusted' for e in manifest['entries']) == 6
    assert len(release['integrity_contradictions']) == 6
    assert sha(ROOT / 'inputs/Grok Bot.app/Contents/Resources/app.asar') == manifest['source']['sha256']
    assert sha(ROOT / 'artifacts/releases/0.71.2/package/Contents/Resources/app.asar') == release['asar_sha256']
    startup = json.loads((ROOT / 'evidence/original-runtime-verification.json').read_bytes())
    assert startup['captured_scripts_verified'] == 45 and not startup['authenticated_scenario_observed']
    access = json.loads((ROOT / 'evidence/goal-access-audit.json').read_bytes())
    assert access['package_capture_scan']['matches'] == []
    assert access['package_capture_scan']['exit_code'] == 1
    assert access['iphone_usb_name_query']['matching_product_names'] == []
    assert access['iphone_usb_name_query']['serials_retained'] is False
    assert not any(v['available'] for v in access['ios_access_tools'].values())
    assert not any(access[k] for k in ['account_state_inspected', 'profiles_read', 'network_used', 'original_app_started'])
    review = json.loads((ROOT / audit['review_file']).read_bytes())
    assert review['kind'] == 'analyst-review' and not review['new_target_observations']
    assert {topic for reviewer in review['reviewers'] for topic in reviewer['topics']} == IDS
    expected_report = render(audit, access)
    if args.build:
        REPORT.write_text(expected_report)
    assert REPORT.read_text() == expected_report
    links = re.findall(r'\]\(([^)]+)\)', expected_report)
    for path in links:
        assert (REPORT.parent / path).is_file(), path
    result = {'schema': 1, 'status': 'passed',
              'checked_at_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
              'scope': 'Research dossier identity, topic coverage and explicit limits; not hidden product behavior',
              'objective_sha256': audit['objective_sha256'], 'questions': 9, 'research_requirements': 7,
              'pinned_artifacts': len(paths), 'report_links_verified': len(links),
              'report_sha256': sha(REPORT), 'audit_sha256': sha(AUDIT),
              'target_executed': False, 'network_used': False, 'server_internals_verified': False,
              'question_knowledge_complete': False, 'product_parity_proven': False}
    if args.write_result:
        (ROOT / 'evidence/goal-audit-verification.json').write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n')
    print(json.dumps(result, ensure_ascii=False))


if __name__ == '__main__':
    main()
