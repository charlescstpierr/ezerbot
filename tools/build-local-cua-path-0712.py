#!/usr/bin/env python3
"""Reproduce a static, version-scoped ledger. Never evaluate target JavaScript."""
from pathlib import Path
import argparse
import json
from versioned_evidence_ledger import Ledger, ROOT, sha, verify

REPORT = 'reports/local-cua-path-0712.md'
PROOF = 'evidence/local-cua-path-0712.json'
D = 'dist/local-exec-daemon/main.cjs'
A = 'dist/electron-main/main-app.cjs'
C = 'dist/electron-main/main-core.cjs'


def searches():
    records = []
    for version, manifest, base in [
        ('0.70.2', 'artifacts/manifest.json', 'artifacts/client'),
        ('0.71.2', 'artifacts/releases/0.71.2/asar-inventory.json',
         'artifacts/releases/0.71.2/client'),
    ]:
        entries = json.loads((ROOT / manifest).read_text())['entries']
        files = [e for e in entries if e['kind'] == 'file' and
                 Path(e['path']).suffix in {'.js', '.cjs', '.mjs'}]
        bodies = {}
        for entry in files:
            raw = (ROOT / base / entry['path']).read_bytes()
            assert sha(raw) == entry['sha256']
            bodies[entry['path']] = raw
        for literal in ['--mcp-stdio', 'local-computer-use-gate.json',
                        'sand_enable_local_cua', '"local-tool"']:
            needle = literal.encode()
            hits = []
            for path, raw in bodies.items():
                offsets, start = [], 0
                while (offset := raw.find(needle, start)) >= 0:
                    offsets.append(offset)
                    start = offset + len(needle)
                if offsets:
                    hits.append({'path': path, 'sha256': sha(raw),
                                 'utf8_byte_offsets_zero_based': offsets})
            records.append({'version': version, 'manifest': manifest,
                            'manifest_sha256': sha((ROOT / manifest).read_bytes()),
                            'scanned_javascript_files': len(files), 'literal': literal,
                            'hits': hits, 'scope': 'Exact UTF-8 byte search only; no semantic absence claim'})
    return records


def build():
    ledger = Ledger('0.71.2', REPORT,
                    'Composition active du contrôle graphique local, côté JavaScript Mac 0.71.2. Analyse statique uniquement.')

    def finding(ident, claim, refs, limit):
        anchors = [ledger.excerpt(*ref) for ref in refs]
        ledger.finding(ident, claim, anchors, limit)

    finding('LCP-01',
            'Le daemon compose réellement un broker sN partagé et le fournit aux exécuteurs B4 des providers Gj de comptes. Ce chemin n’est pas déduit de la seule présence de symboles natifs.',
            [(D, 157670, 157716, 'computerUse: se'), (D, 157730, 157735, 'Q.start()'),
             (D, 128135, 128163, 'this.handleLocalTool(e, n, r)'),
             (D, 157823, 157834, 'se.close()')],
            'Composition statique .71; activation effective sur un compte et comportement après connexion non observés. Ne rétroattribue pas ce montage à .70.')
    finding('LCP-02',
            'Le main publie la valeur du flag sand_enable_local_cua dans un marqueur local. Le daemon n’annonce le helper que si sa lecture vaut exactement true. La valeur compilée par défaut est false.',
            [(A, 56364, 56367, 'sand_enable_local_cua'),
             (A, 103187, 103202, 'writeLocalComputerUseGateSync'),
             (C, 32569, 32573, 'local-computer-use-gate.json'),
             (C, 32785, 32787, 'mode: 384'),
             (D, 78560, 78564, 'local-computer-use-gate.json'),
             (D, 79524, 79526, 'us(t), h6e'),
             (D, 79601, 79603, '?.isOn === !0'),
             (D, 128529, 128531, 'this.availableHelper()'),
             (D, 128590, 128592, 'this.options.readGate()')],
            'Le défaut compilé ne donne pas la configuration distante du compte. Lire false empêche une nouvelle admission par ce broker; aucun arrêt immédiat des appels admis n’est montré par ce seul test.')
    finding('LCP-03',
            'Le broker annonce 14 outils graphiques, dont capture, clic, frappe, applications et actions d’accessibilité. Le hello du provider Mac les publie sous mobileLocalTools et annonce le consentement par app.',
            [(D, 82831, 82862, 'computer_app_action'),
             (D, 128135, 128137, 'announcedTools'),
             (D, 156683, 156713, 'mobileLocalTools')],
            'Le nom du champ partagé mobileLocalTools ne prouve aucun exécuteur iPhone. Cette composition est celle du daemon Mac.')
    finding('LCP-04',
            'Le provider surveille puis récupère les demandes du service avec machineId et credential. Le décodeur localTool transporte toolId, inputJson, conversationId et approvalId vers B4, puis les résultats repartent sous les frames locales dédiées.',
            [(D, 156895, 156922, 'watchGrokBotUserComputerRequests'),
             (D, 156985, 157013, 'pollGrokBotUserComputerRequests'),
             (D, 157080, 157110, '.handle(e, (n) => this.enqueue(n))'),
             (D, 157272, 157282, 'case "localTool"'),
             (D, 157126, 157145, 'submitGrokBotUserComputerResponses'),
             (D, 157382, 157415, 'localToolResult')],
            'Ce sont des corps client et leur branchement. Ils ne donnent ni le choix d’outil par le modèle, ni la décision serveur, ni une trace réseau authentifiée.')
    finding('LCP-05',
            'B4 émet accepted avant l’appel du broker, puis permission-pending, une nouvelle acceptation après permission ou pendant la file, et enfin le résultat. Sa décision use-app force authorization:none et décrit le bundle destinataire.',
            [(D, 128265, 128300, 'authorization: { kind: "none" }')],
            'Accepted accuse réception côté client, pas la réussite de l’action. La politique détaillée et les scopes de grants sont traités dans le rapport de consentement séparé.')
    finding('LCP-06',
            'Les appels sont mis en file par conversationId. Le broker réutilise une session enfant par conversation et un compteur de leases maintient son processus pendant les appels. Une libération programme son arrêt après dix minutes d’inactivité.',
            [(D, 128500, 128504, '10 * 6e4'),
             (D, 128532, 128566, 'this.callLines.get(e)'),
             (D, 128568, 128588, 's.release()'),
             (D, 128593, 128605, 'conversationId: e'),
             (D, 128806, 128829, 'this.options.clock.schedule(gsr')],
            'La clé de file n’est pas globale au bureau ni isolée par compte dans ce broker partagé. Le compteur JS de leases ne constitue pas le permis natif de contrôle. L’admission concurrente du companion doit être examinée séparément.')
    finding('LCP-07',
            'Le broker prépare le helper embarqué, lance son exécutable avec --mcp-stdio et --conversation-id, puis utilise JSON-RPC ligne par ligne sur stdin/stdout. Il initialise MCP avec la version 2025-06-18 avant notifications/initialized.',
            [(D, 128607, 128618, 'command: await Iee'),
             (D, 128505, 128505, '2025-06-18'),
             (D, 128858, 128915, '"--mcp-stdio"'),
             (D, 128917, 128928, 'this.pending.delete(e)')],
            'Le stdio décrit seulement la frontière daemon→façade native. Le binaire utilise ensuite CUServiceConnector et CULocalRPCClient vers le companion; le socket interne ne disparaît pas. Le mode MCP existe déjà parmi les symboles du helper .70.')
    finding('LCP-08',
            'Le consentement d’app précède le premier tools/call. Si le résultat signale permission_required, le broker attend les permissions macOS par sondage d’une seconde, pendant cinq minutes au maximum, puis réessaie une fois après vérification de la gate et du signal d’annulation.',
            [(D, 128568, 128588, 'decideAppUse: r.decideAppUse'),
             (D, 128500, 128504, '_sr = 5 * 6e4'),
             (D, 128620, 128648, 'computer_check_permissions'),
             (D, 128728, 128741, 'permission_required')],
            'Le retry interne ne repasse pas par qAe/decideAppUse. Une révocation de grant ultérieure ne signifie donc pas une interruption rétroactive de cet appel déjà admis. Aucun prompt macOS n’a été déclenché dans cette analyse.')
    finding('LCP-09',
            'Chaque requête MCP porte un délai de deux minutes. Son expiration règle la promesse puis arrête l’enfant par SIGTERM et termine les autres requêtes en attente. La file émet un keepalive chaque minute lorsqu’elle attend.',
            [(D, 128500, 128505, 'ysr = 6e4'),
             (D, 128514, 128522, 'intervalMs: ysr'),
             (D, 128545, 128560, 'r.onStillQueued()'),
             (D, 128830, 128856, 'kind: "timed-out"'),
             (D, 128923, 128928, 'kind: "exited"')],
            'Les délais ne sont pas une durée maximale garantie de l’effet OS. L’attente des permissions est une boucle distincte de plusieurs requêtes MCP. Aucun scénario de latence ou de crash n’est exécuté.')
    finding('LCP-10',
            'L’annulation B4 déclenche AbortController. iN annule la requête locale; la session règle sa promesse puis envoie notifications/cancelled. Une annulation simple n’appelle pas directement kill. Le texte client reconnaît que l’action a pu avoir lieu.',
            [(D, 128265, 128269, 'r.cancel = () => s.abort()'),
             (D, 128508, 128512, 'the action may have run'),
             (D, 128743, 128755, 't.cancel(i)'),
             (D, 128847, 128857, 'notifications/cancelled')],
            'Notification d’annulation, arrêt de processus, relâchement des entrées et annulation d’un effet déjà livré sont des opérations différentes. Ce chemin ne promet pas de rollback ni une action unique après redémarrage.')
    finding('LCP-11',
            'Le statut externe done sérialise le résultat MCP. Hormis le cas spécial permission_required, runUntilGranted peut produire done avec un objet interne isError:true. Done ne signifie donc pas réussite de l’action ni fin du travail du bot.',
            [(D, 128620, 128634, '!ksr(i.result)'),
             (D, 128709, 128711, 'status: "done"'),
             (D, 128728, 128738, 't.isError !== !0')],
            'Le consommateur serveur peut interpréter cette erreur interne, mais son handler n’est pas présent. La fin de tâche choisie par le modèle reste inconnue.')

    intro = '''# Contrôle graphique local dans Grok Bot 0.71.2

Le paquet 0.71.2 contient un chemin JavaScript effectivement composé pour faire agir le Mac. Le daemon annonce des outils graphiques au service, reçoit une demande `localTool`, contrôle l’app ciblée, puis lance une façade MCP native par conversation. Cette façade rejoint ensuite le companion par son RPC interne. La présence d’un mode MCP dans le binaire 0.70.2 ne suffisait pas à établir cette composition.

```text
Service distant
  → watch / poll authentifiés du provider Mac
  → décodeur localTool → exécuteur B4
  → broker partagé, gate + consentement d’app, file par conversation
  → enfant CUGrokBotService --mcp-stdio --conversation-id ...
  → CUServiceConnector / CULocalRPCClient → companion natif
  → session de contrôle et actions natives autorisées
```

Ce rapport analyse les sources distribuées, sans lancer la version 0.71.2, se connecter à un compte ou effectuer d’action sur le bureau. Il ne fournit pas la boucle serveur qui choisit les outils. Le mot « mobile » dans `mobileLocalTools` est le nom d’un champ de protocole employé ici par le daemon Mac.

Les faits suivants sont liés aux extraits LF de copies formatées séparément. Le [ledger](../evidence/local-cua-path-0712.json) lie aussi les sources originales à leurs empreintes dans le manifeste du nouvel ASAR. Les résultats .71 restent distincts de ceux de l’original .70.

'''
    sections = [intro]
    for finding_ in ledger.findings:
        sections.append(f"## {finding_['id']}\n\n{finding_['claim_fr']}\n\n{finding_['limitation_fr']}\n\n")
        refs = []
        for ident in finding_['anchors']:
            anchor = next(a for a in ledger.anchors if a['id'] == ident)
            source = ledger.sources[anchor['source']]
            refs.append(f"[{ident}, {Path(anchor['source']).parent.name}:{anchor['start_line']}](../{source['formatted_path']}:{anchor['start_line']})")
        sections.append('Preuves : ' + ', '.join(refs) + '.\n\n')
    sections.append('''## Portée de la comparaison et suite

Le vérificateur rejoue quatre recherches de chaînes exactes dans les fichiers JavaScript de chacun des deux manifestes. `--mcp-stdio`, `local-computer-use-gate.json`, `sand_enable_local_cua` et la chaîne JSON `"local-tool"` sont absents des 600 fichiers .70 recherchés et présents parmi les 596 fichiers .71. Les offsets sont des positions d’octets UTF-8 à partir de zéro. Une absence de chaîne ne prouve pas une absence sémantique; des noms, routes ou encodages différents restent possibles. Le [premier rapport .71](release-0712-discovery.md) portait sur d’autres chemins et ne prétendait pas comparer tout le paquet.

Le [rapport de consentement .71](local-cua-consent-0712.md) précise les grants par compte, action et bundle ainsi que la portée du retrait d’une app. Le [contrôle arm64 de l’original .70](native-arm64-residual.md) confirme seulement neuf corps et six propriétés sélectionnées; il ne constitue pas une équivalence complète de toutes les architectures ou versions.

Le [rapport natif MCP .71](native-0712-mcp.md) établit les contrôles du père, le mapping des noms d’outils, le raccord au socket, les identités distinctes et la libération coopérative après EOF/SIGTERM. Il ne prouve ni unicité du companion en exécution, ni livraison de tous les événements graphiques.

Reproduire les vérifications avec `python3 tools/build-local-cua-path-0712.py --verify`. Sans option, ce script régénère seulement le rapport et son ledger à partir des artefacts sauvegardés. Il ne lance aucun code de la cible et n’utilise pas le réseau. Les questions serveur, iOS, APNs et de garanties de reprise restent ouvertes.
''')
    (ROOT / REPORT).write_text(''.join(sections))
    ledger.write(PROOF, ['Version .71 isolée', 'Sources originales hashées',
                        'Aucun code cible exécuté', 'Annulation distincte de rollback',
                        'Façade MCP distincte du companion RPC'], searches=searches())


if __name__ == '__main__':
    args = argparse.ArgumentParser(description=__doc__)
    args.add_argument('--verify', action='store_true')
    options = args.parse_args()
    if not options.verify:
        build()
    result = verify(PROOF)
    saved = json.loads((ROOT / PROOF).read_text())
    assert saved['bounded_searches'] == searches()
    result['exact_searches_verified'] = len(saved['bounded_searches'])
    print(json.dumps(result, ensure_ascii=False))
