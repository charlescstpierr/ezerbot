# Les neuf sujets importants — résultats de l’approfondissement

Les neuf sujets ont été investigués dans Grok Bot 0.70.2. Cette passe conserve 68 constats et 255 extraits hashés dans quatre rapports, avec huit nouveaux corps de fonctions natives désassemblés. Elle retrace les chemins client accessibles et nomme les règles encore inconnues côté serveur.

Cette enquête utilise la copie sauvegardée et les preuves REA antérieures. Aucun compte, modèle ou action réelle de Grok n’a été exécuté. Les citations vérifient les artefacts livrés. « Chemin actif » signifie relié statiquement à un constructeur ou handler; cela ne signifie pas exécuté pendant cette session.

## Ce qui change la compréhension du système

Une réponse réseau perdue peut suivre une action déjà exécutée. Le client sait reprendre certains envois, mais cette reprise ne garantit pas l’unicité de tous les effets externes. Les limites locales des outils ne sont pas un budget global d’autonomie. La boucle vocale est visible; le moteur texte qui choisit, replanifie et juge le travail reste distant.

Le rappel vocal sélectionne huit textes admissibles dans le transcript. Il ne constitue pas la mémoire durable. Reset conserve mémoire, fichiers et routines selon le message livré. Teach possède son propre cycle de sauvegarde; la fabrication puis la validation d’une skill ne sont pas récupérées dans le client.

Le helper Mac possède un bail de contrôle de session et un nettoyage lors de la disparition du titulaire. Les traces de transport relient les étapes et signalent certains écarts, mais leurs bornes et leur échantillonnage empêchent de les prendre pour un audit exhaustif.

## Réponses par sujet

### 1. Contexte et instructions

Le client compose un prompt vocal et filtre ce qu’il rappelle.

**Établi côté client.** La voie vocale package possède un registre d’outils réel, un rappel de huit textes et des filtres de session/auteur. Les fichiers du message partent comme références. Une priorité est formulée dans le prompt vocal.

**Encore inconnu.** Le prompt texte complet, la sélection des faits et skills, l’extraction des fichiers et l’ordre effectif des instructions restent distants.

Preuves et détails : [Contexte et instructions](goal-context-memory-learning.md). Statut de connaissance : `partially_answered`.

### 2. Mémoire et oubli

Recommencer une conversation conserve les ressources durables.

**Établi côté client.** Le client liste et promeut des faits par bot, conserve les résultats partiels et différencie la décision de carte de la mutation. Reset et oubli durable ont des portées différentes.

**Encore inconnu.** Extraction des faits, correction, suppression, propagation de l’oubli, rétention et droits privés/équipe ne sont pas entièrement établis.

Preuves et détails : [Mémoire et oubli](goal-context-memory-learning.md). Statut de connaissance : `partially_answered`.

### 3. Actions incertaines

Une réponse perdue peut laisser une action déjà exécutée.

**Établi côté client.** Les reprises d’envoi utilisent nonce et statut. Certains handlers MCP avertissent explicitement qu’un timeout peut arriver après l’effet externe. Les caches de déduplication locaux ont leurs propres portées.

**Encore inconnu.** Aucune garantie universelle d’exécution unique des effets shell, fichiers et connecteurs n’est prouvée. Les transactions serveur restent absentes.

Preuves et détails : [Actions incertaines](goal-execution-autonomy.md). Statut de connaissance : `partially_answered`.

### 4. Boucle outils et fin

La boucle vocale est livrée et reliée à ses consommateurs.

**Établi côté client.** Un appel de fonction passe au harness, son résultat retourne comme function_call_output, puis le client arbitre la réponse parlée. La fin d’un appel et la fin d’un travail sont des événements différents.

**Encore inconnu.** Choix des outils, replanification et critère de réussite métier du moteur texte restent inconnus.

Preuves et détails : [Boucle outils et fin](goal-execution-autonomy.md). Statut de connaissance : `partially_answered`.

### 5. Permissions et secrets

Les autorisations et les magasins de secrets suivent des routes distinctes.

**Établi côté client.** Les cartes relient demande, décision et règle persistée. Les échecs de sauvegarde peuvent retomber sur une autorisation unique. Les secrets de bot sont envoyés au backend; le magasin local chiffré est un autre circuit.

**Encore inconnu.** Révoquer une demande ne prouve pas l’arrêt d’un effet déjà lancé. La visibilité finale des secrets par le modèle n’est pas établie.

Preuves et détails : [Permissions et secrets](goal-permissions-secrets.md). Statut de connaissance : `partially_answered`.

### 6. Budgets et limites

Des délais et plafonds locaux existent pour les outils et le transport.

**Établi côté client.** Le MCP backend distingue un délai de 3 minutes et deux exceptions à 45 minutes. Le provider utilisateur borne ses requêtes actives et son backlog. Les tailles de sorties et transferts ont d’autres limites.

**Encore inconnu.** Tours, tokens, coût, durée totale d’un bot et concurrence des sous-agents distants ne se déduisent pas de ces plafonds locaux.

Preuves et détails : [Budgets et limites](goal-execution-autonomy.md). Statut de connaissance : `partially_answered`.

### 7. Ressources partagées

Le helper Mac lie le contrôle à une session et au processus titulaire.

**Établi côté client.** Le bail natif vérifie identité et activation, surveille la sortie du titulaire et passe les entrées maintenues au nettoyage. Le tunnel réseau négocie une possession avec une génération d’attache.

**Encore inconnu.** Le partage des fichiers, des profils navigateur et des écrans entre bots cloud reste incomplet.

Preuves et détails : [Ressources partagées](goal-resources-traceability.md). Statut de connaissance : `partially_answered`.

### 8. Démonstrations et skills

Teach, publication et synchronisation sont plusieurs mécanismes.

**Établi côté client.** Le client suit start, save, discard et réconciliation de la démonstration. La bibliothèque publie puis relit les workflows. Les fichiers optionnels de skill disparaissent dans un adapter protobuf retracé.

**Encore inconnu.** La capture, recording→skill, validation, historique de versions, rollback et sélection de la skill au tour restent serveur.

Preuves et détails : [Démonstrations et skills](goal-context-memory-learning.md). Statut de connaissance : `partially_answered`.

### 9. Traces et audit

Les traces relient des étapes, avec pertes et échantillonnage possibles.

**Établi côté client.** Nonce et traceparent relient envoi, ack et affichage. Le client compare deux sources de transcript et signale les corps introuvables. Certaines files de télémétrie sont bornées et peuvent abandonner des rapports.

**Encore inconnu.** Le contexte exact du modèle, la cause complète d’une décision et la rétention des traces serveur ne sont pas récupérés.

Preuves et détails : [Traces et audit](goal-resources-traceability.md). Statut de connaissance : `partially_answered`.

## Rapports et preuves

| Rapport | Constats | Extraits | Ledger |
|---|---:|---:|---|
| [Contexte, mémoire et apprentissage](../reports/goal-context-memory-learning.md) | 19 | 57 | [JSON](../evidence/goal-context-memory-learning.json) |
| [Exécution et autonomie](../reports/goal-execution-autonomy.md) | 19 | 75 | [JSON](../evidence/goal-execution-autonomy.json) |
| [Permissions et secrets](../reports/goal-permissions-secrets.md) | 14 | 79 | [JSON](../evidence/goal-permissions-secrets.json) |
| [Ressources et traçabilité](../reports/goal-resources-traceability.md) | 16 | 44 | [JSON](../evidence/goal-resources-traceability.json) |

[Vue navigable des neuf sujets](../.lavish/goal-neuf-sujets.html). La [passe précédente](investigation-continuation.md) conserve cronjobs, ordinateurs, compaction, délégation, runtimes et personnalité. Les résultats se complètent; la nouvelle passe ne remplace pas ces preuves.

## Reproduire la vérification

```sh
python3 tools/evidence_ledger.py evidence/goal-context-memory-learning.json evidence/goal-execution-autonomy.json evidence/goal-permissions-secrets.json evidence/goal-resources-traceability.json
python3 tools/verify-goal-investigation.py
python3 tools/verify-dossier.py
```

Le vérificateur de goal contrôle les quatre ledgers, leurs empreintes, les corps natifs, les offsets des champs et la couverture exacte des neuf sujets. Le vérificateur du dossier relit les 673 fichiers expédiés. Ces commandes ne testent pas un runtime authentifié.

## Limite de clôture

Le travail demandé est réalisé comme enquête sur les artefacts accessibles, avec rapports et preuves vérifiables. Il ne constitue pas une récupération des serveurs privés ni une preuve d’équivalence du produit Rust. L’algorithme réel de compaction, le moteur texte, les garanties de transaction et le chemin complet démonstration→skill restent explicitement ouverts.
